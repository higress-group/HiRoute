#!/usr/bin/env python3
"""Reject Simplified-Chinese prose in public Markdown's English/default files."""

import argparse
import fnmatch
import json
import os
from pathlib import Path
import re
import subprocess


HAN = re.compile(r"[\u3400-\u9fff]")
INLINE_CODE = re.compile(r"`[^`]*`")
CHINESE_LINK = re.compile(r"\[[^\]]*[\u3400-\u9fff][^\]]*\]\([^)]*\.(?:zh|zh-CN)\.md(?:#[^)]*)?\)")
MANIFEST = Path(".agents/skills/hiroute-public-pr/references/public-surface.v1.json")
DISCOVERY_EXCLUDES = {".git", "dist", "node_modules", "target", "vendor"}


def working_markdown(root: Path) -> list[Path]:
    files = []
    for directory, children, names in os.walk(root):
        children[:] = [child for child in children if child not in DISCOVERY_EXCLUDES]
        files.extend(Path(directory) / name for name in names if name.endswith(".md"))
    return files


def tracked_markdown(root: Path) -> list[Path]:
    files = set(working_markdown(root))
    try:
        output = subprocess.run(
            ["git", "ls-files", "-z", "--", "*.md"],
            cwd=root,
            check=True,
            capture_output=True,
        ).stdout
        files.update(root / value.decode() for value in output.split(b"\0") if value)
    except (OSError, subprocess.CalledProcessError):
        pass
    return sorted(path for path in files if path.is_file())


def public_markdown(root: Path) -> list[Path]:
    files = tracked_markdown(root)
    manifest_path = root / MANIFEST
    if not manifest_path.is_file():
        return [path for path in files if "vendor" not in path.relative_to(root).parts]

    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    include_files = set(manifest["include_files"])
    include_docs = set(manifest["include_docs"])
    include_roots = tuple(manifest["include_roots"])
    exclude_files = set(manifest["exclude_files"])
    exclude_roots = tuple(manifest["exclude_roots"])
    exclude_globs = tuple(manifest["exclude_doc_globs"])
    selected = []
    for path in files:
        relative = path.relative_to(root).as_posix()
        included = relative in include_files or relative in include_docs or relative.startswith(include_roots)
        excluded = (
            relative in exclude_files
            or relative.startswith(exclude_roots)
            or any(fnmatch.fnmatch(relative, pattern) for pattern in exclude_globs)
        )
        if included and not excluded and not relative.startswith("vendor/"):
            selected.append(path)
    return selected


def is_english_default(path: Path) -> bool:
    return not path.name.endswith((".zh.md", ".zh-CN.md"))


def prose_violations(path: Path) -> list[tuple[int, str]]:
    violations = []
    fence = None
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        stripped = raw.lstrip()
        marker = stripped[:3]
        if marker in {"```", "~~~"}:
            if fence is None:
                fence = marker
            elif marker == fence:
                fence = None
            continue
        if fence is not None:
            continue
        prose = CHINESE_LINK.sub("", raw)
        prose = INLINE_CODE.sub("", prose)
        if HAN.search(prose):
            violations.append((number, raw.strip()))
    return violations


def check(root: Path) -> list[str]:
    failures = []
    for path in public_markdown(root):
        if not is_english_default(path):
            continue
        for number, line in prose_violations(path):
            failures.append(f"{path.relative_to(root)}:{number}: {line}")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    root = args.root.resolve()
    failures = check(root)
    if failures:
        print("public English/default Markdown contains Chinese prose:")
        print("\n".join(f"- {failure}" for failure in failures))
        return 1
    print(f"public language boundary: {len(public_markdown(root))} Markdown files checked")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
