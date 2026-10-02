#!/usr/bin/env python3
"""Freeze published contract inventories; never infer migration support from a release label."""
import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


SNAPSHOT_SCHEMA = "hiroute.release-contract-snapshot/v1"
INDEX_SCHEMA = "hiroute.release-contract-index/v1"
INDEX = "index.v1.json"
TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")
SHA = re.compile(r"[0-9a-f]{40}")
HASH = re.compile(r"[0-9a-f]{64}")
SOURCES = {
    "compatibility_register": "contracts/compatibility-support.v1.json",
    "cli_contract_set": "contracts/cli/contract-set.v1.json",
}
STORAGE_SOURCE = "crates/local-storage/src/migrations/mod.rs"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode()


def digest(value):
    return hashlib.sha256(value).hexdigest()


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], stderr=subprocess.PIPE)


def version(tag):
    match = TAG.fullmatch(tag)
    require(match is not None, "invalid release tag")
    return tuple(int(part) for part in match.groups())


def create_snapshot(repo, release, manifest, revision, repository):
    tag = release["tagName"]
    version(tag)
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository), "invalid repository")
    require(SHA.fullmatch(revision), "source revision must be an exact commit SHA")
    require(release.get("isDraft") is False, "draft is not a published baseline")
    require(isinstance(release.get("isPrerelease"), bool), "missing release channel")
    published = release.get("publishedAt")
    require(isinstance(published, str) and published.endswith("Z"), "missing publication time")
    datetime.fromisoformat(published.replace("Z", "+00:00"))
    resolved = git(repo, "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}").decode().strip()
    require(resolved == revision, "tag differs from the verified package source")
    channel = "preview" if release["isPrerelease"] else "stable"
    require(channel != "stable" or "-" not in tag, "prerelease tag cannot advance stable baseline")
    require(manifest.get("schema") == "hiroute.website.releases/v2", "unknown release manifest")
    records = [item for item in manifest["releases"] if item["version"] == tag[1:]]
    require(len(records) == 1, "release manifest must contain exactly one matching version")
    record = records[0]
    require(record["channel"] == channel, "publication channel differs from manifest")
    actual = {asset["name"]: asset for asset in release["assets"]}
    require(len(actual) == len(release["assets"]), "duplicate published asset")
    artifacts = []
    for artifact in record["artifacts"]:
        names = [(artifact["filename"], artifact["sha256"], artifact["size"])]
        if artifact["kind"] == "standalone":
            names.append((artifact["manifest_filename"], artifact["manifest_sha256"], artifact["manifest_size"]))
        require(f"-{revision[:12]}-" in artifact["filename"], "artifact source differs from tag")
        for filename, sha256, size in names:
            require(Path(filename).name == filename and filename not in (".", ".."), "invalid asset name")
            require(HASH.fullmatch(sha256), "invalid artifact digest")
            asset = actual.get(filename, {})
            require(type(size) is int and size > 0 and asset.get("size") == size, "published asset missing or size mismatch")
            require(asset.get("digest") == "sha256:" + sha256, "published asset digest differs from verified manifest")
            artifacts.append({"filename": filename, "sha256": sha256, "size": size})
    require(artifacts and len({a["filename"] for a in artifacts}) == len(artifacts), "empty or duplicate manifest assets")
    storage = git(repo, "show", f"{revision}:{STORAGE_SOURCE}")
    versions = re.findall(rb"pub const LATEST_SCHEMA_VERSION:\s*u32\s*=\s*([0-9]+);", storage)
    require(len(versions) == 1, "cannot identify released SQL schema")
    contracts = {}
    for key, path in SOURCES.items():
        raw = git(repo, "show", f"{revision}:{path}")
        contracts[key] = {"path": path, "sha256": digest(raw), "content": json.loads(raw)}
    return {
        "schema": SNAPSHOT_SCHEMA,
        "release": {"repository": repository, "tag": tag, "revision": revision,
                    "channel": channel, "published_at": published,
                    "artifacts": sorted(artifacts, key=lambda a: a["filename"])},
        "storage": {"sql_schema_version": int(versions[0]), "source_path": STORAGE_SOURCE,
                    "source_sha256": digest(storage)},
        "contracts": contracts,
    }


