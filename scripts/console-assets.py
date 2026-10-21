#!/usr/bin/env python3
"""Exact-candidate console handoff for managed installed-package acceptance."""
import base64
import fnmatch
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import tempfile


RECEIPT = "hiroute-console-assets.json"
SCHEMA = "hiroute.validation.console-assets/v1"
CASES = (
    "installed_standalone_cli_completes_the_headless_management_loop",
    "installed_cli_quickstart_reuses_resources_and_exact_apply_files",
    "installed_feed_withdrawal_preserves_published_sessions_workers_and_continue",
)
MAX_BYTES = 32 * 1024 * 1024


def selected(cargo, platform):
    """Inspect Cargo/libtest selectors without compiling or listing test binaries."""
    if platform != "linux" or cargo[:2] != ["cargo", "test"]:
        return []
    boundary = cargo.index("--") if "--" in cargo else len(cargo)
    before, after = cargo[2:boundary], cargo[boundary + 1:]
    if "--no-run" in before or "--list" in after or "--ignored" in after:
        return []
    values, flags, filters = {}, set(), []
    takes_value = {"-p", "--package", "--exclude", "--test", "--bin", "--example", "--bench",
                   "--target", "--features", "-F", "-j", "--jobs", "--profile", "--color",
                   "--message-format", "--target-dir", "--manifest-path", "--config"}
    index = 0
    while index < len(before):
        arg = before[index]
        key, equal, value = arg.partition("=")
        if key in takes_value:
            if not equal:
                index += 1
                value = before[index]
            values.setdefault("--package" if key == "-p" else key, []).append(value)
        elif arg.startswith("-p") and not arg.startswith("--"):
            values.setdefault("--package", []).append(arg[2:])
        elif arg.startswith("-"):
            flags.add(arg)
        else:
            filters.append(arg)
        index += 1
    packages = values.get("--package", [])
    if (packages and "--workspace" not in flags
            and not any(fnmatch.fnmatchcase("hiroute-daemon", p.split("@", 1)[0]) for p in packages)):
        return []
    if any(fnmatch.fnmatchcase("hiroute-daemon", p) for p in values.get("--exclude", [])):
        return []
    if values.get("--target") and not any("linux" in p for p in values["--target"]):
        return []
    tests = values.get("--test", [])
    if tests and not any(fnmatch.fnmatchcase("publication_process", p) for p in tests):
        return []
    if not tests and not flags.intersection({"--tests", "--all-targets"}) and (
            flags.intersection({"--lib", "--bins", "--examples", "--benches", "--doc"})
            or any(key in values for key in ("--bin", "--example", "--bench"))):
        return []
    skips = []
    index = 0
    while index < len(after):
        arg = after[index]
        if arg in ("--skip", "--test-threads", "--format", "--color", "--logfile"):
            index += 1
            if arg == "--skip":
                skips.append(after[index])
        elif arg.startswith("--skip="):
            skips.append(arg.split("=", 1)[1])
        elif not arg.startswith("-"):
            filters.append(arg)
        index += 1
    exact = "--exact" in after
    return [case for case in CASES
            if (not filters or any(case == f if exact else f in case for f in filters))
            and not any(case == skip if exact else skip in case for skip in skips)]


def source_revision(repo):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()
    if git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("Console build requires a clean committed candidate")
    return git("rev-parse", "HEAD")


def files(dist):
    """Inventory regular bytes only; reject links, traversal and special files."""
    if dist.is_symlink() or not dist.is_dir():
        raise ValueError("Console dist is missing or is a link")
    result, total = {}, 0
    for path in sorted(dist.rglob("*")):
        mode = path.lstat().st_mode
        if stat.S_ISLNK(mode) or not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
            raise ValueError("Console dist contains a link or special file")
        if not stat.S_ISREG(mode) or path == dist / RECEIPT:
            continue
        data = path.read_bytes()
        total += len(data)
        if total > MAX_BYTES:
            raise ValueError("Console dist exceeds the transfer limit")
        result[path.relative_to(dist).as_posix()] = data
    if not result.get("index.html"):
        raise ValueError("Console dist must contain a nonempty index.html")
    return result


