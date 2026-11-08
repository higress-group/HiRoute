"""Recognize bounded release bookkeeping changes using the actual base and diff."""
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys


CATALOG = "apps/website/data/releases.json"
SHORT_DOWNLOADS = "apps/website/data/short-downloads.json"
WINDOWS_POLICY = "scripts/windows-desktop-cases.json"
LEDGER = "contracts/releases/"
INDEX = LEDGER + "index.v1.json"


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def added_row(before, after):
    if not isinstance(before, list) or not isinstance(after, list) or len(after) != len(before) + 1:
        raise ValueError("expected exactly one appended release record")
    added = [row for row in after if row not in before]
    if len(added) != 1 or [row for row in after if row != added[0]] != before:
        raise ValueError("historical release records changed")
    return added[0]


def catalog_scope(paths, base_json, current_json, versions):
    allowed = {CATALOG, SHORT_DOWNLOADS, WINDOWS_POLICY, "README.md", "README.zh-CN.md"}
    if any(path not in allowed and not (path.startswith("apps/website/content/guides/")
                                       and path.endswith(".md")) for path in paths):
        return False
    before, after = base_json(CATALOG), current_json(CATALOG)
    if (set(before) != {"schema", "releases"} or set(after) != set(before)
            or before["schema"] != "hiroute.website.releases/v3" or after["schema"] != before["schema"]):
        return False
    release = added_row(before["releases"], after["releases"])
    version = release["version"]
    versions.parse_version(version)
    if any(row["version"] == version for row in before["releases"]):
        return False
    if SHORT_DOWNLOADS in paths and added_row(base_json(SHORT_DOWNLOADS), current_json(SHORT_DOWNLOADS)) != version:
        return False
    if WINDOWS_POLICY not in paths:
        return True
    old_policy, policy = base_json(WINDOWS_POLICY), current_json(WINDOWS_POLICY)
    if ({key: value for key, value in policy.items() if key != "release_decisions"}
            != {key: value for key, value in old_policy.items() if key != "release_decisions"}
            or policy["schema"] != "hiroute.windows-desktop-acceptance/v1"):
        return False
    decision = added_row(old_policy["release_decisions"], policy["release_decisions"])
    if set(decision) != {"version", "revision", "installer_sha256", "decision_url", "deferred_cases"}:
        return False
    revision, digest = decision["revision"], decision["installer_sha256"]
    if (decision["version"] != version or not re.fullmatch(r"[0-9a-f]{40}", revision)
            or not re.fullmatch(r"[0-9a-f]{64}", digest)):
        return False
    installers = [item for item in release["artifacts"] if item["platform"] == "Windows"]
    if len(installers) != 1 or any(installers[0].get(key) != value for key, value in {
        "filename": f"HiRoute-{version}-{revision[:12]}-windows-x86_64-setup.exe",
        "kind": "desktop", "format": "nsis", "architecture": "x86_64", "sha256": digest,
    }.items()):
        return False
    load("release-contracts").catalog_assets(release, revision)
    # Validate the decision shape and mandatory cases without inventing native runs.
    # Publication still verifies real evidence on the exact installer bytes.
    load("package-windows-shell").native_deferral(
        {**decision, "native_runtime": "deferred", "acceptance_decision": decision["decision_url"]}, policy)
    return True


def classify(root, base, paths):
    """Return the proven scope or leave selection to the conservative planner."""
    if CATALOG not in paths and INDEX not in paths:
        return None
    try:
        # A symlink or deletion is not a version projection or catalog append.
        if any((root / path).is_symlink() or not (root / path).is_file() for path in paths):
            return None
        versions = load("product-version")
        ancestor = subprocess.check_output(["git", "merge-base", "HEAD", base], cwd=root, text=True).strip()
        if CATALOG in paths:
            def base_json(path):
                return json.loads(subprocess.check_output(["git", "show", f"{ancestor}:{path}"], cwd=root))

            def current_json(path):
                return json.loads((root / path).read_bytes())

            if catalog_scope(paths, base_json, current_json, versions):
                return "catalog"
        elif (any(path in versions.VERSION_PATHS for path in paths)
              and any(path != INDEX and path.startswith(LEDGER) for path in paths)
              and all(path in versions.VERSION_PATHS or (Path(path).parent == Path(LEDGER)
                                                        and path.endswith(".json")) for path in paths)):
            versions.scope(root, ancestor)
            versions.check(root)
            ledger = load("release-contracts")
            ledger.check_append_only(root, root / LEDGER, ancestor)
            ledger.read_directory(root / LEDGER)
            return "record"
    except (AttributeError, KeyError, OSError, TypeError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Release bookkeeping proof did not apply: {error}", file=sys.stderr)
    return None
