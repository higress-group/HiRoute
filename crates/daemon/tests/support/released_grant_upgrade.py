"""v0.3.1 package -> current startup and managed Agent attribution, without DB writes.

Run with the repository and verified Linux x86_64 release archive as arguments.
Current binaries come from HIROUTE_VALIDATION_PRODUCT_BIN_DIR or target/debug.
Native probes and the external upstream use the existing bounded fixtures; all
connection transactions, grant issuance, upgrade and requests use real binaries.
"""
import hashlib
import ctypes
import http.client
import json
import os
import signal
import sqlite3
import threading
from types import SimpleNamespace
from pathlib import Path
import subprocess
import sys
import tarfile
import time

from publication_product import Product, encoded
from publication_process import (bootstrap, configure_model_settings_v2, prepare_discovered_source, save_compute_candidate)
from model_connections_product import save_native_source
from publication_requests import request

RELEASE_REVISION = "13856abd873da5b4373d39322493b4cc105c4867"
PACKAGE_SHA256 = "dfa35ada12fcef9d0d2e10e680422cd0bf6a0454a8809c27879b74ba932d21aa"


def message(product, token, prompt):
    client = http.client.HTTPConnection("127.0.0.1", product.port, timeout=45)
    try:
        client.request("POST", "/v1/messages", body=encoded({
            "model": product.model_alias, "max_tokens": 16,
            "messages": [{"role": "user", "content": prompt}],
        }), headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
        response = client.getresponse()
        return response.status, response.read()
    finally:
        client.close()


def sessions(product):
    deadline = time.monotonic() + 10
    while True:
        _, result = product.public_cli("sessions list --include-unlinked --limit 50", success=False)
        if result["status"] == "succeeded":
            return result["data"]["sessions"]
        assert result["status"] == "unavailable" and time.monotonic() < deadline, result
        time.sleep(.1)


def upstream_fixture(product, contents):
    binary = product.cpa_fixture / "cpa_upstream.py"
    binary.write_bytes(contents)
    product.cpa_args = [
        "--cpa-binary", str(binary), "--cpa-sha256", hashlib.sha256(contents).hexdigest(),
    ]


def source_rows(product):
    path = product.storage / 'live/control.db'
    with sqlite3.connect(path.as_uri() + '?mode=ro', uri=True) as connection:
        return dict(connection.execute('SELECT source_id,source_json FROM compute_management_sources'))


def released_fixture_children(product):
    """Capture only this daemon's counted external fixture before interrupting it."""
    binary = product.cpa_fixture / 'cpa_upstream.py'
    expected = hashlib.sha256(binary.read_bytes()).hexdigest()
    children = []
    for path in (product.storage / 'cpa/state').glob('*/owner.lock/owner.json'):
        owner = json.loads(path.read_text())
        assert owner['owner_pid'] == product.process.pid
        assert owner['binary_sha256_hex'] == expected
        pid = owner['cpa_pid']
        assert str(binary).encode() in Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
        children.append(pid)
    return children


def stop_adopted_fixtures(children):
    """Model retiring the old external component before installing a different version.

    waitpid proves these are our adopted children and pins their identity before signaling.
    Never remove ownership records or edit the released operation/source data.
    """
    for pid in children:
        completed, _ = os.waitpid(pid, os.WNOHANG)
        if completed:
            continue
        os.kill(pid, signal.SIGTERM)
        deadline = time.monotonic() + 5
        while not os.waitpid(pid, os.WNOHANG)[0]:
            assert time.monotonic() < deadline, 'owned external fixture did not stop'
            time.sleep(.01)


def park_released_source_edit(product, source_id):
    """Stop the real released process only after observing its committed unfinished journal.

    SQLite is read-only. No hook, current serializer or database repair creates old inputs.
    A fast completed edit is retained and the next ordinary rename uses fresh revisions.
    """
    terminal = {'succeeded', 'failed', 'rolled_back', 'cancelled'}
    for attempt in range(5):
        snapshot = product.control('ListCompute', {})['data']
        source = next(row for row in snapshot['sources'] if row['source_id'] == source_id)
        change = {'schema':'hiroute.compute-management-change/v2',
            'subject':{'kind':'saved_source','source_id':source_id},
            'expected_revisions':snapshot['revisions'],'selected_model_refs':[],
            'intent':'save_ready','key_edits':[],
            'edit':{'action':'rename','display_name':'Released source recovery ' + str(attempt)}}
        preview = product.control('PreviewComputeSave', {'change':change})['data']
        key = 'released-source-interruption-' + str(attempt)
        request = {'spec':preview['spec'],'accept_digest':preview['accept_digest'],
            'expected_revisions':preview['expected_revisions'],'idempotency_key':key}
        outcomes = []
        def apply():
            try:
                outcomes.append(product.control('ApplyComputeSave', request))
            except (OSError, ValueError, AssertionError) as error:
                outcomes.append(type(error).__name__)
        writer = threading.Thread(target=apply, daemon=True)
        with sqlite3.connect((product.storage / 'live/control.db').as_uri() + '?mode=ro', uri=True, timeout=1) as connection:
            writer.start()
            deadline = time.monotonic() + 5
            parked = None
            while writer.is_alive() and time.monotonic() < deadline:
                row = connection.execute('SELECT operation_id,state,operation_json FROM operations WHERE idempotency_key=?',(key,)).fetchone()
                if row and row[1] not in terminal:
                    product.process.send_signal(signal.SIGSTOP)
                    row = connection.execute('SELECT operation_id,state,operation_json FROM operations WHERE idempotency_key=?',(key,)).fetchone()
                    if row and row[1] not in terminal:
                        parked = row
                        break
                    product.process.send_signal(signal.SIGCONT)
                time.sleep(.0001)
            if parked:
                operation = json.loads(parked[2])
                mutation = operation['plan']['control']['compute_management_mutation']
                assert mutation['schema'] == 'hiroute.compute-management-mutation/v2', mutation['schema']
                children = released_fixture_children(product)
                product.process.kill()
                product.stop(diagnostic_failure=True)
                writer.join(timeout=16)
                assert not writer.is_alive(), 'old apply connection did not terminate'
                stop_adopted_fixtures(children)
                return {'operation_id':parked[0],'observed_state':parked[1],
                    'plan_digest':hashlib.sha256(encoded(operation['plan'])).hexdigest(),
                    'source_id':source_id,'desired':mutation['desired'],
                    'retired_external_fixture_children':len(children)}
        writer.join(timeout=16)
        assert not writer.is_alive(), 'ordinary released edit did not finish'
        assert outcomes and isinstance(outcomes[0],dict), outcomes
    raise AssertionError('could not observe an unfinished released source edit; no upgrade claim')


def run(repository, archive):
    assert sys.platform.startswith('linux'), 'this exact package acceptance runs on Linux'
    # The isolated harness adopts its interrupted daemon's children so a counted external
    # component can be stopped and reaped before switching its authenticated version.
    libc = ctypes.CDLL(None, use_errno=True)
    assert libc.prctl(36, 1, 0, 0, 0) == 0, 'cannot enable fixture child reaping'
    archive = Path(archive).resolve()
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == PACKAGE_SHA256, "wrong released package"
    candidate = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
    product = Product(repository)
    current_bin = product.bin
    try:
        released = product.root / "released"
        released.mkdir(mode=0o700)
        # Only extract the authenticated archive. Do not overwrite caller-owned paths.
        with tarfile.open(archive) as package:
            for member in package.getmembers():
                assert not Path(member.name).is_absolute() and ".." not in Path(member.name).parts
                assert member.isfile() or member.isdir(), "unexpected release archive member"
            package.extractall(released)
        product.bin = released / "bin"
        product.enable_cpa()
        # Each daemon pins its own external-component version. Use that release's
        # exact fixture before upgrade, then the candidate fixture after replacing
        # the daemon; do not weaken either component handshake.
        upstream_fixture(product, subprocess.check_output([
            "git", "show", RELEASE_REVISION + ":crates/daemon/tests/support/cpa_upstream.py",
        ], cwd=repository))
        bootstrap(product)
        # The released producer creates registered, manual and unknown-provenance source facts.
        registered_candidate, validation = prepare_discovered_source(product)
        registered = save_compute_candidate(product, registered_candidate, 'released-registered', validation)
        manual = save_native_source(product, SimpleNamespace(base_url='https://gateway.invalid/v1'),
            token='not-a-live-api-key', variant='released-manual', context_tokens=23456,
            inventory_path=None, cli_input=True, source_display_name='Released manual source')
        unknown = save_native_source(product, SimpleNamespace(base_url='https://gateway.invalid/v1'),
            token='not-a-live-api-key', variant='released-unknown', unknown=True,
            inventory_path=None, cli_input=True)
        connections = {"agent_codex_default": product.agent_connection}
        product.agent_context_id = None
        configure_model_settings_v2(
            product, [product.plan_id], "released-claude", agent_id="agent_claude_default")
        connections["agent_claude_default"] = product.agent_connection
        tokens = {agent: product.bearer(connection) for agent, connection in connections.items()}
        for agent, token in tokens.items():
            send = request if agent == "agent_codex_default" else None
            status, body = (send(product, product.model_alias, "before upgrade " + agent, token)
                            if send else message(product, token, "before upgrade " + agent))
            assert status == 200 and b"fixture answer" in body, (agent, status, body)
        assert len(sessions(product)) == 2, "released requests did not persist"
        native_before = {path: path.read_bytes() for path in
                         (product.settings, product.codex_settings)}
        sources_before = source_rows(product)
        pending = park_released_source_edit(product, manual['source_id'])
        expected_sources = {source_id:json.loads(raw) for source_id,raw in sources_before.items()}
        expected_sources[manual['source_id']] = pending['desired']
        product.bin = current_bin
        upstream_fixture(product, (
            Path(repository) / "crates/daemon/tests/support/cpa_upstream.py"
        ).read_bytes())
        product.env["HIROUTE_RECOVERY_PACKAGE"] = str(released)
        for restart in range(2):
            product.start()
            actual_sources = {source_id:json.loads(raw) for source_id,raw in source_rows(product).items()}
            assert actual_sources == expected_sources, 'upgrade changed source facts or replayed the old edit twice'
            operation = product.control('GetOperation', {'operation_id':pending['operation_id']})['data']
            assert operation['state'] == 'succeeded', operation
            for saved in [manual, unknown]:
                defaults = product.control('GetComputeDefaults', {'source_id':saved['source_id']})['data']
                fields = next(model for model in defaults['models'] if model['binding_id'] == saved['binding_id'])['fields']
                assert all(field['intent'] == 'preserve_saved' and field['effective_value'] == field['saved_value'] for field in fields.values()), fields
            # Companion policy migration cannot rewrite supported released source bytes or
            # authorize an inherited environment. Reads perform no provider request.
            for saved in [manual, unknown]:
                network = product.public_cli('compute network show', {'source_id': saved['source_id']})[1]['data']
                assert network['policy'] == {'mode': 'direct'} and network['network_revision'] == 1, network
                assert network['application_state'] == 'applied' and network['applied_revision'] == 1, network
            assert source_rows(product)[unknown['source_id']] == sources_before[unknown['source_id']], 'policy conversion rewrote released source JSON'
            assert expected_sources[manual['source_id']]['models'][0]['capabilities']['context_tokens']['value'] == 23456
            for path, original in native_before.items():
                assert path.read_bytes() == original, "startup rewrote native settings"
            for agent, old_token in tokens.items():
                assert product.bearer(connections[agent]) == old_token, "startup rotated a grant"
                prompt = f"after upgrade {restart} {agent}"
                status, body = (request(product, product.model_alias, prompt, old_token)
                                if agent == "agent_codex_default"
                                else message(product, old_token, prompt))
                assert status == 200 and b"fixture answer" in body, (agent, status, body)
            deadline = time.monotonic() + 10
            while True:
                observed = sessions(product)
                if len(observed) == 2 * (restart + 1):
                    break
                assert time.monotonic() < deadline, observed
                time.sleep(.1)
            assert {row["agent_id"] for row in observed} == set(connections), observed
            assert all(row["agent_id"] for row in observed), observed
            product.stop()
        print(json.dumps({
            "scenario": "released-connected-agent-attribution-upgrade", "state": "green",
            "source_revision": RELEASE_REVISION, "package_sha256": PACKAGE_SHA256,
            "candidate": candidate, "released_sessions": 2, "current_attributed_sessions": 4,
            "agents": sorted(connections), "current_startups": 2,
            "native_settings_and_grants_unchanged": True,
            "native_policy_direct_and_source_bytes_unchanged": True,
            "released_pending_state": pending['observed_state'],
            "retired_external_fixture_children": pending['retired_external_fixture_children'],
        }), flush=True)
    finally:
        product.close()


if __name__ == "__main__":
    run(sys.argv[1], sys.argv[2])
