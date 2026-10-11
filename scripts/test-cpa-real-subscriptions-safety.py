#!/usr/bin/env python3
"""Regression proof that filtering happens before native credentials leave a host."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'real_subscription_acceptance', Path(__file__).with_name('test-cpa-real-subscriptions.py'))
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)


class NativeHostFilter(unittest.TestCase):
    def filtered(self, kind, source, expected):
        with tempfile.TemporaryDirectory() as name:
            home = Path(name)
            directory = home / ('.codex' if kind == 'codex' else '.claude')
            directory.mkdir(mode=0o700)
            path = directory / ('auth.json' if kind == 'codex' else '.credentials.json')
            path.write_text(json.dumps(source))
            path.chmod(0o600)
            before = path.read_bytes()
            result = subprocess.run(
                [sys.executable, '-c', acceptance.REMOTE_READ, kind, 'borrow'],
                env={**os.environ, 'HOME': str(home)}, capture_output=True, check=True)
            self.assertEqual(path.read_bytes(), before)
            self.assertNotIn(b'native-refresh-only', result.stdout)
            self.assertNotIn(b'unknown-secret-only', result.stdout)
            parsed = json.loads(result.stdout)
            self.assertEqual(parsed['credential'], expected)
            self.assertFalse(acceptance.forbidden_refresh(parsed['credential']))
            self.assertEqual(parsed['mode'], '0o600')

    def test_codex_omits_refresh_and_unknown_fields_on_source_host(self):
        self.filtered('codex', {
            'auth_mode': 'chatgpt', 'unknown': 'unknown-secret-only',
            'tokens': {'access_token': 'access', 'id_token': 'id', 'account_id': 'account',
                       'refresh_token': 'native-refresh-only', 'unknown': 'unknown-secret-only'}},
            {'auth_mode': 'chatgpt', 'tokens': {'access_token': 'access', 'id_token': 'id', 'account_id': 'account'}})

    def test_claude_omits_refresh_and_unknown_fields_on_source_host(self):
        self.filtered('claude', {
            'unknown': 'unknown-secret-only', 'claudeAiOauth': {
                'accessToken': 'access', 'expiresAt': 1900000000000, 'scopes': ['user:inference'],
                'refreshToken': 'native-refresh-only', 'unknown': 'unknown-secret-only'}},
            {'claudeAiOauth': {'accessToken': 'access', 'expiresAt': 1900000000000, 'scopes': ['user:inference']}})

    def test_unprotected_native_file_transfers_no_credential(self):
        with tempfile.TemporaryDirectory() as name:
            home = Path(name)
            (home / '.codex').mkdir(mode=0o700)
            path = home / '.codex/auth.json'
            path.write_text(json.dumps({'tokens': {'refresh_token': 'native-refresh-only'}}))
            path.chmod(0o644)
            result = subprocess.run(
                [sys.executable, '-c', acceptance.REMOTE_READ, 'codex', 'borrow'],
                env={**os.environ, 'HOME': str(home)}, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b'')
            self.assertNotIn(b'native-refresh-only', result.stderr)

    def test_symlink_native_file_transfers_no_credential(self):
        with tempfile.TemporaryDirectory() as name:
            home = Path(name)
            (home / '.codex').mkdir(mode=0o700)
            held = home / 'native-owner-only.json'
            held.write_text(json.dumps({'tokens': {'refresh_token': 'native-refresh-only'}}))
            held.chmod(0o600)
            (home / '.codex/auth.json').symlink_to(held)
            before = held.read_bytes()
            result = subprocess.run(
                [sys.executable, '-c', acceptance.REMOTE_READ, 'codex', 'borrow'],
                env={**os.environ, 'HOME': str(home)}, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b'')
            self.assertEqual(held.read_bytes(), before)
            self.assertNotIn(b'native-refresh-only', result.stderr)

    def test_gateway_error_classifies_root_code_without_secret_message(self):
        payload = json.dumps({'code': 'PROTOCOL_SEMANTICS_UNSUPPORTED', 'phase': 'canonical_request',
                              'error': {'type': 'invalid_request_error',
                                        'message': 'Rejected borrowed-access-only'}}).encode()
        self.assertEqual(acceptance.safe_gateway_error(payload, {'borrowed-access-only'}), {
            'code': 'PROTOCOL_SEMANTICS_UNSUPPORTED', 'phase': 'canonical_request',
            'type': 'invalid_request_error'})

    def test_gateway_error_omits_profile_email(self):
        payload = json.dumps({'error': {'type': 'invalid_request_error',
                                       'message': 'Failure for private@example.invalid'}}).encode()
        self.assertEqual(acceptance.safe_gateway_error(payload, set()), {'type': 'invalid_request_error'})

    def test_native_integrity_mismatch_cannot_leave_a_green_verdict(self):
        before = {'codex': {'sha256': 'a' * 64}}
        report = {'state': 'green', 'native_sources': before, 'scenarios': []}
        with patch.object(acceptance, 'remote_source', return_value={'sha256': 'b' * 64}):
            acceptance.audit_native_sources(report, before, {'codex': 'fixture-host'})
        self.assertEqual(report['state'], 'red')
        self.assertFalse(report['native_sources']['codex']['unchanged'])
        self.assertEqual(report['scenarios'][-1]['state'], 'red')

    def test_unavailable_final_snapshot_is_red_and_still_checks_other_source(self):
        before = {'codex': {'sha256': 'a' * 64}, 'claude': {'sha256': 'b' * 64}}
        report = {'state': 'green', 'native_sources': before, 'scenarios': []}
        with patch.object(acceptance, 'remote_source', side_effect=[
                RuntimeError('must-not-leak-secret'), {'sha256': 'b' * 64}]) as read:
            acceptance.audit_native_sources(report, before, {'codex': 'fixture-a', 'claude': 'fixture-b'})
        self.assertEqual(read.call_count, 2)
        self.assertEqual(report['state'], 'red')
        self.assertEqual(report['native_sources']['codex']['after_snapshot'], 'unavailable')
        self.assertTrue(report['native_sources']['claude']['unchanged'])
        self.assertNotIn('must-not-leak-secret', json.dumps(report))

    def test_unchanged_native_sources_preserve_the_existing_verdict(self):
        for state in ['green', 'red']:
            before = {'codex': {'sha256': 'a' * 64}}
            report = {'state': state, 'native_sources': before, 'scenarios': []}
            with patch.object(acceptance, 'remote_source', return_value={'sha256': 'a' * 64}):
                acceptance.audit_native_sources(report, before, {'codex': 'fixture-host'})
            self.assertEqual(report['state'], state)
            self.assertTrue(report['native_sources']['codex']['unchanged'])


if __name__ == '__main__':
    unittest.main()
