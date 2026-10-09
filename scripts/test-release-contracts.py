#!/usr/bin/env python3
"""Exercise immutable release inventories against real, isolated Git history."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_contracts", Path(__file__).with_name("release-contracts.py"))
contracts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contracts)


class ReleaseContractsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name) / "repo"
        self.repo.mkdir()
        self.run_git("init", "-q")
        self.run_git("config", "user.name", "Fixture")
        self.run_git("config", "user.email", "fixture@example.invalid")
        self.write(contracts.STORAGE_SOURCE, "pub const LATEST_SCHEMA_VERSION: u32 = 7;\n")
        for path in contracts.SOURCES.values():
            self.write(path, '{"contract":"original/v1"}\n')
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "released producer")
        self.revision = self.run_git("rev-parse", "HEAD").strip()
        self.run_git("tag", "v1.2.0")
        self.directory = Path(self.temporary.name) / "ledger"
        self.release, self.manifest = self.metadata("v1.2.0")

    def run_git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], stderr=subprocess.PIPE, text=True)

    def write(self, path, value):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)

    def metadata(self, tag, preview=False):
        name = f"HiRoute-{tag[1:]}-{self.revision[:12]}-macos-arm64-trial.dmg"
        release = {"tagName": tag, "isDraft": False, "isPrerelease": preview,
                   "publishedAt": "2026-01-01T01:02:03Z",
                   "assets": [{"name": name, "size": 5, "digest": "sha256:" + "1" * 64}]}
        manifest = {"schema": "hiroute.website.releases/v2", "releases": [{
            "version": tag[1:], "channel": "preview" if preview else "stable",
            "artifacts": [{"kind": "desktop", "filename": name, "sha256": "1" * 64, "size": 5}]}]}
        return release, manifest

    def snapshot(self, release=None, manifest=None, revision=None):
        return contracts.create_snapshot(self.repo, release or self.release, manifest or self.manifest,
                                         revision or self.revision, "example/HiRoute")

    def test_reads_released_contract_instead_of_current_worktree(self):
        self.write(contracts.STORAGE_SOURCE, "pub const LATEST_SCHEMA_VERSION: u32 = 999;\n")
        for path in contracts.SOURCES.values():
            self.write(path, '{"contract":"changed/v2"}\n')
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "future work")
        snapshot = self.snapshot()
        self.assertEqual(snapshot["storage"]["sql_schema_version"], 7)
        self.assertEqual(snapshot["contracts"]["cli_contract_set"]["content"]["contract"], "original/v1")
        contracts.verify_sources(self.repo, snapshot)

    def test_repeat_is_byte_identical_and_cannot_replace_old_snapshot(self):
        snapshot = self.snapshot()
        contracts.record_snapshot(self.directory, snapshot)
        before = {p.name: p.read_bytes() for p in self.directory.iterdir()}
        contracts.record_snapshot(self.directory, snapshot)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.directory.iterdir()})
        changed = copy.deepcopy(snapshot)
        changed["storage"]["sql_schema_version"] = 8
        with self.assertRaisesRegex(ValueError, "immutable"):
            contracts.record_snapshot(self.directory, changed)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.directory.iterdir()})

    def test_older_rerun_and_preview_do_not_replace_latest_stable(self):
        old = self.snapshot()
        contracts.record_snapshot(self.directory, old)
        for tag, preview in [("v1.3.0", False), ("v2.0.0-rc.1", True)]:
            self.run_git("tag", tag)
            release, manifest = self.metadata(tag, preview)
            contracts.record_snapshot(self.directory, self.snapshot(release, manifest))
        index = contracts.record_snapshot(self.directory, old)
        self.assertEqual(index["latest_stable"], "v1.3.0")
        self.assertEqual(len(index["releases"]), 3)
        self.assertEqual(contracts.current(self.repo, self.directory)["release"]["tag"], "v1.3.0")

    def test_invalid_publication_inputs_fail_before_writes(self):
        for mutation in [lambda r, m: r.update(isDraft=True),
                         lambda r, m: r.update(publishedAt=None),
                         lambda r, m: r["assets"].clear(),
                         lambda r, m: r["assets"][0].update(size=9),
                         lambda r, m: r["assets"][0].update(digest="sha256:" + "2" * 64),
                         lambda r, m: m["releases"][0].update(channel="preview"),
                         lambda r, m: m["releases"][0]["artifacts"][0].update(sha256="bad")]:
            release, manifest = copy.deepcopy(self.release), copy.deepcopy(self.manifest)
            mutation(release, manifest)
            with self.assertRaises(ValueError):
                contracts.record_snapshot(self.directory, self.snapshot(release, manifest))
            self.assertFalse(self.directory.exists())
        with self.assertRaisesRegex(ValueError, "tag differs"):
            self.snapshot(revision="0" * 40)

    def test_partial_snapshot_write_can_finish_only_the_exact_pending_record(self):
        snapshot = self.snapshot()
        self.directory.mkdir()
        (self.directory / "v1.2.0.json").write_bytes(contracts.encoded(snapshot))
        with self.assertRaisesRegex(ValueError, "lack their index"):
            contracts.read_directory(self.directory)
        contracts.record_snapshot(self.directory, snapshot)
        self.assertEqual(contracts.current(self.repo, self.directory), snapshot)
        self.run_git("tag", "v1.3.0")
        release, manifest = self.metadata("v1.3.0")
        next_snapshot = self.snapshot(release, manifest)
        (self.directory / "v1.3.0.json").write_bytes(contracts.encoded(next_snapshot))
        with self.assertRaisesRegex(ValueError, "index/hash mismatch"):
            contracts.read_directory(self.directory)
        contracts.record_snapshot(self.directory, next_snapshot)
        self.assertEqual(contracts.current(self.repo, self.directory), next_snapshot)

    def test_corruption_and_relabelled_contract_are_rejected(self):
        snapshot = self.snapshot()
        contracts.record_snapshot(self.directory, snapshot)
        snapshot["contracts"]["cli_contract_set"]["content"]["contract"] = "changed/v2"
        (self.directory / "v1.2.0.json").write_bytes(contracts.encoded(snapshot))
        with self.assertRaisesRegex(ValueError, "index/hash mismatch"):
            contracts.read_directory(self.directory)
        with self.assertRaisesRegex(ValueError, "released contract"):
            contracts.verify_sources(self.repo, snapshot)

    def test_moved_tag_is_not_silently_adopted(self):
        snapshot = self.snapshot()
        self.write("new-file", "different commit")
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "new source")
        self.run_git("tag", "-f", "v1.2.0")
        with self.assertRaisesRegex(ValueError, "tag has moved"):
            contracts.verify_sources(self.repo, snapshot)

    def test_empty_ledger_has_no_inferred_baseline(self):
        with self.assertRaisesRegex(ValueError, "no stable published baseline"):
            contracts.current(self.repo, self.directory)

    def test_recorded_metadata_shape_is_checked(self):
        for mutate in [lambda r: r.update(repository="invalid"),
                       lambda r: r.update(published_at="not-a-dateZ"),
                       lambda r: r.update(artifacts=[]),
                       lambda r: r["artifacts"][0].update(sha256="bad"),
                       lambda r: r["artifacts"][0].update(size=0)]:
            snapshot = self.snapshot()
            mutate(snapshot["release"])
            with self.assertRaises(ValueError):
                contracts.validate_snapshot(snapshot, "v1.2.0.json")

    def test_pr_base_preserves_prior_snapshots_even_when_index_is_rehashed(self):
        self.directory = self.repo / "contracts/releases"
        snapshot = self.snapshot()
        contracts.record_snapshot(self.directory, snapshot)
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "record baseline")
        base = self.run_git("rev-parse", "HEAD").strip()
        contracts.check_append_only(self.repo, self.directory, base)
        self.run_git("tag", "v1.3.0", self.revision)
        release, manifest = self.metadata("v1.3.0")
        contracts.record_snapshot(self.directory, self.snapshot(release, manifest))
        contracts.check_append_only(self.repo, self.directory, base)
        old = self.directory / "v1.2.0.json"
        snapshot["release"]["artifacts"][0]["sha256"] = "2" * 64
        old.write_bytes(contracts.encoded(snapshot))
        # Deliberately repair the index: source/shape checks alone cannot see this edit.
        snapshots = {p.name: json.loads(p.read_bytes()) for p in self.directory.glob("v*.json")}
        (self.directory / contracts.INDEX).write_bytes(contracts.encoded(contracts.make_index(snapshots)))
        contracts.read_directory(self.directory)
        contracts.verify_sources(self.repo, snapshot)
        with self.assertRaisesRegex(ValueError, "changed or deleted"):
            contracts.check_append_only(self.repo, self.directory, base)
        old.unlink()
        with self.assertRaisesRegex(ValueError, "changed or deleted"):
            contracts.check_append_only(self.repo, self.directory, base)

    def test_actions_check_resolves_the_actual_pr_base(self):
        event = Path(self.temporary.name) / "event.json"
        event.write_text(json.dumps({"pull_request": {"base": {"sha": self.revision}}}))
        env = {"GITHUB_ACTIONS": "true", "GITHUB_EVENT_NAME": "pull_request", "GITHUB_EVENT_PATH": str(event)}
        self.assertEqual(contracts.pull_request_base(env), self.revision)
        self.assertIsNone(contracts.pull_request_base({}))
        event.write_text('{}')
        with self.assertRaises(KeyError):
            contracts.pull_request_base(env)

    def replacement(self):
        old = self.snapshot()
        self.directory = self.repo / "contracts/releases"
        contracts.record_snapshot(self.directory, old)
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "record baseline")
        base = self.run_git("rev-parse", "HEAD").strip()
        self.write(contracts.STORAGE_SOURCE, "pub const LATEST_SCHEMA_VERSION: u32 = 8;\n")
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "replacement producer")
        self.revision = self.run_git("rev-parse", "HEAD").strip()
        self.run_git("tag", "-f", "v1.2.0")
        self.release, self.manifest = self.metadata("v1.2.0")
        return old, self.snapshot(), base

    def test_explicit_supersession_preserves_evidence_and_advances_current(self):
        old, new, base = self.replacement()
        previous = old["release"]["revision"]
        before = (self.directory / "v1.2.0.json").read_bytes()
        with self.assertRaisesRegex(ValueError, "immutable"):
            contracts.record_snapshot(self.directory, new)
        with self.assertRaisesRegex(ValueError, "immutable"):
            contracts.record_snapshot(self.directory, new, "0" * 40)
        contracts.record_snapshot(self.directory, new, previous)
        archived = self.directory / "archive" / contracts.archive_name(old)
        self.assertEqual(archived.read_bytes(), before)
        self.assertEqual(contracts.current(self.repo, self.directory), new)
        contracts.check_append_only(self.repo, self.directory, base)
        contracts.verify_sources(self.repo, old, archived=True)
        with self.assertRaisesRegex(ValueError, "tag has moved"):
            contracts.verify_sources(self.repo, old)
        files = {p: p.read_bytes() for p in self.directory.rglob("*.json")}
        contracts.record_snapshot(self.directory, new, previous)
        contracts.record_snapshot(self.directory, new)
        self.assertEqual(files, {p: p.read_bytes() for p in self.directory.rglob("*.json")})
        self.run_git("add", ".")
        self.run_git("commit", "-qm", "record replacement")
        archived.write_bytes(before + b" ")
        for ref in [base, "HEAD"]:
            with self.assertRaisesRegex(ValueError, "changed or deleted"):
                contracts.check_append_only(self.repo, self.directory, ref)

    def test_replacement_index_write_recovery_requires_exact_archived_revision(self):
        old, new, _ = self.replacement()
        previous = old["release"]["revision"]
        index = (self.directory / contracts.INDEX).read_bytes()
        contracts.record_snapshot(self.directory, new, previous)
        (self.directory / contracts.INDEX).write_bytes(index)
        with self.assertRaisesRegex(ValueError, "index/hash mismatch"):
            contracts.record_snapshot(self.directory, new)
        with self.assertRaisesRegex(ValueError, "lacks archived"):
            contracts.record_snapshot(self.directory, new, "0" * 40)
        contracts.record_snapshot(self.directory, new, previous)
        self.assertEqual(contracts.current(self.repo, self.directory), new)

    def test_supersession_cannot_change_identity_or_rewrite_same_revision(self):
        old, new, _ = self.replacement()
        previous = old["release"]["revision"]
        for key, value in [("repository", "other/project"), ("channel", "preview")]:
            changed = copy.deepcopy(new)
            changed["release"][key] = value
            with self.assertRaisesRegex(ValueError, "identity"):
                contracts.record_snapshot(self.directory, changed, previous)
        changed = copy.deepcopy(old)
        changed["release"]["artifacts"][0]["sha256"] = "3" * 64
        with self.assertRaisesRegex(ValueError, "new source revision"):
            contracts.record_snapshot(self.directory, changed, previous)
        self.assertEqual(contracts.read_directory(self.directory)["v1.2.0.json"], old)

    def test_archive_symlink_or_edited_source_is_rejected(self):
        old, new, _ = self.replacement()
        external = Path(self.temporary.name) / "external"
        external.mkdir()
        archive = self.directory / "archive"
        archive.symlink_to(external, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            contracts.record_snapshot(self.directory, new, old["release"]["revision"])
        self.assertEqual(list(external.iterdir()), [])
        archive.unlink()
        contracts.record_snapshot(self.directory, new, old["release"]["revision"])
        old["storage"]["sql_schema_version"] = 999
        with self.assertRaisesRegex(ValueError, "released storage"):
            contracts.verify_sources(self.repo, old, archived=True)


if __name__ == "__main__":
    unittest.main()
