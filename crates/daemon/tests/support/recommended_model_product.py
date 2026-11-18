"""Candidate feed -> registered Qwen4 connection -> published trial on current/released binaries.

The repository argument owns the unmodified product binaries and package assets. --feed can
select a newer publication without rebuilding that product. Only the feed download and the
external provider/Agent are fixtures; checks, saves, publication and execution use public CLI.
"""
import argparse
import http.server
import json
from pathlib import Path
import ssl
import socket
import subprocess
import threading

from model_feed_product import restart, worker
from standalone_headless_product import InstalledProduct, apply_from_preview, wire

MODEL = 'model.catalog.qwen-4-max'
UPSTREAM = 'qwen4-max'
HOSTS = ('dashscope.aliyuncs.com', 'token-plan.cn-beijing.maas.aliyuncs.com')


class TrialProduct(InstalledProduct):
    def cli(self, *arguments, **kwargs):
        return super().cli(*arguments, '--error-details', **kwargs)


class Provider:
    """Private HTTPS provider reached through a process-local DNS/socket test fixture."""
    def __init__(self, root):
        self.requests = []
        self.reject = False
        self.models = [UPSTREAM]
        config = root / 'openssl.cnf'
        config.write_text('[req]\ndistinguished_name=subject\n[subject]\n')
        self.ca = root / 'ca.pem'
        ca_key = root / 'ca-key.pem'
        key = root / 'key.pem'
        certificate = root / 'provider.pem'
        subprocess.run(['openssl', 'req', '-config', str(config), '-x509', '-newkey', 'rsa:2048',
                        '-nodes', '-keyout', str(ca_key), '-out', str(self.ca), '-days', '1',
                        '-subj', '/CN=HiRoute isolated provider',
                        '-addext', 'basicConstraints=critical,CA:TRUE'],
                       capture_output=True, check=True, timeout=30)
        csr = root / 'provider.csr'
        subprocess.run(['openssl', 'req', '-config', str(config), '-new', '-newkey', 'rsa:2048',
                        '-nodes', '-keyout', str(key), '-out', str(csr), '-subj', '/CN=' + HOSTS[0]],
                       capture_output=True, check=True, timeout=30)
        extensions = root / 'extensions.cnf'
        extensions.write_text('basicConstraints=critical,CA:FALSE\nsubjectAltName=' +
                              ','.join('DNS:' + h for h in HOSTS) + '\n')
        subprocess.run(['openssl', 'x509', '-req', '-in', str(csr), '-CA', str(self.ca),
                        '-CAkey', str(ca_key), '-CAcreateserial', '-out', str(certificate),
                        '-days', '1', '-extfile', str(extensions)],
                       capture_output=True, check=True, timeout=30)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(certificate, key)
        context.set_alpn_protocols(['http/1.1'])
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *_args):
                pass

            def reply(self, value, status=200):
                raw = wire(value)
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(raw)))
                self.send_header('Connection', 'close')
                self.end_headers()
                self.wfile.write(raw)

            def record(self, body=None):
                owner.requests.append({'method': self.command, 'host': self.headers.get('Host'),
                                       'path': self.path, 'model': (body or {}).get('model')})
                # Record no authorization material, including fixture credentials.
                if owner.reject:
                    self.reply({'error': {'type': 'invalid_api_key'}}, 401)
                    return False
                assert self.headers.get('Authorization', '').startswith('Bearer ')
                return True

            def do_GET(self):
                if self.record():
                    assert self.path == '/api/v1/models', self.path
                    self.reply({'object': 'list', 'data': [{'id': m, 'object': 'model'} for m in owner.models]})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                if not self.record(body):
                    return
                assert body['model'] == UPSTREAM, body
                probe = any(t.get('function', t).get('name') == 'hiroute_probe' for t in body.get('tools', []))
                if self.path.endswith('/chat/completions'):
                    message = {'role': 'assistant', 'content': 'native product answer'}
                    if probe:
                        message['tool_calls'] = [{'id': 'probe', 'type': 'function', 'function': {
                            'name': 'hiroute_probe', 'arguments': '{"value":"ok"}'}}]
                    value = {'id': 'chatcmpl_trial', 'object': 'chat.completion', 'created': 1, 'model': UPSTREAM,
                             'choices': [{'index': 0, 'message': message, 'finish_reason': 'tool_calls' if probe else 'stop'}],
                             'usage': {'prompt_tokens': 4, 'completion_tokens': 3, 'total_tokens': 7}}
                elif self.path.endswith('/responses'):
                    value = {'id': 'resp_trial', 'object': 'response', 'status': 'completed', 'model': UPSTREAM,
                             'output': [{'id': 'msg_trial', 'type': 'message', 'role': 'assistant', 'status': 'completed',
                                         'content': [{'type': 'output_text', 'text': 'native product answer', 'annotations': []}]}],
                             'usage': {'input_tokens': 4, 'output_tokens': 3, 'total_tokens': 7}}
                else:
                    assert self.path.endswith('/messages'), self.path
                    value = {'id': 'msg_trial', 'type': 'message', 'role': 'assistant', 'model': UPSTREAM,
                             'content': [{'type': 'text', 'text': 'native product answer'}], 'stop_reason': 'end_turn',
                             'usage': {'input_tokens': 4, 'output_tokens': 3}}
                self.reply(value)

        self.direct = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.direct.socket = context.wrap_socket(self.direct.socket, server_side=True)
        self.direct_thread = threading.Thread(target=self.direct.serve_forever, daemon=True)
        self.direct_thread.start()
        self.dial = root / 'dial.json'
        self.dial.write_bytes(wire({'schema_version': 'hiroute.gateway.e2e-dial-map/v1',
            'targets': [{'authority': h, 'address': '127.0.0.1:' + str(self.direct.server_port),
                         'ca_pem': self.ca.read_text()} for h in HOSTS]}))
        # Keep released binaries and registry hosts untouched. Interpose only these two
        # provider names and their loopback TLS socket inside the isolated product process.
        # Reqwest retains its ordinary HTTPS/SNI/certificate and authentication checks.
        source = root / 'provider-transport.c'
        source.write_text(r'''
#define _GNU_SOURCE
#include <arpa/inet.h>
#include <dlfcn.h>
#include <netdb.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
int getaddrinfo(const char *name, const char *service, const struct addrinfo *hints, struct addrinfo **result) {
    int (*original)(const char *, const char *, const struct addrinfo *, struct addrinfo **) = dlsym(RTLD_NEXT, "getaddrinfo");
    if (name && (!strcmp(name, "dashscope.aliyuncs.com") || !strcmp(name, "token-plan.cn-beijing.maas.aliyuncs.com"))) name = "127.0.0.1";
    return original(name, service, hints, result);
}
int connect(int fd, const struct sockaddr *address, socklen_t length) {
    int (*original)(int, const struct sockaddr *, socklen_t) = dlsym(RTLD_NEXT, "connect");
    const char *port = getenv("HIROUTE_RECOMMENDATION_TLS_PORT");
    if (port && address && length >= sizeof(struct sockaddr_in) && address->sa_family == AF_INET) {
        struct sockaddr_in target = *(const struct sockaddr_in *)address;
        if (target.sin_addr.s_addr == htonl(INADDR_LOOPBACK) && target.sin_port == htons(443)) {
            target.sin_port = htons((unsigned short)atoi(port));
            return original(fd, (const struct sockaddr *)&target, sizeof(target));
        }
    }
    return original(fd, address, length);
}
''')
        self.transport = root / 'provider-transport.so'
        subprocess.run(['cc', '-shared', '-fPIC', '-Wall', '-Wextra', '-Werror', str(source),
                        '-o', str(self.transport), '-ldl'], check=True, capture_output=True, timeout=30)

    def close(self):
        self.direct.shutdown()
        self.direct.server_close()
        self.direct_thread.join(timeout=5)


