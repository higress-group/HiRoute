"""Released decision CLI, journaled storage and pinned branch publication on real binaries."""
import copy
import hashlib
import json
import subprocess
import sys
import threading
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from publication_product import Product, V1, encoded
from publication_process import bootstrap, plan_change, judgment_fixture


class DecisionEndpoint(BaseHTTPRequestHandler):
    requests = []
    c_vectors = False
    c_applicability = 0.75
    c_score = 1.0
    native_history_assessment = False

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        self.requests.append((self.headers.get('Authorization'), body))
        if 'questions' in body:
            result = {'answers': {key: ({'type': 'choice', 'probabilities': {'applicable': self.c_applicability, 'not_established': 1 - self.c_applicability}}
                if self.c_vectors and question['type'] == 'choice' and set(question['criteria']) == {'applicable', 'not_established'}
                else {'type': 'score', 'score': self.c_score}
                if self.c_vectors and question['type'] == 'score' and len(question['criteria']) == 3
                else {'type': 'choice', 'choice': next(iter(question['criteria']))}
                if question['type'] == 'choice' else {'type': 'score', 'probabilities': {'0': 0.9, '1': 0.1}})
                for key, question in body['questions'].items()}}
        else:
            result = {'category': body['route']['categories'][0]['id'], 'group': 'regular'}
            if self.native_history_assessment and isinstance(body.get('preceding_stage'), dict):
                result['assessment'] = {'score': 0.75, 'partial': False}
        raw = encoded(result)
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def log_message(self, *_args):
        pass


