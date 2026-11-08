#!/usr/bin/env python3
"""Bind release catalog bytes to a successful candidate workflow."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "hiroute.release-qualification/v1"
TARGETS = {
    "aarch64-apple-darwin": ("desktop", "macOS", "arm64", "dmg"),
    "x86_64-apple-darwin": ("desktop", "macOS", "x86_64", "dmg"),
    "aarch64-unknown-linux-gnu": ("standalone", "Linux", "aarch64", "tar.gz"),
    "x86_64-unknown-linux-gnu": ("standalone", "Linux", "x86_64", "tar.gz"),
    "x86_64-pc-windows-msvc": ("desktop", "Windows", "x86_64", "nsis"),
}
_spec = importlib.util.spec_from_file_location("release_contracts", ROOT / "scripts/release-contracts.py")
ledger = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(ledger)


def read(path):
    return json.loads(path.read_bytes())


def qualify(results, revision, version, run_id, attempt):
    ledger.version("v" + version)
    ledger.require(ledger.SHA.fullmatch(revision) and run_id > 0 and attempt > 0,
                   "invalid candidate workflow identity")
    selected = {}
    for directory in results.iterdir():
        name = re.fullmatch(r"release-result-(.+)-" + revision + r"-([1-9][0-9]*)", directory.name)
        ledger.require(name and name[1] in TARGETS and int(name[2]) <= attempt,
                       "unexpected candidate producer: " + directory.name)
        target, producer_attempt = name[1], int(name[2])
        if target not in selected or producer_attempt > selected[target][0]:
            selected[target] = (producer_attempt, directory)
    ledger.require(set(selected) == set(TARGETS), "qualification requires every public package producer")
    rows = []
    producers = {}
    for target, (producer_attempt, directory) in sorted(selected.items()):
        paths = list(directory.rglob("result.json"))
        ledger.require(len(paths) == 1, "expected one package result for " + target)
        result = read(paths[0])
        expected_status = "awaiting_runtime" if target.endswith("windows-msvc") else "completed"
        ledger.require(result["revision"] == revision and result["target"] == target
                       and result["status"] == expected_status and result["package_integrity"] == "green",
                       "package result does not bind a verified candidate for " + target)
        row = result["website_artifact"]
        ledger.require(tuple(row[key] for key in ("kind", "platform", "architecture", "format")) == TARGETS[target]
                       and f"-{version}-{revision[:12]}-" in row["filename"],
                       "package identity differs from its producer or version: " + target)
        rows.append(row)
        producers[target] = {"attempt": producer_attempt, "artifact": directory.name}
    return {"schema": SCHEMA, "revision": revision, "version": version,
            "run_id": run_id, "run_attempt": attempt, "producers": producers,
            "assets": ledger.catalog_assets({"artifacts": rows}, revision)}


def matches(receipt, run, revision, tag, assets):
    return (receipt.get("schema") == SCHEMA and receipt.get("revision") == revision
            and receipt.get("version") == tag[1:] and receipt.get("run_id") == run["databaseId"]
            and receipt.get("run_attempt") == run["attempt"]
            and all(asset in receipt.get("assets", []) for asset in assets))


def recorded(repo, repository, tag, revision, release, assets):
    directory = repo / "contracts/releases"
    name = tag + ".json"
    if not (directory / name).is_file():
        return False
    snapshot = ledger.read_directory(directory)[name]
    published = snapshot["release"]
    if (published["repository"], published["tag"], published["revision"], published["channel"], published["artifacts"]) != (
            repository, tag, revision, release["channel"], assets):
        return False
    ledger.verify_sources(repo, snapshot)
    return True


def verify(repo, repository, manifest, tag, revision):
    ledger.version(tag)
    ledger.require(ledger.SHA.fullmatch(revision), "source revision must be an exact commit SHA")
    releases = [row for row in manifest["releases"] if row["version"] == tag[1:]]
    ledger.require(len(releases) == 1, "expected one release catalog record")
    release = releases[0]
    if not any(row["platform"] == "Windows" for row in release["artifacts"]):
        return {"qualification": "not required without a Windows installer"}
    assets = ledger.catalog_assets(release, revision)
    if recorded(repo, repository, tag, revision, release, assets):
        return {"qualification": "recorded release", "revision": revision}
    runs = json.loads(subprocess.check_output([
        "gh", "run", "list", "--repo", repository, "--workflow", "build-release.yml",
        "--commit", revision, "--event", "workflow_dispatch", "--status", "success",
        "--json", "databaseId,attempt,headSha,conclusion",
    ], text=True))
    for run in runs:
        if run["headSha"] != revision or run["conclusion"] != "success":
            continue
        with tempfile.TemporaryDirectory(prefix="hiroute-qualification-") as scratch:
            name = f"qualified-release-{revision}-{run['attempt']}"
            result = subprocess.run(["gh", "run", "download", str(run["databaseId"]), "--repo", repository,
                                     "--name", name, "--dir", scratch], capture_output=True, text=True)
            if result.returncode:
                print(f"Qualification unavailable for run {run['databaseId']}: {result.stderr.strip()}", file=sys.stderr)
                continue
            receipt = read(Path(scratch) / "qualification.json")
            if matches(receipt, run, revision, tag, assets):
                return {"qualification": "candidate workflow", "revision": revision,
                        "run_id": run["databaseId"], "run_attempt": run["attempt"]}
    raise ValueError("no successful candidate qualification matches the release catalog bytes")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest="action", required=True)
    create = actions.add_parser("qualify")
    create.add_argument("--results", type=Path, required=True)
    create.add_argument("--revision", required=True)
    create.add_argument("--version", required=True)
    create.add_argument("--run-id", type=int, required=True)
    create.add_argument("--attempt", type=int, required=True)
    check = actions.add_parser("verify")
    check.add_argument("--repo", type=Path, default=ROOT)
    check.add_argument("--repository", required=True)
    check.add_argument("--manifest", type=Path, required=True)
    check.add_argument("--tag", required=True)
    check.add_argument("--revision", required=True)
    args = parser.parse_args()
    try:
        if args.action == "qualify":
            value = qualify(args.results, args.revision, args.version, args.run_id, args.attempt)
        else:
            value = verify(args.repo, args.repository, read(args.manifest), args.tag, args.revision)
    except (KeyError, OSError, TypeError, ValueError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    print(json.dumps(value, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
