#!/usr/bin/env python3
"""Stage qualified Linux inputs and sign exact native Omarchy packages."""
import argparse
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "scripts"
ARCHITECTURES = ("x86_64", "aarch64")
KEY = "assets/packaging/omarchy-release-key.asc"
PRIMARY = "CB5F8ADEDC272ED8879D206C5642FA57EA8DF169"
QUALIFICATION = "hiroute.release-qualification/v1"
CANDIDATE = "hiroute.omarchy-release-inputs/v1"
SIGNED = "hiroute.omarchy-signed-packages/v1"
STANDALONE = runpy.run_path(str(SCRIPTS / "package-standalone.py"))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(path.read_bytes())


def write(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def output(*command):
    return subprocess.check_output(command, cwd=ROOT)


def asset(directory, name, rows):
    matches = [row for row in rows if row.get("filename") == name]
    require(len(matches) == 1, f"Expected one qualified identity for {name}")
    path = directory / name
    require(path.is_file() and not path.is_symlink()
            and path.stat().st_size == matches[0]["size"]
            and STANDALONE["digest"](path) == matches[0]["sha256"], f"Release asset bytes differ for {name}")
    return path


def stable_version(value):
    require(isinstance(value, str) and re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value),
            "Omarchy release preparation requires a stable version")
    return tuple(map(int, value.split(".")))


def check_candidate(revision, version):
    require(isinstance(revision, str) and re.fullmatch(r"[0-9a-f]{40}", revision),
            "Expected a full candidate SHA")
    prior = read(ROOT / "contracts/releases/index.v1.json")["latest_stable"]
    require(stable_version(version) > stable_version(prior.removeprefix("v")),
            "The Omarchy candidate must be newer than the recorded stable release")
    require(not output("git", "tag", "--list", "v" + version).strip(),
            "The release tag already exists")
    require(subprocess.run(["git", "merge-base", "--is-ancestor", revision, "HEAD"],
                           cwd=ROOT).returncode == 0, "Build the frozen candidate from reviewed main")


def standalone_name(version, revision, arch):
    return f"hiroute-{version}-{revision[:12]}-{arch}-unknown-linux-gnu.tar.gz"


def candidate_run(repository, run_id):
    run = json.loads(output("gh", "api", f"repos/{repository}/actions/runs/{run_id}"))
    require(run["id"] == run_id and run["event"] == "workflow_dispatch"
            and run["status"] == "completed" and run["conclusion"] == "success"
            and run["path"] == ".github/workflows/build-release.yml"
            and run["head_repository"]["full_name"] == repository
            and run["head_branch"] == "main",
            "Use a successful build-release-candidates run dispatched on main")
    return run


def qualified_runtime(directory, name, rows, revision, version, arch):
    archive = asset(directory, name, rows)
    manifest = asset(directory, name + ".json", rows)
    identity = STANDALONE["verify"](manifest, archive)
    require((identity["revision"], identity["version"], identity["target"])
            == (revision, version, f"{arch}-unknown-linux-gnu"), "Qualified runtime identity differs")
    return archive, manifest


def prepare(repository, run_id, destination):
    run = candidate_run(repository, run_id)
    revision, attempt = run["head_sha"], run["run_attempt"]
    require(re.fullmatch(r"[0-9a-f]{40}", revision) and type(attempt) is int and attempt > 0,
            "Invalid candidate run identity")

    def download(name, directory):
        subprocess.run(["gh", "run", "download", str(run_id), "-R", repository,
                        "--name", name, "--dir", str(directory)], check=True)

    with tempfile.TemporaryDirectory(prefix="hiroute-qualified-") as scratch:
        scratch = Path(scratch)
        download(f"qualified-release-{revision}-{attempt}", scratch)
        receipt = read(scratch / "qualification.json")
        require((receipt["schema"], receipt["revision"], receipt["run_id"], receipt["run_attempt"])
                == (QUALIFICATION, revision, run_id, attempt), "Candidate qualification receipt differs")
        version = receipt["version"]
        check_candidate(revision, version)
        destination.mkdir(parents=True)
        for arch in ARCHITECTURES:
            target = f"{arch}-unknown-linux-gnu"
            producer = receipt["producers"][target]["attempt"]
            require(type(producer) is int and 0 < producer <= attempt, "Invalid Linux producer attempt")
            download(f"release-{target}-{revision}-{producer}", scratch / arch)
            name = standalone_name(version, revision, arch)
            archives = list((scratch / arch).rglob(name))
            require(len(archives) == 1, "Expected one archive in the Linux producer artifact")
            archive, manifest = qualified_runtime(
                archives[0].parent, name, receipt["assets"], revision, version, arch)
            directory = destination / arch
            directory.mkdir()
            shutil.copyfile(archive, directory / name)
            shutil.copyfile(manifest, directory / manifest.name)
        write(destination / "candidate.json", {
            "schema": CANDIDATE, "revision": revision, "version": version,
            "run_id": run_id, "run_attempt": attempt, "assets": receipt["assets"],
        })


def previous(repository, destination):
    prior = read(ROOT / "contracts/releases/index.v1.json")["latest_stable"]
    version = prior.removeprefix("v")
    stable_version(version)
    release = read(ROOT / "contracts/releases" / (prior + ".json"))["release"]
    revision = release["revision"]
    require(release["tag"] == prior and release["repository"] == repository
            and release["channel"] == "stable"
            and output("git", "rev-parse", f"{prior}^{{commit}}").decode().strip() == revision,
            "Released predecessor differs from the contract ledger")
    arch = "x86_64"
    name = standalone_name(version, revision, arch)
    destination.mkdir(parents=True)
    subprocess.run(["gh", "release", "download", prior, "-R", repository, "--dir", str(destination),
                    "--pattern", name, "--pattern", name + ".json"], check=True)
    qualified_runtime(destination, name, release["artifacts"], revision, version, arch)
    (destination / "install-predecessor.py").write_bytes(output("git", "show", f"{revision}:scripts/install-standalone.py"))
    write(destination / "predecessor.json", {"revision": revision, "version": version, "archive": name})


