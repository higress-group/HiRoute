#!/usr/bin/env python3
"""Prepare verified native CI packages using the production packaging boundary."""
import argparse
import importlib.util
import json
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / f"{name}.py")
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def command(*args):
    return subprocess.check_output(list(map(str, args)), cwd=ROOT, text=True, encoding="utf-8").strip()


def apply_acl(*args):
    # icacls writes localized status text in the Windows console code page.
    # No caller consumes it: preserve failure status without decoding that text.
    subprocess.run(["icacls", *map(str, args)], cwd=ROOT, check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def trusted_extraction(packager, manifest, archive, trusted):
    verified = packager.verify(manifest, archive)
    trusted.mkdir(parents=True, exist_ok=False)
    for name, (payload, _) in packager.safe_members(archive, verified["files"]).items():
        path = trusted / name.removeprefix("bin/")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
    shutil.copyfile(manifest, trusted / "package-manifest.json")
    # Inheritance flags apply to directories. Applying (OI)(CI) recursively to
    # regular files can leave an empty DACL while icacls reports success.
    for entry in [trusted] + sorted(trusted.rglob("*")):
        flags = "(OI)(CI)" if entry.is_dir() else ""
        apply_acl(entry, "/inheritance:r", "/grant:r",
                f"*S-1-5-32-544:{flags}F", f"*S-1-5-18:{flags}F", f"*S-1-5-11:{flags}RX")
    apply_acl(trusted, "/setowner", "*S-1-5-32-544", "/T")
    return verified


def restage(args):
    if platform.system() != "Windows":
        raise ValueError("Native Windows required")
    if command("git", "rev-parse", "HEAD") != args.expected_sha:
        raise ValueError("Restaged package does not belong to the candidate checkout")
    command("git", "diff", "--exit-code", "HEAD", "--")
    packager = module("package-standalone")
    manifest, archive = args.manifest.resolve(strict=True), args.archive.resolve(strict=True)
    verified = packager.verify(manifest, archive)
    package_revision = args.package_revision or args.expected_sha
    if not re.fullmatch(r"[a-f0-9]{40}", package_revision):
        raise ValueError("Restaging requires an exact committed package revision")
    if verified["revision"] != package_revision or verified["target"] != packager.host_target():
        raise ValueError("Replay requires the explicitly bound host-matching Windows package")
    trusted = args.output.resolve() / "administrator-package"
    trusted_extraction(packager, manifest, archive, trusted)
    return dict(source_revision=package_revision, consumer_revision=args.expected_sha,
                target=verified["target"], manifest=str(manifest),
                archive=str(archive), administrator_cli=str(trusted / "hiroute.exe"),
                worker_daemon=str(trusted / "hirouted.exe"), package_directory=str(trusted),
                archive_sha256=packager.digest(archive), manifest_sha256=packager.digest(manifest),
                production_runtime="not_executed", permission_source="fresh_verified_complete_extraction")


def prepare(args):
    if platform.system() != "Windows":
        raise ValueError("Native Windows required")
    revision = command("git", "rev-parse", "HEAD")
    if not re.fullmatch(r"[a-f0-9]{40}", args.expected_sha) or revision != args.expected_sha:
        raise ValueError("Package source does not match exact candidate")
    command("git", "diff", "--exit-code", "HEAD", "--")
    packager = module("package-standalone")
    target = packager.host_target()
    if target not in ("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"):
        raise ValueError("Native Windows package target required")
    pin, _ = module("build-cpa").pinned_source()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    if args.cpa_evidence is None:
        previous = output / "cpa"
        previous.mkdir()
        provenance = module("build-cpa").build(args.source_repo, target, previous / "cliproxyapi.exe",
                                             previous / "go-bootstrap-tests.log")
        provenance.update(build_run_id=args.build_run_id, validation_revision=revision)
        (previous / "provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
        shutil.copyfile(previous / "cliproxyapi.LICENSE", previous / "CPA.LICENSE")
    else:
        previous = args.cpa_evidence.resolve(strict=True)
    provenance = json.loads((previous / "provenance.json").read_text(encoding="utf-8-sig"))
    if any(provenance.get(key) != value for key, value in pin.items()):
        raise ValueError("Reused CPA evidence does not match the unchanged source and patch")
    cpa = previous / "cliproxyapi.exe"
    if packager.digest(cpa) != provenance["sha256"] or cpa.stat().st_size != provenance["size"]:
        raise ValueError("CPA bytes do not match the original native build evidence")
    passed = []
    for line in (previous / "go-bootstrap-tests.log").read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("Action") == "pass" and event.get("Test"):
            passed.append(event["Test"])
    if sorted(passed) != ["TestLocalMachinePasswordPreservesRemotePolicy", "TestLocalManagementPasswordPipeBounds"]:
        raise ValueError("Original CPA native build lacks the required bootstrap security cases")
    frontend = ROOT / "apps/desktop"
    command("npm.cmd", "ci", "--prefix", frontend)
    command("npm.cmd", "run", "build", "--prefix", frontend)
    notices = output / "notices"
    command(sys.executable, ROOT / "scripts/collect-third-party-licenses.py", "--cargo-target", target,
            "--cpa-source-repo", args.source_repo.resolve(strict=True),
            "--npm-root", frontend, "--output", notices)
    version = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())["version"]
    base = [sys.executable, ROOT / "scripts/package-standalone.py", "build",
            "--version", version, "--revision", revision, "--target", target,
            "--hiroute", ROOT / "target/debug/hiroute.exe",
            "--hirouted", ROOT / "target/debug/hirouted.exe", "--cpa-binary", cpa,
            "--cpa-version", pin["version"], "--cpa-license", previous / "CPA.LICENSE",
            "--notices", notices, "--console-assets", frontend / "dist"]
    first = json.loads(command(*base, "--output", output / "package"))
    second = json.loads(command(*base, "--output", output / "repeated-package"))
    archive, manifest = Path(first["archive"]), Path(first["manifest"])
    if (packager.digest(archive) != packager.digest(Path(second["archive"]))
            or manifest.read_bytes() != Path(second["manifest"]).read_bytes()):
        raise ValueError("Production package generation is not deterministic")
    # The administrator receives a separate complete verified extraction. Its source
    # does not come from an owner-writable prepared installation or request parameter.
    trusted = output / "administrator-package"
    trusted_extraction(packager, manifest, archive, trusted)
    result = dict(version=1, source_revision=revision, target=target,
                  manifest=str(manifest), archive=str(archive),
                  administrator_cli=str(trusted / "hiroute.exe"),
                  worker_daemon=str(trusted / "hirouted.exe"), package_directory=str(trusted),
                  archive_sha256=packager.digest(archive), manifest_sha256=packager.digest(manifest),
                  deterministic_publication="green", production_runtime="not_executed",
                  cpa_original_run=provenance.get("build_run_id", args.cpa_original_run),
                  cpa_original_validation_revision=provenance.get("validation_revision", args.cpa_original_revision),
                  cpa_original_provenance=provenance)
    (output / "package-evidence.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-sha", required=True)
    parser.add_argument("--source-repo", type=Path)
    parser.add_argument("--restage-only", action="store_true")
    parser.add_argument("--package-revision", help="Explicit baseline revision when restaging an upgrade package")
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--cpa-evidence", type=Path)
    parser.add_argument("--build-run-id")
    parser.add_argument("--cpa-original-run")
    parser.add_argument("--cpa-original-revision")
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    if arguments.restage_only and (arguments.manifest is None or arguments.archive is None):
        parser.error("Restaging requires both manifest and archive")
    if not arguments.restage_only and arguments.source_repo is None:
        parser.error("Building requires the exact CPA source repository")
    if arguments.package_revision is not None and not arguments.restage_only:
        parser.error("An explicit package revision is valid only for restaging")
    try:
        print(json.dumps(restage(arguments) if arguments.restage_only else prepare(arguments)))
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
