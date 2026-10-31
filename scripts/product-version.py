#!/usr/bin/env python3
"""Maintain one product version across shipped components and their lockfiles."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
CARGO_PACKAGES = {
    "crates/application-api/Cargo.toml": "hiroute-application-api",
    "crates/cli/Cargo.toml": "hiroute-cli",
    "crates/daemon/Cargo.toml": "hiroute-daemon",
    "apps/desktop/src-tauri/Cargo.toml": "hiroute-desktop",
    "crates/diagnostics/Cargo.toml": "hiroute-diagnostics",
    "apps/shell/src-tauri/Cargo.toml": "hiroute-shell",
}
JSON_MANIFESTS = (
    "apps/desktop/src-tauri/tauri.conf.json", "apps/desktop/package.json",
    "apps/shell/src-tauri/tauri.conf.json", "apps/shell/package.json",
)
NPM_LOCKS = ("apps/desktop/package-lock.json", "apps/shell/package-lock.json")
HELLO_SCHEMA = "contracts/cli/local-control-hello.v2.schema.json"
CONTRACT_SET = "contracts/cli/contract-set.v1.json"
VERSION_PATHS = (*CARGO_PACKAGES, *JSON_MANIFESTS, *NPM_LOCKS, "Cargo.lock", HELLO_SCHEMA, CONTRACT_SET)
CORE = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
SEMVER = re.compile(CORE + r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?")
PACKAGE = re.compile(r"(?ms)^\[package\]\s*\n(.*?)(?=^\[|\Z)")
DECLARATION = re.compile(r'^version[ \t]*=[ \t]*"([^"]+)"[ \t]*$', re.M)
LOCK_PACKAGE = re.compile(r'(?m)^\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"')


def parse_version(value):
    match = SEMVER.fullmatch(value) if isinstance(value, str) else None
    if not match or any(part.isdigit() and len(part) > 1 and part.startswith("0")
                        for part in (match[4] or "").split(".")):
        raise ValueError("invalid product semantic version")
    return tuple(map(int, match.group(1, 2, 3))), match[4]


def after_stable(version, stable):
    core, _ = parse_version(version)
    baseline, prerelease = parse_version(stable)
    if prerelease:
        raise ValueError("stable baseline cannot be a prerelease")
    # Build metadata never raises SemVer precedence; prereleases of the same
    # core are below the stable release. A later core is therefore required.
    return core > baseline


def version_values(sources):
    values = {}
    for relative in CARGO_PACKAGES:
        section = PACKAGE.search(sources[relative])
        matches = DECLARATION.findall(section[1]) if section else []
        if len(matches) != 1:
            raise ValueError("missing product package version: " + relative)
        values[relative] = matches[0]
    for relative in (*JSON_MANIFESTS, *NPM_LOCKS):
        value = json.loads(sources[relative])
        values[relative] = value["version"]
        if relative in NPM_LOCKS:
            values[relative + ":packages/root"] = value["packages"][""]["version"]
    lock = sources["Cargo.lock"]
    for name in CARGO_PACKAGES.values():
        matches = [version for package, version in LOCK_PACKAGE.findall(lock) if package == name]
        if len(matches) != 1:
            raise ValueError("missing or ambiguous product lock entry: " + name)
        values["Cargo.lock:" + name] = matches[0]
    values[HELLO_SCHEMA] = json.loads(sources[HELLO_SCHEMA])["properties"]["client_version"]["const"]
    return values


def sources_at(root):
    return {path: (root / path).read_text() for path in VERSION_PATHS}


def assert_versions(root, expected=None):
    sources = sources_at(root)
    values = version_values(sources)
    expected = expected or values[JSON_MANIFESTS[0]]
    parse_version(expected)
    for path, actual in values.items():
        if actual != expected:
            raise ValueError(f"release version mismatch: {path} is {actual}, expected {expected}")
    recorded = json.loads(sources[CONTRACT_SET])["files"][Path(HELLO_SCHEMA).name]
    if recorded != schema_digest(sources[HELLO_SCHEMA]):
        raise ValueError("stale Local Control schema digest in " + CONTRACT_SET)
    return expected


def schema_digest(source):
    return "sha256:" + hashlib.sha256(source.encode()).hexdigest()


def json_string(source, pattern, value):
    result, count = re.subn(pattern, lambda match: match[1] + json.dumps(value), source, flags=re.M)
    if count != 1:
        raise ValueError("expected one version projection field")
    return result


def json_version(source, version, indent):
    pattern = re.compile(r'(?m)^(' + ' ' * indent + r'"version"\s*:\s*)"[^"]+"')
    result, count = pattern.subn(lambda match: match[1] + json.dumps(version), source)
    if count != 1:
        raise ValueError("expected one product JSON version field")
    return result


def baseline(root):
    index = json.loads((root / "contracts/releases/index.v1.json").read_text())
    if index["schema"] != "hiroute.release-contract-index/v1":
        raise ValueError("unsupported release index")
    tag = index["latest_stable"]
    matches = [row for row in index["releases"] if row["tag"] == tag and row["channel"] == "stable"]
    if len(matches) != 1 or not tag.startswith("v"):
        raise ValueError("latest stable release is not recorded")
    _, prerelease = parse_version(tag[1:])
    if prerelease or not re.fullmatch(r"[0-9a-f]{40}", matches[0]["revision"]):
        raise ValueError("invalid stable release identity")
    return tag[1:], matches[0]["revision"]


def check(root):
    version = assert_versions(root)
    stable, _ = baseline(root)
    if not after_stable(version, stable):
        raise ValueError(f"development version {version} must be later than recorded stable {stable}; run product-version.py advance")
    return {"version": version, "latest_stable": stable, "result": "green"}


def project(sources, version):
    parse_version(version)
    # Read and validate every source before the first write. The lockfiles are
    # projections of these same package versions, not a second version policy.
    version_values(sources)
    updates = {}
    for relative in CARGO_PACKAGES:
        source = sources[relative]
        section = PACKAGE.search(source)
        body = DECLARATION.sub(lambda _: f'version = "{version}"', section[1], count=1)
        updates[relative] = source[:section.start(1)] + body + source[section.end(1):]
    for relative in (*JSON_MANIFESTS, *NPM_LOCKS):
        source = json_version(sources[relative], version, 2)
        if relative in NPM_LOCKS:
            entry = re.search(r'(?ms)^    "": \{\n(.*?)(?=^    \})', source)
            if entry is None:
                raise ValueError("missing npm root package: " + relative)
            source = source[:entry.start(1)] + json_version(entry[1], version, 6) + source[entry.end(1):]
        updates[relative] = source
    updates["Cargo.lock"] = LOCK_PACKAGE.sub(
        lambda match: match[0].replace(f'version = "{match[2]}"', f'version = "{version}"')
        if match[1] in CARGO_PACKAGES.values() else match[0], sources["Cargo.lock"])
    # These are the only version-dependent fields in the CLI generator output.
    # Preserve every other contract byte; the Rust generator parity test remains
    # authoritative for the complete contract set.
    updates[HELLO_SCHEMA] = json_string(sources[HELLO_SCHEMA],
        r'^(    "client_version": \{\n      "const": )"[^"]+"', version)
    filename = Path(HELLO_SCHEMA).name
    json.loads(sources[CONTRACT_SET])["files"][filename]
    updates[CONTRACT_SET] = json_string(sources[CONTRACT_SET],
        r'^(    "' + re.escape(filename) + r'": )"[^"]+"', schema_digest(updates[HELLO_SCHEMA]))
    return updates


def set_version(root, version):
    sources = sources_at(root)
    updates = project(sources, version)
    changed = [path for path, value in updates.items() if sources[path] != value]
    for relative in changed:
        (root / relative).write_text(updates[relative])
    return changed


def scope(root, base):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(root), *args], text=True)
    ancestor = git("merge-base", base, "HEAD").strip()
    changed = git("diff", "--name-only", ancestor, "HEAD").splitlines()
    if any(path not in VERSION_PATHS and not path.startswith("contracts/releases/") for path in changed):
        raise ValueError("existing automation branch contains unrelated changes")
    original = {path: git("show", f"{ancestor}:{path}") for path in VERSION_PATHS}
    before, after = project(original, "0.0.0"), project(sources_at(root), "0.0.0")
    if before != after:
        raise ValueError("existing automation branch changes more than product version fields")
    return {"result": "green"}


def advance(root):
    previous = assert_versions(root)
    stable, _ = baseline(root)
    version = previous
    if not after_stable(previous, stable):
        (major, minor, patch), _ = parse_version(stable)
        version = f"{major}.{minor}.{patch + 1}-dev.1"
    return {"previous_version": previous, "version": version, "latest_stable": stable,
            "changed": set_version(root, version)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "advance", "set", "paths", "scope"))
    parser.add_argument("version", nargs="?")
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--base")
    args = parser.parse_args()
    if (args.command == "set") != (args.version is not None):
        parser.error("only set requires a VERSION argument")
    if (args.command == "scope") != (args.base is not None):
        parser.error("only scope requires --base")
    if args.command == "paths":
        print("\n".join(VERSION_PATHS))
        return
    try:
        if args.command == "check":
            result = check(args.repo)
        elif args.command == "advance":
            result = advance(args.repo)
        elif args.command == "scope":
            result = scope(args.repo, args.base)
        else:
            result = {"version": args.version, "changed": set_version(args.repo, args.version)}
    except (ValueError, KeyError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, str(error) + "\n")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
