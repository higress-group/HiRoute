"""Versioned CLI and real Gateway against a local Decisions endpoint; no paid service."""
import hashlib
import copy
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import sys
import threading
import time

from publication_product import Product, SENTINEL, V1, encoded
from publication_process import bootstrap
from model_connections_product import (MODEL, NATIVE_TOKEN, NativeUpstream, gateway_request,
                                       publish_plan_and_agent, save_native_source)


class Endpoint(BaseHTTPRequestHandler):
    requests = []
    reject_input = False
    failure = None

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        authorization = self.headers.get('Authorization')
        self.requests.append((authorization, body))
        if self.reject_input:
            self.respond(400, {'error': {'message': 'request rejected'}})
            return
        if authorization == 'Bearer ' + SENTINEL:
            self.respond(401, {'error': {'message': 'fixture key rejected'}})
            return
        if self.failure is not None:
            status, code = self.failure
            self.respond(status, {'error': {'code': code, 'message': 'unpublished provider detail'}})
            return
        answers = []
        for question in body['questions']:
            if question['type'] == 'choice':
                values = [choice['value'] for choice in question['choices']]
                answers.append({'name': question['name'], 'type': 'choice', 'choice': values[0],
                    'probabilities': [{'value': value, 'probability': 1.0 if index == 0 else 0.0}
                                      for index, value in enumerate(values)]})
            else:
                levels = question['levels']
                answers.append({'name': question['name'], 'type': 'score', 'score': 0.2,
                    'probabilities': [{'value': index, 'label': level['label'],
                                       'probability': 0.8 if index == 0 else (0.2 if index == 1 else 0.0)}
                                      for index, level in enumerate(levels)]})
        self.respond(200, {'answers': answers})

    def respond(self, status, body):
        body = encoded(body)
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


