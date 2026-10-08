#!/usr/bin/env python3
"""Build the exact managed CPA source and local bootstrap patch. No latest/fallback binary."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parent.parent
PIN = REPO / "vendor/cpa/source.json"
TARGETS = {
    "x86_64-unknown-linux-gnu": ("linux", "amd64"),
    "aarch64-unknown-linux-gnu": ("linux", "arm64"),
    "aarch64-apple-darwin": ("darwin", "arm64"),
    "x86_64-apple-darwin": ("darwin", "amd64"),
}


def pinned_source():
    pin = json.loads(PIN.read_text())
    patch = PIN.parent / pin["patch"]
    if hashlib.sha256(patch.read_bytes()).hexdigest() != pin["patch_sha256"]:
        raise ValueError("CPA source patch checksum mismatch")
    return pin, patch


def build(source_repo, target, output):
    pin, patch = pinned_source()
    system, arch = TARGETS[target]
    output = Path(output).resolve()
    if output.exists():
        raise ValueError("CPA output already exists; retain or explicitly remove your own artifact")
    archive = subprocess.check_output(["git", "-C", str(source_repo), "archive", pin["commit"]])
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="hiroute-cpa-source-") as temporary:
        source = Path(temporary)
        # This archive is produced locally from the exact pinned Git tree.
        subprocess.run(["tar", "-xf", "-", "-C", str(source)], input=archive, check=True)
        subprocess.run(["git", "apply", str(patch)], cwd=source, check=True)
        env = os.environ.copy()
        env.update(GOOS=system, GOARCH=arch, CGO_ENABLED="0")
        flags = " ".join(f"-X main.{name}={pin[key]}" for name, key in
                         (("Version", "version"), ("Commit", "commit"), ("BuildDate", "built_at")))
        subprocess.run(["go", "build", "-trimpath", "-buildvcs=false", "-ldflags", flags,
                        "-o", str(output), "./cmd/server"], cwd=source, env=env, check=True)
        license_output = output.with_suffix(".LICENSE")
        shutil.copyfile(source / "LICENSE", license_output)
        license_output.chmod(0o644)
    output.chmod(0o755)
    evidence = {**pin, "target": target, "sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
                "size": output.stat().st_size,
                "go_version": subprocess.check_output(["go", "version", str(output)], text=True).strip().split(": ", 1)[-1]}
    provenance_output = output.with_suffix(".provenance.json")
    provenance_output.write_text(json.dumps(evidence, indent=2) + "\n")
    provenance_output.chmod(0o644)
    return evidence


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-repo", type=Path, required=True)
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(build(args.source_repo, args.target, args.output), indent=2))
