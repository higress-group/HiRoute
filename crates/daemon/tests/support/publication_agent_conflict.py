"""Crash a real Agent Apply, edit its native file, and park the lossless file tail."""
import json
import socket
import sqlite3
import sys
from publication_process import (
    bootstrap,
    model_settings_spec_v2,
    plan_change,
    prepare_agent_settings_v2,
)
from publication_product import Product


def run(repository):
    product = Product(repository)
    try:
        product.enable_cpa()
        bootstrap(product)
        change = plan_change(product, 'create', 'other', display_name='Other default')
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, 'other')
        other = preview['plan_head']['reference']['plan_id']
        spec = model_settings_spec_v2(product, [product.plan_id, other], other)
        product.stop()
        product.start('before_install')
        _preview, body, capability = prepare_agent_settings_v2(
            product, spec, 'interrupted-agent')
        status, _result = product.cli(
            'agents connect apply', body, capability, success=False)
        assert status != 0, 'crash must not be reported as success'
        product.stop(crash=True)
        settings = product.agent_settings_path
        independent = settings.read_bytes() + b'user_note = "preserve this independent edit"\n'
        settings.write_bytes(independent)
        product.start()
        assert settings.read_bytes() == independent, 'recovery overwrote user file'
        # Read-only evidence after the production recovery ran; never seed or mutate business rows.
        databases = list(product.storage.rglob('control.db'))
        assert len(databases) == 1
        # The intentional crash can leave committed WAL state that requires SQLite to recreate
        # lock sidecars before reading. Open only the existing database, then forbid SQL writes.
        with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=rw', uri=True) as db:
            db.execute('PRAGMA query_only=ON')
            operations = [json.loads(row[0]) for row in db.execute(
                'SELECT operation_json FROM operations')]
        interrupted = [op for op in operations if op['idempotency']['key'] == 'interrupted-agent']
        assert len(interrupted) == 1
        operation = interrupted[0]
        assert operation['state'] == 'activating', operation['state']
        assert operation['safe_error_code'] == 'EFFECT_OWNERSHIP_LOST'
        assert operation['steps'][-1]['status'] == 'started'
        receipt = next(step['terminal_result'] for step in operation['steps']
                       if step['kind'] == 'activate')
        assert json.loads(receipt)['schema'] == 'hiroute.settings-service-completion/v2'
        operation_generation = operation['generation']
        # The service segment is committed, so startup stays available; the independently edited
        # native file remains a parked tail and its unfinalized grant is not re-exported.
        with socket.socket() as sock:
            assert sock.connect_ex(('127.0.0.1', product.port)) == 0
        try:
            product.bearer(product.agent_connection)
            raise AssertionError('pending Agent grant became externally available')
        except AssertionError as error:
            assert str(error) == 'protected Agent grant unavailable'
        for cache_state in ('present', 'missing', 'corrupt'):
            product.stop()
            cache = product.root / 'gateway-lkg'
            if cache_state == 'missing':
                cache.unlink()
            elif cache_state == 'corrupt':
                cache.write_bytes(b'not authoritative publication data')
            product.start()
            assert settings.read_bytes() == independent
            with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=rw', uri=True) as db:
                db.execute('PRAGMA query_only=ON')
                replayed = [json.loads(row[0]) for row in db.execute(
                    'SELECT operation_json FROM operations')]
            replayed = [op for op in replayed if op['operation_id'] == operation['operation_id']]
            assert len(replayed) == 1
            assert replayed[0]['state'] == 'activating'
            assert replayed[0]['safe_error_code'] == 'SETTINGS_TAIL_PENDING'
            assert replayed[0]['generation'] > operation_generation
            assert replayed[0]['steps'] == operation['steps'], 'startup retried the native file tail'
            assert next(step['terminal_result'] for step in replayed[0]['steps']
                        if step['kind'] == 'activate') == receipt, 'startup discarded the committed receipt'
            operation_generation = replayed[0]['generation']
            with socket.socket() as sock:
                assert sock.connect_ex(('127.0.0.1', product.port)) == 0, 'service did not reopen'
            try:
                product.bearer(product.agent_connection)
                raise AssertionError('pending Agent grant became externally available')
            except AssertionError as error:
                assert str(error) == 'protected Agent grant unavailable'
        assert not (product.cpa_fixture / 'attempts.jsonl').exists(), 'recovery issued a model attempt'
        print(json.dumps({'scenario': 'agent-user-edit-recovery', 'state': 'green',
                          'operation_state': replayed[0]['state'],
                          'safe_error_code': replayed[0]['safe_error_code'],
                          'user_file_preserved': True, 'cold_restart_cache_cases': 3,
                          'service_receipt_preserved': True, 'upstream_attempts': 0}), flush=True)
    finally:
        product.close()


if __name__ == '__main__':
    run(sys.argv[1])
