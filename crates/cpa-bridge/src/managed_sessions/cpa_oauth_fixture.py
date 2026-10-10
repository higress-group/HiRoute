#!/usr/bin/python3
"""Synthetic external CPA process for private login lifetime regressions.

All credentials and callback codes are fixtures. HiRoute's registry, locks, process
supervision and routing admission remain real; this server does not implement
HiRoute's control API or saved state. Never point it at a native credential store.
"""
import http.server
import json
import os
from pathlib import Path
import re
import sys
import threading
import time
from urllib.parse import parse_qs, urlparse

VERSION = '__HIR_CPA_VERSION__'
if '--help' in sys.argv:
    print('CLIProxyAPI Version: ' + VERSION)
    sys.exit(0)

controls = Path(__file__).resolve().parent
config = Path(sys.argv[sys.argv.index('--config') + 1]).read_text()


def scalar(key):
    match = re.search(r'^\s*' + re.escape(key) + r':\s*(.+)$', config, re.M)
    return match.group(1).strip().strip('"\'')


auth_dir = Path(scalar('auth-dir'))
management = scalar('secret-key')
assert '--local-password-stdin' in sys.argv
assert sys.stdin.read() == management
downstream = re.search(r'api-keys:\s*\n\s*-\s*(\S+)', config).group(1).strip('"\'')
state = 'fixture_state_' + auth_dir.parent.name
provider = None
completed = False
guard = threading.Lock()
with (controls / 'spawns.jsonl').open('a') as log:
    log.write(json.dumps({'pid': os.getpid(), 'login_ref': auth_dir.parent.name}) + '\n')


def credential():
    for path in auth_dir.glob('*.json'):
        return path, json.loads(path.read_text())
    return None, None


def record_catalog(event, **facts):
    if not (controls / 'record-model-catalog').exists():
        return
    descriptor = os.open(controls / 'catalog.jsonl',
                         os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    with os.fdopen(descriptor, 'w') as stream:
        stream.write(json.dumps({'pid': os.getpid(), 'event': event, **facts}) + '\n')


def write_credential(kind):
    global completed
    delay = controls / 'delay-write'
    if delay.exists():
        (auth_dir.parent / 'exchange-started').touch()
        time.sleep(float(delay.read_text()))
    subject = kind + '-fixture-account'
    value = {
        'type': kind, 'access_token': 'fixture-managed-access',
        'refresh_token': 'fixture-managed-refresh',
        'expired': '2000-01-01T00:00:00Z',
        'account_id' if kind == 'codex' else 'account_uuid': subject,
    }
    # Deliberately allow a late exchange to recreate its store. The registry must
    # terminate the owned process before deleting it, rather than trusting DELETE.
    auth_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    path = auth_dir / 'credential.json'
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, 'w') as stream:
        json.dump(value, stream)
    with guard:
        completed = True
    (auth_dir.parent / 'exchange-finished').touch()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def log_message(self, *_):
        pass

    def send(self, value, status=200, version=VERSION):
        body = json.dumps(value).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        if version is not None:
            self.send_header('x-cpa-version', version)
        self.end_headers()
        self.wfile.write(body)

    def authorized(self, secret):
        if self.headers.get('Authorization') != 'Bearer ' + secret:
            self.send({'error': 'unauthorized'}, 401)
            return False
        return True

    def do_GET(self):
        global provider
        parsed = urlparse(self.path)
        query = parse_qs(parsed.query)
        if parsed.path == '/healthz':
            return self.send({'status': 'ok'})
        secret = management if parsed.path.startswith(('/v0/', '/v8/')) else downstream
        if not self.authorized(secret):
            return
        if parsed.path == '/v8/management/oauth/auth-url':
            assert query.get('is_webui') == ['false']
            provider = query['provider'][0]
            assert provider in ('codex', 'claude')
            host = 'auth.openai.com' if provider == 'codex' else 'claude.ai'
            return self.send({'status': 'ok', 'state': state,
                              'url': 'https://' + host + '/oauth/authorize?state=' + state
                                     + '&code_challenge_method=S256'},
                             version=None if (controls / 'missing-auth-url-version').exists()
                             else VERSION)
        if parsed.path == '/v8/management/oauth/status':
            assert query.get('state') == [state]
            with guard:
                return self.send({'status': 'ok' if completed else 'wait'},
                                 version=None if (controls / 'missing-status-version').exists()
                                 else VERSION)
        path, data = credential()
        if parsed.path == '/v1/models' or parsed.path.endswith('/models'):
            model = ('gpt-5.3-codex-spark' if data and data['type'] == 'codex'
                     else 'claude-sonnet-4-6')
            model_override = controls / 'catalog-model'
            if model_override.exists():
                model = model_override.read_text().strip()
            blocked = (controls / 'block-model-catalog').exists()
            models = ([{'id': data['prefix'] + '/' + model}]
                      if data and data.get('prefix') and not blocked else [])
            record_catalog('models', route=parsed.path, blocked=blocked,
                           model_count=len(models))
            return self.send({'data' if parsed.path == '/v1/models' else 'models': models})
        if parsed.path in ('/v0/management/auth-files', '/v8/management/credentials'):
            name = query.get('name', [''])[0]
            if name == '.__hiroute_ready_probe__' or data is None:
                return self.send({'files': []})
            record_catalog('inventory')
            if (controls / 'catalog-auth-unavailable').exists():
                return self.send({'error': 'fixture inventory unavailable'}, 503)
            assert path.name == name
            return self.send({'files': [{
                'id': path.name, 'name': path.name, 'auth_index': 'fixture-index',
                'provider': data['type'], 'source': 'file', 'runtime_only': False,
                'account_type': 'oauth', 'status': 'active', 'disabled': False,
                'unavailable': False, 'status_message': '',
                'request_retry': data.get('request_retry', 0),
            }]})
        self.send({'error': 'unexpected fixture route'}, 404)

    def do_POST(self):
        if not self.authorized(management):
            return
        assert self.path == '/v8/management/oauth/callback'
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        assert body['provider'] == provider and body['state'] == state
        assert body['code'] == 'fixture-authorization-code'
        with (controls / 'callbacks.jsonl').open('a') as log:
            log.write(json.dumps({'login_ref': auth_dir.parent.name}) + '\n')
        if (controls / 'delay-write').exists():
            threading.Thread(target=write_credential, args=(provider,), daemon=True).start()
        else:
            write_credential(provider)
        # Pinned v8 callback is a public OAuth ACK outside the management middleware.
        # Its missing version header must exercise the real bridge consumer contract.
        self.send({'status': 'ok'},
                  version='8.0.4-hiroute.999' if (controls / 'callback-version-mismatch').exists()
                  else None)

    def do_DELETE(self):
        if not self.authorized(management):
            return
        parsed = urlparse(self.path)
        assert parsed.path == '/v8/management/oauth/session'
        assert parse_qs(parsed.query).get('state') == [state]
        self.send({'status': 'ok'})

    def do_PATCH(self):
        if not self.authorized(management):
            return
        assert self.path == '/v0/management/auth-files/fields'
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        record_catalog('patch')
        path, data = credential()
        assert body.pop('name') == path.name
        data.update(body)
        path.write_text(json.dumps(data))
        path.chmod(0o600)
        self.send({})


server = http.server.ThreadingHTTPServer(('127.0.0.1', int(scalar('port'))), Handler)
server.daemon_threads = True
server.serve_forever()
