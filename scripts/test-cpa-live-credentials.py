#!/usr/bin/env python3
"""Regression: real-account smoke must never acquire native refresh authority."""
import json
from datetime import datetime, timezone
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "crates/daemon/tests/support"))
from cpa_luna_gateway_live import copy_private_codex_auth


class AccessOnlyCopy(unittest.TestCase):
    def test_refresh_and_unknown_secret_fields_never_leave_native_store(self):
        with tempfile.TemporaryDirectory() as root:
            source, destination = Path(root) / "native.json", Path(root) / "isolated.json"
            source.write_text(json.dumps({
                "auth_mode": "chatgpt", "future_secret": "never-copy-top-level",
                "last_refresh": "native-metadata-must-not-be-copied",
                "tokens": {"access_token": "borrowed-access", "id_token": "identity",
                           "account_id": "account", "refresh_token": "native-refresh-only",
                           "future_secret": "never-copy-nested"}}))
            source.chmod(0o600)
            original = source.read_bytes()
            before = datetime.now(timezone.utc)
            secrets = copy_private_codex_auth(source, destination)
            self.assertEqual(source.read_bytes(), original)
            isolated = json.loads(destination.read_bytes())
            refreshed = datetime.fromisoformat(isolated.pop('last_refresh').replace('Z', '+00:00'))
            self.assertLessEqual(before, refreshed)
            self.assertLessEqual(refreshed, datetime.now(timezone.utc))
            self.assertEqual(isolated, {
                "auth_mode": "chatgpt", "tokens": {"access_token": "borrowed-access",
                                                     "id_token": "identity", "account_id": "account"}})
            self.assertEqual(destination.stat().st_mode & 0o777, 0o600)
            self.assertNotIn("native-refresh-only", secrets)
            self.assertNotIn(b"native-refresh-only", destination.read_bytes())


if __name__ == "__main__":
    unittest.main()
