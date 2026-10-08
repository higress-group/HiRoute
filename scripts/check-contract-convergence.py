#!/usr/bin/env python3
"""Reject unregistered contract versions and legacy leakage into current paths."""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


REGISTRY_PATH = Path("contracts/compatibility-support.v1.json")
# Storage owns the production declaration and startup support policy. A schema change
# must deliberately update this gate after reviewing migrations/startup_format.rs and
# docs/upgrade-storage-design.md; fixtures cannot satisfy it. This is not a migration test.
DATABASE_SCHEMA_SOURCE = "crates/local-storage/src/migrations/mod.rs"
EXPECTED_DATABASE_SCHEMA_VERSION = 29
VERSION_SUFFIX = re.compile(r"/v[0-9]+")
INTERNAL_CONTRACT = re.compile(r"hiroute(?:\.[a-z0-9][a-z0-9-]*)+/v[0-9]+")
VALID_MODES = {
    "frozen_fixture",
    "recovery_read_only",
    "rejection_fixture",
}
OBSOLETE_LIVE_CONTRACT_FILES = {
    "contracts/cli/local-control-hello.v1.schema.json",
    "contracts/cli/local-control.v1.schema.json",
    "contracts/cli/machine-envelope.v1.schema.json",
    "crates/application-api/src/routing_control.rs",
}
OBSOLETE_LIVE_RUST_FRAGMENTS = {
    "crates/domain/src/routing/materialized.rs": {
        "pub fn new(body: CompiledAgentPlanBodyV1",
    },
    "crates/domain/src/operation/routing.rs": {
        "pub struct RoutingTransactionIntentV1",
        "pub fn from_routing_planner",
    },
}


def path_allowed(path, allowed):
    return any(
        path.startswith(value) if value.endswith("/") else path == value
        for value in allowed
    )


def repository_files(root):
    output = subprocess.check_output(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"], cwd=root
    )
    excluded = {str(REGISTRY_PATH), "scripts/check-contract-convergence.py"}
    return sorted(
        path for path in output.decode().split("\0") if path and path not in excluded
    )


def load_text(root, path):
    try:
        return (root / path).read_text()
    except (UnicodeDecodeError, OSError):
        return ""


def historical_release_snapshot(path, source):
    # These files describe released producers, never current readers/writers or test
    # fixtures. Their full index, digest and Git-source validation belongs to
    # release-contracts.py check. Do not exempt code, arbitrary JSON or other paths.
    if not re.fullmatch(r"contracts/releases/v[0-9][0-9A-Za-z.+-]*\.json", path):
        return False
    try:
        value = json.loads(source)
    except ValueError:
        return False
    return isinstance(value, dict) and value.get("schema") == "hiroute.release-contract-snapshot/v1"


def production_source_path(path):
    """Select shipped/internal Rust sources, not tests, fixtures, docs, or third-party data."""
    value = Path(path)
    if value.suffix != ".rs":
        return False
    parts = value.parts
    if not parts:
        return False
    if parts[0] == "apps":
        selected = "src" in parts
    elif parts[0] == "crates":
        selected = "src" in parts
    elif parts[:2] == ("tools", "release-facts"):
        selected = "src" in parts
    else:
        return False
    if not selected or "tests" in parts or "test_control" in parts:
        return False
    name = value.name.lower()
    return not (
        name == "tests.rs"
        or name.endswith("_tests.rs")
        or name.startswith("test_")
        or "fixture" in name
    )


def without_cfg_test_items(source):
    """Remove items with a directly required positive `test` condition.

    This deliberately leaves production code appearing after a test module in the scan. Braces in
    Rust strings and macros are balanced in the formatted sources; the checker has regression tests
    for both inline rejection tests and a production declaration appended after them.
    """
    output = []
    pending = False
    depth = 0
    # Conservative: unknown/nested cfg expressions remain in the production audit.
    # In particular not(test) and any(test, feature = ...) are production-capable.
    cfg_test = re.compile(
        r"#\s*\[\s*cfg\s*\(\s*(?:test\s*|all\(\s*test\s*(?:,[^\]]*)?\))\s*\)\s*\]"
    )
    for line in source.splitlines(keepends=True):
        if depth:
            depth += line.count("{") - line.count("}")
            if depth <= 0:
                depth = 0
            continue
        if pending:
            stripped = line.strip()
            if not stripped or stripped.startswith("#["):
                continue
            item_depth = line.count("{") - line.count("}")
            if item_depth > 0:
                depth = item_depth
            pending = False
            continue
        if cfg_test.search(line):
            pending = True
            continue
        output.append(line)
    return "".join(output)


