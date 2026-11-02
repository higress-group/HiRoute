#!/usr/bin/env python3
"""Verify locked native acceptance clients; never log in or send provider requests."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import struct
import subprocess
import sys
import urllib.request
import uuid
import zipfile

REPO = Path(__file__).resolve().parent.parent
FIXTURES = REPO / "tools/windows-agent-fixtures"
QODER_MANIFEST = "https://qoder-ide.oss-accelerate.aliyuncs.com/qodercli/channels/1.1.65/manifest.json"
QODER_MANIFEST_SHA = "c88c7f159a95aed57071636b867bc588276f762aaed4640d880dd3cfd062b61e"
QODER_ZIP = "https://qoder-ide.oss-accelerate.aliyuncs.com/qodercli/releases/1.1.65/qodercli-windows-x64.zip"
QODER_SHA = "7094b1b57e72f4fe351c516bbff7089ad5d6b62bef49c27df2738804ba611038"
# The committed Claude npm pin ships this placeholder before its postinstall
# links/copies the matching optional platform package. npm ci ignores scripts.
CLAUDE_PLACEHOLDER_SHA = "6d7abae055d3b598281300a6c835086dec81bf3048f8a2294c5d3e50c8830d7b"


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def materialize_native_entry(source, destination, placeholder_sha):
    """Reproduce only the reviewed native-entry link/copy; refuse other material."""
    expected = digest(source)
    if destination.is_symlink() or not destination.is_file():
        raise ValueError("Native npm entry must be an existing regular placeholder")
    observed = digest(destination)
    if observed == expected:
        return destination
    if observed != placeholder_sha:
        raise ValueError("Native npm entry is neither the pinned placeholder nor native image")
    temporary = destination.with_name(".hiroute-native-entry-" + uuid.uuid4().hex)
    try:
        try:
            os.link(source, temporary)
        except OSError:
            shutil.copyfile(source, temporary)
        if digest(temporary) != expected or digest(destination) != placeholder_sha:
            raise ValueError("Native npm preparation changed before publication")
        os.replace(temporary, destination)
    finally:
        if temporary.exists():
            temporary.unlink()
    return destination


def native(path, host_helper=False):
    path = path.resolve(strict=True)
    with path.open("rb") as handle:
        header = handle.read(64)
        if len(header) != 64 or header[:2] != b"MZ":
            raise ValueError("Native PE required: " + str(path))
        handle.seek(struct.unpack_from("<I", header, 60)[0])
        image = handle.read(6)
    allowed = [b"PE\0\0\x64\x86"]
    if host_helper and os.environ.get("HIROUTE_WINDOWS_EXECUTION_HOST") == "windows11-arm-x64":
        allowed.append(b"PE\0\0\x64\xaa")
    if image not in allowed:
        raise ValueError("Pinned x64 native PE required: " + str(path))
    return path


def machine(path):
    with path.open("rb") as handle:
        header = handle.read(64)
        handle.seek(struct.unpack_from("<I", header, 60)[0] + 4)
        return {0x8664: "x64", 0xAA64: "arm64"}[struct.unpack("<H", handle.read(2))[0]]


def download(url, path, expected):
    with urllib.request.urlopen(url, timeout=120) as response, path.open("xb") as output:
        shutil.copyfileobj(response, output)
    if digest(path) != expected:
        raise ValueError("Fixed upstream artifact digest differs: " + url)


def prepare(output, daemon):
    if sys.platform != "win32":
        raise ValueError("Native Windows runner required")
    output.mkdir(parents=True, exist_ok=False)
    lock = json.loads((FIXTURES / "package-lock.json").read_text(encoding="utf-8"))
    modules = FIXTURES / "node_modules"
    declarations = json.loads((FIXTURES / "package.json").read_text(encoding="utf-8"))["dependencies"]
    for package, version in declarations.items():
        actual = json.loads((modules / package / "package.json").read_text(encoding="utf-8"))
        if actual["name"] != package or actual["version"] != version:
            raise ValueError("Installed npm producer differs from committed fixture: " + package)
        record = lock["packages"]["node_modules/" + package]
        if record["version"] != version or not record.get("integrity"):
            raise ValueError("Fixture lacks fixed npm integrity: " + package)
    node = native(Path(shutil.which("node") or "missing-node"))
    version = subprocess.check_output([node, "--version"], text=True).strip()
    if version != "v22.19.0":
        raise ValueError("Acceptance Node must be fixed v22.19.0, observed " + version)
    qoder_manifest, archive = output / "qoder-manifest.json", output / "qodercli.zip"
    download(QODER_MANIFEST, qoder_manifest, QODER_MANIFEST_SHA)
    download(QODER_ZIP, archive, QODER_SHA)
    with zipfile.ZipFile(archive) as source:
        for item in source.infolist():
            path = PurePosixPath(item.filename)
            if path.is_absolute() or ".." in path.parts or "\\" in item.filename or ":" in item.filename:
                raise ValueError("Unsafe upstream archive entry")
            if (item.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError("Upstream archive symlink refused")
        source.extractall(output / "qoder")
    qoder = list((output / "qoder").rglob("qodercli.exe"))
    codex = list((modules / "@openai/codex-win32-x64").rglob("codex.exe"))
    claude = list((modules / "@anthropic-ai/claude-code-win32-x64").rglob("claude.exe"))
    if any(len(items) != 1 for items in (qoder, codex, claude)):
        raise ValueError("Pinned native producer must have one exact executable")
    claude_manifest = json.loads((modules / "@anthropic-ai/claude-code/package.json").read_text())
    claude_bin = lock["packages"]["node_modules/@anthropic-ai/claude-code"]["bin"]["claude"]
    platform_manifest = json.loads((modules / "@anthropic-ai/claude-code-win32-x64/package.json").read_text(encoding="utf-8"))
    if (claude_bin != "bin/claude.exe" or claude_manifest["bin"]["claude"] != claude_bin
            or platform_manifest["name"] != "@anthropic-ai/claude-code-win32-x64"
            or platform_manifest["version"] != declarations["@anthropic-ai/claude-code"]):
        raise ValueError("Claude declared native producer differs from the locked installation")
    materialize_native_entry(native(claude[0]), modules / "@anthropic-ai/claude-code" / claude_bin,
                             CLAUDE_PLACEHOLDER_SHA)
    env = {
        "HIROUTE_WORKER_NODE": str(node),
        "HIROUTE_WORKER_DAEMON_BINARY": str(native(daemon)),
        "HIROUTE_WORKER_CLI_BINARY": str(native(daemon.parent / "hiroute.exe")),
        "HIROUTE_WORKER_CODEX_BINARY": str(native(codex[0])),
        "HIROUTE_WORKER_CLAUDE_BINARY": str(native(claude[0])),
        "HIROUTE_WORKER_QODER_BINARY": str(native(qoder[0])),
        "HIROUTE_WORKER_CODEX_ACP_ADAPTER": str((modules / "@agentclientprotocol/codex-acp/dist/index.js").resolve(strict=True)),
        "HIROUTE_WORKER_CLAUDE_ACP_ADAPTER": str((modules / "@agentclientprotocol/claude-agent-acp/dist/index.js").resolve(strict=True)),
        "HIROUTE_WORKER_PI_BINARY": str((modules / "@earendil-works/pi-coding-agent/dist/bundle/cli.js").resolve(strict=True)),
        "HIROUTE_WORKER_DSH_BINARY": str((modules / "@deepseek-ai/dsh/lib/bin.js").resolve(strict=True)),
    }
    if digest(Path(env["HIROUTE_WORKER_CODEX_ACP_ADAPTER"])) != "4b76310393d756a0f111687cd9df899720f36f7c59b1eb1d86429484034ff91b":
        raise ValueError("Reviewed Codex adapter entry differs")
    git = Path(shutil.which("git") or "missing-git").resolve(strict=True)
    bash = git.parent.parent / "bin/bash.exe"
    native(bash, host_helper=True)
    pwsh = native(Path(shutil.which("pwsh") or "missing-pwsh"), host_helper=True)
    record = dict(node_version=version, package_lock_sha256=digest(FIXTURES / "package-lock.json"),
                  declarations=declarations, environment=env,
                  discovery_path=list(dict.fromkeys(str(path.parent) for path in
                      (Path(env["HIROUTE_WORKER_CODEX_BINARY"]),
                       Path(env["HIROUTE_WORKER_CLAUDE_BINARY"]),
                       Path(env["HIROUTE_WORKER_QODER_BINARY"]), node, bash, pwsh))),
                  binaries={key: {"path": value, "sha256": digest(Path(value)), "pe_machine": machine(Path(value)) if Path(value).suffix.lower() == ".exe" else None} for key, value in env.items()},
                  bash={"path": str(bash), "sha256": digest(bash), "pe_machine": machine(bash)},
                  pwsh={"path": str(pwsh), "sha256": digest(pwsh), "pe_machine": machine(pwsh)},
                  qoder_manifest_sha256=QODER_MANIFEST_SHA, qoder_archive_sha256=QODER_SHA,
                  runtime="not_executed", evidence_limit="Fixed artifact prerequisites; provider login is not supplied.")
    (output / "clients.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--daemon", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.output, args.daemon), indent=2))


if __name__ == "__main__":
    main()