def run(repository):
    product = Product(repository)
    product.collaboration_only = True
    endpoint = ThreadingHTTPServer(('127.0.0.1', 0), Endpoint)
    thread = threading.Thread(target=endpoint.serve_forever, daemon=True)
    thread.start()
    secondary = 'decision-fixture-secondary-token'
    product.secrets.add(secondary)
    model_upstream = None

    def data(command, payload=None):
        return product.public_cli(command, payload)[1]['data']

    def save_source(intent, key_edits, label):
        snapshot = product.control('ListCompute', {})['data']
        source = next(row for row in snapshot['sources'] if row['source_id'] == product.source_id)
        change = {'schema': 'hiroute.compute-management-change/v2',
            'subject': {'kind': 'saved_source', 'source_id': product.source_id},
            'expected_revisions': snapshot['revisions'],
            'selected_model_refs': [model['model_ref'] for model in source['models']],
            'intent': intent, 'key_edits': key_edits}
        preview = product.control('PreviewComputeSave', {'change': change})['data']
        applied = product.control('ApplyComputeSave', {'spec': preview['spec'],
            'accept_digest': preview['accept_digest'], 'expected_revisions': preview['expected_revisions'],
            'idempotency_key': label})
        assert applied['data']['state'] == 'succeeded', applied
        return next(row for row in product.control('ListCompute', {})['data']['sources']
                    if row['source_id'] == product.source_id)

    def mutate(change, key, slot=None):
        desired = {'change': change}
        if slot is not None:
            desired['input_slot'] = slot
        spec = {'schema_version': V1, 'command_id': 'decision.connections.apply',
                'resource_id': change['id'], 'desired_state': desired}
        preview = data('decision connections apply', {'schema_version': V1, 'spec': spec})
        result = data('decision connections apply', {'schema_version': V1,
            'spec': preview['normalized_spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': key})
        assert result['state'] == 'succeeded', result

    def current(id):
        return next(row for row in data('decision connections list')['services'] if row['service']['id'] == id)

    def test(view, digest=None):
        service = view['service']
        return data('decision connections test', {'id': service['id'], 'revision': service['revision'],
            'expected_configuration_digest': digest or view['configuration_digest']})

    try:
        bootstrap(product)
        options = data('decision connections options --provider openai')
        assert options['selected_provider']['wire_protocol'] == 'openai_decisions'
        assert options['selected_provider']['endpoint'] == 'https://api.openai.com/v1/decisions'
        candidate = {'candidate_ref': 'candidate/decision-second-key', 'candidate_revision': 1}
        product.register_protected_frame({'schema': 'hiroute.protected-input/v1',
            'registration_id': hashlib.sha256(b'decision-second-key').hexdigest(), **candidate, 'secret': secondary})
        source = save_source('save_ready', [{'action': 'add', 'input_candidate': candidate}], 'decision-add-key')
        assert len(source['keys']) == 2, source
        source_view = next(row for row in data('decision sources list')['sources'] if row['source_id'] == product.source_id)
        compatible = data('decision connections options --provider compatible')
        assert compatible['selected_provider']['wire_protocol'] is None
        legacy = data('decision services options --provider compatible')
        assert legacy['selected_provider']['kind'] == 'system_one'
        command = ('decision connections apply --id decision/compatible --name Compatible '
            '--provider compatible --model fixture --endpoint '
            f'http://127.0.0.1:{endpoint.server_port}/v1/decisions --source {product.source_id}')
        code, missing = product.public_cli(command, success=False)
        assert code != 0 and missing['data']['required_choices'] == ['--wire-protocol'], missing
        for codec in ('system_one', 'openai_decisions'):
            preview = data(command + ' --wire-protocol ' + codec)
            assert preview['normalized_spec']['desired_state']['change']['service']['connection']['wire_protocol'] == codec
        shared = {'schema': 'hiroute.decision-service/v2', 'id': 'decision/shared', 'revision': 1,
            'name': 'Shared loopback', 'connection': {'kind': 'native', 'provider': 'openai',
                'wire_protocol': 'openai_decisions', 'model': 'gpt-6-luna',
                'endpoint': f'http://127.0.0.1:{endpoint.server_port}/v1/decisions', 'timeout_ms': 3000,
                'authentication': {'kind': 'shared_source', 'source_id': product.source_id,
                                   'source_lineage_digest': source_view['lineage_digest']}}}
        before = len(Endpoint.requests)
        change = {'id': shared['id'], 'expected_revision': 0, 'service': shared,
                  'expected_source_revisions': [{'source_id': product.source_id, 'revision': source_view['revision']}]}
        stale_change = copy.deepcopy(change)
        stale_change['expected_source_revisions'][0]['revision'] += 1
        def spec(value):
            return {'schema_version': V1, 'command_id': 'decision.connections.apply',
                    'resource_id': shared['id'], 'desired_state': {'change': value}}
        code, rejected = product.public_cli('decision connections apply',
            {'schema_version': V1, 'spec': spec(stale_change)}, success=False)
        assert code != 0 and rejected['error']['code'] == 'REVISION_CONFLICT', rejected
        old_preview = data('decision connections apply', {'schema_version': V1, 'spec': spec(change)})
        save_source('save_disabled', [], 'decision-conflict-disable')
        save_source('save_ready', [], 'decision-conflict-enable')
        source_view = next(row for row in data('decision sources list')['sources'] if row['source_id'] == product.source_id)
        change['expected_source_revisions'][0]['revision'] = source_view['revision']
        fresh_preview = data('decision connections apply', {'schema_version': V1, 'spec': spec(change)})
        # Use fresh overall revisions to exercise the per-source CAS during Apply,
        # rather than failing at the earlier whole-store revision check.
        code, rejected = product.public_cli('decision connections apply', {'schema_version': V1,
            'spec': old_preview['normalized_spec'], 'accept_digest': old_preview['change_digest'],
            'expected_revisions': fresh_preview['expected_revisions'], 'idempotency_key': 'decision-stale-apply'}, success=False)
        assert code != 0 and rejected['error']['code'] == 'REVISION_CONFLICT', rejected
        mutate(change, 'decision-shared-create')
        assert len(Endpoint.requests) == before, 'save called the provider'
        view = current(shared['id'])
        result = test(view)
        assert result['outcome'] == 'passed', result
        assert result['tested_credential_state_revision'] == view['credential_state_revision']
        attempts = Endpoint.requests[before:]
        assert len(attempts) == 2, attempts
        assert attempts[0][0] == 'Bearer ' + SENTINEL and attempts[1][0] == 'Bearer ' + secondary
        assert attempts[1][1]['model'] == 'gpt-6-luna'
        assert attempts[1][1]['input'][0]['content'][0]['type'] == 'input_json'
        assert isinstance(attempts[1][1]['questions'], list)
        before = len(Endpoint.requests)
        assert test(view)['outcome'] == 'passed'
        assert len(Endpoint.requests) == before + 1, 'rejected Key was selected again'
        assert Endpoint.requests[-1][0] == 'Bearer ' + secondary
        before = len(Endpoint.requests)
        Endpoint.reject_input = True
        rejected = test(view)
        Endpoint.reject_input = False
        assert rejected['failure_code'] == 'DECISION_INPUT_REJECTED', rejected
        assert len(Endpoint.requests) == before + 1, 'generic input failure retried another Key'
        assert rejected['tested_credential_state_revision'] == view['credential_state_revision']
        stale = test(view, 'sha256:' + '0' * 64)
        assert stale['failure_code'] == 'DECISION_REVISION_CONFLICT', stale
        assert len(Endpoint.requests) == before + 1, 'stale test reached provider'
        source = save_source('save_disabled', [], 'decision-disable-source')
        disabled = current(shared['id'])
        before = len(Endpoint.requests)
        result = test(disabled)
        assert result['failure_code'] == 'DECISION_SOURCE_DISABLED', result
        assert result['tested_credential_state_revision'] == disabled['credential_state_revision']
        assert len(Endpoint.requests) == before, 'disabled source reached provider'
        save_source('save_ready', [], 'decision-enable-source')
        product.stop()
        product.start()
        assert test(current(shared['id']))['outcome'] == 'passed'

        # Publish an actual Agent route. Its model and Decisions calls each use
        # their own local counted upstream, so no paid provider is contacted.
        model_upstream = NativeUpstream()
        product.install_codex_fixture(MODEL, 'medium', model_upstream.base_url, NATIVE_TOKEN)
        model_source = save_native_source(product, model_upstream, NATIVE_TOKEN, cli_input=True)
        before = len(Endpoint.requests)
        publish_plan_and_agent(product, model_source['binding_id'],
                               native_model_mode='hiroute_only', branch_service=shared)
        assert len(Endpoint.requests) == before, 'publication called the decision provider'
        gateway_request(product)
        assert len(Endpoint.requests) == before + 1
        assert Endpoint.requests[-1][1]['model'] == shared['connection']['model']
        assert model_upstream.requests[-1]['path'] == '/v1/responses'

        expected = []
        for status, code, kind in ((404, 'not_found', 'endpoint_rejected'),
                                   (405, 'method_not_allowed', 'endpoint_rejected'),
                                   (422, 'validation_error', 'input_rejected'),
                                   (422, 'context_length_exceeded', 'context_limit'),
                                   (500, 'internal_error', 'upstream_error')):
            Endpoint.failure = (status, code)
            failure = {'kind': kind, 'http_status': status}
            view = current(shared['id'])
            before = len(Endpoint.requests)
            probe = {
                'id': shared['id'], 'revision': 1,
                'expected_configuration_digest': view['configuration_digest']}
            diagnostic = data('decision network test', {**probe, 'include_decision_failure': True})
            assert diagnostic['decision_failure'] == failure, diagnostic
            assert 'unpublished provider detail' not in json.dumps(diagnostic), diagnostic
            assert len(Endpoint.requests) == before + 1, 'HTTP failure retried another Key'
            gateway_request(product)
            assert len(Endpoint.requests) == before + 2, 'routing failure retried another Key'
            expected.append(failure)
        legacy_probe = data('decision network test', probe)
        assert set(legacy_probe) == {'schema', 'check', 'network', 'failure'}, legacy_probe
        assert legacy_probe['check']['failure_code'] == 'DECISION_TRANSPORT_UNAVAILABLE', legacy_probe
        Endpoint.failure = None

        deadline = time.monotonic() + 30
        while True:
            sessions = data('sessions list --include-unlinked --limit 50')['sessions']
            if sessions:
                session_id = sessions[0]['session_id']
                timeline = data('sessions timeline ' + session_id + ' --include-decision-failure')
                requests = [request for turn in timeline['turns'] for request in turn['requests']]
                failures = [request for request in requests if request.get('decision_failure')]
                if (len(failures) == len(expected) and len(requests) == 1 + len(expected)
                        and all(request['outcome'] == 'accepted' for request in requests)):
                    break
            assert time.monotonic() < deadline, 'decision fallback evidence did not reach the timeline'
            time.sleep(.05)
        assert all(request['decision_failure']['service'] == {'id': shared['id'], 'revision': 1}
                   for request in failures), failures
        assert all(request['decision_failure']['source'] == 'unresolved' for request in failures), failures
        failure_key = lambda failure: (failure['http_status'], failure['kind'])
        assert sorted(failure_key(request['decision_failure']['failure']) for request in failures) == sorted(
            map(failure_key, expected)), failures
        assert 'unpublished provider detail' not in json.dumps(timeline), timeline
        legacy_timeline = data('sessions timeline ' + session_id)
        legacy_requests = [request for turn in legacy_timeline['turns'] for request in turn['requests']]
        assert len(legacy_requests) == len(requests)
        assert all('decision_failure' not in request for request in legacy_requests), legacy_requests

        conversations = data('sessions list --view conversations --period today')
        anchor = next(conversation['analysis_session_id'] for conversation in conversations['conversations']
                      if any(member['session_id'] == session_id for member in conversation['members']))
        routing_command = f'sessions show {anchor} --view routing --analysis-scope {conversations["analysis_scope"]}'
        routing = data(routing_command + ' --include-decision-failure')
        details = {transition['request_id']: transition['decision_failure']
                   for transition in routing['transitions'] if transition.get('decision_failure')}
        assert details == {request['request_id']: request['decision_failure'] for request in failures}, routing
        legacy_routing = data(routing_command)
        assert {transition['request_id'] for transition in legacy_routing['transitions']} == {
            transition['request_id'] for transition in routing['transitions']}
        assert all('decision_failure' not in transition for transition in legacy_routing['transitions']), legacy_routing

        # Switch A to B with only B's revision. A remains the authority for the
        # already published version, while the new version joins B.
        newer = copy.deepcopy(shared)
        newer['revision'] = 2
        newer['connection']['model'] = 'gpt-6-luna-next'
        sources = {row['source_id']: row for row in data('decision sources list')['sources']}
        before_a = sources[product.source_id]['revision']
        source_b = sources[model_source['source_id']]
        newer['connection']['authentication'] = {'kind': 'shared_source',
            'source_id': source_b['source_id'], 'source_lineage_digest': source_b['lineage_digest']}
        mutate({'id': shared['id'], 'expected_revision': 1, 'service': newer,
                'expected_source_revisions': [{'source_id': source_b['source_id'], 'revision': source_b['revision']}]}, 'decision-switch-source')
        sources = {row['source_id']: row for row in data('decision sources list')['sources']}
        assert sources[product.source_id]['revision'] == before_a
        assert sources[source_b['source_id']]['revision'] > source_b['revision']
        before = len(Endpoint.requests)
        gateway_request(product)
        assert len(Endpoint.requests) == before + 1
        assert Endpoint.requests[-1][1]['model'] == shared['connection']['model']

        # Current Desktop records still fail the strict V1 reader with its
        # released machine error. The CLI gives the migration path on stderr.
        before = len(Endpoint.requests)
        for command in ('decision services list', f'decision services test --service {shared["id"]}@2'):
            code, envelope = product.public_cli(command, success=False)
            assert code == 2, envelope
            assert envelope['error']['code'] == 'SCHEMA_INCOMPATIBLE', envelope
            assert envelope['error']['message_key'] == 'decision_service.client_version_required', envelope
            assert b'hiroute decision connections test --service ID@REVISION' in product.outputs[-1]
        for command, hint in (('decision services list', b'decision connections list'),
                              ('decision services test', b'decision connections test'),
                              ('decision connections test', b'historical revision')):
            help_result = subprocess.run([str(product.bin / 'hiroute'), *command.split(), '--help'],
                                         env=product.env, cwd=product.project, capture_output=True,
                                         timeout=30, check=True)
            assert hint in help_result.stdout, help_result.stdout
        assert len(Endpoint.requests) == before, 'legacy errors or help reached the provider'

        head = current(shared['id'])
        assert head['service']['revision'] == 2
        for revision, model in ((1, shared['connection']['model']), (2, newer['connection']['model'])):
            before = len(Endpoint.requests)
            result = data(f'decision connections test --service {shared["id"]}@{revision}')
            assert result['outcome'] == 'passed', result
            assert len(Endpoint.requests) == before + 1
            assert Endpoint.requests[-1][1]['model'] == model
            if revision == 1:
                assert result['tested_configuration_digest'] == view['configuration_digest']
        before = len(Endpoint.requests)
        code, missing = product.public_cli(
            f'decision connections test --service {shared["id"]}@999', success=False)
        assert code != 0 and missing['error']['code'] == 'RESOURCE_NOT_FOUND', missing
        assert len(Endpoint.requests) == before, 'missing revision reached the provider'
        assert test(view)['outcome'] == 'passed'
        assert len(Endpoint.requests) == before + 1
        assert Endpoint.requests[-1][1]['model'] == shared['connection']['model']

        # Shared to independent needs no source revisions; both old memberships
        # remain available to their frozen versions.
        independent = copy.deepcopy(newer)
        independent['revision'] = 3
        independent['connection']['authentication'] = {'kind': 'independent', 'auth_header': {
            'name': 'Authorization', 'value_secret_ref': 'decision/shared/independent'}}
        independent_token = 'Bearer decision-fixture-independent-token'
        product.secrets.add(independent_token)
        product.register_protected_frame({'schema': 'hiroute.protected-input/v1',
            'registration_id': hashlib.sha256(b'decision-independent').hexdigest(),
            'candidate_ref': 'candidate/decision-independent', 'candidate_revision': 1,
            'secret': independent_token})
        mutate({'id': shared['id'], 'expected_revision': 2, 'service': independent,
                'expected_source_revisions': []}, 'decision-switch-independent', 'candidate/decision-independent')
        after = {row['source_id']: row for row in data('decision sources list')['sources']}
        assert after[product.source_id]['revision'] == sources[product.source_id]['revision']
        assert after[source_b['source_id']]['revision'] == sources[source_b['source_id']]['revision']
        assert {row['source_id'] for row in current(shared['id'])['related_sources']} == {product.source_id, source_b['source_id']}
        assert test(current(shared['id']))['outcome'] == 'passed'
        assert Endpoint.requests[-1][0] == independent_token
        save_source('save_disabled', [], 'decision-disable-published-source')
        before = len(Endpoint.requests)
        gateway_request(product)
        assert len(Endpoint.requests) == before, 'disabled decision source reached provider'
        save_source('save_ready', [], 'decision-enable-published-source')
        product.stop()
        product.start()
        gateway_request(product)
        assert len(Endpoint.requests) == before + 1
        assert Endpoint.requests[-1][1]['model'] == shared['connection']['model']
        print(json.dumps({'scenario': 'decision-v2-real-source-loopback', 'state': 'green',
            'cases': ['legacy-cli-migration', 'exact-revision-cli-test',
                      'save-without-network', 'exact-admission', '401-key-failover', '400-no-retry',
                      'stale-digest', 'disabled-source', 'restart', 'published-gateway-route',
                      'frozen-revision', 'source-switch', 'independent-switch',
                      'published-source-disable', 'explicit-compatible-codec',
                      'stale-source-preview-and-apply', 'request-local-http-fallback-evidence']}), flush=True)
    finally:
        Endpoint.failure = None
        if model_upstream is not None:
            model_upstream.close()
        endpoint.shutdown()
        endpoint.server_close()
        thread.join(timeout=5)
        product.close()


if __name__ == '__main__':
    run(Path(sys.argv[1]))