def schema_source_tokens(source):
    """Lex only enough Rust to distinguish this gate's declaration from text.

    This is not a Rust parser or a cfg evaluator. Literals stay opaque, nested
    comments are skipped, and malformed lexical input fails closed.
    """
    raw_string = re.compile(r'(?:b|c)?r(#{0,255})"')
    quoted_string = re.compile(r'(?:b|c)?"(?:\\[\s\S]|[^"\\])*"')
    character = re.compile(r"(?:b)?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|[^\n])|[^'\\\n])'")
    token = re.compile(r"[A-Za-z_]\w*|[0-9][0-9_]*|.")
    position = 0
    while position < len(source):
        if source[position].isspace():
            position += 1
        elif source.startswith("//", position):
            end = source.find("\n", position)
            position = len(source) if end < 0 else end
        elif source.startswith("/*", position):
            depth = 1
            position += 2
            while depth and position < len(source):
                if source.startswith("/*", position):
                    depth += 1
                    position += 2
                elif source.startswith("*/", position):
                    depth -= 1
                    position += 2
                else:
                    position += 1
            if depth:
                raise ValueError("unterminated block comment")
        elif match := raw_string.match(source, position):
            ending = '"' + match[1]
            end = source.find(ending, match.end())
            if end < 0:
                raise ValueError("unterminated raw string")
            position = end + len(ending)
            yield "<literal>"
        elif source.startswith(('"', 'b"', 'c"'), position):
            match = quoted_string.match(source, position)
            if not match:
                raise ValueError("unterminated string")
            position = match.end()
            yield "<literal>"
        elif match := character.match(source, position):
            position = match.end()
            yield "<literal>"
        else:
            match = token.match(source, position)
            position = match.end()
            yield match[0]


def database_schema_declarations(source):
    # Positive allowlist: one plain, unattributed module-level const. Do not infer
    # production availability from arbitrary cfg/cfg_attr expressions or macros.
    versions = []
    item = []
    stack = []
    pairs = {"(": ")", "[": "]", "{": "}"}
    declaration = re.compile(r"pub const LATEST_SCHEMA_VERSION : u32 = ([0-9][0-9_]*) ;")
    for token in schema_source_tokens(source):
        if not stack:
            item.append(token)
            if item[-2:] == ["#", "!"]:
                raise ValueError("inner attributes require an explicit schema gate review")
        if token in pairs:
            stack.append(pairs[token])
        elif token in pairs.values():
            if not stack or stack.pop() != token:
                raise ValueError("unbalanced delimiters")
        if not stack and token in {";", "}"}:
            if any(item[index:index + 2] == ["const", "LATEST_SCHEMA_VERSION"]
                   for index in range(len(item) - 1)):
                match = declaration.fullmatch(" ".join(item))
                versions.append(match[1] if match else None)
            item = []
    if stack:
        raise ValueError("unclosed delimiter")
    return versions


def audit_database_schema(root):
    try:
        versions = database_schema_declarations(load_text(root, DATABASE_SCHEMA_SOURCE))
    except ValueError as error:
        return [f"database schema: {DATABASE_SCHEMA_SOURCE}: {error}"]
    if len(versions) != 1 or versions[0] is None:
        return [f"database schema: {DATABASE_SCHEMA_SOURCE} must contain exactly one unconditional, unattributed top-level LATEST_SCHEMA_VERSION declaration"]
    version = int(versions[0].replace("_", ""))
    if version != EXPECTED_DATABASE_SCHEMA_VERSION:
        return [f"database schema: {DATABASE_SCHEMA_SOURCE} declares V{version}; current storage requires V{EXPECTED_DATABASE_SCHEMA_VERSION} (storage owner must review schema and startup support policy together)"]
    return []


def discover_production_contracts(texts):
    occurrences = {}
    for path, source in texts.items():
        if not production_source_path(path):
            continue
        for contract in INTERNAL_CONTRACT.findall(without_cfg_test_items(source)):
            occurrences.setdefault(contract, set()).add(path)
    return occurrences


def validate_registry(registry):
    errors = []
    if registry.get("schema") != "hiroute.contract-compatibility-support/v1":
        errors.append("registry schema is invalid")
    subjects = registry.get("subjects")
    if not isinstance(subjects, list) or not subjects:
        return [*errors, "registry subjects are missing"]
    names = set()
    legacy_contracts = set()
    production_contracts = registry.get("production_contracts")
    if (
        not isinstance(production_contracts, list)
        or not production_contracts
        or production_contracts != sorted(set(production_contracts))
        or any(
            not isinstance(contract, str)
            or INTERNAL_CONTRACT.fullmatch(contract) is None
            for contract in production_contracts
        )
    ):
        errors.append("production_contracts must be a sorted unique internal contract list")
    for subject in subjects:
        name = subject.get("subject")
        if not isinstance(name, str) or not name or name in names:
            errors.append(f"invalid or duplicate subject: {name!r}")
            continue
        names.add(name)
        prefixes = subject.get("audited_prefixes")
        currents = subject.get("current_contracts")
        supports = subject.get("legacy_support")
        if not all(isinstance(value, list) and value for value in (prefixes, currents)):
            errors.append(f"{name}: audited prefixes and current contracts must be non-empty")
        if not isinstance(supports, list):
            errors.append(f"{name}: legacy_support must be a list")
            continue
        for support in supports:
            contract = support.get("contract")
            required = ("owner", "reason", "removal_condition")
            if not isinstance(contract, str) or not VERSION_SUFFIX.search(contract):
                errors.append(f"{name}: invalid legacy contract {contract!r}")
            elif contract in legacy_contracts:
                errors.append(f"duplicate legacy contract: {contract}")
            else:
                legacy_contracts.add(contract)
            if support.get("mode") not in VALID_MODES:
                errors.append(f"{name}: invalid support mode for {contract}")
            if any(
                not isinstance(support.get(field), str) or not support[field].strip()
                for field in required
            ):
                errors.append(f"{name}: {contract} lacks owner, reason, or removal condition")
            paths = support.get("allowed_paths")
            if not isinstance(paths, list) or not paths or any(
                not isinstance(path, str)
                or path.startswith(("/", "."))
                or ".." in path
                for path in paths
            ):
                errors.append(f"{name}: {contract} has invalid allowed paths")
    forbidden = registry.get("forbidden_contracts")
    if not isinstance(forbidden, list) or any(
        not isinstance(value, str) for value in forbidden
    ):
        errors.append("forbidden_contracts must be a string list")
    return errors


