"""Real role-all pause/drain/cancel, followed by authoritative cache rebuild and requests."""
import concurrent.futures
import json
import os
import select
import secrets
import subprocess
import sys
import time
from publication_product import Product, encoded
from publication_process import bootstrap
from publication_requests import request, attempts


def upgrade(product, action):
    registration = secrets.token_hex(32)
    os.write(product.cap_w, encoded({
        'schema': 'hiroute.launcher-upgrade/v1',
        'registration_id': registration, 'action': action,
    }) + b'\n')
    deadline = time.monotonic() + 5
    frame = b''
    while not frame.endswith(b'\n'):
        remaining = deadline - time.monotonic()
        assert remaining > 0 and select.select([product.cap_ack_r], [], [], remaining)[0], 'upgrade acknowledgement timeout'
        byte = os.read(product.cap_ack_r, 1)
        assert byte, 'upgrade channel closed'
        frame += byte
        assert len(frame) <= 4096
    result = json.loads(frame)
    assert set(result) == {'schema', 'registration_id', 'paused', 'active_calls', 'active_tasks'}
    assert result['schema'] == 'hiroute.launcher-upgrade-status/v1'
    assert result['registration_id'] == registration
    return result


def run(repository):
    candidate = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repository, text=True).strip()
    product = Product(repository)
    try:
        product.enable_debug_diagnostics()
        product.enable_cpa()
        bootstrap(product)
        token = product.bearer(product.agent_connection)
        alias = product.model_alias
        initial_native = product.codex_settings.read_bytes()
        catalog, _ = product.catalog()
        pid = product.process.pid
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            old = executor.submit(request, product, alias, 'hold-old-request', token)
            deadline = time.monotonic() + 10
            while not (product.cpa_fixture / 'held').exists():
                assert not old.done(), old.result()
                assert time.monotonic() < deadline
                time.sleep(.02)
            waiting = upgrade(product, 'prepare')
            assert waiting['paused'] and waiting['active_calls'] >= 1
            status, _ = request(product, alias, 'must wait', token)
            assert status == 503, status
            response = product.control('ListAgentPlans', {}, success=False)
            assert response['status'] == 'unavailable', response
            assert product.process.poll() is None and product.process.pid == pid
            assert attempts(product) == 1, 'paused call reached the upstream'
            resumed = upgrade(product, 'cancel')
            assert not resumed['paused'] and resumed['active_calls'] >= 1
            status, body = request(product, alias, 'cancel restores admission', token)
            assert status == 200 and b'fixture answer' in body
            (product.cpa_fixture / 'release').touch()
            status, body = old.result(timeout=15)
            assert status == 200 and b'fixture answer' in body, (status, body)
        drained = upgrade(product, 'prepare')
        assert drained['paused'] and drained['active_calls'] == 0 and drained['active_tasks'] == 0
        product.stop()
        assert product.process is None, 'exact daemon did not exit'
        cache = product.root / 'gateway-lkg'
        current = cache.read_bytes()
        assert cache.stat().st_mode & 0o777 == 0o600, 'new cache is not private'
        for state in ['corrupt', 'missing', 'stale']:
            if state == 'missing':
                cache.unlink()
            elif state == 'corrupt':
                cache.write_bytes(b'invalid cache; no business authority')
            else:
                value = json.loads(current)
                assert 'source_publication_digest' in value
                value['source_publication_digest'] = 'sha256:' + '0' * 64
                cache.write_bytes(encoded(value))
            product.start()
            rebuilt, _ = product.catalog()
            assert rebuilt == catalog, 'catalog authority changed with the cache'
            assert product.codex_settings.read_bytes() == initial_native, 'restart rewrote native configuration'
            status, body = request(product, alias, 'same grant after cache rebuild', token)
            assert status == 200 and b'fixture answer' in body
            assert cache.read_bytes() == current, 'authoritative cache was not rebuilt'
            product.stop()
        print(json.dumps({'scenario': 'upgrade-drain-and-cache-rebuild', 'state': 'green',
            'candidate': candidate, 'in_flight_request_completed': True,
            'cancel_restored_admission': True, 'replacement_waited_for_exact_exit': True,
            'cache_cases': 3, 'original_grant_and_native_files_retained': True}), flush=True)
    finally:
        if hasattr(product, 'cpa_fixture'):
            (product.cpa_fixture / 'release').touch()
        product.close()


if __name__ == '__main__':
    run(sys.argv[1])
