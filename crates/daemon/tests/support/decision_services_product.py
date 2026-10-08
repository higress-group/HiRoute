"""Released decision CLI, journaled storage and pinned branch publication on real binaries."""
import copy
import hashlib
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from publication_product import Product, V1, encoded
from publication_process import bootstrap, plan_change, judgment_fixture


class DecisionEndpoint(BaseHTTPRequestHandler):
    requests = []

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.requests.append((self.headers.get('Authorization'), body))
        if 'questions' in body:
            result = {'answers': {key: ({'type': 'choice', 'choice': next(iter(question['criteria']))}
                if question['type'] == 'choice' else {'type': 'score', 'probabilities': {'0': 0.9, '1': 0.1}})
                for key, question in body['questions'].items()}}
        else:
            result = {'decision': {'kind': 'categorical', 'choice': body['decision']['options'][0]['id'],
                'refinement': {'kind': 'ordinal', 'probabilities': {'simple': 0.9, 'complex': 0.1}}}}
        raw = encoded(result)
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def log_message(self, *_args):
        pass


def scenario(repository):
    product = Product(repository)
    product.collaboration_only = True
    product.enable_debug_diagnostics()
    endpoint = ThreadingHTTPServer(('127.0.0.1', 0), DecisionEndpoint)
    worker = threading.Thread(target=endpoint.serve_forever, daemon=True)
    worker.start()

    def cli(command, body=None, ok=True):
        args = [str(product.bin / 'hiroute'), *command.split(), '--output', 'json']
        if body is not None:
            args.append('--request-stdin')
        result = subprocess.run(args, input=encoded(body) if body is not None else None,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                env=product.env, cwd=product.project, timeout=60)
        product.outputs.extend([result.stdout, result.stderr])
        value = json.loads(result.stdout)
        if ok:
            assert result.returncode == 0, value
        return value

    def mutate(change, key, ok=True):
        spec = {'schema_version': V1, 'command_id': 'decision.services.apply',
                'resource_id': change['id'], 'desired_state': change}
        response = cli('decision services apply', {'schema_version': V1, 'spec': spec}, ok=ok)
        if not ok and response['status'] != 'succeeded':
            return response
        preview = response['data']
        apply = {'schema_version': V1, 'spec': preview['normalized_spec'],
                 'accept_digest': preview['change_digest'],
                 'expected_revisions': preview['expected_revisions'], 'idempotency_key': key}
        result = cli('decision services apply', apply, ok=ok)
        if ok:
            assert result['data']['state'] == 'succeeded', result
            assert cli('decision services apply', apply)['data'] == result['data'], 'retry duplicated service change'
        else:
            assert result.get('data', {}).get('state') != 'succeeded', result
        return result

    try:
        bootstrap(product)
        assert cli('decision services list')['data']['services'] == []
        assert cli('decision services list invalid', ok=False)['status'] == 'usage_error'
        token = 'Bearer decision-fixture-private-key'
        product.secrets.add(token)
        product.register_protected_frame({'schema': 'hiroute.protected-input/v1',
            'registration_id': hashlib.sha256(b'decision-fixture-input').hexdigest(),
            'candidate_ref': 'candidate/decision-fixture',
            'candidate_revision': 1, 'secret': token})
        service = {'id': 'decision-fixture', 'revision': 1, 'name': 'Built-in decision',
            'connection': {'kind': 'system_one', 'provider': 'compatible', 'model': 'fixture-jev',
                'endpoint': f'http://127.0.0.1:{endpoint.server_port}/systemone', 'timeout_ms': 3000,
                'auth_header': {'name': 'Authorization', 'value_secret_ref': 'decision/fixture/r1'}}}
        mutate({'id': service['id'], 'expected_revision': 0, 'service': service,
                'input_slot': 'candidate/decision-fixture'}, 'decision-create')
        assert cli('decision services list')['data']['services'] == [service]
        tested = cli('decision services test', {'schema': 'hiroute.classifier-decision-test/v1',
            'classifier': {'kind': 'decision_service', 'service': service}})
        assert tested['data']['outcome'] == 'passed', tested
        assert DecisionEndpoint.requests[-1][0] == token
        assert DecisionEndpoint.requests[-1][1]['model'] == 'fixture-jev'
        assert len(DecisionEndpoint.requests[-1][1]['questions']) == 3
        assert DecisionEndpoint.requests[-1][1]['state']['assessment_from'] is None
        # A shared API Key may back independent immutable service versions.
        shared = copy.deepcopy(service)
        shared.update(id='decision-shared', name='Same key, independent connection')
        for revision, value in [(1, token), (2, 'Bearer changed-decision-fixture'), (3, token)]:
            product.secrets.add(value)
            slot = f'candidate/decision-shared-{revision}'
            product.register_protected_frame({'schema': 'hiroute.protected-input/v1',
                'registration_id': hashlib.sha256(slot.encode()).hexdigest(),
                'candidate_ref': slot, 'candidate_revision': 1, 'secret': value})
            shared['revision'] = revision
            shared['connection']['auth_header']['value_secret_ref'] = f'decision/shared/r{revision}'
            mutate({'id': shared['id'], 'expected_revision': revision - 1, 'service': shared,
                    'input_slot': slot}, f'decision-shared-{revision}')
            for saved, expected in [(shared, value), (service, token)]:
                tested = cli('decision services test', {'schema': 'hiroute.classifier-decision-test/v1',
                    'classifier': {'kind': 'decision_service', 'service': saved}})
                assert tested['data']['outcome'] == 'passed', tested
                assert DecisionEndpoint.requests[-1][0] == expected, 'one version mutated another credential'
        mutate({'id': shared['id'], 'expected_revision': 3, 'service': None}, 'decision-shared-delete')
        assert cli('decision services test', {'schema': 'wrong', 'classifier': {'kind': 'local_rules'}}, ok=False)['status'] != 'succeeded'
        selection = product.editor['candidates'][0]
        branches = [{'id': key, 'name': name, 'condition': condition, 'candidates': [selection],
            'primary_candidates': [], 'judgment': None} for key, name, condition in
            [('code', 'Code', 'Programming work'), ('docs', 'Documentation', 'Documentation work')]]
        branches[0]['primary_candidates'] = [selection]
        routing = {'classifier': {'kind': 'decision_service', 'service': copy.deepcopy(service)},
                   'branches': branches, 'default_branch_id': 'docs',
                   'judgment': judgment_fixture(), 'reselect_on_user_message': False}
        routing['judgment']['degree']['instructions'] = 'A complete degree standard.\n' * 1000
        routing['branches'][0]['condition'] = 'Programming tasks with supplied requirements.\n' * 500
        change = plan_change(product, 'update', mode='custom_branches', branch_routing=routing)
        for case in ['unsaved', 'different-content', 'missing-revision', 'direct-rest', 'reserved-id']:
            invalid = copy.deepcopy(change)
            branch_config = invalid['editor']['branch_routing']
            supplied = branch_config['classifier']['service']
            if case == 'unsaved':
                supplied['id'] = 'not-saved'
            elif case == 'different-content':
                supplied['connection']['endpoint'] += '/changed'
            elif case == 'missing-revision':
                supplied['revision'] = 99
            elif case == 'direct-rest':
                branch_config['classifier'] = {'kind': 'rest', 'endpoint': service['connection']['endpoint'], 'timeout_ms': 3000}
            else:
                branch_config['branches'][0]['id'] = 'smart_saving'
            rejected = cli('routing preview', {'change': invalid}, ok=False)
            assert rejected['status'] != 'succeeded', (case, rejected)
        preview = product.preview('routing preview', {'change': change})
        applied, _, _ = product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, 'decision-plan')
        assert applied['data']['state'] == 'succeeded', applied
        next_service = copy.deepcopy(service)
        next_service.update(revision=2, name='Changed connection')
        next_service['connection']['model'] = 'fixture-jev-next'
        mutate({'id': service['id'], 'expected_revision': 1, 'service': next_service}, 'decision-edit')
        mutate({'id': service['id'], 'expected_revision': 0, 'service': service}, 'decision-stale', ok=False)
        mutate({'id': service['id'], 'expected_revision': 2, 'service': None}, 'decision-in-use', ok=False)
        product.stop()
        product.start()
        assert cli('decision services list')['data']['services'] == [next_service]
        plan = cli('routing show ' + product.plan_id)['data']
        assert plan['desired']['strategy']['routing']['classifier']['service'] == service, 'published service revision drifted'
        # The exact saved r1 remains publishable even when the connection list only shows r2.
        historical = plan_change(product, 'update', mode='custom_branches', branch_routing=routing)
        pinned = product.preview('routing preview', {'change': historical})
        product.apply('routing apply', 'ApplyAgentPlanChange', pinned, {'change': historical}, 'decision-publish-historical-r1')
        plan = cli('routing show ' + product.plan_id)['data']
        assert plan['desired']['strategy']['routing']['classifier']['service'] == service
        assert plan['desired']['strategy']['routing']['judgment'] == routing['judgment'], 'long prompts were changed'
        custom = {'id': 'custom-fixture', 'revision': 1, 'name': 'Custom decision', 'connection': {
            'kind': 'custom', 'endpoint': service['connection']['endpoint'], 'timeout_ms': 3000}}
        mutate({'id': custom['id'], 'expected_revision': 0, 'service': custom}, 'custom-create')
        tested = cli('decision services test', {'schema': 'hiroute.classifier-decision-test/v1',
            'classifier': {'kind': 'decision_service', 'service': custom}})
        assert tested['data']['outcome'] == 'passed', tested
        assert set(DecisionEndpoint.requests[-1][1]) == {'decision', 'latest_user', 'visible_conversation', 'history_partial', 'assessment_target'}
        mutate({'id': custom['id'], 'expected_revision': 1, 'service': None}, 'custom-delete')
        assert cli('decision services list')['data']['services'] == [next_service]
        print(json.dumps({'scenario': 'decision-services-real-cli-crud-protocol-pinned-publication', 'state': 'green'}), flush=True)
    finally:
        endpoint.shutdown()
        endpoint.server_close()
        worker.join(timeout=5)
        product.close()


if __name__ == '__main__':
    scenario(sys.argv[1])
