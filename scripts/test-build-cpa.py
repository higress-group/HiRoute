#!/usr/bin/env python3
"""Deterministic CPA build-output regressions."""

import importlib.util
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True

spec = importlib.util.spec_from_file_location(
    "build_cpa", Path(__file__).with_name("build-cpa.py")
)
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class BuildOutputTests(unittest.TestCase):
    def test_every_release_target_builds_the_pinned_go_architecture(self):
        targets = {
            "aarch64-unknown-linux-gnu": ("linux", "arm64"),
            "x86_64-unknown-linux-gnu": ("linux", "amd64"),
            "aarch64-apple-darwin": ("darwin", "arm64"),
            "x86_64-apple-darwin": ("darwin", "amd64"),
        }
        pin, _ = builder.pinned_source()
        for target, expected in targets.items():
            with self.subTest(target=target), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                output = root / "out/cliproxyapi"
                calls = []

                def invoke(command, cwd=None, env=None, **kwargs):
                    calls.append(command)
                    if command[0] == "tar":
                        (Path(command[-1]) / "LICENSE").write_text("fixture license")
                    elif command[0] == "go":
                        self.assertEqual((env["GOOS"], env["GOARCH"]), expected)
                        self.assertEqual(env["CGO_ENABLED"], "0")
                        self.assertIn(pin["commit"], command[command.index("-ldflags") + 1])
                        Path(command[command.index("-o") + 1]).write_bytes(target.encode())
                    return subprocess.CompletedProcess(command, 0)

                with patch.object(builder.subprocess, "run", side_effect=invoke), \
                        patch.object(builder.subprocess, "check_output", side_effect=[b"archive", "binary: go1.fixture"]):
                    result = builder.build(root / "source", target, output)
                self.assertEqual(result["target"], target)
                self.assertEqual(result["commit"], pin["commit"])
                self.assertEqual(json.loads(output.with_suffix(".provenance.json").read_text()), result)
                self.assertEqual([command[0] for command in calls], ["tar", "git", "go"])

    def test_generated_files_have_distribution_safe_modes_under_shared_umask(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "out/cliproxyapi"
            fake_patch = root / "local.patch"
            fake_patch.write_text("")
            pin = {
                "repository": "https://example.invalid/cpa",
                "commit": "a" * 40,
                "version": "fixture-1",
                "built_at": "2026-09-21T00:00:00Z",
                "patch": fake_patch.name,
                "patch_sha256": "0" * 64,
            }

            def fake_run(command, cwd=None, **_kwargs):
                if command[0] == "tar":
                    (Path(command[command.index("-C") + 1]) / "LICENSE").write_text(
                        "fixture license\n"
                    )
                elif command[0] == "go":
                    Path(command[command.index("-o") + 1]).write_bytes(b"binary")
                return subprocess.CompletedProcess(command, 0)

            def fake_check_output(command, **kwargs):
                if command[0] == "git":
                    return b"fixture archive"
                self.assertEqual(command[:2], ["go", "version"])
                return "cliproxyapi: go1.fixture\n" if kwargs.get("text") else b""

            previous_umask = os.umask(0o002)
            try:
                with patch.object(builder, "pinned_source", return_value=(pin, fake_patch)), \
                        patch.object(builder.subprocess, "run", side_effect=fake_run), \
                        patch.object(
                            builder.subprocess,
                            "check_output",
                            side_effect=fake_check_output,
                        ):
                    builder.build(root / "source", "x86_64-unknown-linux-gnu", output)
            finally:
                os.umask(previous_umask)

            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o755)
            self.assertEqual(
                stat.S_IMODE(output.with_suffix(".LICENSE").stat().st_mode), 0o644
            )
            self.assertEqual(
                stat.S_IMODE(output.with_suffix(".provenance.json").stat().st_mode),
                0o644,
            )


if __name__ == "__main__":
    unittest.main()
