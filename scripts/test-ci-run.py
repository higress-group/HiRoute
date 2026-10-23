#!/usr/bin/env python3
"""CI adapter regression tests; no Rust builds or remote workbench access."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("ci_run", Path(__file__).with_name("ci-run.py"))
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.git("init", "-q")
        self.git("config", "user.name", "CI test")
        self.git("config", "user.email", "ci@example.invalid")
        (self.root / "tracked").write_text("original")
        self.git("add", "tracked")
        self.git("-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
        self.sha = self.git("rev-parse", "HEAD").strip()

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.root, text=True)

    def run_case(self, code, sha=None, timeout=5, require=False):
        self.output = self.root / "output"
        status = ci.execute([sys.executable, "-c", code], self.root, self.output,
                            sha or self.sha, timeout, require)
        return status, json.loads((self.output / "result.json").read_text())

    def test_success_keeps_revision_and_cleans_temporary_directory(self):
        status, result = self.run_case("import os; print(os.environ['TMPDIR'])")
        self.assertEqual(status, 0)
        self.assertEqual(result["revision"], self.sha)
        self.assertEqual(result["scenario_state"], "not_assessed")
        self.assertFalse(Path((self.output / "command.log").read_text().strip()).exists())

    def test_nonzero_exit_is_preserved(self):
        status, result = self.run_case("raise SystemExit(17)")
        self.assertEqual(status, 1)
        self.assertEqual(result["process_exit"], 17)

    def test_ci_completion_emits_failure_feedback_in_result(self):
        text = ('Running tests/runtime.rs (target/debug/deps/runtime-abcdef0123456789)\n'
                'running 1 test\ntest regression ... FAILED\n'
                'test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s\n')
        status, result = self.run_case('print(' + repr(text) + '); raise SystemExit(101)')
        self.assertEqual(status, 1)
        report = json.loads(Path(result['validation_report']['path']).read_text())
        self.assertEqual(report['modules'][0]['failures'], ['regression'])
        self.assertEqual(report['process_exit'], 101)

    def test_wrong_revision_does_not_execute(self):
        status, result = self.run_case("raise SystemExit(0)", sha="0" * 40)
        self.assertEqual(status, 1)
        self.assertIsNone(result["process_exit"])

    def test_dirty_tracked_source_is_rejected(self):
        (self.root / "tracked").write_text("modified")
        status, result = self.run_case("raise SystemExit(0)")
        self.assertEqual(status, 1)
        self.assertIsNone(result["process_exit"])

    def test_timeout_is_not_success(self):
        status, result = self.run_case("import time; time.sleep(30)", timeout=0.1)
        self.assertEqual(status, 1)
        self.assertEqual(result["error"], "timeout")
        self.assertLess(result["process_exit"], 0)

    def test_zero_selected_tests_is_not_success(self):
        status, result = self.run_case("print('test result: ok. 0 passed; 0 failed; 3 ignored')", require=True)
        self.assertEqual(status, 1)
        self.assertEqual(result["process_exit"], 0)

    def test_nonzero_tests(self):
        status, result = self.run_case("print('test result: ok. 3 passed; 0 failed; 0 ignored')", require=True)
        self.assertEqual(status, 0)
        self.assertEqual(result["tests"]["passed"], 3)

    def test_external_target_override_is_rejected(self):
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": "/tmp/shared-ci-target"}):
            status, result = self.run_case("raise SystemExit(0)")
        self.assertEqual(status, 1)
        self.assertIsNone(result["process_exit"])

    def test_cargo_launch_records_environment_and_requested_limit_without_measuring_threads(self):
        # A real child process observes the same environment that result/report record.
        # This is runner evidence, not execution of HiRoute Rust tests.
        binary = self.root / "bin"
        binary.mkdir()
        fixture = ("#!" + sys.executable + "\nimport json, os, sys\n"
                   "if 'metadata' in sys.argv:\n"
                   "    print(json.dumps({'target_directory': os.path.join(os.getcwd(), 'target')}))\n"
                   "elif '--version' in sys.argv:\n    print('fixture toolchain')\n"
                   "else:\n    print('RUST_TEST_THREADS=' + str(os.environ.get('RUST_TEST_THREADS')))\n"
                   "    print('test result: ok. 1 passed; 0 failed; 0 ignored')\n")
        for name in ("cargo", "rustc"):
            (binary / name).write_text(fixture)
            (binary / name).chmod(0o755)
        cases = [(None, [], "default", None), ("4", [], "environment", 4),
                 ("4", ["--", "--test-threads", "2"], "argument", 2)]
        for index, (environment, arguments, source, limit) in enumerate(cases):
            output = self.root / ("cargo-" + str(index))
            with self.subTest(source=source), patch.dict(os.environ, {
                    "PATH": str(binary) + os.pathsep + os.environ["PATH"]}):
                os.environ.pop("RUST_TEST_THREADS", None)
                if environment is not None:
                    os.environ["RUST_TEST_THREADS"] = environment
                with patch.object(ci.os, "cpu_count", return_value=16):
                    self.assertEqual(ci.execute(["cargo", "test", *arguments], self.root, output,
                                                self.sha, 5, True), 0)
            result = json.loads((output / "result.json").read_text())
            report = json.loads((output / "validation-report.json").read_text())
            harness = result["harness_concurrency"]
            self.assertEqual(harness["environment_threads"], environment)
            self.assertEqual(harness["configuration_source"], source)
            self.assertEqual(harness["configured_thread_limit"], limit)
            self.assertEqual(harness["policy"], "selected-command")
            self.assertIsNone(harness["effective_threads"])
            self.assertEqual(harness["effective_state"], "unknown")
            self.assertEqual(report["context"]["harness_concurrency"], harness)
            self.assertIn("RUST_TEST_THREADS=" + str(environment), (output / "command.log").read_text())
            self.assertIn("runtime concurrency is unknown", (output / "validation-report.md").read_text())


if __name__ == "__main__":
    unittest.main()
