#!/usr/bin/env python3
"""Bounded GC regressions in disposable Git worktrees; no real cache deletion."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location("local_rust_gc", Path(__file__).with_name("local-rust-gc.py"))
gc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gc)


def git(*args):
    return subprocess.check_output(["git", *map(str, args)], text=True, stderr=subprocess.DEVNULL).strip()


class ManagedCheckoutGcTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.source = self.root / "source"
        git("init", "-q", self.source)
        git("-C", self.source, "config", "core.hooksPath", "/dev/null")
        git("-C", self.source, "config", "user.name", "Fixture")
        git("-C", self.source, "config", "user.email", "fixture@example.invalid")
        (self.source / ".gitignore").write_text("target/\napps/desktop/src-tauri/gen/\n")
        (self.source / "source.rs").write_text("tracked")
        git("-C", self.source, "add", ".")
        git("-C", self.source, "commit", "-qm", "fixture")
        self.store = gc.runner.Store(self.root / "local-rust", self.root / "validation.lock")

    def checkout(self, rid="a" * 32):
        path = self.store.root / "checkouts" / rid
        git("-C", self.source, "worktree", "add", "-q", "--detach", path)
        (path / "target/debug").mkdir(parents=True)
        (path / "target/debug/binary").write_text("rebuildable")
        (path / "target/evidence").mkdir()
        (path / "target/evidence/report.json").write_text('{"verdict":"red"}\n')
        row = dict(id=rid, kind="managed", repo=str(self.source), checkout=str(path),
                   sha=git("-C", path, "rev-parse", "HEAD"), status="terminal",
                   keep=False, evidence_saved=True, scenario="red", process_exit=1,
                   finished_at=time.time() - 72 * 3600,
                   identity=gc.runner.identity(path),
                   target_identity=gc.runner.identity(path / "target"),
                   debug_identity=gc.runner.identity(path / "target/debug"))
        self.store.save(row)
        return path, row

    def assert_retained(self, path, rid):
        report = gc.sweep(self.store, apply=True)
        self.assertEqual(report["removed"], [])
        self.assertIn(rid, [item["id"] for item in report["retained"]])
        self.assertEqual((path / "target/debug/binary").read_text(), "rebuildable")
        return report

    def test_preview_then_apply_exports_evidence_without_changing_verdict(self):
        path, row = self.checkout()
        before = self.store.record_path(row["id"]).read_bytes()
        preview = gc.sweep(self.store)
        self.assertEqual(preview["selected"], [row["id"]])
        self.assertEqual(preview["removed"], [])
        self.assertTrue(path.exists())
        self.assertEqual(self.store.record_path(row["id"]).read_bytes(), before)
        self.assertEqual(gc.sweep(self.store, apply=True)["removed"], [row["id"]])
        self.assertFalse(path.exists())
        self.assertEqual((self.store.root / "runs" / row["id"] /
                          "artifacts/evidence/report.json").read_text(), '{"verdict":"red"}\n')
        self.assertEqual(self.store.load(row["id"])["scenario"], "red")
        self.assertTrue(self.store.load(row["id"])["removed"])

    def test_keep_and_unconfirmed_evidence_cannot_expire(self):
        path, original = self.checkout()
        for changes in ({"keep": True}, {"evidence_saved": False},
                        {"pilot_reuse": {"key": "fixture"}, "pilot_cleanup_ready": False}):
            with self.subTest(changes=changes):
                self.store.save(dict(original, **changes))
                self.assert_retained(path, original["id"])

    def expiring_checkout(self):
        path, row = self.checkout()
        row.update(retention="always", keep=True, evidence_saved=False,
                   scenario="unassessed", finished_at=time.time() - 4 * 3600)
        self.store.save(row)
        return path, row

    def test_explicit_expiry_exports_evidence_and_reclaims_kept_build_without_assessment(self):
        path, row = self.expiring_checkout()
        original = self.store.record_path(row["id"]).read_bytes()
        preview = gc.sweep(self.store, expire_after_hours=3)
        self.assertEqual(preview["selected"], [row["id"]])
        self.assertEqual(self.store.record_path(row["id"]).read_bytes(), original)
        candidate = self.store.preview_one(row, expire_after_hours=3)
        self.assertEqual(candidate["expires_at"], row["finished_at"] + 3 * 3600)
        result = gc.sweep(self.store, apply=True, expire_after_hours=3)
        self.assertEqual(result["removed"], [row["id"]])
        self.assertFalse(path.exists())
        saved = self.store.load(row["id"])
        self.assertEqual(saved["scenario"], "unassessed")
        self.assertEqual(saved["process_exit"], 1)
        self.assertTrue(saved["evidence_saved"])
        self.assertFalse(saved["keep"])
        self.assertEqual(saved["cleanup"]["expire_after_hours"], 3)
        run = self.store.record_path(row["id"]).parent
        self.assertEqual((run / "artifacts/evidence/report.json").read_text(),
                         '{"verdict":"red"}\n')
        self.assertFalse((run / "artifacts/debug").exists())

    def test_recent_completion_or_renewal_delays_expiry(self):
        path, original = self.expiring_checkout()
        now = time.time()
        for changes in ({"finished_at": now - 2 * 3600},
                        {"retained_at": now - 2 * 3600}):
            with self.subTest(changes=changes):
                self.store.save(dict(original, **changes))
                report = gc.sweep(self.store, apply=True, expire_after_hours=3)
                self.assertEqual(report["selected"], [])
                self.assertTrue(path.exists())

    def test_keep_command_renews_retention(self):
        path, row = self.expiring_checkout()
        with patch.object(gc.runner.sys, "argv", ["local-rust", "keep", row["id"]]), \
                patch.object(gc.runner, "Store", return_value=self.store), patch("builtins.print"):
            self.assertEqual(gc.runner.main(), 0)
        renewed = self.store.load(row["id"])
        self.assertGreater(renewed["retained_at"], row["finished_at"])
        self.assertEqual(gc.sweep(self.store, apply=True, expire_after_hours=3)["removed"], [])
        self.assertTrue(path.exists())

    def test_renewal_between_preview_and_apply_is_rechecked(self):
        path, row = self.expiring_checkout()
        apply = self.store.apply

        def renew_then_apply(rid, token, **policy):
            current = self.store.load(rid)
            current["retained_at"] = time.time()
            self.store.save(current)
            return apply(rid, token, **policy)

        with patch.object(self.store, "apply", side_effect=renew_then_apply):
            report = gc.sweep(self.store, apply=True, expire_after_hours=3)
        self.assertEqual(report["removed"], [])
        self.assertIn("not expired", report["retained"][0]["reason"])
        self.assertTrue(path.exists())

    def test_expiry_policy_is_required_and_bound_to_preview_token(self):
        path, row = self.expiring_checkout()
        token = self.store.preview_one(row, expire_after_hours=3)["token"]
        with self.assertRaises(ValueError):
            self.store.apply(row["id"], token)
        with self.assertRaisesRegex(ValueError, "Candidate changed"):
            self.store.apply(row["id"], token, expire_after_hours=2)
        self.assertTrue(path.exists())

    def test_expiry_keeps_legacy_unconfirmed_evidence_and_open_pilot_ownership(self):
        path, original = self.expiring_checkout()
        legacy = dict(original)
        legacy.pop("retention")
        rows = [legacy, dict(original, pilot_reuse={"key": "fixture"}, pilot_cleanup_ready=False),
                dict(original, pilot_cleanup_ready=True, pilot_leases={"lease": {"state": "open"}})]
        for row in rows:
            with self.subTest(row=row):
                self.store.save(row)
                report = gc.sweep(self.store, apply=True, expire_after_hours=3)
                self.assertEqual(report["removed"], [])
                self.assertEqual(len(report["retained"]), 1)
                self.assertTrue(path.exists())

    def test_expiry_retains_active_changed_and_busy_checkouts(self):
        path, original = self.expiring_checkout()
        self.store.save(dict(original, status="running"))
        self.assertEqual(gc.sweep(self.store, apply=True, expire_after_hours=3)["removed"], [])
        self.store.save(original)
        (path / "source.rs").write_text("valuable changes")
        self.assertEqual(gc.sweep(self.store, apply=True, expire_after_hours=3)["removed"], [])
        (path / "source.rs").write_text("tracked")
        replaced = dict(original, identity=[original["identity"][0] + 1, original["identity"][1]])
        self.store.save(replaced)
        self.assertEqual(gc.sweep(self.store, apply=True, expire_after_hours=3)["removed"], [])
        self.store.save(original)
        with self.store.locked(path):
            self.assertEqual(gc.sweep(self.store, apply=True, expire_after_hours=3)["removed"], [])
        self.assertTrue(path.exists())

    def test_expiry_export_failure_preserves_original_build_and_verdict(self):
        path, row = self.expiring_checkout()
        outside = self.root / "unique-evidence"
        outside.write_text("keep")
        (path / "target/evidence/link").symlink_to(outside)
        report = gc.sweep(self.store, apply=True, expire_after_hours=3)
        self.assertEqual(report["removed"], [])
        self.assertIn("link", report["retained"][0]["reason"])
        self.assertTrue((path / "target/debug/binary").exists())
        self.assertEqual(self.store.load(row["id"]), row)
        self.assertEqual(outside.read_text(), "keep")

    def test_invalid_expiry_timestamps_retain_build(self):
        path, original = self.expiring_checkout()
        for key in ("finished_at", "retained_at"):
            for value in (None, True, float("nan"), float("inf"), -1):
                with self.subTest(key=key, value=value):
                    self.store.save(dict(original, **{key: value}))
                    self.assertEqual(gc.sweep(self.store, apply=True, expire_after_hours=3)["removed"], [])
                    self.assertTrue(path.exists())

    def test_device_inode_and_missing_identity_are_retained(self):
        path, original = self.checkout()
        for key in ("identity", "target_identity", "debug_identity"):
            for change in ("device", "inode", "missing"):
                with self.subTest(key=key, change=change):
                    row = dict(original)
                    if change == "missing":
                        row.pop(key)
                    else:
                        row[key] = list(row[key])
                        row[key][0 if change == "device" else 1] += 1
                    self.store.save(row)
                    self.assert_retained(path, row["id"])

    def test_source_changes_and_generated_files_are_retained(self):
        path, row = self.checkout()
        for name in ("source.rs", "apps/desktop/src-tauri/gen/generated.rs",
                     "apps/desktop/src-tauri/permissions/autogenerated/generated.toml"):
            with self.subTest(name=name):
                output = path / name
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_text("unique content")
                self.assert_retained(path, row["id"])
                self.assertEqual(output.read_text(), "unique content")
                if name == "source.rs":
                    output.write_text("tracked")
                else:
                    output.unlink()

    def test_busy_lock_retains_checkout(self):
        path, row = self.checkout()
        with self.store.locked(path):
            self.assert_retained(path, row["id"])

    def test_new_keep_flag_between_preview_and_apply_is_rechecked(self):
        path, row = self.checkout()
        original_apply = self.store.apply

        def change_then_apply(rid, token):
            current = self.store.load(rid)
            current["keep"] = True
            self.store.save(current)
            return original_apply(rid, token)

        with patch.object(self.store, "apply", side_effect=change_then_apply):
            self.assert_retained(path, row["id"])

    def test_changed_debug_snapshot_is_retained(self):
        path, row = self.checkout()
        original_apply = self.store.apply

        def change_then_apply(rid, token):
            (path / "target/debug/new-evidence").write_text("keep")
            return original_apply(rid, token)

        with patch.object(self.store, "apply", side_effect=change_then_apply):
            self.assert_retained(path, row["id"])
        self.assertEqual((path / "target/debug/new-evidence").read_text(), "keep")

    def test_unknown_age_active_recent_and_developer_are_not_deleted(self):
        path, original = self.checkout()
        for changes in ({"status": "running"}, {"kind": "developer"},
                        {"finished_at": time.time()}, {"finished_at": None},
                        {"finished_at": float("nan")}, {"finished_at": True}):
            with self.subTest(changes=changes):
                self.store.save(dict(original, **changes))
                report = gc.sweep(self.store, apply=True)
                self.assertEqual(report["selected"], [])
                self.assertEqual(report["removed"], [])
                self.assertTrue(path.exists())

    def test_limit_bounds_attempts_even_when_apply_refuses(self):
        first, row = self.checkout("a" * 32)
        second, _ = self.checkout("b" * 32)
        with patch.object(self.store, "apply", side_effect=ValueError("changed")) as apply:
            report = gc.sweep(self.store, apply=True, limit=1)
            self.assertEqual(apply.call_count, 1)
            self.assertEqual(report["selected"], [row["id"]])
        self.assertTrue(first.exists())
        self.assertTrue(second.exists())
        report = gc.sweep(self.store, apply=True, limit=1)
        self.assertEqual(report["removed"], [row["id"]])
        self.assertFalse(first.exists())
        self.assertTrue(second.exists())

    def test_invalid_options_and_records_do_not_delete(self):
        path, row = self.checkout()
        for options in ({"older_than_hours": 47}, {"limit": 0}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                gc.sweep(self.store, apply=True, **options)
        row["id"] = "b" * 32
        self.store.record_path("a" * 32).write_text(json.dumps(row))
        self.assert_retained(path, "a" * 32)

    def test_non_debug_links_block_deletion(self):
        path, row = self.checkout()
        outside = self.root / "unique-evidence"
        outside.write_text("keep")
        (path / "target/evidence/linked").symlink_to(outside)
        self.assert_retained(path, row["id"])
        self.assertEqual(outside.read_text(), "keep")

    def test_cli_rejects_invalid_options_before_store_access(self):
        for args in (["--older-than-hours", "47"], ["--limit", "0"],
                     ["--expire-after-hours", "0"], ["--expire-after-hours", "nan"],
                     ["--expire-after-hours", "inf"],
                     ["--expire-after-hours", "3", "--older-than-hours", "48"]):
            with self.subTest(args=args), patch.object(gc.sys, "argv", ["gc", *args]):
                with patch.object(gc.runner, "Store") as store:
                    with self.assertRaises(SystemExit) as error:
                        gc.main()
                    self.assertEqual(error.exception.code, 2)
                    store.assert_not_called()

    def test_cli_passes_explicit_expiry_to_bounded_sweep(self):
        with patch.object(gc.sys, "argv", ["gc", "--expire-after-hours", "3", "--limit", "10"]), \
                patch.object(gc.runner, "Store", return_value=self.store), \
                patch.object(gc, "sweep", return_value={}) as sweep, patch("builtins.print"):
            self.assertEqual(gc.main(), 0)
        self.assertEqual(sweep.call_args.args, (self.store, False, 48, 10, 3))


if __name__ == "__main__":
    unittest.main()
