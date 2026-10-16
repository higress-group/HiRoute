"""Real managed Native source -> branch publication -> Gateway, including live permission edits."""
import http.server
import json
import sys
import threading
import time

from model_connections_product import (
    MODEL, NATIVE_TOKEN, NativeUpstream, assert_live_credential_switches,
    edit_saved_credentials, gateway_request,
    publish_plan_and_agent, save_native_source,
)
from publication_product import Product, encoded


class Decision(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        selected = body['decision']['options'][0]
        result = {'kind': 'categorical', 'choice': selected['id']}
        if selected.get('refinement') is not None:
            result['refinement'] = {'kind': 'ordinal', 'probabilities': {'simple': 1, 'complex': 0}}
        raw = encoded({'decision': result})
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)


def assert_public_receipts(product):
    """Use fresh same-UID reads after restart, without reusing a Desktop grant nonce."""
    deadline = time.monotonic() + 10
    while True:
        sessions = product.public_cli('sessions list --include-unlinked --limit 50')[1]['data']['sessions']
        receipts = {}
        for session in sessions:
            detail = product.public_cli('sessions show ' + session['session_id'])[1]['data']
            for turn in detail['turns']:
                for receipt_id in turn['receipt_ids']:
                    receipt = product.public_cli('sessions receipt ' + receipt_id)[1]['data']
                    if receipt['trust']['served_model_id'] == product.model_alias:
                        receipts[receipt_id] = receipt
        if len(receipts) == 9 and all(r['facts_completeness'] == 'complete' for r in receipts.values()):
            break
        assert time.monotonic() < deadline, list(receipts)
        time.sleep(.1)
    accepted = [r for r in receipts.values() if r['outcome'] == 'accepted']
    denied = [r for r in receipts.values() if r['outcome'] == 'failed']
    assert len(accepted) == 3 and len(denied) == 6, receipts
    bindings = set()
    for receipt in receipts.values():
        facts = [event['fact'] for event in receipt['ordered_facts']]
        leases = [fact for fact in facts if fact['kind'] == 'credential_lease']
        assert len(leases) == 1, leases
        lease = leases[0]
        assert lease['stable_binding_id'].startswith('candidate/sha256:'), lease
        bindings.add(lease['stable_binding_id'])
        assert lease['outcome'] == ('leased' if receipt['outcome'] == 'accepted' else 'source_disabled'), lease
        if receipt['outcome'] == 'failed':
            assert lease['key_id'] is None and lease['credential_generation'] is None
            assert not any(fact['kind'] == 'attempt_started' for fact in facts)
    assert len(bindings) == 1, 'credential facts lost the frozen branch identity'


def scenario(repository):
    product, upstream = Product(repository), NativeUpstream()
    decision = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Decision)
    worker = threading.Thread(target=decision.serve_forever, daemon=True)
    worker.start()
    product.install_codex_fixture(MODEL, 'medium', upstream.base_url, NATIVE_TOKEN)
    try:
        product.start()
        saved = save_native_source(product, upstream, NATIVE_TOKEN, cli_input=True)
        replacement = edit_saved_credentials(product, upstream, saved)
        product.install_codex_fixture(MODEL, 'medium', upstream.base_url, replacement)
        service = {'id': 'native-branch-decision', 'revision': 1, 'name': 'Branch decision',
                   'connection': {'kind': 'custom', 'timeout_ms': 3000, 'auth_header': None,
                                  'endpoint': f'http://127.0.0.1:{decision.server_port}/decisions'}}
        spec = {'schema_version': {'major': 1, 'minor': 0}, 'command_id': 'decision.services.apply',
                'resource_id': service['id'], 'desired_state': {
                    'id': service['id'], 'expected_revision': 0, 'service': service}}
        preview = product.public_cli('decision services apply', {
            'schema_version': {'major': 1, 'minor': 0}, 'spec': spec})[1]['data']
        applied = product.public_cli('decision services apply', {
            'schema_version': {'major': 1, 'minor': 0}, 'spec': preview['normalized_spec'],
            'accept_digest': preview['change_digest'], 'expected_revisions': preview['expected_revisions'],
            'idempotency_key': 'native-branch-service'})[1]['data']
        assert applied['state'] == 'succeeded', applied
        publish_plan_and_agent(product, saved['binding_id'], branch_service=service)
        gateway_request(product)
        assert_live_credential_switches(product, upstream, saved)
        assert_public_receipts(product)
        print(json.dumps({'state': 'green', 'scenario': 'native-branch-current-credentials',
                          'upstream_attempts': 3, 'disabled_requests_without_upstream': 6,
                          'restart_preserved_branch_and_permission': True}))
    finally:
        product.close()
        upstream.close()
        decision.shutdown()
        decision.server_close()
        worker.join(timeout=5)


if __name__ == '__main__':
    scenario(sys.argv[1])
