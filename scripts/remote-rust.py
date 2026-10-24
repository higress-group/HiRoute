#!/usr/bin/env python3
"""Small SSH build workbench, not a CI runner. Python 3.8+, stdlib only."""
import argparse
import contextlib
import fcntl
import hashlib
import importlib.util
import json
import os
import platform
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import types
import uuid

CONFIG = Path.home() / ".config/hiroute/remote-rust.json"
ROOT = ".local/share/hiroute-rust"
MAX_TMPDIR_BYTES = 40  # Leave at least 66 bytes for nested Linux Unix socket names.
MIN_FREE_BYTES = 30 * 1024**3
RETENTION_POLICIES = ("on-failure", "always", "never")
TERMINAL_STATUSES = ("completed", "failed", "blocked")
RECOVERABLE_SETUP_BLOCKERS = ("console_assets", "cache_server_unhealthy")
WORKBENCH_SLOTS = 8
SHARED_BUILD_JOBS = 8
EXCLUSIVE_BUILD_JOBS = WORKBENCH_SLOTS * SHARED_BUILD_JOBS


def workbench_path(value):
    """A remote HOME-relative path, safe in SSH commands and JSON bootstrap."""
    if not isinstance(value, str):
        raise ValueError("workbench must be a HOME-relative path")
    value = value[2:] if value.startswith("~/") else value
    if (not re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", value)
            or any(part in (".", "..") for part in value.split("/"))):
        raise ValueError("workbench must be a HOME-relative path without traversal or shell syntax")
    return value