@contextmanager
def native_history_decisions(product, plan_id, editor, endpoint_base=DecisionEndpoint):
    """Observe real native routing through a saved service and public Plan edits."""
    endpoint_type = type('NativeHistoryEndpoint', (endpoint_base,),
                         {'requests': [], 'native_history_assessment': True})
    endpoint = ThreadingHTTPServer(('127.0.0.1', 0), endpoint_type)
    worker = threading.Thread(target=endpoint.serve_forever, daemon=True)
    worker.start()
    old_plan, old_editor = product.plan_id, product.editor
    service = {'id': 'native-history', 'revision': 1, 'name': 'Native history witness',
        'connection': {'kind': 'custom', 'endpoint': f'http://127.0.0.1:{endpoint.server_port}/decision',
                       'timeout_ms': 3000}}
    changed = False

    def mutate_service(value, revision, key):
        change = {'id': service['id'], 'expected_revision': revision, 'service': value}
        spec = {'schema_version': V1, 'command_id': 'decision.services.apply',
                'resource_id': service['id'], 'desired_state': change}
        preview = product.public_cli('decision services apply', {'schema_version': V1, 'spec': spec})[1]['data']
        result = product.public_cli('decision services apply', {'schema_version': V1,
            'spec': preview['normalized_spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': key})[1]
        assert result['data']['state'] == 'succeeded', result

    def publish(value, key):
        change = plan_change(product, 'update', **value)
        preview = product.preview('routing preview', {'change': change})
        product.apply('routing apply', 'ApplyAgentPlanChange', preview, {'change': change}, key)

    try:
        mutate_service(service, 0, 'native-history-create')
        product.plan_id, product.editor = plan_id, copy.deepcopy(editor)
        routed = copy.deepcopy(editor)
        routed['mode'] = 'custom_branches'
        routed['branch_routing'] = {'classifier': {'kind': 'decision_service', 'service': service},
            'branches': [{'id': name, 'name': name.title(), 'condition': condition,
                'candidates': routed['candidates'], 'primary_candidates': [], 'judgment': None}
                for name, condition in (('native', 'Ordinary native acceptance tasks'),
                                        ('fallback', 'Other bounded tasks'))],
            'default_branch_id': 'fallback', 'judgment': routed['smart']['judgment'],
            'reselect_on_user_message': True}
        publish(routed, 'native-history-plan')
        changed = True
        yield lambda: [body for _, body in endpoint_type.requests]
    finally:
        try:
            if changed:
                publish(editor, 'native-history-plan-restore')
            # The published temporary version still pins this service even after
            # restoring the head. Retain that immutable reference until the owned
            # Product workspace closes; deleting it would violate the real contract.
        finally:
            product.plan_id, product.editor = old_plan, old_editor
            endpoint.shutdown()
            endpoint.server_close()
            worker.join(timeout=5)


def scenario(repository, routed=False):
    product = Product(repository)
    product.collaboration_only = not routed
    if routed:
        product.enable_cpa()
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
        if routed:
            from c_protocol_product import exercise
            exercise(product, cli, service, DecisionEndpoint, product.editor['candidates'][0])
            return
        tested = cli('decision services test', {'schema': 'hiroute.classifier-decision-test/v1',
            'classifier': {'kind': 'decision_service', 'service': service}})
        assert tested['data']['outcome'] == 'passed', tested
        assert DecisionEndpoint.requests[-1][0] == token
        assert DecisionEndpoint.requests[-1][1]['model'] == 'fixture-jev'
        assert len(DecisionEndpoint.requests[-1][1]['questions']) == 3
        state = DecisionEndpoint.requests[-1][1]['state']
        assert state['assessment_stage'] == []
        assert state['prior_context'] == []
        assert 'assessment_from' not in state and 'visible_conversation' not in state
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
        # This lifecycle case deliberately owns custom text; publication must retain it.
        routing['judgment']['competence']['standard'] = 'custom'
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
        # The same released v2 codecs accept both saved values and current omitted input.
        # Exercise real preview/apply/publication/restart, not only deserialization.
        for mode in ['smart_saving', 'custom_branches']:
            for legacy in [False, True, None]:
                editor = copy.deepcopy(product.editor)
                editor['mode'] = mode
                editor['smart'].update(economy=[selection], primary=[selection])
                editor['branch_routing'] = copy.deepcopy(routing)
                for config in [editor['smart'], editor['branch_routing']]:
                    if legacy is None:
                        config.pop('reselect_on_user_message', None)
                    else:
                        config['reselect_on_user_message'] = legacy
                compatible = plan_change(product, 'update', **editor)
                preview = product.preview('routing preview', {'change': compatible})
                product.apply('routing apply', 'ApplyAgentPlanChange', preview,
                              {'change': compatible}, f'follow-up-{mode}-{legacy}')
                product.stop()
                product.start()
                saved = cli('routing show ' + product.plan_id)['data']
                strategy = saved['desired']['strategy']
                config = strategy['routing'] if mode == 'custom_branches' else strategy
                assert config['reselect_on_user_message'] == bool(legacy), saved
                assert saved['desired']['mode'] == mode
                assert config['judgment'] == editor['branch_routing' if mode == 'custom_branches' else 'smart']['judgment']
        print(json.dumps({'scenario': 'retired-follow-up-legacy-and-omitted-real-cli-publication-restart',
                          'state': 'green', 'cases': 6}), flush=True)
        from c_protocol_product import exercise
        exercise(product, cli, service, DecisionEndpoint, selection)
        custom = {'id': 'custom-fixture', 'revision': 1, 'name': 'Custom decision', 'connection': {
            'kind': 'custom', 'endpoint': service['connection']['endpoint'], 'timeout_ms': 3000}}
        mutate({'id': custom['id'], 'expected_revision': 0, 'service': custom}, 'custom-create')
        tested = cli('decision services test', {'schema': 'hiroute.classifier-decision-test/v1',
            'classifier': {'kind': 'decision_service', 'service': custom}})
        assert tested['data']['outcome'] == 'passed', tested
        assert set(DecisionEndpoint.requests[-1][1]) == {'route', 'latest_user', 'visible_conversation', 'history_partial', 'preceding_stage'}
        mutate({'id': custom['id'], 'expected_revision': 1, 'service': None}, 'custom-delete')
        assert cli('decision services list')['data']['services'] == [next_service]
        print(json.dumps({'scenario': 'decision-services-real-cli-crud-protocol-pinned-publication', 'state': 'green'}), flush=True)
    finally:
        endpoint.shutdown()
        endpoint.server_close()
        worker.join(timeout=5)
        product.close()


if __name__ == '__main__':
    scenario(sys.argv[1], routed=sys.argv[2:] == ['routed'])
