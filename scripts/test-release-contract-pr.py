#!/usr/bin/env python3
"""Run the PR publisher with real local Git remotes and a strictly offline gh fixture."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BRANCH = "chore/release-contracts-v1.2.0"


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name)
        self.repo = self.base / "repo"
        self.repo.mkdir()
        self.env = {**os.environ, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                    "FIXTURE_CONTEXT": str(self.base), "GH_REPO": "example/HiRoute",
                    "GITHUB_EVENT_NAME": "release"}
        self.git("init", "-q", "-b", "main")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.write("crates/local-storage/src/migrations/mod.rs", "pub const LATEST_SCHEMA_VERSION: u32 = 7;\n")
        for path in ("contracts/compatibility-support.v1.json", "contracts/cli/contract-set.v1.json"):
            self.write(path, '{"contract":"released/v1"}\n')
        for path in ("scripts/release-contracts.py", ".github/scripts/release-contract-pr.sh"):
            self.write(path, (ROOT / path).read_text())
        self.commit("released producer")
        revision = self.git("rev-parse", "HEAD").strip()
        self.git("tag", "v1.2.0")
        name = f"HiRoute-1.2.0-{revision[:12]}-macos-arm64-trial.dmg"
        self.release = {"tagName": "v1.2.0", "isDraft": False, "isPrerelease": False,
                        "publishedAt": "2026-01-01T01:02:03Z",
                        "assets": [{"name": name, "size": 5, "digest": "sha256:" + "1" * 64}]}
        (self.base / "release.json").write_text(json.dumps(self.release))
        self.write("apps/website/data/releases.json", json.dumps({
            "schema": "hiroute.website.releases/v2", "releases": [{
                "version": "1.2.0", "channel": "stable", "artifacts": [{
                    "kind": "desktop", "filename": name, "sha256": "1" * 64, "size": 5}]}]}))
        self.commit("verified website manifest")
        self.main = self.git("rev-parse", "HEAD").strip()
        self.env.update(RELEASE_TAG="v1.2.0", RELEASE_REVISION=revision, WEBSITE_REVISION=self.main)
        self.origin = self.base / "origin.git"
        self.git("clone", "--bare", str(self.repo), str(self.origin))
        self.git("remote", "add", "origin", str(self.origin))
        self.git("fetch", "origin")
        fakebin = self.base / "bin"
        fakebin.mkdir()
        gh = fakebin / "gh"
        gh.write_text(f"#!{sys.executable}\n" + '''import json, os, pathlib, sys
p = pathlib.Path(os.environ["FIXTURE_CONTEXT"])
a = sys.argv[1:]
with (p / "calls.jsonl").open("a") as out:
    out.write(json.dumps(a) + "\\n")
if a[:2] == ["release", "view"]:
    print((p / "release.json").read_text())
elif a[:2] == ["pr", "list"]:
    print((p / "pr.json").read_text() if (p / "pr.json").exists() else "{}")
elif a[:2] == ["auth", "setup-git"]:
    pass
elif a[:2] == ["pr", "create"]:
    if os.environ.get("FAIL_CREATE"):
        sys.exit("fixture: PR creation denied")
    assert not (p / "pr.json").exists(), "duplicate PR"
    (p / "pr-body.md").write_text(pathlib.Path(a[a.index("--body-file") + 1]).read_text())
    (p / "pr.json").write_text(json.dumps({"state": "OPEN", "url": "https://example.invalid/pull/1"}))
    print("https://example.invalid/pull/1")
else:
    sys.exit("unexpected gh command")
''')
        gh.chmod(0o755)
        self.env["PATH"] = str(fakebin) + os.pathsep + self.env["PATH"]

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args],
                                       stderr=subprocess.PIPE, text=True, env=self.env)

    def write(self, path, content):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-qm", message)

    def remote(self, ref):
        return self.git("ls-remote", "origin", ref).split()[0]

    def publish(self, success=True, **environment):
        result = subprocess.run(["bash", ".github/scripts/release-contract-pr.sh"], cwd=self.repo,
                                env={**self.env, **environment}, text=True, capture_output=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertEqual(len([line for line in self.git("worktree", "list", "--porcelain").splitlines()
                              if line.startswith("worktree ")]), 1)
        return result

    def calls(self, prefix):
        return [row for row in map(json.loads, (self.base / "calls.jsonl").read_text().splitlines())
                if row[:len(prefix)] == prefix]

    def test_create_repeat_update_and_merged_noop(self):
        self.publish()
        self.assertEqual(self.remote("refs/heads/main"), self.main)
        first = self.remote("refs/heads/" + BRANCH)
        changed = self.git("diff", "--name-only", self.main, first).splitlines()
        self.assertEqual(changed, ["contracts/releases/index.v1.json", "contracts/releases/v1.2.0.json"])
        self.assertIn("does not claim migration acceptance", (self.base / "pr-body.md").read_text())
        self.publish()
        self.assertEqual(self.remote("refs/heads/" + BRANCH), first)
        self.assertEqual(len(self.calls(["pr", "create"])), 1)
        self.write("future.txt", "unrelated main development")
        self.commit("new main")
        self.git("push", "origin", "main")
        next_main = self.remote("refs/heads/main")
        self.publish()
        self.assertEqual(self.remote("refs/heads/main"), next_main)
        self.git("fetch", "origin", BRANCH)
        self.git("merge", "--ff-only", "FETCH_HEAD")
        self.git("push", "origin", "main")
        (self.base / "pr.json").write_text('{"state":"MERGED"}')
        self.publish()
        self.assertEqual(len(self.calls(["pr", "create"])), 1)

    def test_failed_pr_creation_can_retry_existing_branch(self):
        self.publish(False, FAIL_CREATE="1")
        first = self.remote("refs/heads/" + BRANCH)
        self.publish()
        self.assertEqual(first, self.remote("refs/heads/" + BRANCH))
        self.assertEqual(self.remote("refs/heads/main"), self.main)

    def test_bad_identity_is_rejected_before_writes(self):
        self.release["assets"][0]["digest"] = "sha256:" + "2" * 64
        (self.base / "release.json").write_text(json.dumps(self.release))
        self.publish(False)
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/" + BRANCH), "")
        self.assertEqual(self.calls(["pr"]), [])
        self.assertEqual(self.remote("refs/heads/main"), self.main)

    def test_closed_pr_is_not_silently_reopened(self):
        (self.base / "pr.json").write_text('{"state":"CLOSED"}')
        self.publish(False)
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/" + BRANCH), "")
        self.assertEqual(self.calls(["pr", "create"]), [])

    def test_existing_branch_with_unrelated_changes_is_rejected(self):
        self.write("foreign.txt", "another owner's change")
        self.commit("unrelated")
        self.git("push", "origin", "HEAD:refs/heads/" + BRANCH)
        before = self.remote("refs/heads/" + BRANCH)
        self.publish(False)
        self.assertEqual(self.remote("refs/heads/" + BRANCH), before)
        self.assertEqual(self.calls(["pr", "create"]), [])


if __name__ == "__main__":
    unittest.main()