def reporting():
    path = Path(__file__).with_name("validation-report.py")
    if not path.exists():  # Immutable SSH controller's matching helper.
        path = Path(__file__).with_name(Path(__file__).stem + "-report.py")
    spec = importlib.util.spec_from_file_location("validation_report", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def scheduling():
    path = Path(__file__).with_name("validation-schedule.py")
    if not path.exists():
        path = Path(__file__).with_name(Path(__file__).stem + "-schedule.py")
    spec = importlib.util.spec_from_file_location("validation_schedule", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def console_assets():
    path = Path(__file__).with_name("console-assets.py")
    if not path.exists():
        path = Path(__file__).with_name(Path(__file__).stem + "-console.py")
    spec = importlib.util.spec_from_file_location("console_assets", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def full_test(cargo):
    before = cargo[:cargo.index("--")] if "--" in cargo else cargo
    return cargo[:2] == ["cargo", "test"] and "--workspace" in before


class RunBlocked(RuntimeError):
    """The workbench could not start Cargo; this is not a Cargo test failure."""

    def __init__(self, blocker, message):
        super().__init__(message)
        self.blocker = blocker


def command(args, cwd=None, env=None):
    return subprocess.check_output(args, cwd=cwd, env=env, text=True).strip()


LOCKED_COMMAND = (
    "import subprocess,sys\n"
    "sys.exit(subprocess.call(sys.argv[1:], close_fds=True))\n"
)


def start_locked_command(argv, cwd, env, output, handles):
    """Keep runner locks alive after a worker crash without passing them to Cargo's children."""
    descriptors = tuple(handle if isinstance(handle, int) else handle.fileno() for handle in handles)
    return subprocess.Popen(
        [sys.executable, "-c", LOCKED_COMMAND, *argv], cwd=cwd, env=env,
        stdout=output, stderr=subprocess.STDOUT, start_new_session=True,
        pass_fds=descriptors,
    )


def save(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
    temporary.replace(path)


def validate(ref, sha, cargo):
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("SHA must be a full 40-character commit ID")
    if not ref.startswith("refs/heads/") or subprocess.call(
        ["git", "check-ref-format", ref], stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    ):
        raise ValueError("Use an advertised refs/heads/... branch")
    if len(cargo) < 2 or cargo[0] != "cargo" or cargo[1] not in (
        "build", "check", "test", "clippy", "bench", "fmt"
    ):
        raise ValueError("Expected cargo build/check/test/clippy/bench/fmt")
    if cargo[1] == "fmt" and cargo[2:] not in (
        ["--check"], ["--all", "--check"], ["--all", "--", "--check"]
    ):
        raise ValueError("Only read-only cargo fmt --check is supported")
    if any(arg.startswith(("--target-dir", "--config", "--manifest-path")) for arg in cargo):
        raise ValueError("Target/config/manifest overrides are not supported")


@contextlib.contextmanager
def lock(path):
    with path.open("a") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        yield handle


@contextlib.contextmanager
def capacity(root, exclusive, deadline=None, slots=None):
    # Shared locks on the legacy slots prevent overlap with old 20/60-job workers.
    wanted = WORKBENCH_SLOTS if exclusive else (slots or 1)
    if not 1 <= wanted <= WORKBENCH_SLOTS:
        raise ValueError("Capacity must reserve between one and eight slots")
    handles = []
    try:
        while not handles:
            try:
                for index in range(3):
                    handle = (root / ("slot-%s.lock" % index)).open("a")
                    handles.append(handle)
                    fcntl.flock(handle, fcntl.LOCK_SH | fcntl.LOCK_NB)
                admitted = 0
                for index in range(WORKBENCH_SLOTS):
                    handle = (root / ("slot-v2-%s.lock" % index)).open("a")
                    try:
                        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
                        handles.append(handle)
                        admitted += 1
                        if admitted == wanted:
                            break
                    except BlockingIOError:
                        handle.close()
                if admitted != wanted:
                    raise BlockingIOError()
            except BlockingIOError:
                for handle in handles:
                    handle.close()
                handles.clear()
                if deadline is not None and time.monotonic() >= deadline:
                    raise RunBlocked("capacity_timeout", "Capacity wait exhausted the run timeout")
                time.sleep(0.1)
        yield handles
    finally:
        for handle in handles:
            handle.close()


def stats(run, name, server_env):
    # Statistics do not establish server startup or effective scratch health.
    data = json.loads(command(["sccache", "--show-stats", "--stats-format=json"], env=server_env))
    save(run / (name + ".json"), data)
    return data


def admit_cache_server(run, server_env, checkout=None, *, handles):
    """Connect/start through a compile request and exercise effective server scratch.

    --show-stats can succeed with zero statistics when no server exists. A unique
    Rust crate forces a cache miss, so a warm cache cannot conceal obsolete scratch.
    The caller holds its normal compiler-capacity lease throughout admission.
    """
    record = {"state": "checking"}
    save(run / "cache-admission.json", record)
    try:
        # A compilation connects to the existing server or starts one with this
        # environment. Never use --start-server blindly: Unix socket startup can
        # unlink an existing endpoint. File output avoids inherited pipe writers.
        with tempfile.TemporaryDirectory(prefix="probe-", dir=server_env["TMPDIR"]) as directory:
            directory = Path(directory)
            source = directory / "probe.rs"
            source.write_text("pub const ID: &str = " + json.dumps(uuid.uuid4().hex) + ";\n")
            artifact = directory / "libhiroute_cache_probe.rlib"
            with (run / "cache-probe.log").open("w") as output:
                probe = start_locked_command(
                    ["sccache", shutil.which("rustc") or "rustc", "--crate-name", "hiroute_cache_probe",
                     "--crate-type", "lib", "--emit=dep-info,link", str(source), "--out-dir", str(directory)],
                    checkout, server_env, output, handles)
                try:
                    record["probe_exit"] = probe.wait(timeout=30)
                finally:
                    if probe.poll() is None:
                        os.killpg(probe.pid, signal.SIGKILL)
                        probe.wait()
            if record["probe_exit"] or not artifact.is_file():
                raise RuntimeError("cache-miss compilation did not produce a library")
        record["state"] = "ready"
    except (OSError, subprocess.SubprocessError, RuntimeError) as error:
        record.update(state="blocked", error=str(error))
        raise RunBlocked(
            "cache_server_unhealthy",
            "sccache preflight failed; see cache-admission.json and cache-probe.log. "
            "The existing server was not stopped. After all compiler owners are idle, acquire the "
            "host's exclusive compiler-capacity locks, preserve server statistics, and explicitly "
            "stop/start sccache with the recorded stable scratch environment before retrying. "
            "Do not recreate deleted run directories or clear the disk cache.") from error
    finally:
        save(run / "cache-admission.json", record)
    return record


def cache_server_environment(root):
    """Keep server scratch independent of disposable per-run test directories."""
    directory = root.resolve(strict=True) / "sccache-tmp"
    directory.mkdir(mode=0o700, exist_ok=True)
    metadata = directory.lstat()
    if (directory.is_symlink() or not directory.is_dir()
            or metadata.st_uid != os.geteuid()
            or metadata.st_mode & 0o7777 != 0o700):
        raise RunBlocked("unsafe_cache_temporary_directory",
                         "Shared sccache temporary directory must be an owner-only real directory")
    return dict(os.environ, TMPDIR=str(directory), TMP=str(directory), TEMP=str(directory),
                SCCACHE_IDLE_TIMEOUT="0", SCCACHE_CACHE_SIZE="30G")


def check_target(checkout, target_directory):
    # A home directory may itself be a mount-point symlink; target/ may not be.
    expected = checkout.resolve() / "target"
    if Path(target_directory).resolve() != expected or (checkout / "target").is_symlink():
        raise RuntimeError("Cargo target directory is not this checkout's default target/")


def prepare_temp_directory():
    """Create a private, canonical and short directory for one validation run."""
    system_directory = Path(tempfile.gettempdir()).resolve(strict=True)
    # mkdtemp currently adds eight random ASCII characters; verify the actual result too.
    if len(os.fsencode(system_directory / "hr-xxxxxxxx")) > MAX_TMPDIR_BYTES:
        raise RuntimeError("Resolved system temporary path is too long for socket tests; "
                           "configure a shorter TMPDIR in the remote user's environment")
    directory = Path(tempfile.mkdtemp(prefix="hr-", dir=system_directory))
    try:
        os.chmod(directory, 0o700)
        metadata = directory.lstat()
        if (directory.is_symlink() or not directory.is_dir()
                or metadata.st_uid != os.geteuid()
                or metadata.st_mode & 0o7777 != 0o700
                or directory.resolve(strict=True) != directory
                or len(os.fsencode(directory)) > MAX_TMPDIR_BYTES):
            raise RuntimeError("Private temporary directory validation failed")
    except Exception as error:
        raise RuntimeError("Temporary directory retained for diagnosis at %s: %s" % (directory, error)) from error
    return {"path": str(directory), "system_directory": str(system_directory),
            "mode": "0700", "uid": metadata.st_uid,
            "path_bytes": len(os.fsencode(directory))}


def validate_run_id(run_id):
    if not re.fullmatch(r"\d{8}-\d{6}-[a-f0-9]{8}", run_id):
        raise ValueError("Invalid run ID")


def worker_alive(pid):
    if not isinstance(pid, int) or pid <= 0:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def worktree_paths(repo):
    output = command(["git", "--git-dir", str(repo), "worktree", "list", "--porcelain"])
    return {Path(line[9:]).resolve() for line in output.splitlines() if line.startswith("worktree ")}


def checkout_lease(root, run_id):
    """Reserve one retained checkout for a command or its owner-led cleanup."""
    validate_run_id(run_id)
    handle = (root / ("checkout-" + run_id + ".lock")).open("a")
    try:
        fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        handle.close()
        raise RunBlocked("checkout_busy", "Retained checkout is already in use")
    return handle


def recoverable_setup(result):
    return (result.get("status") == "blocked"
            and result.get("blocker") in RECOVERABLE_SETUP_BLOCKERS
            and result.get("process_exit") is None
            and result.get("command_started_at") is None
            and result.get("command_finished_at") is None
            and not result.get("timed_out"))


def checkout_recovery_modes(first, latest):
    """Resource admission and command generation share the same stopped-state policy.

    Liveness, leases and current source are checked separately at acquisition.
    A recoverable setup result remains ineligible as executed test evidence.
    """
    if latest.get("timed_out"):
        return ()
    setup = recoverable_setup(latest)
    modes = []
    if (first.get("retention") == "always" and latest.get("plan")
            and latest.get("executor_digest")
            and (latest.get("status") in ("completed", "failed") or setup)):
        modes.append("continue")
    if ((first.get("retention") == "always"
         or (first.get("retention") == "on-failure" and recoverable_setup(first)))
            and (latest.get("status") in ("completed", "failed") or setup)):
        modes.append("reuse")
    return tuple(modes)


def checkout_run(root, run_id):
    validate_run_id(run_id)
    path = root / "runs" / run_id / "result.json"
    if not path.is_file():
        raise RunBlocked("checkout_missing", "Retained checkout run is missing")
    result = json.loads(path.read_text())
    if result.get("id") != run_id:
        raise RunBlocked("checkout_changed", "Retained checkout run identity differs")
    return result


def checkout_owner(root, run_id):
    previous = checkout_run(root, run_id)
    owner = previous.get("checkout_owner", run_id)
    validate_run_id(owner)
    return owner


def retained_checkout(root, request, owner, mode):
    """Resolve mutable checkout state independently of its immutable owner result."""
    selected = request[mode + "_checkout"]
    first = checkout_run(root, owner)
    state = json.loads((root / ("checkout-" + owner + ".json")).read_text())
    previous = checkout_run(root, state["run_id"])
    checkout = root / "worktrees" / owner
    keys = ("origin", "ref", "plan", "executor_digest", "schedule_digest", "report_digest", "console_digest")
    if mode == "reuse":
        keys += ("executor_sha",)
    if (mode not in checkout_recovery_modes(first, previous)
            or (selected != previous["id"] and (mode == "continue" or selected != owner))
            or first.get("checkout_owner", owner) != owner
            or previous.get("checkout_owner", previous["id"]) != owner
            or worker_alive(previous.get("worker_pid"))
            or state.get("sha") != previous.get("sha")
            or (mode == "reuse" and previous.get("sha") != request["sha"])
            or any(previous.get(key) != request.get(key) for key in keys)
            or any(row.get("checkout") != str(checkout)
                   or row.get("cleanup", {}).get("checkout", {}).get("state") in ("removed", "absent")
                   for row in (first, previous))
            or not checkout.is_dir() or checkout.is_symlink()
            or checkout.resolve(strict=True) not in worktree_paths(root / "repository.git")
            or command(["git", "-C", str(checkout), "rev-parse", "HEAD"]) != previous["sha"]
            or command(["git", "-C", str(checkout), "status", "--porcelain", "--untracked-files=all"])):
        raise RunBlocked("continuation_changed" if mode == "continue" else "checkout_changed",
                         "Recovery requires the current stopped checkout, same plan/driver, and clean source")
    return checkout, previous


def verify_reused_checkout(root, request):
    owner = checkout_owner(root, request["reuse_checkout"])
    return retained_checkout(root, request, owner, "reuse")[0]


def continuation_owner(root, request):
    return checkout_owner(root, request["continue_checkout"])


def continue_checkout(root, request, owner):
    """Called with repository and checkout leases held; never rewrites old evidence."""
    checkout, previous = retained_checkout(root, request, owner, "continue")
    state_path = root / ("checkout-" + owner + ".json")
    command(["git", "-C", str(checkout), "merge-base", "--is-ancestor", previous["sha"], request["sha"]])
    # Record intent first: interrupted transitions cannot be mistaken for the previous run.
    save(state_path, dict(run_id=request["id"], sha=request["sha"], previous_run=previous["id"]))
    command(["git", "-C", str(checkout), "checkout", "--detach", request["sha"]])
    return checkout


def checkout_recovery_command(root, result, frontend_dist=None):
    """Use the existing SSH entry so recovery retains the selected host/workbench."""
    owner = result["checkout_owner"]
    first = result if owner == result["id"] else checkout_run(root, owner)
    modes = checkout_recovery_modes(first, result)
    rerun = ["python3", "scripts/remote-rust.py", "--host", result["ssh_host"],
             "--workbench", result.get("workbench", ROOT), "run", "--ref", result["ref"],
             "--sha", result["sha"], "--retention", "always", "--timeout", str(result["timeout"])]
    if frontend_dist:
        rerun += ["--frontend-dist", str(frontend_dist)]
    if modes:
        mode = modes[0]
        rerun += ["--" + mode + "-checkout", result["id"] if mode == "continue" else owner]
    if result.get("schedule"):
        rerun += ["--schedule", result["schedule"]]
    if result.get("exclusive"):
        rerun += ["--exclusive"]
    if result.get("plan"):
        rerun += ["--plan", result["plan"]]
    rerun += ["--phase", result.get("phase", "focused")]
    for related in result.get("related_runs", []):
        rerun += ["--related-run", related]
    return shlex.join(rerun + ["--", *result["command"]])


def cleanup_temp_directory(info):
    """Remove only a runner-created private TMPDIR; return diagnostics instead of raising."""
    if not isinstance(info, dict) or not info.get("path"):
        return {"state": "absent"}
    directory = Path(info["path"])
    try:
        if not os.path.lexists(str(directory)):
            return {"state": "absent", "path": str(directory)}
        system_directory = Path(info["system_directory"]).resolve(strict=True)
        metadata = directory.lstat()
        if (directory.is_symlink() or not directory.is_dir()
                or directory.parent.resolve(strict=True) != system_directory
                or not directory.name.startswith("hr-")
                or metadata.st_uid != os.geteuid()
                or metadata.st_mode & 0o7777 != 0o700):
            raise RuntimeError("temporary directory no longer matches the runner safety contract")
        shutil.rmtree(directory)
        return {"state": "removed", "path": str(directory)}
    except Exception as error:
        return {"state": "retained", "path": str(directory), "error": str(error)}


def cleanup_checkout(root, result):
    """Remove one terminal, generated checkout without ever accepting an arbitrary path."""
    checkout_text = result.get("checkout")
    if not checkout_text:
        return {"state": "absent"}
    checkout = Path(checkout_text)
    if result.get("checkout_owner") and result["checkout_owner"] != result.get("id"):
        return {"state": "retained", "path": str(checkout), "reason": "owned by " + result["checkout_owner"]}
    try:
        lease = checkout_lease(root, result["id"])
    except RunBlocked as error:
        return {"state": "retained", "path": str(checkout), "error": str(error)}
    try:
        repo = root / "repository.git"
        if not checkout.exists():
            registered = worktree_paths(repo)
            if checkout.resolve() in registered:
                raise RuntimeError("checkout is missing on disk but remains registered by the bare repository")
            return {"state": "absent", "path": str(checkout)}
        worktree_root = (root / "worktrees").resolve(strict=True)
        resolved = checkout.resolve(strict=True)
        if resolved.parent != worktree_root or resolved.name != result.get("id"):
            raise RuntimeError("checkout is not this run's managed direct child")
        if resolved not in worktree_paths(repo):
            raise RuntimeError("checkout is not registered by the managed bare repository")
        status = subprocess.run(["git", "-C", str(resolved), "status", "--porcelain"],
                                text=True, capture_output=True, check=True).stdout.splitlines()
        command_args = ["git", "--git-dir", str(repo), "worktree", "remove"]
        # A test may leave __pycache__ or other artifacts in this generated checkout.
        # It is still safe to force only after all ownership and liveness checks above.
        if status:
            command_args.append("--force")
        command_args.append(str(resolved))
        subprocess.run(command_args, text=True, capture_output=True, check=True)
        if resolved.exists() or resolved in worktree_paths(repo):
            raise RuntimeError("worktree remains after Git reported successful removal")
        return {"state": "removed", "path": str(resolved),
                "method": "git-worktree-remove-force" if status else "git-worktree-remove",
                "untracked_or_modified": status}
    except Exception as error:
        return {"state": "retained", "path": str(checkout), "error": str(error)}
    finally:
        lease.close()


def cleanup_validation_ref(root, run_id, checkout_state):
    if checkout_state not in ("removed", "absent"):
        return {"state": "retained", "reason": "checkout was not safely removed"}
    repo = root / "repository.git"
    ref = "refs/validation/" + run_id
    try:
        present = subprocess.run(["git", "--git-dir", str(repo), "show-ref", "--verify", "--quiet", ref],
                                 check=False).returncode == 0
        if not present:
            return {"state": "absent", "ref": ref}
        subprocess.run(["git", "--git-dir", str(repo), "update-ref", "-d", ref], check=True)
        return {"state": "removed", "ref": ref}
    except Exception as error:
        return {"state": "retained", "ref": ref, "error": str(error)}


def cleanup_terminal_run(root, run_id, current_worker=False):
    """Persist cleanup facts while retaining all run logs and result evidence."""
    validate_run_id(run_id)
    result_path = root / "runs" / run_id / "result.json"
    if not result_path.exists():
        raise RuntimeError("Unknown run ID")
    result = json.loads(result_path.read_text())
    if result.get("status") not in TERMINAL_STATUSES:
        raise RuntimeError("Only completed, failed, or blocked runs may be cleaned")
    if not current_worker and worker_alive(result.get("worker_pid")):
        raise RuntimeError("Run worker is still alive; refusing cleanup")
    # Worktree removal and ref deletion serialize with fetch/worktree-add operations.
    with lock(root / "repository.lock"):
        checkout = cleanup_checkout(root, result)
        validation_ref = cleanup_validation_ref(root, run_id,
            "removed" if result.get("checkout_owner") and result["checkout_owner"] != run_id else checkout["state"])
    temporary = cleanup_temp_directory(result.get("temporary_directory"))
    result["cleanup"] = {
        "policy": result.get("retention", "always"),
        "requested_at": time.time(),
        "checkout": checkout,
        "temporary_directory": temporary,
        "validation_ref": validation_ref,
        "run_logs_preserved": True,
    }
    save(result_path, result)
    return result["cleanup"]


def should_auto_cleanup(result):
    policy = result.get("retention", "always")
    if policy == "never":
        return True
    return policy == "on-failure" and result.get("status") == "completed"


def retain_cleanup_facts(result, reason):
    result["cleanup"] = {
        "policy": result.get("retention", "always"),
        "state": "retained",
        "reason": reason,
        "run_logs_preserved": True,
    }


def gc_terminal_runs(root, older_than_days, apply, include_retained):
    """Clean aged terminal runs only; never infer that a live or unknown run is safe."""
    if older_than_days < 0:
        raise ValueError("GC retention age must be non-negative")
    cutoff = time.time() - older_than_days * 24 * 60 * 60
    candidates, skipped, cleaned = [], [], []
    for result_path in sorted((root / "runs").glob("*/result.json")):
        try:
            result = json.loads(result_path.read_text())
            run_id = result["id"]
            validate_run_id(run_id)
        except Exception as error:
            skipped.append({"path": str(result_path), "reason": "invalid result: " + str(error)})
            continue
        if result.get("status") not in TERMINAL_STATUSES:
            skipped.append({"id": run_id, "reason": "run is not terminal"})
            continue
        if result.get("finished_at", float("inf")) > cutoff:
            skipped.append({"id": run_id, "reason": "younger than retention age"})
            continue
        if result.get("retention", "always") == "always" and not include_retained:
            skipped.append({"id": run_id, "reason": "explicitly retained"})
            continue
        if worker_alive(result.get("worker_pid")):
            skipped.append({"id": run_id, "reason": "recorded worker PID is alive"})
            continue
        candidate = {"id": run_id, "status": result["status"],
                     "retention": result.get("retention", "always")}
        candidates.append(candidate)
        if apply:
            try:
                candidate["cleanup"] = cleanup_terminal_run(root, run_id)
                cleaned.append(candidate)
            except Exception as error:
                candidate["error"] = str(error)
    return {"apply": apply, "older_than_days": older_than_days,
            "candidates": candidates, "cleaned": cleaned, "skipped": skipped,
            "run_logs_preserved": True}


def test_summary(log, cargo):
    parsed = reporting().parse_log(log)
    modules = parsed["modules"]
    # Only complete, uniquely attributed Cargo targets are independent results.
    # A child summary may be the last observed output of an interrupted parent.
    counts = {key: sum(module["counts"][key] for module in modules if module["complete"])
              for key in ("passed", "failed", "ignored")}
    if cargo[1] != "test" or "--no-run" in cargo:
        state = "not_assessed"
    elif counts["failed"]:
        state = "tests_failed"
    elif not modules or parsed["unassigned_summaries"] or not all(module["complete"] for module in modules):
        state = "unrecognized_test_output"
    elif not counts["passed"]:
        state = "zero_tests_passed"
    else:
        state = "rust_tests_passed"
    return {"counts": counts, "state": state,
            "scenario_state": "not_assessed",
            "note": "Counts use complete Cargo targets and exclude nested child summaries. "
                    "Rust counts do not establish Product E2E green/expected_red/red; inspect scenario artifacts."}


def scheduled_test_summary(log, cargo, schedule_result=None):
    summary = test_summary(log, cargo)
    if schedule_result is not None and "counts" in schedule_result:
        # The scheduler counts terminal Cargo targets. Raw command output also
        # contains child test summaries printed by process tests.
        counts = schedule_result["counts"]
        summary["counts"] = counts
        summary["state"] = ("rust_tests_passed" if schedule_result.get("complete")
                            else "tests_failed" if counts["failed"]
                            else "not_assessed")
        summary["note"] += " Scheduled counts exclude nested child test summaries."
    return summary


def worker(request_path):
    run = request_path.parent
    root = run.parent.parent
    request = json.loads(request_path.read_text())
    result = dict(request, status="waiting", process_exit=None, worker_pid=os.getpid(),
                  submitted_at=request.get("submitted_at", time.time()), queued_at=time.time())
    result_path = run / "result.json"
    save(result_path, result)
    related = []
    reused_lease = None
    queued_clock = time.monotonic()
    execution_clock = None
    os.environ["PATH"] = str(Path.home() / ".cargo/bin") + ":" + os.environ["PATH"]
    try:
        validate(request["ref"], request["sha"], request["command"])
        related = reporting().related_reports(root / "runs", request)
        reporting().preflight(result, related)
        save(result_path, result)
        result.update(platform=sys.platform, architecture=platform.machine(), capacity_policy="8x8-v2")
        if request.get("retention") not in RETENTION_POLICIES:
            raise RuntimeError("Unknown checkout retention policy")
        if os.environ.get("CARGO_TARGET_DIR") or os.environ.get("CARGO_BUILD_TARGET_DIR"):
            raise RuntimeError("Remove target directory overrides from the remote environment")
        # Every configured repository shares the original account-wide locks.
        # Existing controllers still use these paths, including legacy slots.
        capacity_root = Path.home() / workbench_path(request["capacity_root"]) if request.get("capacity_root") else root
        capacity_root.mkdir(parents=True, exist_ok=True)
        with capacity(capacity_root, request["exclusive"]) as slots:
            result["capacity_acquired_at"] = time.time()
            result["queue_seconds"] = round(time.monotonic() - queued_clock, 3)
            execution_clock = time.monotonic()
            if shutil.disk_usage(root).free < MIN_FREE_BYTES:
                raise RunBlocked("insufficient_disk", "Less than 30 GiB free; validation did not start")
            repo = root / "repository.git"
            selected_checkout = request.get("continue_checkout") or request.get("reuse_checkout")
            owner_id = checkout_owner(root, selected_checkout) if selected_checkout else None
            reused_lease = checkout_lease(root, owner_id or request["id"])
            checkout = root / "worktrees" / (owner_id or request["id"])
            checkout.parent.mkdir(exist_ok=True)
            result["checkout_started_at"] = time.time()
            with lock(root / "repository.lock"):
                if not repo.exists():
                    command(["git", "init", "--bare", str(repo)])
                    command(["git", "--git-dir", str(repo), "remote", "add", "origin", request["origin"]])
                git = ["git", "--git-dir", str(repo)]
                if command(git + ["remote", "get-url", "origin"]) != request["origin"]:
                    raise RuntimeError("Remote workbench already belongs to a different origin")
                fetched = "refs/validation/" + request["id"]
                command(git + ["fetch", "--no-tags", "origin", "+" + request["ref"] + ":" + fetched])
                command(git + ["merge-base", "--is-ancestor", request["sha"], fetched])
                command(git + ["merge-base", "--is-ancestor", request["executor_sha"], fetched])
                result["fetched_ref_sha"] = command(git + ["rev-parse", fetched])
                if request.get("continue_checkout"):
                    continue_checkout(root, request, owner_id)
                elif owner_id:
                    verify_reused_checkout(root, request)
                else:
                    command(git + ["worktree", "add", "--detach", str(checkout), request["sha"]])
                save(root / ("checkout-" + (owner_id or request["id"]) + ".json"),
                     dict(run_id=request["id"], sha=request["sha"]))
            result.update(status="running", checkout=str(checkout), checkout_owner=owner_id or request["id"],
                          started_at=time.time(), checkout_finished_at=time.time())
            save(result_path, result)
            try:
                result["console_assets"] = console_assets().preflight(
                    checkout, request, sys.platform, request.get("frontend_dist"))
            except (ValueError, OSError, KeyError) as error:
                result["console_assets"] = dict(state="blocked", reason=str(error))
                staging = Path.home() / ".cache/hiroute/console-assets" / request["id"]
                host = request["ssh_host"]
                result["console_assets"]["preparation"] = [
                    "Use clean candidate " + request["sha"] + " on the configured frontend target:",
                    "python3 scripts/validation.py frontend ci",
                    "python3 scripts/validation.py frontend build --console-assets",
                    "From the machine with that dist, copy it to a separate backend staging directory:",
                    shlex.join(["ssh", host, shlex.join(["mkdir", "-p", str(staging)])]),
                    shlex.join(["scp", "-r", "apps/desktop/dist", host + ":" + str(staging) + "/"])]
                raise RunBlocked("console_assets", str(error) +
                                 "; prepare with validation.py frontend build --console-assets, then pass "
                                 "--frontend-dist to SSH backend run; see result.console_assets.preparation") from error
            save(result_path, result)
            server_env = cache_server_environment(root)
            result["cache_server_environment"] = {
                "temporary_directory": server_env["TMPDIR"], "idle_timeout": "0",
            }
            result["cache_admission"] = admit_cache_server(
                run, server_env, checkout, handles=[*slots, reused_lease])
            before = stats(run, "cache-before", server_env)
            if before.get("max_cache_size") != 30 * 1024**3:
                raise RuntimeError("sccache server must be configured with a 30 GiB cache; do not restart a busy shared server")
            result["temporary_directory"] = prepare_temp_directory()
            os.environ["TMPDIR"] = result["temporary_directory"]["path"]
            save(result_path, result)
            os.environ.update(CARGO_BUILD_JOBS=str(
                EXCLUSIVE_BUILD_JOBS if request["exclusive"] else SHARED_BUILD_JOBS
            ),
                              CARGO_INCREMENTAL="0", RUSTC_WRAPPER=shutil.which("sccache") or "sccache",
                              SCCACHE_CACHE_SIZE="30G")
            # Cargo metadata performs no Rust compilation and observes inherited Cargo config.
            metadata = json.loads(command(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], checkout))
            check_target(checkout, metadata["target_directory"])
            result["rustc"] = command(["rustc", "--version"], checkout)
            result["cargo"] = command(["cargo", "--version"], checkout)
            result["build_jobs"] = int(os.environ["CARGO_BUILD_JOBS"])
            save(result_path, result)
            with (run / "command.log").open("w") as output:
                output.write("SHA: %s\nCommand: %s\n" % (request["sha"], shlex.join(request["command"])))
                output.write("TMPDIR: %s\n" % os.environ["TMPDIR"])
                output.write("SCCACHE_SERVER_TMPDIR: %s\n" % server_env["TMPDIR"])
                output.flush()
                result["command_started_at"] = time.time()
                if request.get("schedule"):
                    scheduler = scheduling()
                    if request["schedule"] == "workspace-dag":
                        # Keep the admitted eight-slot reservation across the DAG.
                        # Child monitors inherit it, so a worker crash cannot free
                        # capacity while compiled tests are still running.
                        result["schedule_result"] = scheduler.execute_dag(
                            types.SimpleNamespace(**globals()), root, run, checkout,
                            request, reused_lease, output, slots)
                    else:
                        # The legacy schedule reserves individual phase slots.
                        for handle in slots:
                            handle.close()
                        slots.clear()
                        result["schedule_result"] = scheduler.execute(
                            types.SimpleNamespace(**globals()), root, run, checkout,
                            request, reused_lease, output)
                    result["process_exit"] = result["schedule_result"]["process_exit"]
                    result["timed_out"] = result["schedule_result"]["timed_out"]
                else:
                    process = start_locked_command(request["command"], checkout, None, output, [*slots, reused_lease])
                    try:
                        result["process_exit"] = process.wait(timeout=request["timeout"])
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        result["process_exit"] = process.wait()
                        result["timed_out"] = True
                result["command_finished_at"] = time.time()
            stats(run, "cache-after", server_env)
            result["tests"] = scheduled_test_summary(
                (run / "command.log").read_text(errors="replace"), request["command"],
                result.get("schedule_result") if request.get("schedule") else None)
            bad_tests = result["tests"]["state"] in ("tests_failed", "zero_tests_passed", "unrecognized_test_output")
            result["status"] = "failed" if result["process_exit"] != 0 or bad_tests else "completed"
    except RunBlocked as error:
        result.update(status="blocked", blocker=error.blocker, error=str(error))
        if recoverable_setup(result) and result.get("checkout_owner"):
            frontend = (Path.home() / ".cache/hiroute/console-assets" / request["id"] / "dist"
                        if error.blocker == "console_assets" else request.get("frontend_dist"))
            result["recovery"] = dict(command=checkout_recovery_command(root, result, frontend),
                                      after_worker_stops=True)
            if error.blocker == "console_assets":
                result["console_assets"]["preparation"] += [
                    "After this worker stops, rerun from the same committed driver; "
                    "--frontend-dist is a path on the backend host:", result["recovery"]["command"]]
    except Exception as error:
        result.update(status="failed", error=str(error))
    finally:
        result["finished_at"] = time.time()
        if execution_clock is not None:
            result["execution_seconds"] = round(time.monotonic() - execution_clock, 3)
            result["performance_goal_met"] = result["execution_seconds"] <= 300
        result["end_to_end_seconds"] = round(time.monotonic() - queued_clock, 3)
        try:
            reporting().write_report(run, result, related)
        except Exception as error:
            result["validation_report"] = dict(complete=False, error=str(error))
        if execution_clock is not None:
            # The authoritative duration includes publishing the final report.
            result["execution_seconds"] = round(time.monotonic() - execution_clock, 3)
            result["performance_goal_met"] = result["execution_seconds"] <= 300
            result["end_to_end_seconds"] = round(time.monotonic() - queued_clock, 3)
            if isinstance(result.get("schedule_result"), dict):
                result["schedule_result"]["performance_seconds"] = result["execution_seconds"]
                result["schedule_result"]["performance_goal_met"] = result["performance_goal_met"]
            try:
                reporting().write_report(run, result, related)
            except Exception as error:
                result["validation_report"] = dict(complete=False, error=str(error))
        require_dag_report(result)
        save(result_path, result)
        if reused_lease is not None:
            reused_lease.close()
        if should_auto_cleanup(result):
            cleanup_terminal_run(root, request["id"], current_worker=True)
        else:
            retain_cleanup_facts(result, "retention policy preserves this terminal run")
            save(result_path, result)


def require_dag_report(result):
    """A green full DAG requires its independently parsed final report."""
    if result.get("schedule") != "workspace-dag" or result.get("status") != "completed":
        return
    report = result.get("validation_report")
    if not isinstance(report, dict) or report.get("complete") is not True:
        result["status"] = "failed"
        detail = report.get("error") if isinstance(report, dict) else None
        result["error"] = "Required validation report is incomplete" + (
            ": " + str(detail) if detail else "")


# Bootstrap is deliberately independent of the candidate: it runs the committed driver
# selected by the caller, records its SHA, and never copies their uncommitted source.
BOOTSTRAP = '''import hashlib,json,os,pathlib,re,subprocess,sys,uuid
os.umask(0o077)
p=json.load(sys.stdin)
relative=p.get("workbench",".local/share/hiroute-rust")
p["workbench"]=relative
parts=relative.split("/")
if not re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*",relative) or any(part in (".","..") for part in parts): raise RuntimeError("invalid workbench path")
r=pathlib.Path.home()/relative
r.mkdir(parents=True,exist_ok=True)
c=r/"controllers";c.mkdir(exist_ok=True)
source=p.pop("source")
if hashlib.sha256(source.encode()).hexdigest()!=p["executor_digest"]: raise RuntimeError("driver digest mismatch")
driver=c/(p["executor_sha"]+".py")
temporary=c/((p.get("id") or "maintenance-"+uuid.uuid4().hex)+".tmp")
temporary.write_text(source)
try:
 try: os.link(str(temporary),str(driver))
 except FileExistsError: pass
 if driver.read_text()!=source: raise RuntimeError("immutable driver mismatch")
finally: temporary.unlink()
for kind in ("report","schedule","console"):
 helper=p.pop(kind+"_source",None)
 if helper is not None:
  if hashlib.sha256(helper.encode()).hexdigest()!=p[kind+"_digest"]: raise RuntimeError(kind+" helper digest mismatch")
  destination=c/(p["executor_sha"]+"-"+kind+".py")
  temporary=c/(uuid.uuid4().hex+".tmp")
  temporary.write_text(helper)
  try:
   try: os.link(str(temporary),str(destination))
   except FileExistsError: pass
   if destination.read_text()!=helper: raise RuntimeError("immutable "+kind+" helper mismatch")
  finally: temporary.unlink()
mode=p.pop("mode","worker")
if mode=="maintenance":
 subprocess.run([sys.executable,str(driver)]+p["driver_args"],check=True)
elif mode=="worker":
 run=r/"runs"/p["id"];run.mkdir(parents=True)
 (run/"request.json").write_text(json.dumps(p))
 (run/"result.json").write_text(json.dumps(dict(p,status="queued",process_exit=None)))
 with (run/"worker.log").open("w") as log:
  subprocess.Popen([sys.executable,str(driver),"_worker",str(run/"request.json")],stdin=subprocess.DEVNULL,stdout=log,stderr=log,start_new_session=True)
 print(json.dumps({"id":p["id"],"sha":p["sha"],"remote_path":str(run)}))
else: raise RuntimeError("unknown bootstrap mode")
'''


# Poll on the execution host; only one compact observation returns to the caller.
# The run's result.json is atomically replaced by save(), so each read is complete.
WAIT_BOOTSTRAP = '''import json,pathlib,sys,time
run_id,seconds,root=sys.argv[1],float(sys.argv[2]),pathlib.Path(sys.argv[3]).expanduser()
path=root/"runs"/run_id/"result.json"
deadline=time.monotonic()+seconds
while True:
 try:
  record=json.loads(path.read_text())
 except FileNotFoundError:
  print(json.dumps({"run_id":run_id,"wait_state":"missing"}),flush=True)
  sys.exit(2)
 except (OSError,ValueError):
  print(json.dumps({"run_id":run_id,"wait_state":"invalid_result"}),flush=True)
  sys.exit(2)
 if record.get("id")!=run_id:
  print(json.dumps({"run_id":run_id,"wait_state":"invalid_result"}),flush=True)
  sys.exit(2)
 status=record.get("status")
 terminal=status in ("completed","failed","blocked")
 remaining=deadline-time.monotonic()
 if terminal or remaining<=0:
  result={"run_id":run_id,"wait_state":"terminal" if terminal else "timeout",
          "status":status,"sha":record.get("sha")}
  if terminal:
   for key in ("process_exit","execution_seconds","performance_goal_met","queue_seconds","blocker"):
    if key=="performance_goal_met" and record.get("schedule")!="workspace-dag": continue
    if key in record: result[key]=record[key]
   if record.get("error"): result["error"]=str(record["error"])[:300]
   tests=record.get("tests") or {}
   if tests.get("state") not in (None,"not_assessed"):
    result["tests"]={key:tests[key] for key in ("state","counts") if key in tests}
   schedule=record.get("schedule_result") or {}
   if schedule:
    result["schedule"]={key:schedule[key] for key in
      ("complete","targets","executed_targets","doctest_modules") if key in schedule}
   report=record.get("validation_report")
   if report or record.get("schedule")=="workspace-dag":
    result["validation_report"]={"complete":report.get("complete") is True if isinstance(report,dict) else False}
    if isinstance(report,dict) and report.get("error"):
     result["validation_report"]["error"]=str(report["error"])[:300]
  print(json.dumps(result,separators=(",",":")),flush=True)
  break
 time.sleep(min(1.0,remaining))
'''


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "_worker":
        worker(Path(sys.argv[2]))
        return
    if len(sys.argv) in (3, 4) and sys.argv[1] == "_cleanup":
        root = Path.home() / workbench_path(sys.argv[3] if len(sys.argv) == 4 else ROOT)
        print(json.dumps(cleanup_terminal_run(root, sys.argv[2]), indent=2, ensure_ascii=False))
        return
    if len(sys.argv) in (5, 6) and sys.argv[1] == "_gc":
        root = Path.home() / workbench_path(sys.argv[5] if len(sys.argv) == 6 else ROOT)
        print(json.dumps(gc_terminal_runs(root, int(sys.argv[2]),
                                          sys.argv[3] == "apply", sys.argv[4] == "include-retained"),
                         indent=2, ensure_ascii=False))
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", help="Override host from ~/.config/hiroute/remote-rust.json")
    parser.add_argument("--workbench", help="Remote HOME-relative root; overrides remote-rust.json")
    sub = parser.add_subparsers(dest="action", required=True)
    run = sub.add_parser("run", help="Queue a focused Cargo command against a pushed commit")
    run.add_argument("--ref", required=True, help="Advertised branch, e.g. refs/heads/codex/my-proposal")
    run.add_argument("--sha", help="Default: current HEAD")
    run.add_argument("--frontend-dist", help="Execution-host staging directory from validation.py frontend build --console-assets")
    run.add_argument("--exclusive", action="store_true", help="Reserve all 8 slots, 64 compiler jobs (bench/Gates)")
    run.add_argument("--timeout", type=int, default=3600)
    run.add_argument("--retention", choices=RETENTION_POLICIES, default="on-failure",
                     help="Checkout/TMPDIR retention: on-failure (default), always, or never")
    run.add_argument("--reuse-checkout", metavar="RUN_ID",
                     help="Reuse a retained owner's current exact checkout and target under an exclusive lease")
    run.add_argument("--continue-checkout", metavar="PREVIOUS_RUN_ID",
                     help="Continue the latest stopped or recoverable setup run to a descendant SHA on the same plan")
    run.add_argument("--schedule", choices=("workspace-smoke", "workspace-dag"),
                     help="Prepare backend builds, then run smoke and remaining tests in separate shared slots")
    reporting().add_arguments(run)
    run.add_argument("command", nargs=argparse.REMAINDER)
    for name in ("status", "logs"):
        item = sub.add_parser(name)
        item.add_argument("id")
    wait = sub.add_parser("wait", help="Wait up to 55 seconds and return one compact status")
    wait.add_argument("id")
    wait.add_argument("--timeout", type=int, default=50, metavar="SECONDS")
    cleanup = sub.add_parser("cleanup", help="Safely reclaim one completed validation run while retaining its logs")
    cleanup.add_argument("id")
    gc = sub.add_parser("gc", help="List or reclaim aged terminal validation worktrees; dry-run by default")
    gc.add_argument("--older-than", type=int, default=7, metavar="DAYS")
    gc.add_argument("--apply", action="store_true", help="Perform cleanup; omit for a read-only preview")
    gc.add_argument("--include-retained", action="store_true",
                    help="Also select runs submitted with --retention always")
    args = parser.parse_args()
    if not args.host or not args.workbench:
        config = json.loads(CONFIG.read_text()) if CONFIG.exists() else {}
    if not args.host:
        args.host = config.get("host")
    try:
        workbench = workbench_path(args.workbench or config.get("workbench", ROOT))
    except ValueError as error:
        parser.error(str(error))
    if not args.host or args.host.startswith("-") or any(c.isspace() for c in args.host):
        parser.error("Configure an SSH host in ~/.config/hiroute/remote-rust.json")
    if args.action == "run":
        if args.reuse_checkout:
            validate_run_id(args.reuse_checkout)
        if args.continue_checkout:
            validate_run_id(args.continue_checkout)
            if args.reuse_checkout:
                parser.error("Choose reuse-checkout or continue-checkout")
        sha = args.sha or command(["git", "rev-parse", "HEAD"])
        cargo = args.command[1:] if args.command[:1] == ["--"] else args.command
        validate(args.ref, sha, cargo)
        if cargo[1] == "bench" and not args.exclusive:
            parser.error("Bench requires --exclusive to avoid competing workbench builds")
        if args.schedule:
            scheduling().commands(cargo)
        executor_sha = command(["git", "rev-parse", "HEAD"])
        source = command(["git", "show", executor_sha + ":scripts/remote-rust.py"]) + "\n"
        payload = dict(id=time.strftime("%Y%m%d-%H%M%S-") + uuid.uuid4().hex[:8],
                       sha=sha, ref=args.ref, command=cargo, exclusive=args.exclusive or full_test(cargo),
                       timeout=args.timeout, origin=command(["git", "remote", "get-url", "origin"]),
                       workbench=workbench, capacity_root=ROOT,
                       retention=args.retention, executor_sha=executor_sha,
                       reuse_checkout=args.reuse_checkout, continue_checkout=args.continue_checkout,
                       schedule=args.schedule,
                       ssh_host=args.host, frontend_dist=args.frontend_dist,
                       executor_digest=hashlib.sha256(source.encode()).hexdigest(), source=source)
        payload.update(reporting().options(args), submitted_at=time.time())
        helper = command(["git", "show", executor_sha + ":scripts/validation-report.py"]) + "\n"
        payload.update(report_source=helper, report_digest=hashlib.sha256(helper.encode()).hexdigest())
        scheduler = command(["git", "show", executor_sha + ":scripts/validation-schedule.py"]) + "\n"
        payload.update(schedule_source=scheduler, schedule_digest=hashlib.sha256(scheduler.encode()).hexdigest())
        console = command(["git", "show", executor_sha + ":scripts/console-assets.py"]) + "\n"
        payload.update(console_source=console, console_digest=hashlib.sha256(console.encode()).hexdigest())
        # bashrc owns the user's proxy settings. No credentials or proxy URLs are serialized.
        remote = "source ~/.bashrc >/dev/null 2>&1; exec python3 -c " + shlex.quote(BOOTSTRAP)
        subprocess.run(["ssh", "-o", "BatchMode=yes", args.host, "bash -lc " + shlex.quote(remote)],
                       input=json.dumps(payload), text=True, check=True)
    elif args.action == "wait":
        try:
            validate_run_id(args.id)
        except ValueError as error:
            parser.error(str(error))
        if not 0 <= args.timeout <= 55:
            parser.error("wait --timeout must be between 0 and 55 seconds")
        remote = ("python3 -c " + shlex.quote(WAIT_BOOTSTRAP) + " "
                  + shlex.quote(args.id) + " " + str(args.timeout) + " "
                  + shlex.quote("~/" + workbench))
        completed = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
                                    args.host, remote])
        if completed.returncode:
            raise SystemExit(completed.returncode)
    elif args.action in ("status", "logs"):
        try:
            validate_run_id(args.id)
        except ValueError as error:
            parser.error(str(error))
        filename = "result.json" if args.action == "status" else "command.log"
        operation = "cat" if args.action == "status" else "tail -n 100"
        subprocess.run(["ssh", "-o", "BatchMode=yes", args.host,
                        operation + " ~/" + workbench + "/runs/" + args.id + "/" + filename], check=True)
    else:
        if args.action == "cleanup":
            try:
                validate_run_id(args.id)
            except ValueError as error:
                parser.error(str(error))
            driver_args = ["_cleanup", args.id, workbench]
        else:
            if args.older_than < 0:
                parser.error("--older-than must be non-negative")
            driver_args = ["_gc", str(args.older_than), "apply" if args.apply else "preview",
                           "include-retained" if args.include_retained else "default", workbench]
        executor_sha = command(["git", "rev-parse", "HEAD"])
        source = command(["git", "show", executor_sha + ":scripts/remote-rust.py"]) + "\n"
        payload = dict(mode="maintenance", workbench=workbench, driver_args=driver_args, executor_sha=executor_sha,
                       executor_digest=hashlib.sha256(source.encode()).hexdigest(), source=source)
        remote = "source ~/.bashrc >/dev/null 2>&1; exec python3 -c " + shlex.quote(BOOTSTRAP)
        subprocess.run(["ssh", "-o", "BatchMode=yes", args.host, "bash -lc " + shlex.quote(remote)],
                       input=json.dumps(payload), text=True, check=True)


if __name__ == "__main__":
    main()