def manifest(sha, contents):
    return dict(schema=SCHEMA, source_revision=sha,
                files={name: hashlib.sha256(data).hexdigest() for name, data in contents.items()})


def build(repo, env):
    sha = source_revision(repo)
    dist = repo / "apps/desktop/dist"
    if dist.is_symlink():
        raise ValueError("Console dist must not be a link")
    # A failed build must not leave a previous build's provenance usable.
    receipt = dist / RECEIPT
    if receipt.is_symlink():
        raise ValueError("Console receipt must not be a link")
    receipt.unlink(missing_ok=True)
    code = subprocess.call(["npm", "--prefix", "apps/desktop", "run", "build"], cwd=repo, env=env)
    if code:
        return code
    if source_revision(repo) != sha:
        raise ValueError("Console source changed during the build")
    receipt.write_text(json.dumps(manifest(sha, files(dist)), indent=2) + "\n")
    return 0


def verify(dist, sha):
    contents = files(dist)
    receipt = dist / RECEIPT
    if receipt.is_symlink() or not receipt.is_file():
        raise ValueError("Console build receipt is missing; use validation.py frontend build --console-assets")
    value = json.loads(receipt.read_text())
    if value != manifest(sha, contents):
        raise ValueError("Console source revision or file hashes differ from the candidate receipt")
    return value


def bundle(dist):
    receipt = dist / RECEIPT
    if receipt.is_symlink():
        raise ValueError("Console receipt must not be a link")
    sha = json.loads(receipt.read_text())["source_revision"]
    value = verify(dist, sha)
    contents = files(dist)
    if value != manifest(sha, contents):
        raise ValueError("Console bytes changed while preparing transfer")
    return dict(receipt=value, contents={name: base64.b64encode(data).decode("ascii")
                                        for name, data in contents.items()})


def stage(checkout, sha, value):
    contents = {}
    for name, encoded in value["contents"].items():
        path = PurePosixPath(name)
        if (not name or path.is_absolute() or path.as_posix() != name or ".." in path.parts
                or "\\" in name or name == RECEIPT):
            raise ValueError("Invalid console artifact path")
        contents[name] = base64.b64decode(encoded, validate=True)
    if (not re.fullmatch(r"[0-9a-f]{40}", sha) or not contents.get("index.html")
            or sum(map(len, contents.values())) > MAX_BYTES
            or value["receipt"] != manifest(sha, contents)):
        raise ValueError("Console transfer source revision or file hashes differ from the candidate")
    parent = checkout / "apps/desktop"
    if any(p.is_symlink() for p in (checkout, checkout / "apps", parent, parent / "dist")):
        raise ValueError("Console destination must not traverse links")
    # Caller holds the managed checkout lease. Never replace a developer checkout.
    temporary = Path(tempfile.mkdtemp(prefix=".console-", dir=parent))
    try:
        for name, data in contents.items():
            path = temporary / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        (temporary / RECEIPT).write_text(json.dumps(value["receipt"], indent=2) + "\n")
        verify(temporary, sha)
        dist = parent / "dist"
        if dist.exists():
            shutil.rmtree(dist)
        temporary.rename(dist)
    finally:
        if temporary.exists():
            shutil.rmtree(temporary)


def preflight(checkout, request, platform, supplied=None):
    cases = selected(request["command"], platform)
    if not cases:
        return dict(state="not_required", cases=[])
    if supplied is not None:
        stage(checkout, request["sha"], supplied)
    value = verify(checkout / "apps/desktop/dist", request["sha"])
    return dict(state="verified", cases=cases, source_revision=value["source_revision"],
                files=len(value["files"]), receipt_sha256=hashlib.sha256(
                    json.dumps(value, sort_keys=True).encode()).hexdigest())