def run(repository, feed, evidence=None, browser=False, selected_option=None, baseline_feed=None):
    product = TrialProduct(repository)
    provider = Provider(product.root)
    stage = 'install'
    observations = {}
    try:
        for name in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY',
                     'http_proxy', 'https_proxy', 'all_proxy', 'no_proxy'):
            product.env.pop(name, None)
        fixture = product.root / 'feed'
        fixture.mkdir()
        fixture.joinpath('publication.json').write_bytes(Path(baseline_feed or feed).read_bytes())
        publication = json.loads(Path(feed).read_bytes())
        product.env.update(HIROUTE_TEST_MODEL_FEED_ROOT=str(fixture),
                           LD_PRELOAD=str(provider.transport), SSL_CERT_FILE=str(provider.ca),
                           HIROUTE_RECOMMENDATION_TLS_PORT=str(provider.direct.server_port),
                           HIROUTE_E2E_DIAL_CONFIG=str(provider.dial))
        product.install()
        if browser:
            assert evidence is not None, '--browser requires --evidence for screenshots'
            with socket.socket() as listener:
                listener.bind(('127.0.0.1', 0))
                port = listener.getsockname()[1]
            product.cli('console', 'init', '--mode', 'loopback', '--port', str(port))
        stage = 'feed-refresh'
        product.start()
        status = product.cli('models', 'feed', 'refresh')['data']
        runner = Path(__file__).resolve().parents[4] / 'apps/desktop/tests/v3/visual/released-recommendation-trial.mjs'
        if baseline_feed:
            before = product.cli('models', 'availability')['data']
            model = next(m for m in before['models'] if m['model_configuration_id'] == MODEL)
            assert model['availability']['kind'] == 'descriptive_only' and not model['binding_ids'], model
            launch = product.cli('console', 'open')['data']
            subprocess.run(['node', str(runner), str(Path(evidence) / 'before'), selected_option],
                           input=wire({'url': launch['url'], 'expectedDisabled': True}), check=True, timeout=90)
            fixture.joinpath('publication.json').write_bytes(Path(feed).read_bytes())
            status = product.cli('models', 'feed', 'refresh')['data']
        assert status['selected']['data_revision'] == publication['data_revision'], status
        # The released test feed transport has a fixed synthetic directory. Disable it after
        # refresh; all following account checks use the real released HTTPS provider client.
        product.env.pop('HIROUTE_TEST_MODEL_FEED_ROOT')
        restart(product)
        stage = 'recommendation-options'
        before = product.cli('models', 'availability')['data']
        recommendation = next(m for m in before['models'] if m['model_configuration_id'] == MODEL)
        assert before['featured'][0]['model_configuration_id'] == MODEL, before['featured']
        assert recommendation['availability']['kind'] == 'needs_connection' and not recommendation['binding_ids'], recommendation
        observations['before'] = before
        choices = product.cli('compute', 'connection', 'options')['data']
        observations['options'] = choices
        options = [o for o in choices['options'] if MODEL in o['model_configuration_ids']]
        assert {o['connection_option_id'] for o in options} == {'bailian.payg.cn.v1'}, options
        if selected_option:
            options = [o for o in options if o['connection_option_id'] == selected_option]
            assert options, 'requested connection option is unavailable'
        discovered = product.cli('worker', 'dependencies', 'discover')['data']
        revision = next(x['revision'] for x in discovered['selection_revisions'] if x['harness'] == 'codex_cli')
        product.cli('worker', 'dependencies', 'select', '--request-stdin', payload={'harness': 'codex_cli',
                    'adapter_path': str(product.adapter), 'cli_path': str(product.codex), 'expected_selection_revision': revision})
        for option in options:
            option_id = option['connection_option_id']
            stage = option_id
            print(json.dumps({'scenario': 'recommended-qwen4-registered-trial', 'stage': stage,
                              'product_revision': product.sha}), flush=True)
            saved_before = {s['source_id'] for s in product.cli('compute', 'list')['data']['sources']}
            candidate_ref = 'candidate/recommendation/' + option_id
            product.register_secret(candidate_ref, 'isolated-recommendation-key')
            product.secrets.add('isolated-recommendation-key')
            request = {'kind': 'registered', 'request': {
                'connection_option_id': option_id, 'display_name': option_id,
                'expected_catalog': product.cli('compute', 'connection', 'options')['data']['catalog'],
                'candidate_ref': candidate_ref, 'lineage_ref': 'lineage/' + option_id,
                'edit_revision': 1, 'check_id': 'check/' + option_id,
                'input_candidate': {'candidate_ref': candidate_ref, 'candidate_revision': 1},
                'inference_model_id': UPSTREAM}}
            # Rejected credentials cannot become a connected recommendation.
            provider.reject = True
            network_edit = {'expected_network_revision': 0, 'policy': {'mode': 'direct'}}
            network_request = {'kind': 'registered', 'request': {'request': request['request'], 'network_edit': network_edit}}
            denied = product.cli('compute', 'connection', 'test-network', '--request-stdin', payload=network_request)['data']['check']
            assert denied['authentication'] == 'rejected' and denied['inference'] != 'verified', denied
            assert {s['source_id'] for s in product.cli('compute', 'list')['data']['sources']} == saved_before, 'a failed check saved a source'
            provider.reject = False
            request['request'].update(edit_revision=2, check_id='check/verified/' + option_id)
            checked = product.cli('compute', 'connection', 'test-network', '--request-stdin', payload=network_request)['data']['check']
            assert checked['inference'] == 'verified' and checked['inference_model_id'] == UPSTREAM, checked
            matching = [m for m in checked['candidate']['models'] if m['upstream_model_id'] == UPSTREAM]
            assert len(matching) == 1 and matching[0]['selectable'], checked
            # Follow the old form's exact selected subset, never every model in a catalog.
            candidate = dict(checked['candidate'], models=matching)
            if browser:
                launch = product.cli('console', 'open')['data']
                subprocess.run(['node', str(runner), str(Path(evidence) / option_id), option_id],
                               input=wire({'url': launch['url']}), check=True, timeout=180)
            else:
                change = {'schema': 'hiroute.compute-management-change/v2',
                    'subject': {'kind': 'candidate', 'candidate': candidate['candidate']},
                    'expected_revisions': product.cli('compute', 'list')['data']['revisions'],
                    'selected_model_refs': [m['model_ref'] for m in matching], 'intent': 'save_ready', 'key_edits': []}
                preview = product.cli('compute', 'connection', 'preview-network', '--request-stdin', payload={
                    'schema': 'hiroute.compute-management-change/v4', 'change': change, 'network_edit': network_edit})['data']
                apply_from_preview(product, ('compute', 'connection', 'apply'), preview, 'recommendation-save-' + option_id)
            source = next(s for s in product.cli('compute', 'list')['data']['sources'] if s['display_name'] == option_id)
            member, = [m for m in source['models'] if m['upstream_model_id'] == UPSTREAM]
            assert member['catalog_configuration_id'] == MODEL, member
            available = product.cli('models', 'availability')['data']
            model = next(m for m in available['models'] if m['model_configuration_id'] == MODEL)
            assert model['availability']['kind'] == 'executable' and member['binding_id'] in model['binding_ids'], model
            observations[option_id] = {'checked': checked, 'source': source, 'availability': available,
                                       'routing_options': product.cli('routing', 'options', '--request-stdin', payload={})['data']}
            if evidence:
                Path(evidence).mkdir(parents=True, exist_ok=True)
                (Path(evidence) / 'observations.json').write_bytes(wire(observations))
            apply_file = product.root / (option_id + '-route.json')
            created = product.cli('routing', 'preview', '--name', 'Qwen4 trial ' + option_id.split('.')[1],
                '--purpose', 'Try the recommended model', '--mode', 'fixed_model', '--model', member['binding_id'],
                '--delegation', 'enable', '--worker-harness', 'codex_cli', '--worker-protocol', 'responses',
                '--apply-file', str(apply_file))['data']['preview']
            product.cli('routing', 'apply', '--request-file', str(apply_file))
            plan = created['plan_head']['reference']['plan_id']
            worker(product, plan, 'recommendation-execute-' + option_id)
        report = {'scenario': 'recommended-qwen4-registered-trial', 'state': 'green', 'product_revision': product.sha,
                  'feed_revision': publication['data_revision'], 'sequence': publication['sequence'],
                  'options': [o['connection_option_id'] for o in options], 'provider_requests': provider.requests}
        if evidence:
            evidence = Path(evidence)
            evidence.mkdir(parents=True, exist_ok=True)
            (evidence / 'native.json').write_bytes(wire(dict(report, observations=observations)))
        print(json.dumps(report), flush=True)
    except Exception:
        print(json.dumps({'scenario': 'recommended-qwen4-registered-trial', 'state': 'red', 'stage': stage,
                          'product_revision': product.sha, 'provider_requests': provider.requests}), flush=True)
        raise
    finally:
        try:
            product.close()
        finally:
            provider.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('repository', type=Path)
    parser.add_argument('--feed', type=Path)
    parser.add_argument('--evidence', type=Path)
    parser.add_argument('--browser', action='store_true')
    parser.add_argument('--option', choices=['bailian.payg.cn.v1'])
    parser.add_argument('--baseline-feed', type=Path)
    args = parser.parse_args()
    if args.browser and (not args.option or not args.evidence):
        parser.error('--browser requires --option and --evidence to keep each trial isolated')
    if args.baseline_feed and not args.browser:
        parser.error('--baseline-feed requires --browser')
    run(args.repository, args.feed or args.repository / 'assets/model-feed/payload.json', args.evidence, args.browser, args.option, args.baseline_feed)
