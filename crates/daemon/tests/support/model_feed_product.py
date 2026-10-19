"""Installed CLI/daemon/Gateway lifecycle with signed feed and external provider/ACP fixtures.

Only the integration-test-hooks binary accepts the private feed transport fixture. Product
state is changed exclusively through released commands. TLS dialing uses the existing E2E map.
"""
import copy
import http.server
import json
import os
from pathlib import Path
import shutil
import ssl
import subprocess
import sys
import threading
import time

from standalone_headless_product import InstalledProduct, apply_plan, plan_change, save_candidate, terminal, wire
from publication_process import judgment_fixture

MODEL = 'model.feed.regression'
UPSTREAM = 'feed-regression'
OPTION = 'bailian.payg.cn.v1'


class FeedUpstream:
    def __init__(self, root):
        self.requests = []
        self.tls_failures = []
        self.tls_connections = 0
        cert, key = root / 'ca.pem', root / 'key.pem'
        result = subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes',
            '-keyout', str(key), '-out', str(cert), '-days', '1', '-subj', '/CN=dashscope.aliyuncs.com',
            '-addext', 'basicConstraints=critical,CA:FALSE', '-addext', 'subjectAltName=DNS:dashscope.aliyuncs.com'], capture_output=True, timeout=30)
        assert result.returncode == 0, 'test TLS certificate generation failed'
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert body['model'] == UPSTREAM, body
                owner.requests.append({'path': self.path, 'model': body['model']})
                answer = 'native product answer'
                if self.path.endswith('/responses'):
                    value = {'id': 'resp_feed', 'object': 'response', 'status': 'completed', 'model': UPSTREAM,
                        'output': [{'id': 'msg_feed', 'type': 'message', 'role': 'assistant', 'status': 'completed',
                            'content': [{'type': 'output_text', 'text': answer, 'annotations': []}]}],
                        'usage': {'input_tokens': 4, 'output_tokens': 3, 'total_tokens': 7}}
                elif self.path.endswith('/messages'):
                    value = {'id': 'msg_feed', 'type': 'message', 'role': 'assistant', 'model': UPSTREAM,
                        'content': [{'type': 'text', 'text': answer}], 'stop_reason': 'end_turn',
                        'usage': {'input_tokens': 4, 'output_tokens': 3}}
                else:
                    value = {'id': 'chatcmpl_feed', 'object': 'chat.completion', 'created': int(time.time()), 'model': UPSTREAM,
                        'choices': [{'index': 0, 'message': {'role': 'assistant', 'content': answer}, 'finish_reason': 'stop'}],
                        'usage': {'prompt_tokens': 4, 'completion_tokens': 3, 'total_tokens': 7}}
                raw = wire(value)
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

        class Server(http.server.ThreadingHTTPServer):
            def get_request(self):
                try:
                    connection, address = super().get_request()
                except ssl.SSLError as error:
                    # Report stable TLS error codes, never arbitrary exception text or credentials.
                    owner.tls_failures.append({'kind': type(error).__name__, 'errno': error.errno,
                                               'library': error.library, 'reason': error.reason})
                    raise
                owner.tls_connections += 1
                return connection, address

        self.server = Server(('127.0.0.1', 0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        # This fixture implements HTTP/1.1 only; advertise it during TLS negotiation.
        context.set_alpn_protocols(['http/1.1'])
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.dial = root / 'dial.json'
        self.dial.write_bytes(wire({'schema_version': 'hiroute.gateway.e2e-dial-map/v1', 'targets': [{
            'authority': 'dashscope.aliyuncs.com',
            'address': '127.0.0.1:' + str(self.server.server_port), 'ca_pem': cert.read_text()}]}))

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def publication(product, sequence, present=True):
    bundle = json.loads((product.repo / 'assets/release-facts/current/bundle/model-data.json').read_text())
    original = 'model.bailian.qwen3-max-2026-01-23'
    definition = copy.deepcopy(next(x for x in bundle['data']['models'] if x['model_configuration_id'] == original))
    definition.update(model_configuration_id=MODEL, display_name='Feed regression model')
    canonical = copy.deepcopy(next(x for x in bundle['metadata_catalog']['canonical_models']
                                  if 'qwen3-max-2026-01-23' in x['upstream_ids']))
    canonical.update(model_key=UPSTREAM, upstream_ids=[UPSTREAM], display_name='Feed regression model')
    record = {'definition': definition, 'canonical': canonical, 'native_reasoning': {
        'model_configuration_id': MODEL, 'capability': {'kind': 'toggle', 'parameter': 'enable_thinking'}}}
    records = [{'kind': 'model', 'record': record}]
    for capability in bundle['data']['model_endpoint_capabilities']:
        if capability['model_configuration_id'] != original:
            continue
        capability = copy.deepcopy(capability)
        capability.update(model_configuration_id=MODEL, upstream_model_id=UPSTREAM,
                          capability_id='capability.feed.' + str(len(records)))
        records.append({'kind': 'model_endpoint_capability', 'record': capability})
    offer = copy.deepcopy(next(x for x in bundle['data']['offers'] if x['offer_id'] == 'offer.bailian.payg.cn'))
    offer['model_configuration_ids'].append(MODEL)
    records.append({'kind': 'offer', 'record': offer})
    records.append({'kind': 'price_rate', 'record': {
        'price_rate_id': 'price.feed.regression.usd', 'revision': 1, 'offer_ref': offer['offer_id'],
        'model_configuration_id': MODEL, 'currency': 'USD',
        'input_micros_per_million': 1234567, 'output_micros_per_million': 2345678}})
    for record in records:
        record.update(min_client_version='0.1.0', included_on='2026-10-18')
    return {'schema': 'hiroute.model-feed/v1', 'sequence': sequence, 'issued_at': '2026-10-18T00:00:00Z',
            'source_revision': product.sha, 'records': records if present else [], 'featured': []}


def restart(product):
    product.daemon.terminate()
    product.daemon.wait(timeout=30)
    product.outputs.extend((product.daemon.stdout.read(), product.daemon.stderr.read()))
    product.daemon = None
    product.start()


def raw(product, *arguments, payload):
    result = subprocess.run([str(product.cli_path), *arguments, '--request-stdin', '--output', 'json'],
        input=wire(payload), cwd=product.project, env=product.env, capture_output=True, timeout=45)
    product.outputs.extend((result.stdout, result.stderr))
    return json.loads(result.stdout)


def worker(product, plan, key, previous=None):
    args = ('worker', 'exec', '--plan', plan, '--cwd', str(product.project)) if previous is None else (
        'worker', 'continue', '--task', previous['task_id'], '--expected-latest-run', previous['run_id'])
    accepted = product.cli(*args, '--no-wait', '--submission-key', key, '--file', '-', payload='feed lifecycle work')['data']
    product.cli('worker', 'wait', '--run', accepted['run_id'], '--wait-timeout', '30')
    deadline = time.monotonic() + 30
    while True:
        result = product.cli('worker', 'result', '--run', accepted['run_id'])['data']
        if terminal(result['run_state']):
            assert result['run_state'] == 'succeeded' and 'native product answer' in result['result'], {
                'result': result, 'events': product.cli('worker', 'read', '--run', accepted['run_id'])['data'],
                'task': next(x for x in product.cli('worker', 'list')['data']['tasks'] if x['task_id'] == accepted['task_id'])}
            task = next(x for x in product.cli('worker', 'list')['data']['tasks'] if x['task_id'] == accepted['task_id'])
            if task['run']['cleanup'] == 'complete' and task['resumable_until_ms'] is not None:
                return accepted
        assert time.monotonic() < deadline, result
        time.sleep(.1)


def main_session(product):
    result = subprocess.run([str(product.codex), 'exec', 'new session after feed update'],
        cwd=product.project, env=product.env, capture_output=True, timeout=45)
    product.outputs.extend((result.stdout, result.stderr))
    assert result.returncode == 0 and b'native product answer' in result.stdout, result


def preserve_failure_diagnostics(product, stage, upstream):
    """Retain safe daemon JSONL before the installed fixture removes its state."""
    report = {'scenario': 'installed-signed-feed-lifecycle', 'state': 'red',
              'candidate': product.sha, 'stage': stage,
              'upstream_calls': len(upstream.requests),
              'tls_connections': upstream.tls_connections, 'tls_failures': upstream.tls_failures}
    try:
        product.stop()
        destination = product.repo / 'target/product-e2e-evidence' / ('model-feed-' + str(os.getpid()))
        destination.mkdir(mode=0o700, parents=True, exist_ok=False)
        copied = []
        for directory in product.state.rglob('diagnostics'):
            for source in directory.glob('daemon/*.jsonl'):
                if source.is_symlink() or not source.is_file():
                    continue
                target = destination / source.name
                shutil.copy2(source, target)
                target.chmod(0o600)
                copied.append(source.name)
        report['diagnostics'] = {'files': sorted(copied), 'path': str(destination),
                                 'state': 'retained' if copied else 'unavailable'}
        (destination / 'report.json').write_bytes(wire(report))
    except (OSError, subprocess.SubprocessError):
        report['diagnostics'] = {'state': 'unavailable'}
    print(json.dumps(report), flush=True)


def run(repository):
    product = InstalledProduct(repository)
    fixture = product.root / 'feed-fixture'
    fixture.mkdir(mode=0o700)
    upstream = FeedUpstream(fixture)
    stage = 'install'
    try:
        product.env.update(HIROUTE_TEST_MODEL_FEED_ROOT=str(fixture), HIROUTE_E2E_DIAL_CONFIG=str(upstream.dial))
        included = publication(product, 17)
        (fixture / 'publication.json').write_bytes(wire(included))
        product.install()
        product.start()
        assert product.cli('models', 'feed', 'status')['data']['retrieval_enabled']
        status = product.cli('models', 'feed', 'refresh')['data']
        assert status['held']['sequence'] == 17 and status['counts']['applied'] >= 3, status
        stage = 'registered-check-save-publish'
        print(json.dumps({'scenario': 'installed-signed-feed-lifecycle', 'stage': stage}), flush=True)
        candidate_ref = 'candidate/feed/product'
        product.register_secret(candidate_ref, 'feed-product-key')
        product.secrets.add('feed-product-key')
        checked_request = {'kind': 'registered', 'request': {
            'connection_option_id': OPTION, 'display_name': 'Feed product connection',
            'expected_catalog': product.cli('compute', 'connection', 'options')['data']['catalog'],
            'candidate_ref': candidate_ref, 'lineage_ref': 'lineage/feed/product', 'edit_revision': 1,
            'check_id': 'check/feed/product', 'input_candidate': {'candidate_ref': candidate_ref, 'candidate_revision': 1}}}
        checked = product.cli('compute', 'connection', 'test', payload=checked_request, *['--request-stdin'])['data']
        save_candidate(product, checked['candidate'], None, 'feed-product-save')
        source = next(x for x in product.cli('compute', 'list')['data']['sources'] if x['display_name'] == 'Feed product connection')
        binding = source['models'][0]['binding_id']
        editor = {'schema': 'hiroute.plan-editor/v2', 'display_name': 'Feed product route', 'purpose': 'Frozen feed execution',
            'mode': 'fixed_model', 'candidates': [{'binding_id': binding, 'reasoning': {'kind': 'toggle', 'enabled': True}}],
            'smart': {'economy': [], 'primary': [], 'judgment': judgment_fixture(), 'reselect_on_user_message': False,
                      'classifier': {'kind': 'local_rules'}, 'complex_keywords': []},
            'free': {'candidates': [], 'primary': [], 'primary_fallback': False}, 'requirements': {},
            'delegation_enabled': True, 'work': {'harness': 'codex_cli', 'protocol': 'responses'},
            'limits': {'maximum_attempts': 1, 'request_timeout_ms': 30000, 'attempt_timeout_ms': 30000}}
        created, _, _ = apply_plan(product, plan_change('create', editor, creation_key='feed-product'), 'feed-publish')
        plan = created['plan_head']['reference']['plan_id']
        frozen = product.cli('routing', 'show', plan)['data']
        # Source changes can leave prices pending. Normal startup activates their snapshot.
        restart(product)
        prices = product.cli('prices', 'effective', '--request-stdin', payload={'targets': [
            {'query_id': kind, 'target_locator': {'kind': 'binding', 'binding_id': binding},
             'currency': 'USD', 'valuation_kind': kind} for kind in ['usage_estimate', 'api_equivalent']]})['data']
        assert not prices['pending_activation'] and len(prices['items']) == 2, prices
        by_kind = {item['query_id']: item for item in prices['items']}
        for item in by_kind.values():
            assert item['quote']['exact_target']['model_identity'] == {'kind': 'catalog_model', 'id': MODEL}, item
        quote = by_kind['usage_estimate']['quote']
        assert quote['rates']['input_uncached'] == {'state': 'known', 'micros_per_million_tokens': 1234567}, quote
        assert quote['rates']['output'] == {'state': 'known', 'micros_per_million_tokens': 2345678}, quote
        # API-equivalent references keep their existing separate, explicit pricing contract.
        reference = by_kind['api_equivalent']
        assert reference['quote']['unknown_reasons'] == ['price_not_collected'], reference
        reference_rates = {key: {'state': 'known', 'micros_per_million_tokens': 3456789}
                           for key in ['input_uncached', 'output', 'cache_read', 'cache_write']}
        preview = product.cli('prices', 'override', 'preview', '--request-stdin', payload={
            **reference['edit_context'], 'currency': 'USD', 'valuation_kind': 'api_equivalent',
            'action': {'kind': 'set', 'rates': reference_rates}})['data']
        applied = product.cli('prices', 'override', 'apply', '--request-stdin', payload={
            'spec': preview['spec'], 'accept_digest': preview['change_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'feed-api-reference'})['data']
        assert applied['state'] == 'succeeded', applied
        configured = product.cli('prices', 'effective', '--request-stdin', payload={'targets': [{
            'query_id': 'api', 'target_locator': {'kind': 'binding', 'binding_id': binding},
            'currency': 'USD', 'valuation_kind': 'api_equivalent'}]})['data']
        assert not configured['pending_activation'], configured
        quote = configured['items'][0]['quote']
        assert quote['rates'] == reference_rates and quote['applied_override_refs'], quote
        print(json.dumps({'scenario': 'signed-feed-default-and-api-reference-prices', 'state': 'green'}), flush=True)
        discovered = product.cli('worker', 'dependencies', 'discover')['data']
        revision = next(x['revision'] for x in discovered['selection_revisions'] if x['harness'] == 'codex_cli')
        product.cli('worker', 'dependencies', 'select', '--request-stdin', payload={'harness': 'codex_cli',
            'adapter_path': str(product.adapter), 'cli_path': str(product.codex), 'expected_selection_revision': revision})
        codex = next(x for x in product.cli('agents', 'scan')['data']['agents'] if x['agent_id'] == 'agent_codex_default')
        product.cli('agents', 'check', 'agent_codex_default', '--scope', 'native-authentication')
        spec = {'schema_version': {'major': 2, 'minor': 0}, 'context_id': codex['codex_access']['root_context_id'],
            'model': {'intent': 'configure', 'settings': {'mode': 'codex_default', 'native_model_mode': 'hiroute_only',
                'fixed_models': [], 'allowed_plan_ids': [plan], 'default_selection': {'kind': 'plan', 'plan_id': plan}}}}
        preview = product.cli('agents', 'connect', 'preview', '--request-stdin', payload={'spec': spec})['data']
        applied = product.cli('agents', 'connect', 'apply', '--request-stdin', payload={
            'spec': preview['spec'], 'accept_digest': preview['accept_digest'], 'dependency_digest': preview['dependency_digest'],
            'expected_revisions': preview['expected_revisions'], 'idempotency_key': 'feed-agent'})
        assert applied['operation']['state'] == 'succeeded', applied
        main_session(product)
        prior = worker(product, plan, 'feed-before')
        prior = worker(product, plan, 'feed-before-continue', prior)
        new_draft = {'change': plan_change('update', editor, plan, frozen['head']['head_revision'])}
        assert product.cli('routing', 'preview', '--request-stdin', payload=new_draft)['status'] == 'succeeded'
        stage = 'withdraw-remove-restart'
        print(json.dumps({'scenario': 'installed-signed-feed-lifecycle', 'stage': stage}), flush=True)
        withdrawn = publication(product, 18, False)
        (fixture / 'publication.json').write_bytes(wire(withdrawn))
        assert product.cli('models', 'feed', 'refresh')['data']['held']['sequence'] == 18
        removal = product.cli('models', 'feed', 'remove', 'preview')['data']
        product.cli('models', 'feed', 'remove', 'apply', '--request-stdin', payload={
            'accepted_digest': removal['digest'], 'also_disable_retrieval': True})
        restart(product)
        assert product.cli('models', 'feed', 'status')['data']['highest_accepted_sequence'] == 18
        stage = 'frozen-execution-after-withdrawal'
        print(json.dumps({'scenario': 'installed-signed-feed-lifecycle', 'stage': stage}), flush=True)
        main_session(product)
        worker(product, plan, 'feed-new-after')
        worker(product, plan, 'feed-continue-after', prior)
        retained = product.cli('routing', 'show', plan)['data']
        assert retained['head'] == frozen['head'] and retained['desired'] == frozen['desired'], 'published route changed'
        stage = 'new-admission-rejected'
        print(json.dumps({'scenario': 'installed-signed-feed-lifecycle', 'stage': stage}), flush=True)
        available = product.cli('models', 'availability')['data']
        retained_model = next(x for x in available['models'] if x['model_configuration_id'] == MODEL)
        assert retained_model['availability']['kind'] == 'used_by_published_plans' and not retained_model['binding_ids'], retained_model
        options = product.cli('routing', 'options', '--request-stdin', payload={})['data']
        assert all(x['binding_id'] != binding for x in options['candidates']), options
        # Exactly the same previously accepted draft must stop compiling with withdrawn facts.
        rejected = raw(product, 'routing', 'preview', payload=new_draft)
        assert rejected['status'] != 'succeeded', rejected
        snapshot = product.cli('compute', 'list')['data']
        saved = next(x for x in snapshot['sources'] if x['source_id'] == source['source_id'])
        change = {'schema': 'hiroute.compute-management-change/v2', 'subject': {'kind': 'saved_source', 'source_id': source['source_id']},
            'expected_revisions': snapshot['revisions'], 'selected_model_refs': [m['model_ref'] for m in saved['models']],
            'intent': 'save_ready', 'key_edits': []}
        rejected = raw(product, 'compute', 'connection', 'preview', payload={'change': change})
        assert rejected['status'] != 'succeeded' and rejected['error']['code'] != 'INVALID_ARGUMENTS', rejected
        recheck = copy.deepcopy(checked_request)
        recheck['request'].update(expected_catalog=product.cli('compute', 'connection', 'options')['data']['catalog'],
            existing_source_id=source['source_id'], expected_source_revision=saved['revision'], check_id='check/feed/withdrawn')
        rejected = raw(product, 'compute', 'connection', 'test', payload=recheck)
        assert rejected['status'] != 'succeeded' or not any(m['selectable'] for m in rejected['data']['candidate']['models']), rejected
        stage = 'rollback-and-identical-restoration'
        print(json.dumps({'scenario': 'installed-signed-feed-lifecycle', 'stage': stage}), flush=True)
        (fixture / 'publication.json').write_bytes(wire(included))
        status = product.cli('models', 'feed', 'refresh')['data']
        assert status['last_attempt']['reason'] == 'older_sequence' and status['held'] is None, status
        (fixture / 'publication.json').write_bytes(wire(withdrawn))
        assert product.cli('models', 'feed', 'refresh')['data']['held']['sequence'] == 18
        conflicting = publication(product, 18)
        (fixture / 'publication.json').write_bytes(wire(conflicting))
        assert product.cli('models', 'feed', 'refresh')['data']['last_attempt']['reason'] == 'same_sequence_conflicting_digest'
        stage = 'embedded-absorption'
        print(json.dumps({'scenario': 'installed-signed-feed-lifecycle', 'stage': stage}), flush=True)
        restored = publication(product, 19)
        (fixture / 'publication.json').write_bytes(wire(restored))
        assert product.cli('models', 'feed', 'refresh')['data']['held']['sequence'] == 19
        (fixture / 'absorbed.json').write_bytes(wire(restored))
        restart(product)
        absorbed = product.cli('models', 'feed', 'status')['data']
        assert absorbed['counts']['ignored_embedded'] >= 3 and absorbed['counts']['applied'] == 0, absorbed
        main_session(product)
        admitted = product.cli('routing', 'preview', '--request-stdin', payload=new_draft)
        assert admitted['status'] == 'succeeded', 'absorption must admit a new draft without rechecking the saved source'
        assert len(upstream.requests) >= 7 and all(x['model'] == UPSTREAM for x in upstream.requests)
        print(json.dumps({'scenario': 'installed-signed-feed-lifecycle', 'state': 'green', 'candidate': product.sha,
            'upstream_calls': len(upstream.requests), 'published_session_after_withdrawal': True,
            'new_worker_after_withdrawal': True, 'continue_after_restart': True, 'new_admission_rejected': True,
            'rollback_and_restore': True, 'embedded_absorption': True}))
    except Exception as error:
        preserve_failure_diagnostics(product, stage, upstream)
        raise AssertionError(('feed lifecycle stage', stage, error)) from error
    finally:
        upstream.close()
        product.close()


if __name__ == '__main__':
    run(sys.argv[1])