def audit(root, registry, paths=None):
    errors = validate_registry(registry)
    if errors:
        return errors
    paths = repository_files(root) if paths is None else sorted(paths)
    for path in sorted(OBSOLETE_LIVE_CONTRACT_FILES):
        if (root / path).is_file():
            errors.append(f"obsolete live contract file reintroduced: {path}")
    texts = {path: load_text(root, path) for path in paths}
    texts = {path: source for path, source in texts.items()
             if not historical_release_snapshot(path, source)}
    production = discover_production_contracts(texts)
    declared_production = set(registry["production_contracts"])
    for contract in sorted(set(production) - declared_production):
        locations = ", ".join(sorted(production[contract]))
        errors.append(f"undeclared production contract {contract}: {locations}")
    for contract in sorted(declared_production - set(production)):
        errors.append(f"stale production contract declaration: {contract}")
    for path, fragments in OBSOLETE_LIVE_RUST_FRAGMENTS.items():
        value = texts.get(path, "")
        for fragment in fragments:
            if fragment in value:
                errors.append(
                    f"obsolete live contract producer reintroduced in {path}: {fragment}"
                )
    rejection_paths = registry.get("rejection_paths", [])
    all_current = set()
    all_legacy = {}
    for subject in registry["subjects"]:
        currents = set(subject["current_contracts"])
        all_current.update(currents)
        for support in subject["legacy_support"]:
            all_legacy[support["contract"]] = support
        known = currents | {
            support["contract"] for support in subject["legacy_support"]
        }
        for prefix in subject["audited_prefixes"]:
            pattern = re.compile(re.escape(prefix) + r"/v[0-9]+")
            for path, value in texts.items():
                for contract in pattern.findall(value):
                    if contract not in known and not path_allowed(path, rejection_paths):
                        errors.append(
                            f"{path}: unregistered {subject['subject']} contract {contract}"
                        )
    for contract in sorted(all_current):
        if not any(contract in value for value in texts.values()):
            errors.append(f"current contract is not implemented: {contract}")
    production_by_family = {}
    for contract in production:
        family = contract.rsplit("/v", 1)[0]
        production_by_family.setdefault(family, set()).add(contract)
    for family, contracts in sorted(production_by_family.items()):
        if len(contracts) < 2:
            continue
        classified = any(
            family in subject["audited_prefixes"]
            and contracts.issubset(
                set(subject["current_contracts"])
                | {
                    support["contract"]
                    for support in subject["legacy_support"]
                }
            )
            for subject in registry["subjects"]
        )
        if not classified:
            errors.append(
                "unclassified production multi-version family "
                f"{family}: {', '.join(sorted(contracts))}"
            )
    for contract, support in sorted(all_legacy.items()):
        occurrences = [path for path, value in texts.items() if contract in value]
        if not occurrences:
            errors.append(f"stale compatibility registration has no occurrence: {contract}")
        for path in occurrences:
            if not path_allowed(path, support["allowed_paths"]):
                errors.append(
                    f"{path}: legacy contract escapes registered support: {contract}"
                )
    for contract in registry["forbidden_contracts"]:
        for path, value in texts.items():
            if contract in value:
                errors.append(f"{path}: forbidden contract reintroduced: {contract}")
    joined = "\n".join(texts.values())
    errors.extend(audit_database_schema(root))
    if "control-v1.sock" in joined:
        errors.append("legacy Local Control socket name reintroduced")
    return sorted(set(errors))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    arguments = parser.parse_args()
    root = arguments.root.resolve()
    registry = json.loads((root / REGISTRY_PATH).read_text())
    errors = audit(root, registry)
    if errors:
        for error in errors:
            print(f"contract-convergence: {error}", file=sys.stderr)
        return 1
    supports = sum(
        len(subject["legacy_support"]) for subject in registry["subjects"]
    )
    print(
        json.dumps(
            {
                "state": "green",
                "subjects": len(registry["subjects"]),
                "legacy_supports": supports,
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
