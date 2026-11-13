"""Recognize bounded release bookkeeping changes using the actual base and diff."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys


CATALOG = "apps/website/data/releases.json"
SHORT_DOWNLOADS = "apps/website/data/short-downloads.json"
RELEASE_REVIEWS = "apps/website/docs/release-reviews/"
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
    allowed = {CATALOG, SHORT_DOWNLOADS, "README.md", "README.zh-CN.md"}
    if any(path not in allowed and not (path.startswith("apps/website/content/guides/")
                                       and path.endswith(".md"))
           and not path.startswith(RELEASE_REVIEWS) for path in paths):
        return False
    before, after = base_json(CATALOG), current_json(CATALOG)
    if (set(before) != {"schema", "releases"} or set(after) != set(before)
            or before["schema"] != "hiroute.website.releases/v3" or after["schema"] != before["schema"]):
        return False
    release = added_row(before["releases"], after["releases"])
    version = release["version"]
    versions.parse_version(version)
    reviews = [path for path in paths if path.startswith(RELEASE_REVIEWS)]
    if reviews and reviews != [f"{RELEASE_REVIEWS}v{version}.json"]:
        return False
    if any(row["version"] == version for row in before["releases"]):
        return False
    if SHORT_DOWNLOADS in paths and added_row(base_json(SHORT_DOWNLOADS), current_json(SHORT_DOWNLOADS)) != version:
        return False
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