def sign_packages(packages, public_key, signing_key, passphrase, verifier):
    require(signing_key and passphrase, "Configure both omarchy-signing environment secrets")
    with tempfile.TemporaryDirectory(prefix="hiroute-omarchy-signing-") as home:
        gpg = ["gpg", "--no-options", "--homedir", home, "--batch", "--no-tty"]
        try:
            subprocess.run([*gpg, "--quiet", "--import"], input=signing_key, check=True)
            listing = subprocess.check_output([*gpg, "--with-colons", "--list-secret-keys"], text=True)
            rows = [line.split(":") for line in listing.splitlines()]
            access = [(row[0], row[14]) for row in rows if row[0] in {"sec", "ssb"}]
            fingerprints = [row[9] for row in rows if row[0] == "fpr"]
            require(access == [("sec", "#"), ("ssb", "+")] and len(fingerprints) == 2
                    and fingerprints[0] == PRIMARY and re.fullmatch(r"[0-9A-F]{40}", fingerprints[1]),
                    "Import only the HiRoute signing subkey and its primary-key stub")
            for package in packages:
                signature = package.with_name(package.name + ".sig")
                subprocess.run([*gpg, "--pinentry-mode", "loopback", "--passphrase-fd", "0",
                                "--local-user", fingerprints[1] + "!", "--output", str(signature),
                                "--detach-sign", str(package)], input=passphrase + b"\n", check=True)
                require(verifier(package, public_key) == PRIMARY, "The signature does not match the published HiRoute key")
        finally:
            subprocess.run(["gpgconf", "--homedir", home, "--kill", "gpg-agent"], check=False)


def sign(inputs, unsigned, destination):
    signing_key = os.environ.pop("OMARCHY_SIGNING_SUBKEY", "").encode()
    passphrase = os.environ.pop("OMARCHY_SIGNING_PASSPHRASE", "").encode()
    metadata = read(inputs / "candidate.json")
    require(metadata["schema"] == CANDIDATE, "Expected qualified Omarchy input metadata")
    revision, version = metadata["revision"], metadata["version"]
    check_candidate(revision, version)
    packager = runpy.run_path(str(SCRIPTS / "package-linux-desktop.py"))

    with tempfile.TemporaryDirectory(prefix="hiroute-signed-", dir=destination.parent) as scratch:
        scratch = Path(scratch)
        stage = scratch / "assets"
        stage.mkdir()
        key = scratch / "omarchy-release-key.asc"
        key.write_bytes(output("git", "show", f"{revision}:{KEY}"))
        rows = []
        packages = []
        for arch in ARCHITECTURES:
            name = standalone_name(version, revision, arch)
            archive, manifest = qualified_runtime(
                inputs / arch, name, metadata["assets"], revision, version, arch)
            directory = unsigned / f"omarchy-unsigned-{arch}"
            candidates = [p for p in directory.iterdir() if p.is_file() and not p.is_symlink()]
            require(len(candidates) == 1, f"Expected one unsigned native package for {arch}")
            source, = candidates
            suffix = next((fmt for fmt in ("pkg.tar.xz", "pkg.tar.zst") if source.name.endswith("." + fmt)), None)
            require(suffix, "Unexpected Arch package format")
            package = stage / f"HiRoute-{version}-{revision[:12]}-omarchy-{arch}.{suffix}"
            shutil.copyfile(source, package)
            identity = packager["inspect_package"](package, manifest, archive)
            require(identity["architecture"] == arch, "Arch package architecture differs")
            package.with_name(package.name + ".sha256").write_text(f"{identity['sha256']}  {package.name}\n")
            rows.append({**identity, "kind": "desktop", "platform": "Omarchy", "distribution": "openpgp",
                         "format": suffix, "filename": package.name, "size": package.stat().st_size,
                         "standalone_sha256": STANDALONE["digest"](archive),
                         "standalone_manifest_sha256": STANDALONE["digest"](manifest)})
            packages.append(package)
        sign_packages(packages, key, signing_key, passphrase, packager["verify_signature"])
        for package, row in zip(packages, rows):
            for label, extension in (("signature", ".sig"), ("checksum", ".sha256")):
                companion = package.with_name(package.name + extension)
                row[f"{label}_filename"] = companion.name
                row[f"{label}_sha256"] = STANDALONE["digest"](companion)
                row[f"{label}_size"] = companion.stat().st_size
        write(stage / "signed.json", {
            "schema": SIGNED, "revision": revision, "version": version, "run_id": metadata["run_id"],
            "run_attempt": metadata["run_attempt"], "signing_fingerprint": PRIMARY, "packages": rows,
        })
        stage.rename(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subcommands = parser.add_subparsers(dest="action", required=True)
    stage = subcommands.add_parser("prepare")
    stage.add_argument("--repository", required=True)
    stage.add_argument("--run-id", required=True, type=int)
    predecessor = subcommands.add_parser("previous")
    predecessor.add_argument("--repository", required=True)
    signer = subcommands.add_parser("sign")
    signer.add_argument("--inputs", required=True, type=Path)
    signer.add_argument("--unsigned", required=True, type=Path)
    for command in (stage, predecessor, signer):
        command.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        if args.action == "prepare":
            prepare(args.repository, args.run_id, args.output)
        elif args.action == "previous":
            previous(args.repository, args.output)
        else:
            sign(args.inputs, args.unsigned, args.output)
    except (KeyError, OSError, TypeError, ValueError, subprocess.CalledProcessError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
