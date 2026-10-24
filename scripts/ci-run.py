#!/usr/bin/env python3
"""Synchronous Linux CI command runner; independent of the SSH workbench."""
import argparse
import importlib.util
import json
import os
import platform
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

spec = importlib.util.spec_from_file_location("validation_report", Path(__file__).with_name("validation-report.py"))
reporting = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reporting)


def test_counts(log):
    rows = re.findall(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", log)
    return dict(zip(("passed", "failed", "ignored"),
                    (sum(int(row[i]) for row in rows) for i in range(3))))


def execute(command, root, output, expected_sha, timeout, require_tests=False, feedback=None,
            harness_policy="selected-command"):
    output.mkdir(parents=True, exist_ok=False)
    result = {"id": output.name, "command": command, "expected_sha": expected_sha,
              "process_exit": None, "status": "failed", "scenario_state": "not_assessed",
              "started_at": time.time(), "cpu_count": os.cpu_count(),
              "runner_os": os.environ.get("RUNNER_OS"),
              "runner_arch": os.environ.get("RUNNER_ARCH"), "platform": sys.platform,
              "architecture": platform.machine(), "submitted_at": time.time(),
              **(feedback or {})}
    process = None
    related = []
    previous = {}

    def interrupted(signum, _frame):
        raise InterruptedError("CI signal %s" % signum)

    try:
        related = reporting.related_reports(output.parent, result)
        for sig in (signal.SIGTERM, signal.SIGINT):
            previous[sig] = signal.signal(sig, interrupted)
        actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
        result["revision"] = actual
        reporting.preflight(result, related)
        if not re.fullmatch(r"[0-9a-f]{40}", expected_sha) or actual != expected_sha:
            raise ValueError("checkout does not match the exact requested SHA")
        subprocess.run(["git", "diff", "--exit-code", "HEAD", "--"], cwd=root,
                       check=True, stdout=subprocess.DEVNULL)
        for key in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"):
            if os.environ.get(key):
                raise ValueError("remove " + key + "; use checkout-local target/")
        env = dict(os.environ)
        env.setdefault("CARGO_BUILD_JOBS", str(max(1, os.cpu_count() or 1)))
        env["CARGO_INCREMENTAL"] = "0"
        result["build_jobs"] = env["CARGO_BUILD_JOBS"]
        result["harness_concurrency"] = reporting.harness_concurrency(command, env, harness_policy)
        if command[0] == "cargo":
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                cwd=root, env=env, text=True))
            if Path(metadata["target_directory"]).resolve() != root / "target":
                raise ValueError("Cargo config must use checkout-local target/")
            for tool in ("rustc", "cargo"):
                result[tool] = subprocess.check_output([tool, "--version"], cwd=root, text=True).strip()
        # A short private TMPDIR keeps Unix socket paths below Linux's limit.
        with tempfile.TemporaryDirectory(prefix="hr-ci-", dir="/tmp") as temporary:
            env["TMPDIR"] = temporary
            with (output / "command.log").open("w") as log:
                result["command_started_at"] = time.time()
                process = subprocess.Popen(command, cwd=root, env=env, stdout=log,
                                           stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    result["process_exit"] = process.wait(timeout=timeout)
                finally:
                    # End descendants before removing their temporary files, including on cancel.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.wait()
                    result["command_finished_at"] = time.time()
            counts = test_counts((output / "command.log").read_text(errors="replace"))
            result["tests"] = counts
            if result["process_exit"] != 0:
                result["error"] = "command failed"
            elif require_tests and (not counts["passed"] or counts["failed"]):
                result["error"] = "expected non-zero passing Rust tests without failures"
            else:
                result["status"] = "completed"
    except subprocess.TimeoutExpired:
        result["error"] = "timeout"
    except Exception as error:
        result["error"] = str(error)
    finally:
        if process is not None and result["process_exit"] is None:
            result["process_exit"] = process.returncode
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        result["finished_at"] = time.time()
        result["checkout"] = str(root)
        reporting.write_report(output, result, related)
        (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps(result), flush=True)
        if result["status"] != "completed" and (output / "command.log").exists():
            print((output / "command.log").read_text(errors="replace")[-12000:], flush=True)
    return 0 if result["status"] == "completed" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--name", required=True)
    parser.add_argument("--expected-sha", default=os.environ.get("GITHUB_SHA"), required=False)
    parser.add_argument("--timeout", type=int, default=3600)
    parser.add_argument("--require-tests", action="store_true")
    parser.add_argument("--harness-policy", choices=("selected-command", "hosted-default"),
                        default="selected-command", help="Documented harness policy; does not change the command")
    reporting.add_arguments(parser)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not re.fullmatch(r"[a-z0-9-]+", args.name) or not command or args.timeout <= 0 or not args.expected_sha:
        parser.error("provide a safe name, exact SHA, positive timeout, and command")
    root = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()).resolve()
    return execute(command, root, root / "artifacts/ci" / args.name,
                   args.expected_sha, args.timeout, args.require_tests, reporting.options(args), args.harness_policy)


if __name__ == "__main__":
    raise SystemExit(main())