def validate_snapshot(snapshot, filename):
    require(snapshot.get("schema") == SNAPSHOT_SCHEMA, "unknown snapshot schema")
    release = snapshot["release"]
    version(release["tag"])
    require(filename == release["tag"] + ".json", "snapshot filename/tag mismatch")
    require(SHA.fullmatch(release["revision"]), "invalid recorded revision")
    require(release["channel"] in ("stable", "preview"), "unknown channel")
    require(release["channel"] != "stable" or "-" not in release["tag"], "preview in stable ledger")
    require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", release["repository"]), "invalid recorded repository")
    published = release["published_at"]
    require(isinstance(published, str) and published.endswith("Z"), "invalid recorded publication time")
    datetime.fromisoformat(published.replace("Z", "+00:00"))
    artifacts = release["artifacts"]
    require(isinstance(artifacts, list) and artifacts, "missing recorded artifacts")
    names = []
    for artifact in artifacts:
        name = artifact["filename"]
        require(isinstance(name, str) and Path(name).name == name and name not in (".", ".."), "invalid recorded asset name")
        require(f"-{release['revision'][:12]}-" in name, "recorded asset source differs from tag")
        require(HASH.fullmatch(artifact["sha256"]), "invalid recorded asset digest")
        require(type(artifact["size"]) is int and artifact["size"] > 0, "invalid recorded asset size")
        names.append(name)
    require(names == sorted(set(names)), "recorded assets must be sorted and unique")
    require(type(snapshot["storage"]["sql_schema_version"]) is int and
            snapshot["storage"]["sql_schema_version"] > 0, "missing storage contract")
    require(set(snapshot["contracts"]) == set(SOURCES), "incomplete contract inventory")
    for key, path in SOURCES.items():
        contract = snapshot["contracts"][key]
        require(contract["path"] == path and HASH.fullmatch(contract["sha256"]), "invalid source contract")
        require(isinstance(contract["content"], dict), "invalid contract content")


def make_index(snapshots):
    records = []
    for name, snapshot in sorted(snapshots.items()):
        validate_snapshot(snapshot, name)
        release = snapshot["release"]
        records.append({"tag": release["tag"], "revision": release["revision"],
                        "channel": release["channel"], "file": name,
                        "sha256": digest(encoded(snapshot))})
    repositories = {s["release"]["repository"] for s in snapshots.values()}
    require(len(repositories) <= 1, "mixed repositories in release ledger")
    stable = [row for row in records if row["channel"] == "stable"]
    keys = [version(row["tag"]) for row in stable]
    require(len(set(keys)) == len(keys), "ambiguous stable versions")
    latest = max(stable, key=lambda row: version(row["tag"]))["tag"] if stable else None
    return {"schema": INDEX_SCHEMA, "latest_stable": latest, "releases": records}


def read_directory(directory, pending=None):
    require(not directory.is_symlink(), "ledger directory must not be a symlink")
    snapshots = {}
    for path in sorted(directory.glob("*.json")):
        require(not path.is_symlink() and path.is_file(), "invalid ledger entry")
        if path.name != INDEX:
            snapshots[path.name] = json.loads(path.read_bytes())
    index_path = directory / INDEX
    require(not index_path.is_symlink(), "ledger index must not be a symlink")
    prior = dict(snapshots)
    if pending is not None:
        filename = pending["release"]["tag"] + ".json"
        if prior.get(filename) == pending:
            del prior[filename]
    if index_path.exists():
        index = json.loads(index_path.read_bytes())
        require(index == make_index(snapshots) or
                (pending is not None and index == make_index(prior)), "ledger index/hash mismatch")
    else:
        require(not snapshots or (pending is not None and not prior), "existing snapshots lack their index")
    return snapshots


def atomic_write(path, data):
    with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".release-contract-", delete=False) as handle:
        temporary = Path(handle.name)
        try:
            handle.write(data)
            handle.flush()
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)


