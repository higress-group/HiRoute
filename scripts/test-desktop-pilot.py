#!/usr/bin/env python3
"""Focused safety tests for the Desktop Pilot launcher."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SPEC = importlib.util.spec_from_file_location(
    "desktop_pilot", Path(__file__).with_name("desktop-pilot.py")
)
pilot = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pilot)


class ConfigurationTests(unittest.TestCase):
    def test_relocated_driver_uses_explicit_repository_and_local_default_still_works(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            repository = root / "repository"
            config_root = repository / "apps/desktop/src-tauri"
            config_root.mkdir(parents=True)
            production = {"app": {"windows": [{"label": "main", "title": "repository window"}]}}
            overlay = {"identifier": "fixture.pilot", "app": {"windows": []}}
            (config_root / "tauri.conf.json").write_text(json.dumps(production))
            (config_root / "tauri.pilot.conf.json").write_text(json.dumps(overlay))
            frontend = root / "frontend"
            frontend.mkdir()
            (frontend / "index.html").write_text("fixture")
            for explicit_repository in (True, False):
                with self.subTest(explicit_repository=explicit_repository):
                    scripts = root / "tooling-bundle" if explicit_repository else repository / "scripts"
                    scripts.mkdir()
                    driver = scripts / "desktop-pilot.py"
                    driver.write_text(Path(pilot.__file__).read_text())
                    command = [sys.executable, str(driver), "config", "--frontend-dist", str(frontend)]
                    if explicit_repository:
                        command += ["--repo", str(repository)]
                    result = subprocess.run(command, text=True, capture_output=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    config = json.loads(result.stdout)
                    self.assertEqual(config["identifier"], "fixture.pilot")
                    self.assertEqual(config["build"], {"frontendDist": str(frontend)})
                    self.assertEqual(config["app"]["windows"], [
                        {"label": "main", "title": "repository window", "incognito": True}
                    ])
                    if explicit_repository:
                        # A wrapper-injected --repo must not be overridden via an abbreviation.
                        rejected = subprocess.run(
                            [*command, "--rep", str(root / "different-repository")],
                            text=True, capture_output=True,
                        )
                        self.assertEqual(rejected.returncode, 2, rejected.stderr)
                        self.assertIn("unrecognized arguments: --rep", rejected.stderr)
            self.assertEqual(json.loads((config_root / "tauri.conf.json").read_text()), production)
            self.assertEqual(json.loads((config_root / "tauri.pilot.conf.json").read_text()), overlay)


class ProcessOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        root = Path(self.temporary.name).resolve()
        self.app = root / "hiroute-desktop"
        self.daemon = root / "hirouted"
        for path, content in ((self.app, b"app"), (self.daemon, b"daemon")):
            path.write_bytes(content)
            path.chmod(0o700)
        self.row = {
            "pid": 101,
            "pgid": 101,
            "owner_uid": os.geteuid(),
            "artifacts": {
                "app": pilot.executable_artifact(self.app),
                "daemon": pilot.executable_artifact(self.daemon),
            },
            "process_identities": [
                {
                    "pid": 101,
                    "role": "app",
                    "start_time": "app-start",
                    "executable": str(self.app),
                },
                {
                    "pid": 102,
                    "role": "daemon",
                    "start_time": "daemon-start",
                    "executable": str(self.daemon),
                },
            ],
        }

    def tearDown(self):
        self.temporary.cleanup()

    @staticmethod
    def member(pid, ppid, pgid):
        return {
            "pid": pid,
            "ppid": ppid,
            "pgid": pgid,
            "uid": os.geteuid(),
            "state": "S",
            "command": "ignored-by-identity-check",
        }

    def start_time(self, pid):
        return {101: "app-start", 102: "daemon-start"}[pid]

    def executable(self, pid):
        return {101: self.app, 102: self.daemon}[pid]

    def test_pid_reuse_refuses_signal(self):
        members = [self.member(101, 1, 101)]
        with mock.patch.object(pilot, "process_rows", return_value=members), mock.patch.object(
            pilot, "process_start_time", return_value="different-start"
        ), mock.patch.object(pilot, "process_executable", return_value=self.app), mock.patch.object(
            pilot.os, "killpg"
        ) as killpg:
            with self.assertRaisesRegex(ValueError, "PID 101 was reused"):
                pilot.stop_group(self.row, timeout=0)
        killpg.assert_not_called()

    def test_one_instance_never_signals_the_other_group(self):
        first = [self.member(101, 1, 101), self.member(102, 101, 101)]
        other = [self.member(201, 1, 201), self.member(202, 201, 201)]
        with mock.patch.object(pilot, "process_rows", side_effect=[first + other, other]), mock.patch.object(
            pilot, "process_start_time", side_effect=self.start_time
        ), mock.patch.object(pilot, "process_executable", side_effect=self.executable), mock.patch.object(
            pilot.os, "killpg"
        ) as killpg:
            stopped = pilot.stop_group(self.row, timeout=0)
        self.assertEqual(stopped, [101, 102])
        killpg.assert_called_once_with(101, signal.SIGTERM)

    def test_unprovable_executable_refuses_signal(self):
        members = [self.member(101, 1, 101)]
        with mock.patch.object(pilot, "process_rows", return_value=members), mock.patch.object(
            pilot, "process_start_time", return_value="app-start"
        ), mock.patch.object(
            pilot, "process_executable", return_value=Path("/bin/true")
        ), mock.patch.object(pilot.os, "killpg") as killpg:
            with self.assertRaisesRegex(ValueError, "executable identity changed"):
                pilot.stop_group(self.row, timeout=0)
        killpg.assert_not_called()

    def test_missing_leader_does_not_reduce_daemon_check_to_basename(self):
        members = [self.member(102, 1, 101)]
        with mock.patch.object(pilot, "process_rows", return_value=members), mock.patch.object(
            pilot, "process_start_time", return_value="different-daemon-start"
        ), mock.patch.object(pilot, "process_executable", return_value=self.daemon), mock.patch.object(
            pilot.os, "killpg"
        ) as killpg:
            with self.assertRaisesRegex(ValueError, "PID 102 was reused"):
                pilot.stop_group(self.row, timeout=0)
        killpg.assert_not_called()

    def test_unrecorded_group_member_refuses_signal(self):
        members = [self.member(101, 1, 101), self.member(103, 101, 101)]
        with mock.patch.object(pilot, "process_rows", return_value=members), mock.patch.object(
            pilot, "process_start_time", side_effect=self.start_time
        ), mock.patch.object(pilot, "process_executable", side_effect=self.executable), mock.patch.object(
            pilot.os, "killpg"
        ) as killpg:
            with self.assertRaisesRegex(ValueError, "unrecorded PID 103"):
                pilot.stop_group(self.row, timeout=0)
        killpg.assert_not_called()

    def test_remaining_members_after_sigkill_are_an_error(self):
        members = [self.member(101, 1, 101)]
        with mock.patch.object(pilot, "process_rows", return_value=members), mock.patch.object(
            pilot, "process_start_time", return_value="app-start"
        ), mock.patch.object(pilot, "process_executable", return_value=self.app), mock.patch.object(
            pilot.time, "monotonic", side_effect=[0, 1, 4, 7]
        ), mock.patch.object(pilot.os, "killpg") as killpg:
            with self.assertRaisesRegex(RuntimeError, "still has verified members after SIGKILL"):
                pilot.stop_group(self.row, timeout=0)
        self.assertEqual(
            killpg.call_args_list,
            [mock.call(101, signal.SIGTERM), mock.call(101, signal.SIGKILL)],
        )

    def test_owned_child_exiting_between_stop_snapshot_and_identity_read_is_stopped(self):
        members = [self.member(101, 1, 101), self.member(102, 101, 101)]
        for unavailable in ("start_time", "executable"):
            with self.subTest(unavailable=unavailable):
                starts = ["app-start", "daemon-start", "daemon-start"]
                executables = [self.app, self.daemon, self.daemon]
                if unavailable == "start_time":
                    starts[-1] = ProcessLookupError("process exited after ps")
                else:
                    executables[-1] = ProcessLookupError("process exited after start identity")
                with mock.patch.object(
                    pilot, "process_rows", side_effect=[members, [members[1]], []]
                ), mock.patch.object(
                    pilot, "process_start_time", side_effect=starts
                ), mock.patch.object(
                    pilot, "process_executable", side_effect=executables
                ), mock.patch.object(pilot.os, "killpg") as killpg:
                    stopped = pilot.stop_group(self.row, timeout=0)
                self.assertEqual(stopped, [101, 102])
                killpg.assert_called_once_with(101, signal.SIGTERM)

    def test_unresolved_pid_still_present_after_stop_is_not_assumed_exited(self):
        members = [self.member(101, 1, 101), self.member(102, 101, 101)]
        for current in (members[1], self.member(102, 1, 202)):
            with self.subTest(current_group=current["pgid"]):
                with mock.patch.object(
                    pilot, "process_rows", side_effect=[members, [members[1]], [current]]
                ), mock.patch.object(
                    pilot, "process_start_time",
                    side_effect=["app-start", "daemon-start", ProcessLookupError("identity unavailable")],
                ), mock.patch.object(
                    pilot, "process_executable", side_effect=self.executable
                ), mock.patch.object(pilot.os, "killpg") as killpg:
                    with self.assertRaisesRegex(ProcessLookupError, "identity unavailable"):
                        pilot.stop_group(self.row, timeout=0)
                killpg.assert_called_once_with(101, signal.SIGTERM)

    def test_replaced_child_after_sigterm_refuses_further_signal(self):
        members = [self.member(101, 1, 101), self.member(102, 101, 101)]
        with mock.patch.object(
            pilot, "process_rows", side_effect=[members, [members[1]]]
        ), mock.patch.object(
            pilot, "process_start_time", side_effect=["app-start", "daemon-start", "new-daemon-start"]
        ), mock.patch.object(
            pilot, "process_executable", side_effect=self.executable
        ), mock.patch.object(pilot.os, "killpg") as killpg:
            with self.assertRaisesRegex(ValueError, "PID 102 was reused"):
                pilot.stop_group(self.row, timeout=0)
        killpg.assert_called_once_with(101, signal.SIGTERM)

    def test_exit_race_rechecks_new_group_members_before_declaring_stop_complete(self):
        members = [self.member(101, 1, 101), self.member(102, 101, 101)]
        with mock.patch.object(
            pilot, "process_rows", side_effect=[members, [members[1]], [self.member(103, 1, 101)]]
        ), mock.patch.object(
            pilot, "process_start_time",
            side_effect=["app-start", "daemon-start", ProcessLookupError("exited")],
        ), mock.patch.object(
            pilot, "process_executable", side_effect=self.executable
        ), mock.patch.object(pilot.os, "killpg") as killpg:
            with self.assertRaisesRegex(ValueError, "unrecorded PID 103"):
                pilot.stop_group(self.row, timeout=0)
        killpg.assert_called_once_with(101, signal.SIGTERM)

    def test_child_exiting_during_sigkill_wait_is_also_stopped(self):
        members = [self.member(101, 1, 101), self.member(102, 101, 101)]
        with mock.patch.object(
            pilot, "process_rows", side_effect=[members, [members[1]], [members[1]], [], []]
        ), mock.patch.object(
            pilot, "process_start_time",
            side_effect=["app-start", "daemon-start", "daemon-start", ProcessLookupError("exited")],
        ), mock.patch.object(
            pilot, "process_executable", side_effect=self.executable
        ), mock.patch.object(
            pilot.time, "monotonic", side_effect=[0, 1, 2, 3]
        ), mock.patch.object(pilot.os, "killpg") as killpg:
            stopped = pilot.stop_group(self.row, timeout=0)
        self.assertEqual(stopped, [101, 102])
        self.assertEqual(
            killpg.call_args_list,
            [mock.call(101, signal.SIGTERM), mock.call(101, signal.SIGKILL)],
        )

    @unittest.skipUnless(sys.platform == "darwin", "requires the native macOS process identity API")
    def test_native_macos_exit_between_ps_and_proc_pidinfo_completes_stop(self):
        # Desktop's native build already requires the platform C compiler. Build an
        # owned fixture: relocated Apple system binaries may be killed before exec.
        subprocess.run(
            ["/usr/bin/cc", "-x", "c", "-", "-o", str(self.app)],
            input="#include <unistd.h>\nint main(void) { sleep(30); return 0; }\n",
            text=True,
            capture_output=True,
            check=True,
        )
        self.app.chmod(0o700)
        self.row["artifacts"]["app"] = pilot.executable_artifact(self.app)
        process = subprocess.Popen([str(self.app), "30"], start_new_session=True)
        real_rows = pilot.process_rows
        real_start_time = pilot.process_start_time
        try:
            self.row.update(pid=process.pid, pgid=process.pid, process_identities=[{
                "pid": process.pid,
                "role": "app",
                "start_time": real_start_time(process.pid),
                "executable": str(pilot.process_executable(process.pid)),
            }])
            snapshots = []
            native_lookup_failures = []

            def snapshot_with_one_exit_race():
                if len(snapshots) == 1:
                    # Deliver the already observed ps snapshot after the real TERM
                    # exits/reaps our child. proc_pidinfo must run against the OS.
                    process.wait(timeout=5)
                    snapshots.append(snapshots[0])
                    return snapshots[0]
                current = real_rows()
                snapshots.append(current)
                return current

            def native_start_time(pid):
                try:
                    return real_start_time(pid)
                except ProcessLookupError:
                    native_lookup_failures.append(pid)
                    raise

            with mock.patch.object(
                pilot, "process_rows", side_effect=snapshot_with_one_exit_race
            ), mock.patch.object(pilot, "process_start_time", side_effect=native_start_time):
                self.assertEqual(pilot.stop_group(self.row, timeout=0), [process.pid])
            self.assertEqual(native_lookup_failures, [process.pid])
            self.assertIsNotNone(process.returncode)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


class ManagedBuildTests(unittest.TestCase):
    SHA = "a" * 40
    RUN_ID = "b" * 32

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        temporary = Path(self.temporary.name).resolve()
        self.state = temporary / "local-rust"
        self.runs = self.state / "runs"
        self.checkouts = self.state / "checkouts"
        for path in (self.state, self.runs, self.checkouts):
            path.mkdir(mode=0o700)
        self.checkout = self.checkouts / self.RUN_ID
        self.debug = self.checkout / "target" / "debug"
        self.debug.mkdir(parents=True, mode=0o700)
        self.app = self.debug / "hiroute-desktop"
        self.daemon = self.debug / "hirouted"
        for path in (self.app, self.daemon):
            path.write_bytes(path.name.encode())
            path.chmod(0o700)
        self.result_directory = self.runs / self.RUN_ID
        self.result_directory.mkdir(mode=0o700)
        self.session_root = temporary / "must-not-be-created"

    def tearDown(self):
        self.temporary.cleanup()

    def save_result(self, command):
        result = {
            "id": self.RUN_ID,
            "kind": "managed",
            "checkout": str(self.checkout),
            "sha": self.SHA,
            "command": command,
            "status": "terminal",
            "process_exit": 0,
            "keep": True,
            "scenario": "unassessed",
            "identity": pilot.directory_identity(self.checkout),
            "target_identity": pilot.directory_identity(self.checkout / "target"),
            "debug_identity": pilot.directory_identity(self.debug),
        }
        (self.result_directory / "result.json").write_text(json.dumps(result), encoding="utf-8")

    def git_output(self, checkout, *arguments):
        values = {
            ("rev-parse", "--show-toplevel"): str(self.checkout),
            ("rev-parse", "HEAD"): self.SHA,
            ("status", "--porcelain", "--untracked-files=no"): "",
        }
        return values[arguments]

    def args(self, app):
        return argparse.Namespace(
            build_run=self.RUN_ID,
            app=str(app),
            source_sha=self.SHA,
            root=str(self.session_root),
            data_root=None,
            process_home=None,
            diagnostic_level="debug",
            timeout=0.1,
        )

    def test_release_build_is_rejected_before_root_or_process(self):
        self.save_result(
            [
                "cargo",
                "build",
                "--locked",
                "--release",
                "--features",
                "hiroute-desktop/desktop-runtime",
                "-p",
                "hiroute-desktop",
                "-p",
                "hiroute-daemon",
                "--bin",
                "hiroute-desktop",
                "--bin",
                "hirouted",
            ]
        )
        with mock.patch.object(pilot, "LOCAL_RUST_ROOT", self.state), mock.patch.object(
            pilot.subprocess, "Popen"
        ) as popen:
            with self.assertRaisesRegex(ValueError, "release/custom-profile"):
                pilot.start(self.args(self.app))
        popen.assert_not_called()
        self.assertFalse(self.session_root.exists())

    def test_wrong_artifact_is_rejected_before_root_or_process(self):
        self.save_result(
            [
                "cargo",
                "build",
                "--locked",
                "--features",
                "hiroute-desktop/desktop-pilot",
                "-p",
                "hiroute-desktop",
                "-p",
                "hiroute-daemon",
                "--bin",
                "hiroute-desktop",
                "--bin",
                "hirouted",
            ]
        )
        wrong = Path(self.temporary.name).resolve() / "wrong-hiroute-desktop"
        wrong.write_bytes(b"wrong")
        wrong.chmod(0o700)
        with mock.patch.object(pilot, "LOCAL_RUST_ROOT", self.state), mock.patch.object(
            pilot, "git_output", side_effect=self.git_output
        ), mock.patch.object(pilot.subprocess, "Popen") as popen:
            with self.assertRaisesRegex(ValueError, "managed Pilot debug artifact"):
                pilot.start(self.args(wrong))
        popen.assert_not_called()
        self.assertFalse(self.session_root.exists())

    def test_declared_source_sha_must_match_managed_result_before_launch(self):
        self.save_result(
            [
                "cargo",
                "build",
                "--locked",
                "--features",
                "hiroute-desktop/desktop-pilot",
                "-p",
                "hiroute-desktop",
                "-p",
                "hiroute-daemon",
                "--bin",
                "hiroute-desktop",
                "--bin",
                "hirouted",
            ]
        )
        args = self.args(self.app)
        args.source_sha = "c" * 40
        with mock.patch.object(pilot, "LOCAL_RUST_ROOT", self.state), mock.patch.object(
            pilot.subprocess, "Popen"
        ) as popen:
            with self.assertRaisesRegex(ValueError, "does not match the managed build result"):
                pilot.start(args)
        popen.assert_not_called()
        self.assertFalse(self.session_root.exists())

    def test_ambiguous_hirouted_build_is_rejected_before_root_or_process(self):
        self.save_result(
            [
                "cargo",
                "build",
                "--locked",
                "--features",
                "hiroute-desktop/desktop-pilot",
                "--bin",
                "hiroute-desktop",
                "--bin",
                "hirouted",
            ]
        )
        with mock.patch.object(pilot, "LOCAL_RUST_ROOT", self.state), mock.patch.object(
            pilot.subprocess, "Popen"
        ) as popen:
            with self.assertRaisesRegex(ValueError, "explicitly select hiroute-desktop"):
                pilot.start(self.args(self.app))
        popen.assert_not_called()
        self.assertFalse(self.session_root.exists())

    def test_agent_cli_is_attested_when_explicitly_built_with_the_candidate(self):
        cli = self.debug / "hiroute"
        cli.write_bytes(b"cli")
        cli.chmod(0o700)
        self.save_result(
            [
                "cargo",
                "build",
                "--locked",
                "--features",
                "hiroute-desktop/desktop-pilot",
                "-p",
                "hiroute-desktop",
                "-p",
                "hiroute-daemon",
                "-p",
                "hiroute-cli",
                "--bin",
                "hiroute-desktop",
                "--bin",
                "hirouted",
                "--bin",
                "hiroute",
            ]
        )
        with mock.patch.object(pilot, "LOCAL_RUST_ROOT", self.state), mock.patch.object(
            pilot, "git_output", side_effect=self.git_output
        ):
            build = pilot.verify_managed_pilot_build(self.RUN_ID, str(self.app), self.SHA)
        self.assertEqual(build["artifacts"]["cli"]["path"], str(cli))
        self.assertEqual(build["artifacts"]["cli"]["sha256"], pilot.file_sha256(cli))

    def test_partial_agent_cli_selection_is_rejected(self):
        self.save_result(
            [
                "cargo",
                "build",
                "--locked",
                "--features",
                "hiroute-desktop/desktop-pilot",
                "-p",
                "hiroute-desktop",
                "-p",
                "hiroute-daemon",
                "-p",
                "hiroute-cli",
                "--bin",
                "hiroute-desktop",
                "--bin",
                "hirouted",
            ]
        )
        with mock.patch.object(pilot, "LOCAL_RUST_ROOT", self.state), mock.patch.object(
            pilot, "git_output", side_effect=self.git_output
        ):
            with self.assertRaisesRegex(ValueError, "Agent smoke builds must explicitly select"):
                pilot.verify_managed_pilot_build(self.RUN_ID, str(self.app), self.SHA)


class ExistingDataRootTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        parent = Path(self.temporary.name).resolve()
        self.session = parent / "session"
        self.session.mkdir(mode=0o700)
        self.data = parent / "saved-test-data"
        self.data.mkdir(mode=0o700)

    def tearDown(self):
        self.temporary.cleanup()

    def test_existing_data_root_is_reused_without_copy(self):
        marker = self.data / "source-identity"
        marker.write_text("source/managed-same", encoding="utf-8")
        selected = pilot.select_data_root(self.session, str(self.data))
        self.assertEqual(selected, self.data)
        self.assertEqual((selected / marker.name).read_text(encoding="utf-8"), marker.read_text())
        self.assertFalse((self.session / "data").exists())

    def test_symlink_data_root_is_rejected(self):
        alias = Path(self.temporary.name) / "data-alias"
        alias.symlink_to(self.data, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "must not be a symlink"):
            pilot.select_data_root(self.session, str(alias))

    def test_launch_environment_isolates_process_home(self):
        runtime = self.session / "runtime"
        temporary = self.session / "tmp"
        for path in (runtime, temporary):
            path.mkdir(mode=0o700)
        with mock.patch.dict(os.environ, {"HOME": "/real-user-home",
                                        "CODEX_HOME": "/daily-codex",
                                        "CLAUDE_CONFIG_DIR": "/daily-claude",
                                        "QODER_CONFIG_DIR": "/daily-qoder"}):
            environment = pilot.launch_environment(runtime, temporary, self.data, self.data)
            self.assertEqual(os.environ["CODEX_HOME"], "/daily-codex")
        self.assertEqual(environment["HOME"], str(self.data))
        self.assertEqual(environment["CODEX_HOME"], str(self.data / ".codex"))
        self.assertEqual(environment["CLAUDE_CONFIG_DIR"], str(self.data / ".claude"))
        self.assertEqual(environment["QODER_CONFIG_DIR"], str(self.data / ".qoder"))
        self.assertEqual(environment["HIROUTE_DESKTOP_TEST_ROOT"], str(self.data))
        self.assertEqual(environment["XDG_RUNTIME_DIR"], str(runtime))
        self.assertEqual(environment["TMPDIR"], str(temporary))

    def test_isolated_native_context_does_not_borrow_provider_credentials(self):
        variables = {key: 'synthetic-parent-value' for key in (
            'OPENAI_API_KEY', 'ANTHROPIC_AUTH_TOKEN', 'CLAUDE_CODE_OAUTH_TOKEN',
            'CODEX_CONFIG', 'CODEX_API_KEY', 'AWS_PROFILE', 'GOOGLE_APPLICATION_CREDENTIALS',
            'OPENROUTER_API_KEY', 'AZURE_OPENAI_API_KEY', 'HIROUTE_RUN_TOKEN',
            'CLAUDE_CODE_USE_BEDROCK', 'QODER_PERSONAL_ACCESS_TOKEN', 'QODER_MODEL',
            'QODER_SUBAGENT_MODEL')}
        with mock.patch.dict(os.environ, variables):
            isolated = pilot.launch_environment(self.session, self.session, self.data, self.data)
            ordinary = pilot.launch_environment(self.session, self.session, self.data, None)
            for key in variables:
                self.assertNotIn(key, isolated)
                self.assertEqual(ordinary[key], variables[key])
                self.assertEqual(os.environ[key], variables[key])


class DiagnosticLevelSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.data = self.root / "data"
        self.data.mkdir(mode=0o700)
        self.runtime = self.root / "runtime"
        self.temporary_directory = self.root / "tmp"
        for path in (self.runtime, self.temporary_directory):
            path.mkdir(mode=0o700)

    def tearDown(self):
        self.temporary.cleanup()

    def environment(self, selection):
        with mock.patch.dict(os.environ, {pilot.DIAGNOSTIC_LEVEL_ENV: "warn"}):
            return pilot.launch_environment(
                self.runtime,
                self.temporary_directory,
                self.data,
                None,
                pilot.diagnostic_override(selection),
            )

    def test_default_selection_applies_debug_to_the_launched_process_only(self):
        self.assertEqual(pilot.diagnostic_override("debug"), "debug")
        with mock.patch.dict(os.environ, {pilot.DIAGNOSTIC_LEVEL_ENV: "warn"}) as environment:
            launched = self.environment("debug")
            self.assertEqual(launched[pilot.DIAGNOSTIC_LEVEL_ENV], "debug")
            self.assertEqual(environment[pilot.DIAGNOSTIC_LEVEL_ENV], "warn")
        # Only the one explicitly selected level reaches the isolated instance.
        for level in pilot.DIAGNOSTIC_LEVELS:
            self.assertEqual(self.environment(level)[pilot.DIAGNOSTIC_LEVEL_ENV], level)

    def test_persisted_selection_never_sets_or_inherits_an_override(self):
        self.assertIsNone(pilot.diagnostic_override("persisted"))
        self.assertNotIn(pilot.DIAGNOSTIC_LEVEL_ENV, self.environment("persisted"))

    def test_a_fifth_level_selection_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "unknown diagnostic level selection"):
            pilot.diagnostic_override("trace")

    def test_invalid_selection_fails_before_any_launch_side_effect(self):
        args = argparse.Namespace(
            build_run="b" * 32,
            app=str(self.root / "hiroute-desktop"),
            source_sha=None,
            root=str(self.root / "must-not-be-created"),
            data_root=None,
            process_home=None,
            diagnostic_level="persisted-ish",
            timeout=0.1,
        )
        with mock.patch.object(pilot.subprocess, "Popen") as popen:
            with self.assertRaisesRegex(ValueError, "unknown diagnostic level selection"):
                pilot.start(args)
        popen.assert_not_called()
        self.assertFalse((self.root / "must-not-be-created").exists())

    def test_default_start_argument_is_debug(self):
        parsed = pilot.build_parser().parse_args(
            ["start", "--app", "/tmp/hiroute-desktop", "--build-run", "b" * 32]
        )
        self.assertEqual(parsed.diagnostic_level, "debug")
        persisted = pilot.build_parser().parse_args(
            [
                "start",
                "--app",
                "/tmp/hiroute-desktop",
                "--build-run",
                "b" * 32,
                "--diagnostic-level",
                "persisted",
            ]
        )
        self.assertEqual(persisted.diagnostic_level, "persisted")
        with self.assertRaises(SystemExit):
            pilot.build_parser().parse_args(
                ["start", "--app", "/tmp/hiroute-desktop", "--build-run", "b" * 32, "--diagnostic-level", "trace"]
            )


class DiagnosticsIndexTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name).resolve()
        self.data = self.root / "data"
        self.desktop = self.data / "diagnostics" / "desktop"
        self.desktop.mkdir(parents=True, mode=0o700)

    def tearDown(self):
        self.temporary.cleanup()

    def record(self, event, level="info", **extra):
        row = {
            "schema": pilot.DIAGNOSTIC_RECORD_SCHEMA,
            "timestamp_ms": 1,
            "monotonic_ms": 0,
            "component": "desktop",
            "boot_id": "0" * 32,
            "sequence": 0,
            "level": level,
            "level_revision": 0,
            "event": event,
        }
        row.update(extra)
        return json.dumps(row) + "\n"

    def write_log(self, directory, text, name="current.jsonl"):
        path = directory / name
        path.write_text(text, encoding="utf-8")
        path.chmod(0o600)
        return path

    def test_index_reports_level_evidence_and_completeness_without_contents(self):
        text = self.record(
            {"level_applied": {"level": "debug", "revision": 0, "source": "smoke_override"}}
        ) + self.record({"stage_begin": {"stage": "ready_wait"}}, level="debug")
        (self.desktop / "current.jsonl").write_text(text, encoding="utf-8")
        report = pilot.diagnostics_index(self.data)
        self.assertFalse(report["complete"])
        desktop = report["roles"]["desktop"]
        self.assertEqual(desktop["state"], "present")
        self.assertEqual(desktop["log_bytes"], len(text))
        self.assertEqual(
            desktop["evidence"]["level_applied"],
            {"level": "debug", "revision": 0, "source": "smoke_override"},
        )
        self.assertEqual(desktop["evidence"]["degraded_events"], 0)
        self.assertEqual(desktop["evidence"]["records"], 2)
        self.assertEqual(report["roles"]["daemon"]["state"], "missing")

    def test_index_reports_gaps_and_writer_loss_without_contents(self):
        text = self.record(
            {"diagnostics_degraded": {"reason": "writer_write_failed", "os_errno": None}},
            level="warn",
        ) + self.record(
            {
                "writer_stats": {
                    "rotated": 0,
                    "flushes": 1,
                    "bytes_written": 10,
                    "write_failures": 2,
                    "lost_at_shutdown": 1,
                }
            },
            level="warn",
        )
        self.write_log(self.desktop, text)
        evidence = pilot.diagnostics_index(self.data)["roles"]["desktop"]["evidence"]
        self.assertEqual(evidence["degraded_events"], 1)
        self.assertEqual(evidence["degraded_reasons"], ["writer_write_failed"])
        self.assertEqual(evidence["write_failures"], 2)
        self.assertEqual(evidence["lost_at_shutdown"], 1)
        self.assertEqual(
            sorted(evidence),
            [
                "degraded_events",
                "degraded_reasons",
                "level_applied",
                "lost_at_shutdown",
                "records",
                "write_failures",
            ],
            "evidence carries only the bounded fields, never a raw record",
        )

    def test_the_newest_rotated_file_wins_and_a_partial_tail_is_skipped(self):
        self.write_log(
            self.desktop,
            self.record({"level_applied": {"level": "info", "revision": 4, "source": "persisted"}}),
            name="previous-1.jsonl",
        )
        # A record still being appended has no trailing newline and is skipped, not an error.
        self.write_log(
            self.desktop,
            self.record(
                {"level_applied": {"level": "debug", "revision": 6, "source": "persisted"}}
            )[:-1],
        )
        evidence = pilot.diagnostics_index(self.data)["roles"]["desktop"]["evidence"]
        self.assertEqual(evidence["level_applied"]["level"], "info")
        self.assertEqual(evidence["level_applied"]["revision"], 4)
        self.assertEqual(evidence["records"], 1)

    def test_the_applied_level_is_the_newest_complete_record(self):
        self.write_log(
            self.desktop,
            self.record({"level_applied": {"level": "info", "revision": 0, "source": "default"}})
            + self.record({"level_applied": {"level": "debug", "revision": 1, "source": "persisted"}}),
        )
        evidence = pilot.read_level_evidence(self.desktop)
        self.assertEqual(
            evidence["level_applied"],
            {"level": "debug", "revision": 1, "source": "persisted"},
            "a saved level must not be reported as the startup default",
        )

    def test_an_older_rotated_file_never_overrides_the_newest_level(self):
        # The newer file holds only a half-written record, so the newest readable one is the
        # rotated file's, not the record of the file older than that.
        self.write_log(
            self.desktop,
            self.record({"level_applied": {"level": "info", "revision": 1, "source": "persisted"}}),
            name="previous-1.jsonl",
        )
        self.write_log(
            self.desktop,
            self.record({"level_applied": {"level": "warn", "revision": 2, "source": "persisted"}}),
            name="previous-2.jsonl",
        )
        self.write_log(
            self.desktop,
            self.record({"level_applied": {"level": "error", "revision": 3, "source": "persisted"}})[
                :-1
            ],
        )
        evidence = pilot.read_level_evidence(self.desktop)
        self.assertEqual(
            evidence["level_applied"],
            {"level": "info", "revision": 1, "source": "persisted"},
        )

    def test_the_newest_stats_observation_wins_across_rotated_files(self):
        self.write_log(
            self.desktop,
            self.record(
                {
                    "writer_stats": {
                        "rotated": 1,
                        "flushes": 9,
                        "bytes_written": 90,
                        "write_failures": 4,
                        "lost_at_shutdown": 2,
                    }
                },
                level="warn",
            ),
        )
        self.write_log(
            self.desktop,
            self.record(
                {
                    "writer_stats": {
                        "rotated": 0,
                        "flushes": 1,
                        "bytes_written": 10,
                        "write_failures": 0,
                        "lost_at_shutdown": 0,
                    }
                }
            ),
            name="previous-1.jsonl",
        )
        evidence = pilot.read_level_evidence(self.desktop)
        self.assertEqual(
            (evidence["write_failures"], evidence["lost_at_shutdown"]),
            (4, 2),
            "an older file's snapshot must not overwrite the last observation",
        )

    def test_a_previous_boot_is_never_reported_as_the_current_level(self):
        self.write_log(
            self.desktop,
            self.record(
                {"level_applied": {"level": "debug", "revision": 7, "source": "persisted"}},
                boot_id="a" * 32,
            )
            + self.record(
                {
                    "writer_stats": {
                        "rotated": 0,
                        "flushes": 3,
                        "bytes_written": 30,
                        "write_failures": 3,
                        "lost_at_shutdown": 0,
                    }
                },
                level="warn",
                boot_id="a" * 32,
            ),
            name="previous-1.jsonl",
        )
        self.write_log(
            self.desktop,
            self.record({"stage_begin": {"stage": "ready_wait"}}, boot_id="b" * 32),
        )
        evidence = pilot.read_level_evidence(self.desktop)
        self.assertIsNone(
            evidence["level_applied"],
            "a restarted process must not inherit the previous boot's level",
        )
        self.assertIsNone(evidence["write_failures"])
        self.assertIsNone(evidence["lost_at_shutdown"])

    def test_the_current_boot_level_is_found_in_a_rotated_file(self):
        current = "b" * 32
        self.write_log(
            self.desktop,
            self.record(
                {"level_applied": {"level": "warn", "revision": 2, "source": "persisted"}},
                boot_id=current,
            )
            + self.record(
                {"level_applied": {"level": "debug", "revision": 9, "source": "persisted"}},
                boot_id="a" * 32,
            ),
            name="previous-1.jsonl",
        )
        self.write_log(
            self.desktop,
            self.record({"stage_begin": {"stage": "ready_wait"}}, boot_id=current)
            + self.record(
                {"level_applied": {"level": "error", "revision": 3, "source": "persisted"}},
                boot_id=current,
            ),
        )
        evidence = pilot.read_level_evidence(self.desktop)
        self.assertEqual(
            evidence["level_applied"],
            {"level": "error", "revision": 3, "source": "persisted"},
            "rotation must not hide the current boot's newest level",
        )

    def test_index_counts_unexpected_or_linked_entries_without_naming_them(self):
        (self.desktop / "current.jsonl").write_text("", encoding="utf-8")
        (self.desktop / "listener-secret.log").write_text("x", encoding="utf-8")
        (self.desktop / "current.jsonl-alias").symlink_to(self.desktop / "current.jsonl")
        desktop = pilot.diagnostics_index(self.data)["roles"]["desktop"]
        self.assertEqual(desktop["unexpected_entries"], 2)
        self.assertEqual([item["name"] for item in desktop["files"]], ["current.jsonl"])
        self.assertEqual(desktop["log_bytes"], 0)

    def test_evidence_reads_are_bounded_and_owned(self):
        oversized = self.write_log(
            self.desktop,
            self.record({"stage_begin": {"stage": "ready_wait"}}, pad="x" * pilot.MAX_DIAGNOSTIC_RECORD_BYTES),
        )
        with self.assertRaisesRegex(ValueError, "exceeds its bound"):
            pilot.read_level_evidence(self.desktop)
        oversized.unlink()
        target = self.write_log(self.desktop, self.record({"stage_begin": {"stage": "ready_wait"}}))
        alias = self.desktop / "current-alias.jsonl"
        alias.symlink_to(target)
        original = self.desktop / "current.jsonl"
        original.rename(self.desktop / "moved.jsonl")
        alias.rename(original)
        with self.assertRaisesRegex(ValueError, "not an owned real file"):
            pilot.read_level_evidence(self.desktop)
        original.unlink()
        evidence = pilot.read_level_evidence(self.desktop)
        self.assertEqual(evidence["records"], 0)
        self.assertIsNone(evidence["level_applied"])

    def test_wrong_schema_is_refused(self):
        self.write_log(
            self.desktop,
            json.dumps(
                {
                    "schema": "hiroute.diagnostic-event/v2",
                    "event": {"stage_begin": {"stage": "ready_wait"}},
                }
            )
            + "\n",
        )
        with self.assertRaisesRegex(ValueError, "schema is unexpected"):
            pilot.read_level_evidence(self.desktop)


if __name__ == "__main__":
    unittest.main()
