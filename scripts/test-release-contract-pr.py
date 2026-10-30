#!/usr/bin/env python3
"""Run the PR publisher with real local Git remotes and a strictly offline gh fixture."""
import json
import hashlib
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BRANCH = "chore/release-contracts-v1.2.0"
VERSION_SPEC = importlib.util.spec_from_file_location("product_version", ROOT / "scripts/product-version.py")
product = importlib.util.module_from_spec(VERSION_SPEC)
VERSION_SPEC.loader.exec_module(product)


class PublisherFixture(unittest.TestCase):
    release_version = "1.2.0"

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
        self.write("contracts/compatibility-support.v1.json", '{"contract":"released/v1"}\n')
        for path in ("scripts/release-contracts.py", "scripts/product-version.py", ".github/scripts/release-contract-pr.sh"):
            self.write(path, (ROOT / path).read_text())
        self.version_paths = product.VERSION_PATHS
        for path, name in product.CARGO_PACKAGES.items():
            self.write(path, f'[package]\nname = "{name}"\nversion = "{self.release_version}"\n')
        for path in product.JSON_MANIFESTS:
            self.write(path, json.dumps({"name": "fixture", "version": self.release_version}, indent=2) + '\n')
        for path in product.NPM_LOCKS:
            self.write(path, json.dumps({"version": self.release_version, "packages": {"": {"version": self.release_version}}}, indent=2) + '\n')
        self.write("Cargo.lock", '\n\n'.join(f'[[package]]\nname = "{name}"\nversion = "{self.release_version}"' for name in product.CARGO_PACKAGES.values()) + '\n')
        hello = json.dumps({"properties": {"client_version": {"const": self.release_version}}}, indent=2) + '\n'
        self.write("contracts/cli/local-control-hello.v2.schema.json", hello)
        self.write("contracts/cli/contract-set.v1.json", json.dumps({"files": {
            "local-control-hello.v2.schema.json": "sha256:" + hashlib.sha256(hello.encode()).hexdigest()}}, indent=2) + '\n')
        self.commit("released producer")
        revision = self.git("rev-parse", "HEAD").strip()
        tag = "v" + self.release_version
        self.git("tag", tag)
        name = f"HiRoute-{self.release_version}-{revision[:12]}-macos-arm64-trial.dmg"
        self.release = {"tagName": tag, "isDraft": False, "isPrerelease": False,
                        "publishedAt": "2026-01-01T01:02:03Z",
                        "assets": [{"name": name, "size": 5, "digest": "sha256:" + "1" * 64}]}
        (self.base / "release.json").write_text(json.dumps(self.release))
        self.write("apps/website/data/releases.json", json.dumps({
            "schema": "hiroute.website.releases/v2", "releases": [{
                "version": self.release_version, "channel": "stable", "artifacts": [{
                    "kind": "desktop", "filename": name, "sha256": "1" * 64, "size": 5}]}]}))
        self.commit("verified website manifest")
        self.main = self.git("rev-parse", "HEAD").strip()
        self.env.update(RELEASE_TAG=tag, RELEASE_REVISION=revision, WEBSITE_REVISION=self.main)
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