def record_snapshot(directory, snapshot):
    snapshots = read_directory(directory, pending=snapshot)
    filename = snapshot["release"]["tag"] + ".json"
    if filename in snapshots:
        require(snapshots[filename] == snapshot, "published contract snapshot is immutable")
    snapshots[filename] = snapshot
    index = make_index(snapshots)
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / filename
    if not path.exists():
        atomic_write(path, encoded(snapshot))
    atomic_write(directory / INDEX, encoded(index))
    return index


def verify_sources(repo, snapshot):
    release = snapshot["release"]
    revision = release["revision"]
    tag_revision = git(repo, "rev-parse", "--verify", f"refs/tags/{release['tag']}^{{commit}}").decode().strip()
    require(tag_revision == revision, "recorded tag has moved")
    for key, path in SOURCES.items():
        raw = git(repo, "show", f"{revision}:{path}")
        require(snapshot["contracts"][key] == {"path": path, "sha256": digest(raw), "content": json.loads(raw)},
                "snapshot differs from released contract")
    raw = git(repo, "show", f"{revision}:{STORAGE_SOURCE}")
    versions = re.findall(rb"pub const LATEST_SCHEMA_VERSION:\s*u32\s*=\s*([0-9]+);", raw)
    require(len(versions) == 1 and snapshot["storage"] == {
        "sql_schema_version": int(versions[0]), "source_path": STORAGE_SOURCE,
        "source_sha256": digest(raw)}, "snapshot differs from released storage format")


def current(repo, directory):
    snapshots = read_directory(directory)
    tag = make_index(snapshots)["latest_stable"]
    require(tag is not None, "no stable published baseline")
    snapshot = snapshots[tag + ".json"]
    verify_sources(repo, snapshot)
    return snapshot


def pull_request_base(environment):
    if environment.get("GITHUB_ACTIONS") != "true" or environment.get("GITHUB_EVENT_NAME") != "pull_request":
        return None
    event = json.loads(Path(environment["GITHUB_EVENT_PATH"]).read_bytes())
    base = event["pull_request"]["base"]["sha"]
    require(SHA.fullmatch(base), "invalid PR base revision")
    return base


def check_append_only(repo, directory, base):
    # The index advances, but every snapshot already merged on the PR base is fixed.
    revision = git(repo, "rev-parse", "--verify", f"{base}^{{commit}}").decode().strip()
    paths = git(repo, "ls-tree", "-r", "--name-only", revision, "--", "contracts/releases").decode().splitlines()
    for path in paths:
        name = Path(path).name
        if not path.endswith(".json") or name == INDEX:
            continue
        target = directory / name
        require(target.is_file() and not target.is_symlink() and
                target.read_bytes() == git(repo, "show", f"{revision}:{path}"),
                f"historical snapshot changed or deleted: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("record", "check", "current"))
    parser.add_argument("--repo", type=Path, default=Path.cwd())
    parser.add_argument("--directory", type=Path, default=Path("contracts/releases"))
    parser.add_argument("--release-json", type=Path)
    parser.add_argument("--manifest", type=Path, default=Path("apps/website/data/releases.json"))
    parser.add_argument("--revision")
    parser.add_argument("--repository")
    parser.add_argument("--base", help="check historical snapshots against this Git ref; defaults to the Actions PR base")
    args = parser.parse_args()
    if args.command == "record":
        require(args.release_json and args.revision and args.repository, "record requires release JSON, revision and repository")
        snapshot = create_snapshot(args.repo, json.loads(args.release_json.read_bytes()),
                                   json.loads(args.manifest.read_bytes()), args.revision, args.repository)
        result = record_snapshot(args.directory, snapshot)
    elif args.command == "check":
        base = args.base or pull_request_base(os.environ)
        if base:
            check_append_only(args.repo, args.directory, base)
        snapshots = read_directory(args.directory)
        result = make_index(snapshots)
        require(result["releases"], "empty release ledger")
        for snapshot in snapshots.values():
            verify_sources(args.repo, snapshot)
    else:
        result = current(args.repo, args.directory)
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
