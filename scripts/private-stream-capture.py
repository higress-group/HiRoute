#!/usr/bin/env python3
"""Run one isolated Unix Debug gateway/daemon with bounded private capture.

The executable must come from the supplied committed candidate's managed build.
This tool supplies provenance, NOT a build attestation. Never use a daily daemon.
No network requests or model calls are issued by this tool.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import time

GROUP_STOP_GRACE_SECONDS = 5


def require_supervision_primitives():
    if not all(hasattr(os, name) for name in ("waitid", "P_PID", "WEXITED", "WNOHANG", "WNOWAIT")):
        raise ValueError("capture requires non-reaping Unix waitid (macOS: Python 3.13 or newer)")


def candidate_exited(child):
    # Keep the exited leader waitable so its PID/session cannot be recycled
    # before stop has signalled every member of our process group.
    return os.waitid(os.P_PID, child.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None


def write_private(path, data):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)


def create_session(root, source_sha, binary, client, seconds):
    root.mkdir(mode=0o700)
    stamp = int(time.time())
    session = dict(source_sha=source_sha, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                   client=client, expires_at=stamp + seconds, delete_after=stamp + 86400)
    write_private(root / "session.json", json.dumps(session).encode())
    return session


def darwin_group_contains_only_leader(pid):
    if sys.platform != "darwin":
        return False
    import ctypes
    libc = ctypes.CDLL(None, use_errno=True)
    libc.sysctl.argtypes = [ctypes.POINTER(ctypes.c_int), ctypes.c_uint, ctypes.c_void_p,
                           ctypes.POINTER(ctypes.c_size_t), ctypes.c_void_p, ctypes.c_size_t]
    libc.sysctl.restype = ctypes.c_int

    def snapshot(selector):
        # Darwin sys/sysctl.h: CTL_KERN=1, KERN_PROC=14, PID=1, PGRP=2.
        # Compare the opaque records instead of depending on kinfo_proc layout.
        mib = (ctypes.c_int * 4)(1, 14, selector, pid)
        size = ctypes.c_size_t()
        if libc.sysctl(mib, 4, None, ctypes.byref(size), None, 0) != 0:
            return None
        if not 0 < size.value <= 16 * 1024 * 1024:
            return None
        buffer = ctypes.create_string_buffer(size.value)
        if libc.sysctl(mib, 4, buffer, ctypes.byref(size), None, 0) != 0:
            return None
        return buffer.raw[:size.value]

    leader = snapshot(1)
    return bool(leader) and snapshot(2) == leader


def signal_owned_group(child, sig, created_isolated_session):
    try:
        os.killpg(child.pid, sig)
    except ProcessLookupError:
        pass
    except PermissionError:
        # Darwin returns EPERM for a group containing only an unsignalable
        # zombie. Any additional member or unavailable kernel proof fails closed.
        if not (created_isolated_session and candidate_exited(child)
                and darwin_group_contains_only_leader(child.pid)):
            raise


def stop(child, *, created_isolated_session=False):
    if child.returncode is not None:
        raise ValueError("candidate leader was reaped before process-group cleanup")
    try:
        if os.getpgid(child.pid) != child.pid or os.getsid(child.pid) != child.pid:
            raise ValueError("candidate does not own its isolated session/process group")
    except ProcessLookupError:
        # macOS may hide a zombie's PGID/SID before waitpid reaps it. Popen's
        # successful start_new_session establishes this ID; WNOWAIT proves the
        # leader still anchors it against reuse. Never infer ownership from a
        # missing process alone.
        if not created_isolated_session or not candidate_exited(child):
            raise
    # The waitable leader anchors this group ID even when it has already
    # exited. A descendant may still own capture files or ignore SIGTERM.
    try:
        signal_owned_group(child, signal.SIGTERM, created_isolated_session)
        deadline = time.monotonic() + GROUP_STOP_GRACE_SECONDS
        while time.monotonic() < deadline:
            time.sleep(min(0.02, max(0, deadline - time.monotonic())))
    finally:
        # Graceful termination of the retention owner can interrupt the grace
        # wait. It must still stop descendants before attempting raw cleanup.
        try:
            signal_owned_group(child, signal.SIGKILL, created_isolated_session)
        finally:
            child.wait(timeout=5)


def supervise(args):
    require_supervision_primitives()
    if not 1 <= args.seconds <= 3600:
        raise ValueError("duration must be 1..3600 seconds")
    if not 1 <= args.attempts <= 3:
        raise ValueError("attempt budget must be 1..3")
    if not args.command:
        raise ValueError("supply the isolated candidate executable and its arguments")
    command = args.command[1:] if args.command[0] == "--" else args.command
    binary = Path(command[0]).resolve(strict=True)
    source = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    if source != args.source_sha or subprocess.check_output(["git", "status", "--porcelain"]):
        raise ValueError("run from the clean, exact committed source checkout")
    root = args.root.resolve()
    if root.exists():
        raise ValueError("capture root must be new")
    session = create_session(root, source, binary, args.client, args.seconds)
    retain_session(args, root, binary, command, source, session["delete_after"])


def retain_session(args, root, binary, command, source, delete_after):
    require_supervision_primitives()
    # This bounded owner survives the CLI returning. It owns the candidate child,
    # stops/reaps it, then removes the raw session at expiry (or on graceful stop).
    # No shared daemon, system scheduler, or daily process is modified.
    read_fd, write_fd = os.pipe()
    owner = os.fork()
    if owner == 0:
        os.close(read_fd)
        os.setsid()
        with open(os.devnull, "wb") as null:
            os.dup2(null.fileno(), 1)
            os.dup2(null.fileno(), 2)
        def terminate(_signum, _frame):
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            raise SystemExit(1)
        signal.signal(signal.SIGTERM, terminate)
        signal.signal(signal.SIGINT, terminate)
        reports = os.fdopen(write_fd, "w")
        def report(value):
            value["retention_owner_pid"] = os.getpid()
            print(json.dumps(value), file=reports, flush=True)
        exit_code = 0
        # A backwards wall-clock adjustment must not extend retention.
        deadline = time.monotonic() + max(0, min(86400, delete_after - time.time()))
        try:
            supervise_candidate(args, root, binary, command, source, report)
            reports.close()  # The CLI can return; this owner remains until deletion.
            while time.time() < delete_after and time.monotonic() < deadline:
                time.sleep(min(60, max(0, deadline - time.monotonic()),
                               max(0, delete_after - time.time())))
        except BaseException:
            exit_code = 1
        finally:
            # Reporting is best effort once the CLI reader has gone. A failed
            # flush can fail again on close, but must not skip raw deletion.
            # Graceful stop has already reached candidate group cleanup; do not
            # let a second signal interrupt this mandatory cleanup section.
            signal.signal(signal.SIGTERM, signal.SIG_IGN)
            signal.signal(signal.SIGINT, signal.SIG_IGN)
            try:
                reports.close()
            except BaseException:
                exit_code = 1
            finally:
                try:
                    # SIGKILL is asynchronous: after reaping the leader, a killed
                    # descendant may still be releasing its kernel flock. This
                    # bounded wait applies only after our group-stop path.
                    cleanup(root, require_expired=False, lock_wait_seconds=5)
                except BaseException:
                    exit_code = 1
                finally:
                    os._exit(exit_code)
    os.close(write_fd)
    try:
        with os.fdopen(read_fd) as reports:
            for line in reports:
                print(line.rstrip(), flush=True)
    except BaseException:
        os.kill(owner, signal.SIGTERM)
        os.waitpid(owner, 0)
        raise
    return owner


def supervise_candidate(args, root, binary, command, source, report):
    env = dict(os.environ, HIROUTE_PRIVATE_STREAM_CAPTURE=str(root))
    # Third-party stdout is not evidence; discard it instead of an unbounded log.
    child = None
    reason = "process_exit"
    try:
        with open(os.devnull, "wb") as log:
            child = subprocess.Popen([str(binary), *command[1:]], env=env, stdout=log,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            deadline = time.monotonic() + args.seconds
            report({"capture_root": str(root), "source_sha": source,
                    "pid": child.pid, "model_request_budget": args.attempts})
            while not candidate_exited(child):
                files = list(root.glob("*.capture"))
                if (root / "stopped").exists():
                    reason = "first_gateway_failure"
                    break
                if len(files) >= args.attempts and not capture_active(root):
                    reason = "attempt_budget"
                    break
                if time.monotonic() >= deadline:
                    reason = "time_budget"
                    break
                # Bound all files including private child logs, beyond writer's own budget.
                if sum(p.stat().st_size for p in root.iterdir() if p.is_file()) >= 32 * 1024 * 1024:
                    reason = "byte_budget"
                    break
                time.sleep(0.02)
    finally:
        if child is not None:
            stop(child, created_isolated_session=True)
        report({"stop_reason": reason, "process_exit": child.returncode if child else None,
                "scenario": "unassessed", "retention_seconds": 86400})


def capture_lock(root):
    fd = os.open(root / "active.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode):
        os.close(fd)
        raise ValueError("unsafe capture lock")
    return os.fdopen(fd, "rb+")


def capture_active(root):
    with capture_lock(root) as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            return True
    return False


def cleanup(root, require_expired=True, lock_wait_seconds=0):
    # Only this exact, expired session. Never walk a general evidence tree.
    if root.is_symlink() or root.resolve() != root or not root.is_dir():
        raise ValueError("unsafe root")
    with capture_lock(root) as lock:
        deadline = time.monotonic() + max(0, lock_wait_seconds)
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError as error:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise ValueError("capture still owned; stop its process first") from error
                time.sleep(min(0.02, remaining))
        session = json.loads((root / "session.json").read_text())
        if require_expired and int(time.time()) < session["delete_after"]:
            raise ValueError("session has not reached retention expiry")
        if any(p.is_symlink() or not p.is_file() for p in root.iterdir()):
            raise ValueError("unexpected session entry")
        shutil.rmtree(root)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    run = sub.add_parser("run")
    run.add_argument("--root", type=Path, required=True)
    run.add_argument("--source-sha", required=True)
    run.add_argument("--client", required=True)
    run.add_argument("--seconds", type=int, default=300)
    run.add_argument("--attempts", type=int, default=3)
    run.add_argument("command", nargs=argparse.REMAINDER)
    gc = sub.add_parser("cleanup")
    gc.add_argument("root", type=Path)
    args = parser.parse_args()
    supervise(args) if args.action == "run" else cleanup(args.root)


if __name__ == "__main__":
    main()