class PublicationTests(PublisherFixture):
    def test_create_repeat_update_and_merged_noop(self):
        self.publish()
        self.assertEqual(self.remote("refs/heads/main"), self.main)
        first = self.remote("refs/heads/" + BRANCH)
        changed = self.git("diff", "--name-only", self.main, first).splitlines()
        self.assertEqual(set(changed), {"contracts/releases/index.v1.json", "contracts/releases/v1.2.0.json", *self.version_paths})
        sources = {path: self.git('show', first + ':' + path) for path in self.version_paths}
        self.assertEqual(set(product.version_values(sources).values()), {'1.2.1-dev.1'})
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

    def test_retry_preserves_newer_main_version_and_dependency_changes(self):
        self.publish()
        product.set_version(self.repo, "1.3.0-dev.1")
        path = "crates/cli/Cargo.toml"
        self.write(path, (self.repo / path).read_text() + '\n[dependencies]\nexample = "2"\n')
        lock = self.repo / "Cargo.lock"
        lock.write_text(lock.read_text() + '\n[[package]]\nname = "example"\nversion = "2.0.0"\n')
        hello = self.repo / product.HELLO_SCHEMA
        value = json.loads(hello.read_text())
        value["description"] = "new main contract documentation"
        hello.write_text(json.dumps(value, indent=2) + '\n')
        product.set_version(self.repo, "1.3.0-dev.1")
        self.commit("prepare newer main version with dependencies and contracts")
        self.git("push", "origin", "main")
        main_sources = product.sources_at(self.repo)
        self.publish()
        self.git("fetch", "origin", BRANCH)
        self.git("merge", "--ff-only", "FETCH_HEAD")
        self.assertEqual(product.check(self.repo)["version"], "1.3.0-dev.1")
        self.assertEqual(product.sources_at(self.repo), main_sources)
        self.assertEqual(len(self.calls(["pr", "create"])), 1)
        head = self.remote("refs/heads/" + BRANCH)
        self.publish()
        self.assertEqual(self.remote("refs/heads/" + BRANCH), head)

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

    def test_existing_branch_with_unrelated_changes_in_a_version_manifest_is_rejected(self):
        self.publish()
        self.git('fetch', 'origin', BRANCH)
        self.git('switch', '--detach', 'FETCH_HEAD')
        path = self.repo / 'apps/desktop/package.json'
        value = json.loads(path.read_text())
        value['scripts'] = {'build': 'unrelated command'}
        path.write_text(json.dumps(value, indent=2) + '\n')
        self.commit('unrelated manifest content')
        self.git('push', 'origin', 'HEAD:refs/heads/' + BRANCH)
        before = self.remote('refs/heads/' + BRANCH)
        self.publish(False)
        self.assertEqual(self.remote('refs/heads/' + BRANCH), before)

    def test_explicit_replacement_uses_new_branch_and_preserves_old_bytes(self):
        self.publish()
        self.git("fetch", "origin", BRANCH)
        self.git("merge", "--ff-only", "FETCH_HEAD")
        self.git("push", "origin", "main")
        previous = self.env["RELEASE_REVISION"]
        original = (self.repo / "contracts/releases/v1.2.0.json").read_bytes()
        self.write("replacement.txt", "new exact package source")
        self.commit("replacement producer")
        revision = self.git("rev-parse", "HEAD").strip()
        self.git("tag", "-f", "v1.2.0")
        manifest_path = self.repo / "apps/website/data/releases.json"
        manifest_path.write_text(manifest_path.read_text().replace(previous[:12], revision[:12]))
        self.release["assets"][0]["name"] = self.release["assets"][0]["name"].replace(previous[:12], revision[:12])
        (self.base / "release.json").write_text(json.dumps(self.release))
        self.commit("replacement package manifest")
        self.git("push", "origin", "main")
        self.git("push", "--force", "origin", "refs/tags/v1.2.0")
        self.main = self.git("rev-parse", "HEAD").strip()
        self.env.update(RELEASE_REVISION=revision, WEBSITE_REVISION=self.main)
        (self.base / "pr.json").unlink()  # The new revision branch has no PR yet.
        self.publish(False)
        self.publish(False, SUPERSEDE_UNPROMOTED_REVISION="0" * 40)
        self.publish(SUPERSEDE_UNPROMOTED_REVISION=previous)
        branch = BRANCH + "-" + revision[:12]
        self.git("fetch", "origin", branch)
        archived = f"contracts/releases/archive/v1.2.0.{previous}.json"
        self.assertEqual(self.git("show", "FETCH_HEAD:" + archived).encode(), original)
        self.git("merge", "--ff-only", "FETCH_HEAD")
        self.git("push", "origin", "main")
        (self.base / "pr.json").write_text('{"state":"MERGED"}')
        self.publish(SUPERSEDE_UNPROMOTED_REVISION=previous)
        self.assertEqual(len(self.calls(["pr", "create"])), 2)


class SkippedReleaseTests(PublisherFixture):
    release_version = "0.3.1"

    def test_publish_directly_from_031_development_to_050(self):
        self.publish()
        self.git("fetch", "origin", "chore/release-contracts-v0.3.1")
        self.git("merge", "--ff-only", "FETCH_HEAD")
        self.git("push", "origin", "main")
        self.assertEqual(product.check(self.repo)["version"], "0.3.2-dev.1")
        old_snapshot = (self.repo / "contracts/releases/v0.3.1.json").read_bytes()
        product.set_version(self.repo, "0.5.0")
        self.commit("prepare 0.5.0 candidate")
        revision = self.git("rev-parse", "HEAD").strip()
        self.git("tag", "v0.5.0")
        self.release["tagName"] = "v0.5.0"
        name = f"HiRoute-0.5.0-{revision[:12]}-macos-arm64-trial.dmg"
        self.release["assets"][0]["name"] = name
        (self.base / "release.json").write_text(json.dumps(self.release))
        self.write("apps/website/data/releases.json", json.dumps({
            "schema": "hiroute.website.releases/v3", "releases": [{
                "version": "0.5.0", "channel": "stable", "artifacts": [{
                    "kind": "desktop", "filename": name, "sha256": "1" * 64, "size": 5}]}]}))
        self.commit("verified 0.5.0 catalog")
        self.git("push", "origin", "main", "refs/tags/v0.5.0")
        self.env.update(RELEASE_TAG="v0.5.0", RELEASE_REVISION=revision,
                        WEBSITE_REVISION=self.git("rev-parse", "HEAD").strip())
        (self.base / "pr.json").unlink()
        self.publish()
        self.git("fetch", "origin", "chore/release-contracts-v0.5.0")
        self.git("merge", "--ff-only", "FETCH_HEAD")
        result = product.check(self.repo)
        self.assertEqual(result["version"], "0.5.1-dev.1")
        self.assertEqual(result["latest_stable"], "0.5.0")
        self.assertEqual((self.repo / "contracts/releases/v0.3.1.json").read_bytes(), old_snapshot)
        self.assertEqual(len(self.calls(["pr", "create"])), 2)


if __name__ == "__main__":
    unittest.main()
