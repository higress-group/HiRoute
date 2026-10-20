"""Recover an Agent settings publication interrupted before its service receipt."""
import json
import sqlite3
import sys
import time

from publication_process import (
    bootstrap,
    model_settings_spec_v2,
    plan_change,
    prepare_agent_settings_v2,
)
from publication_product import Product
from publication_requests import request
from released_grant_upgrade import sessions


def interrupted_operation(product, key):
    # Observe production journal writes; never seed or modify business data.
    with sqlite3.connect(f'file:{product.storage}/live/control.db?mode=rw', uri=True) as db:
        db.execute('PRAGMA query_only=ON')
        found = [json.loads(row[0]) for row in db.execute(
            'SELECT operation_json FROM operations')]
    found = [operation for operation in found if operation['idempotency']['key'] == key]
    assert len(found) == 1, 'interrupted settings transaction must remain unique'
    return found[0]


def run(repository, boundary):
    assert boundary in ('after_target', 'after_active')
    product = Product(repository)
    try:
        product.enable_debug_diagnostics()
        product.enable_cpa()
        # Save to HiRoute's separate Codex profile so the user's original native files
        # must remain byte-identical even as the accepted managed selection changes.
        product.codex_access_mode = 'profile'
        bootstrap(product)
        native_before = {path: path.read_bytes()
                         for path in (product.codex_settings, product.settings)}
        assert product.agent_settings_path != product.codex_settings
        change = plan_change(product, 'create', 'recovery-plan', display_name='Recovery plan')
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change},
                      'recovery-plan')
        other = preview['plan_head']['reference']['plan_id']
        alias = preview['plan_head']['model_alias']
        spec = model_settings_spec_v2(product, [product.plan_id, other], other)
        product.stop()
        product.start(boundary)
        accepted, body, capability = prepare_agent_settings_v2(product, spec, 'interrupted-agent')
        status, result = product.cli('agents connect apply', body, capability, success=False)
        assert status != 0, result
        product.stop(crash=True)
        interrupted = interrupted_operation(product, 'interrupted-agent')
        assert interrupted['state'] == 'activating', interrupted['state']
        activate = next(step for step in interrupted['steps'] if step['kind'] == 'activate')
        assert activate.get('terminal_result') is None, 'fault must precede the service receipt'

        # Startup itself must complete the journal before any client replays Apply.
        product.start()
        recovered = product.cli('operations get ' + interrupted['operation_id'])[1]['data']
        assert recovered['state'] == 'succeeded', recovered
        _, replay = product.cli('agents connect apply', body, capability)
        assert replay['data']['operation_id'] == interrupted['operation_id'], replay
        assert replay['data']['state'] == 'succeeded', replay
        selection = product.cli('agents connect status ' + product.agent_context_id)[1]['data']
        assert selection['current_selection'] == accepted['spec']['model']['settings'], selection
        for path, original in native_before.items():
            assert path.read_bytes() == original, 'recovery changed a user native file'
        managed_after = product.agent_settings_path.read_bytes()
        for restart in range(2):
            status, response = request(product, alias, f'recovered settings {boundary} {restart}')
            assert status == 200 and b'fixture answer' in response, (status, response)
            deadline = time.monotonic() + 10
            while True:
                observed = sessions(product)
                if len(observed) == restart + 1:
                    break
                assert time.monotonic() < deadline, observed
                time.sleep(.1)
            assert all(row['agent_id'] == 'agent_codex_default' for row in observed), observed
            product.stop()
            if restart == 0:
                product.start()
                assert product.agent_settings_path.read_bytes() == managed_after
                for path, original in native_before.items():
                    assert path.read_bytes() == original
        print(json.dumps({
            'scenario': 'agent-settings-publication-recovery', 'boundary': boundary,
            'state': 'green', 'daemon_fault_exit': 86, 'operation_state': 'succeeded',
            'current_attributed_sessions': 2, 'native_user_files_unchanged': True,
            'recovered_profile_stable': True,
        }), flush=True)
    finally:
        product.close()


if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
