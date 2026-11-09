#!/usr/bin/env python3
"""Release admission and real OpenPGP subkey fixtures without production credentials."""
import importlib.util
import io
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("omarchy_release", SCRIPTS / "omarchy-release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class CandidateAdmission(unittest.TestCase):
    def test_a_recorded_version_cannot_enter_preparation_or_signing(self):
        tag = release.read(release.ROOT / "contracts/releases/index.v1.json")["latest_stable"]
        with patch.object(release, "output") as run:
            with self.assertRaisesRegex(ValueError, "newer than the recorded stable release"):
                release.check_candidate("a" * 40, tag[1:])
        run.assert_not_called()

    def test_success_on_another_ref_does_not_admit_a_release(self):
        repo = "example/repository"
        run = {"id": 42, "event": "workflow_dispatch", "status": "completed", "conclusion": "success",
               "path": ".github/workflows/build-release.yml", "head_repository": {"full_name": repo},
               "head_branch": "main"}
        with patch.object(release, "output", return_value=json.dumps(run).encode()):
            self.assertEqual(release.candidate_run(repo, 42), run)
        for change in ({"head_branch": "topic"}, {"conclusion": "failure"},
                       {"path": ".github/workflows/other.yml"}, {"event": "pull_request"},
                       {"head_repository": {"full_name": "someone/fork"}}):
            with self.subTest(change=change):
                with patch.object(release, "output", return_value=json.dumps({**run, **change}).encode()):
                    with self.assertRaisesRegex(ValueError, "successful build-release-candidates"):
                        release.candidate_run(repo, 42)

    def test_qualified_bytes_reject_a_same_size_replacement_and_duplicate_rows(self):
        with tempfile.TemporaryDirectory() as scratch:
            directory = Path(scratch)
            package = directory / "archive"
            package.write_bytes(b"verified")
            row = {"filename": package.name, "size": package.stat().st_size, "sha256": release.STANDALONE["digest"](package)}
            self.assertEqual(release.asset(directory, package.name, [row]), package)
            with self.assertRaisesRegex(ValueError, "one qualified identity"):
                release.asset(directory, package.name, [row, row])
            package.write_bytes(b"replaced")
            with self.assertRaisesRegex(ValueError, "bytes differ"):
                release.asset(directory, package.name, [row])


class InstallMetadata(unittest.TestCase):
    def test_signer_rejects_privileged_modes_links_and_extra_members(self):
        verify = runpy.run_path(str(SCRIPTS / "package-linux-desktop.py"))["verify_install_metadata"]
        executable = "usr/bin/hiroute-desktop"
        with tempfile.TemporaryDirectory() as scratch:
            package = Path(scratch) / "fixture.pkg.tar.xz"

            def write(binary, extra=None):
                with tarfile.open(package, "w:xz") as archive:
                    directory = tarfile.TarInfo("usr/bin")
                    directory.type, directory.mode = tarfile.DIRTYPE, 0o755
                    archive.addfile(directory)
                    archive.addfile(binary, io.BytesIO(b"binary"))
                    if extra is not None:
                        archive.addfile(extra)

            binary = tarfile.TarInfo(executable)
            binary.mode, binary.size = 0o755, 6
            write(binary)
            verify(package, {executable}, executable)

            for change in ({"mode": 0o4755}, {"mode": 0o2755}, {"uid": 1000},
                           {"type": tarfile.SYMTYPE, "linkname": "/bin/sh", "size": 0},
                           {"type": tarfile.LNKTYPE, "linkname": "/bin/sh", "size": 0},
                           {"pax_headers": {"SCHILY.xattr.security.capability": "fixture"}}):
                with self.subTest(change=change):
                    invalid = tarfile.TarInfo(executable)
                    invalid.mode, invalid.size = binary.mode, binary.size
                    for field, value in change.items():
                        setattr(invalid, field, value)
                    write(invalid)
                    with self.assertRaisesRegex(ValueError, "Unexpected Arch package"):
                        verify(package, {executable}, executable)

            extra = tarfile.TarInfo("etc")
            extra.type, extra.mode = tarfile.DIRTYPE, 0o755
            write(binary, extra)
            with self.assertRaisesRegex(ValueError, "Unexpected Arch package"):
                verify(package, {executable}, executable)


@unittest.skipUnless(all(shutil.which(tool) for tool in ("gpg", "gpgv", "gpgconf")),
                     "OpenPGP tools required")
class SigningSubkey(unittest.TestCase):
    def test_signs_only_with_the_expected_subkey_and_public_identity(self):
        verifier = runpy.run_path(str(SCRIPTS / "package-linux-desktop.py"))["verify_signature"]
        with tempfile.TemporaryDirectory(prefix="hroute-gpg-", dir="/tmp") as scratch:
            work = Path(scratch)
            home = work / "keys"
            home.mkdir(mode=0o700)
            gpg = ["gpg", "--no-options", "--homedir", str(home), "--batch", "--no-tty"]
            passphrase = b"fixture-only"

            def unlock(*arguments):
                return subprocess.run([*gpg, "--pinentry-mode", "loopback", "--passphrase-fd", "0", *arguments],
                                      input=passphrase + b"\n", check=True, stdout=subprocess.PIPE).stdout

            def fingerprints():
                lines = subprocess.check_output([*gpg, "--with-colons", "--list-keys"], text=True).splitlines()
                return [line.split(":")[9] for line in lines if line.startswith("fpr:")]

            try:
                unlock("--quick-generate-key", "HiRoute Test <test@example.invalid>", "ed25519", "cert", "1d")
                unlock("--quick-add-key", fingerprints()[0], "ed25519", "sign", "1d")
                primary, signer = fingerprints()
                public = work / "key.asc"
                public.write_bytes(subprocess.check_output([*gpg, "--armor", "--export", primary]))
                full_key = unlock("--armor", "--export-secret-keys", primary)
                subkey = unlock("--armor", "--export-secret-subkeys", signer + "!")
            finally:
                subprocess.run(["gpgconf", "--homedir", str(home), "--kill", "gpg-agent"], check=False)

            package = work / "test.pkg.tar.zst"
            package.write_bytes(b"exact fixture package")
            with patch.object(release, "PRIMARY", primary):
                with self.assertRaisesRegex(ValueError, "primary-key stub"):
                    release.sign_packages([package], public, full_key, passphrase, verifier)
                self.assertFalse(package.with_name(package.name + ".sig").exists())
                release.sign_packages([package], public, subkey, passphrase, verifier)
                self.assertEqual(verifier(package, public), primary)
            package.write_bytes(b"changed fixture")
            with self.assertRaises(subprocess.CalledProcessError):
                verifier(package, public)


if __name__ == "__main__":
    unittest.main()
