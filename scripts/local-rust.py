#!/usr/bin/env python3
"""Opt-in local platform validation and explicit retired-debug reclamation (Unix)."""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
import platform
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid

spec = importlib.util.spec_from_file_location("remote_rust", Path(__file__).with_name("remote-rust.py"))
remote = importlib.util.module_from_spec(spec)
spec.loader.exec_module(remote)
DEFAULT_ROOT = Path.home() / ".cache/hiroute/local-rust"
MAC_LOCK = Path.home() / ".cache/hiroute/mac-rust-validation.lock"
HOST_LOCK = MAC_LOCK if sys.platform == "darwin" else Path.home() / ".cache/hiroute/local-rust-validation.lock"
TOOLCHAIN_BIN = Path.home() / ".cargo/bin"
GIB = 1024 ** 3


def private_temp():
    system = Path(tempfile.gettempdir()).resolve(strict=True)
    if len(os.fsencode(system / "hr-xxxxxxxx")) > remote.MAX_TMPDIR_BYTES:
        # macOS per-user /var/folders paths exceed socket budgets. /tmp is an OS
        # convention, resolved before use, never a machine-specific mounted path.
        system = Path("/tmp").resolve(strict=True)
    if len(os.fsencode(system / "hr-xxxxxxxx")) > remote.MAX_TMPDIR_BYTES:
        raise ValueError("No short system temporary directory; configure TMPDIR")
    path = Path(tempfile.mkdtemp(prefix="hr-", dir=system))
    os.chmod(path, 0o700)
    return dict(path=str(path), system_directory=str(system))


def git(repo, *args):
    return remote.command(["git", "-C", str(repo), *args])


def toolchain_environment():
    """PATH that locates the required tools, appending the standard rustup bin only
    when a tool is missing from PATH. A toolchain already on PATH is never overridden."""
    path = os.environ.get("PATH", "")
    required = ("cargo", "rustc", "sccache", "git")
    missing = [name for name in required if shutil.which(name, path=path) is None]
    if missing and TOOLCHAIN_BIN.is_dir():
        path = os.pathsep.join(part for part in (path, str(TOOLCHAIN_BIN)) if part)
        missing = [name for name in required if shutil.which(name, path=path) is None]
    if missing:
        raise ValueError("Missing on PATH: %s; load the Cargo environment first, "
                         'for example with . "$HOME/.cargo/env"' % ", ".join(missing))
    return path


def identity(path):
    info = path.lstat()
    if path.is_symlink() or not path.is_dir() or info.st_uid != os.geteuid():
        raise ValueError("Not an owned real directory: " + str(path))
    return [info.st_dev, info.st_ino]


def fingerprint(path):
    """Snapshot metadata without following links; no content or secrets in reports."""
    digest = hashlib.sha256()
    size = 0
    for parent, dirs, files in os.walk(path, followlinks=False):
        dirs.sort()
        for name in sorted(dirs + files):
            item = Path(parent) / name
            info = item.lstat()
            digest.update(repr((str(item.relative_to(path)), info.st_dev, info.st_ino,
                                info.st_mode, info.st_size, info.st_mtime_ns)).encode())
            size += info.st_blocks * 512
    return digest.hexdigest(), size


class Store:
    def __init__(self, root=DEFAULT_ROOT, global_lock=HOST_LOCK):
        self.root = Path(root).expanduser().resolve()
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        if identity(self.root) and self.root.stat().st_mode & 0o077:
            raise ValueError("State root must be private (0700)")
        for name in ("runs", "checkouts", "locks"):
            (self.root / name).mkdir(exist_ok=True, mode=0o700)
            identity(self.root / name)
        self.global_lock = Path(global_lock)
        self.global_lock.parent.mkdir(parents=True, exist_ok=True)

    def record_path(self, key):
        if not re.fullmatch(r"[a-f0-9]{32}", key):
            raise ValueError("Invalid record ID")
        return self.root / "runs" / key / "result.json"

    def load(self, key):
        return json.loads(self.record_path(key).read_text())

    def save(self, row):
        path = self.record_path(row["id"])
        path.parent.mkdir(exist_ok=True, mode=0o700)
        remote.save(path, row)

    @contextlib.contextmanager
    def locked(self, checkout, *, wait=False):
        # Only an explicitly queued run waits; cleanup always uses the default fast failure.
        key = hashlib.sha256(str(checkout).encode()).hexdigest()
        with contextlib.ExitStack() as stack:
            handles = []
            for path in (self.global_lock, self.root / "locks" / (key + ".lock")):
                handle = stack.enter_context(path.open("a"))
                remote.fcntl.flock(handle, remote.fcntl.LOCK_EX | (0 if wait else remote.fcntl.LOCK_NB))
                handles.append(handle)
            yield handles

    def verify(self, row):
        checkout = Path(row["checkout"])
        if checkout.resolve() != checkout or identity(checkout) != row["identity"]:
            raise ValueError("Checkout was replaced")
        if git(checkout, "rev-parse", "--show-toplevel") != str(checkout):
            raise ValueError("Not the recorded worktree root")
        if git(checkout, "rev-parse", "HEAD") != row["sha"]:
            raise ValueError("Checkout revision changed")
        if row["kind"] == "managed":
            if checkout.parent != self.root / "checkouts" or checkout.name != row["id"]:
                raise ValueError("Not a managed checkout")
            if git(checkout, "status", "--porcelain", "--untracked-files=all"):
                raise ValueError("Source changes/untracked files retained")
            ignored = git(checkout, "ls-files", "--others", "--ignored", "--exclude-standard")
            if any(not p.startswith("target/") for p in ignored.splitlines()):
                raise ValueError("Ignored files outside target retained")
        target = checkout / "target"
        # Lock admission can fail before Cargo creates a target. Only a managed,
        # never-started run without recorded target identities may use this path.
        # A later-created target (including a dangling link) still fails closed.
        if (row["kind"] == "managed" and row.get("process_exit") is None
                and not row.get("command_started_at")
                and "target_identity" not in row and "debug_identity" not in row
                and not os.path.lexists(target)):
            if git(checkout, "ls-files", "target"):
                raise ValueError("Target contains tracked files")
            return checkout, target / "debug"
        identity(target)
        if target.resolve() != target or identity(target) != row["target_identity"]:
            raise ValueError("Target was replaced")
        debug = target / "debug"
        if identity(debug) != row["debug_identity"]:
            raise ValueError("Debug directory was replaced")
        if git(checkout, "ls-files", "target"):
            raise ValueError("Target contains tracked files")
        return checkout, debug

    def preview_one(self, row):
        report = {"id": row["id"], "checkout": row["checkout"], "state": "skipped"}
        try:
            with self.locked(row["checkout"]):
                if row.get("removed") or row.get("keep") or row["status"] not in ("terminal", "retired"):
                    raise ValueError("Active, kept, removed, or unknown lifecycle")
                if row.get("pilot_reuse") and not row.get("pilot_cleanup_ready"):
                    raise ValueError("Pilot owners have not finished their registered sessions")
                if not row.get("evidence_saved"):
                    raise ValueError("Evidence not confirmed/exported")
                _, debug = self.verify(row)
                stamp, size = fingerprint(debug)
                token = hashlib.sha256((json.dumps(row, sort_keys=True) + stamp).encode()).hexdigest()
                report.update(state="candidate", token=token, bytes=size,
                              target=str(debug), sha=row["sha"], kind=row["kind"])
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            report["reason"] = str(error)
        return report

    def preview(self):
        return [self.preview_one(self.load(p.parent.name))
                for p in sorted((self.root / "runs").glob("*/result.json"))]

    def apply(self, key, token):
        row = self.load(key)
        # Hold locks throughout revalidation and deletion, including snapshot comparison.
        with self.locked(row["checkout"]):
            current_row = self.load(key)
            if current_row["checkout"] != row["checkout"]:
                raise ValueError("Record checkout changed")
            row = current_row
            if row.get("keep") or row.get("removed") or row["status"] not in ("terminal", "retired"):
                raise ValueError("Not an inactive cleanup candidate")
            if row.get("pilot_reuse") and not row.get("pilot_cleanup_ready"):
                raise ValueError("Pilot owners have not finished their registered sessions")
            if not row.get("evidence_saved"):
                raise ValueError("Evidence not saved")
            checkout, debug = self.verify(row)
            stamp, size = fingerprint(debug)
            current = hashlib.sha256((json.dumps(row, sort_keys=True) + stamp).encode()).hexdigest()
            if token != current:
                raise ValueError("Candidate changed; preview again")
            if row["kind"] == "managed":
                # Preserve all non-debug target artifacts before removing this disposable tree.
                if os.path.lexists(checkout / "target"):
                    self.export_target(row)
                remote.command(["git", "-C", row["repo"], "worktree", "remove", "--force", str(checkout)])
            else:
                shutil.rmtree(debug)  # Never remove a developer worktree or its release/evidence.
            row.update(removed=True, cleaned_at=time.time(), reclaimed_debug_bytes=size)
            if row.get("temporary_directory"):
                row["temporary_cleanup"] = remote.cleanup_temp_directory(row["temporary_directory"])
            self.save(row)
            return {"id": key, "state": "removed", "bytes": size}

    def export_target(self, row):
        target = Path(row["checkout"]) / "target"
        destination = self.record_path(row["id"]).parent / "artifacts"
        for entry in target.iterdir():
            if entry.name == "debug":
                continue
            # Preserve even unknown artifacts; links are refused, not followed outside the tree.
            items = [entry] + (list(entry.rglob("*")) if entry.is_dir() else [])
            if any(p.is_symlink() for p in items):
                raise ValueError("Artifact contains links; retained for manual inspection")
            destination.mkdir(exist_ok=True)
            dest = destination / entry.name
            if dest.exists():
                if dest.is_dir():
                    shutil.rmtree(dest)
                else:
                    dest.unlink()
            if entry.is_dir():
                shutil.copytree(entry, dest)
            else:
                shutil.copy2(entry, dest)

    def retire(self, checkout):
        checkout = Path(checkout).resolve(strict=True)
        with self.locked(checkout):
            row = dict(id=uuid.uuid4().hex, kind="developer", checkout=str(checkout),
                       identity=identity(checkout), sha=git(checkout, "rev-parse", "HEAD"),
                       status="retired", evidence_saved=True, keep=False, finished_at=time.time(),
                       target_identity=identity(checkout / "target"),
                       debug_identity=identity(checkout / "target/debug"))
            self.verify(row)
            self.save(row)
            return row

    def gc(self, apply=False, older_days=7, target_free=None):
        results = []
        for candidate in self.preview():
            row = self.load(candidate["id"])
            if row["kind"] != "managed" or row.get("finished_at", time.time()) > time.time() - older_days * 86400:
                continue  # Developer retirement is always manual, never pressure GC.
            if candidate["state"] != "candidate":
                results.append(candidate)
                continue
            if apply:
                if target_free and shutil.disk_usage(self.root).free >= target_free:
                    break
                try:
                    candidate = self.apply(row["id"], candidate["token"])
                except (OSError, ValueError, subprocess.SubprocessError) as error:
                    candidate = dict(id=row["id"], state="retained", reason=str(error))
            results.append(candidate)
        return results

    def ensure_space(self):
        if shutil.disk_usage(self.root).free < 30 * GIB:
            self.gc(apply=True, target_free=60 * GIB)
        if shutil.disk_usage(self.root).free < 30 * GIB:
            raise ValueError("Less than 30 GiB free; build not started")


def run(store, args, prepare=None, on_record=None):
    repo = Path(args.repo).resolve(strict=True)
    # A non-interactive shell often omits the Cargo environment; resolve it before any
    # record, worktree or tool call, refusing with a remedy when it cannot be resolved.
    os.environ["PATH"] = toolchain_environment()
    cargo = args.command[1:] if args.command[:1] == ["--"] else args.command
    reporter = remote.reporting()
    feedback = reporter.options(args)
    related = reporter.related_reports(store.root / "runs", feedback)
    remote.validate(args.ref, args.sha, cargo)
    source = getattr(args, "source", "origin")
    jobs = getattr(args, "jobs", 2)
    if source not in ("origin", "local") or not isinstance(jobs, int) or jobs < 1:
        raise ValueError("Expected source origin/local and positive build jobs")
    if args.timeout <= 0:
        raise ValueError("Timeout must be positive")
    if any(k in os.environ for k in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR")):
        raise ValueError("Remove target directory overrides")
    store.ensure_space()
    key = uuid.uuid4().hex
    checkout = store.root / "checkouts" / key
    row = dict(id=key, kind="managed", repo=str(repo), checkout=str(checkout), sha=args.sha,
               command=cargo, status="preparing", keep=args.keep, evidence_saved=False,
               process_exit=None, scenario="unassessed", candidate_source=source,
               build_jobs=jobs, wait_for_lock=getattr(args, "wait_for_lock", False),
               platform=sys.platform, architecture=platform.machine(),
               submitted_at=time.time(), **feedback)
    row["executor_sha256"] = {
        name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
        for name in ("local-rust.py", "remote-rust.py", "validation-report.py")
    }
    reporter.preflight(row, related)
    store.save(row)
    if on_record is not None:
        on_record(row)
    # Local execution can validate a committed candidate without publishing it.
    # Fetch into a private ref in either case; never compile the caller's dirty tree.
    ref = "refs/hiroute-local-validation/" + key
    try:
        row["checkout_started_at"] = time.time()
        git(repo, "fetch", "--no-tags", "origin" if source == "origin" else str(repo), args.ref + ":" + ref)
        git(repo, "merge-base", "--is-ancestor", args.sha, ref)
        git(repo, "worktree", "add", "--detach", str(checkout), args.sha)
    except subprocess.SubprocessError as error:
        row.update(status="terminal", scenario="red", error=str(error), finished_at=time.time())
        reporter.write_report(store.record_path(key).parent, row, related)
        store.save(row)
        return row
    finally:
        git(repo, "update-ref", "-d", ref)
    row["identity"] = identity(checkout)
    row["checkout_finished_at"] = time.time()
    row["status"] = "running"
    store.save(row)
    # Start/cache-query before locking. The command monitor keeps lock FDs away
    # from Cargo and any sccache server it starts later.
    env = dict(os.environ, CARGO_BUILD_JOBS=str(jobs), CARGO_INCREMENTAL="0", RUSTC_WRAPPER="sccache")
    process = None
    try:
        server_env = remote.cache_server_environment(store.root)
        row["cache_server_environment"] = {
            "temporary_directory": server_env["TMPDIR"],
            "idle_timeout": "0",
        }
        before = remote.stats(store.record_path(key).parent, "cache-before", server_env)
        row["queued_at"] = time.time()
        store.save(row)
        with store.locked(checkout, wait=row["wait_for_lock"]) as handles:
            row["capacity_acquired_at"] = time.time()
            if row["wait_for_lock"] and shutil.disk_usage(store.root).free < 30 * GIB:
                raise ValueError("Less than 30 GiB free after waiting; build not started")
            info = private_temp()
            row["temporary_directory"] = info
            env["TMPDIR"] = info["path"]
            store.save(row)
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                cwd=checkout, env=env, text=True))
            remote.check_target(checkout, metadata["target_directory"])
            for tool in ("rustc", "cargo"):
                row[tool] = subprocess.check_output([tool, "--version"], cwd=checkout, env=env, text=True).strip()
            if prepare is not None:
                prepare(checkout, env, row, tuple(h.fileno() for h in handles))
                store.save(row)
            with (store.record_path(key).parent / "command.log").open("w") as output:
                row["command_started_at"] = time.time()
                process = remote.start_locked_command(cargo, checkout, env, output, handles)
                try:
                    row["process_exit"] = process.wait(timeout=args.timeout)
                finally:
                    if process.poll() is None:
                        os.killpg(process.pid, signal.SIGKILL)
                        row["process_exit"] = process.wait()
                    row["command_finished_at"] = time.time()
            row["tests"] = remote.test_summary((store.record_path(key).parent / "command.log").read_text(errors="replace"), cargo)
            bad = row["tests"]["state"] in ("tests_failed", "zero_tests_passed", "unrecognized_test_output")
            row["scenario"] = "red" if row["process_exit"] != 0 or bad else ("green" if args.cargo_only else "unassessed")
            row["evidence_saved"] = bool(args.cargo_only)
            row["cache_before"] = before
            row["cache_after"] = remote.stats(
                store.record_path(key).parent,
                "cache-after",
                server_env,
            )
    except (Exception, KeyboardInterrupt) as error:
        row["error"] = str(error)
        row["scenario"] = "red"
    finally:
        row.update(status="terminal", finished_at=time.time())
        reporter.write_report(store.record_path(key).parent, row, related)
        for name, path in (("target_identity", checkout / "target"), ("debug_identity", checkout / "target/debug")):
            if path.is_dir() and not path.is_symlink():
                row[name] = identity(path)
        store.save(row)
    if row["scenario"] == "green" and not row["keep"]:
        candidate = store.preview_one(row)
        if candidate["state"] == "candidate":
            store.apply(key, candidate["token"])
    return store.load(key)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    p = sub.add_parser("run", help="Exact committed candidate in an isolated worktree")
    p.add_argument("--repo", default=".")
    p.add_argument("--ref", required=True)
    p.add_argument("--sha", required=True)
    p.add_argument("--source", choices=("origin", "local"), default="origin")
    p.add_argument("--jobs", type=int, default=2)
    p.add_argument("--wait-for-lock", action="store_true",
                   help="Wait on the existing exclusive host/checkout locks before starting Cargo")
    p.add_argument("--keep", action="store_true")
    p.add_argument("--cargo-only", action="store_true", help="Assert pure Cargo check: no external scenario verdict or unique debug artifacts")
    p.add_argument("--timeout", type=int, default=3600)
    remote.reporting().add_arguments(p)
    p.add_argument("command", nargs=argparse.REMAINDER)
    sub.add_parser("preview", help="Read-only registered candidate list with snapshot tokens")
    sub.add_parser("status", help="Read saved result and validation feedback").add_argument("id")
    p = sub.add_parser("retire", help="Explicitly retire developer debug; never auto-detect inactivity")
    p.add_argument("--checkout", required=True)
    p.add_argument("--evidence-saved", action="store_true", required=True,
                   help="Confirm owner stopped use and unique debug evidence is saved elsewhere")
    p = sub.add_parser("apply", help="Recheck and delete one unchanged preview candidate")
    p.add_argument("id")
    p.add_argument("--token", required=True)
    p = sub.add_parser("gc", help="Aged managed validation only; never developer worktrees")
    p.add_argument("--apply", action="store_true")
    p.add_argument("--older-than", type=int, default=7)
    for action in ("keep", "release"):
        sub.add_parser(action).add_argument("id")
    p = sub.add_parser("finish", help="Record actual scenario verdict after exporting unique debug evidence")
    p.add_argument("id")
    p.add_argument("--verdict", choices=("green", "expected_red", "red"), required=True)
    p.add_argument("--evidence-saved", action="store_true", required=True)
    args = parser.parse_args()
    store = Store()
    if args.action == "run":
        result = run(store, args)
    elif args.action == "status":
        result = store.load(args.id)
    elif args.action == "preview":
        result = store.preview()
    elif args.action == "retire":
        result = store.retire(args.checkout)
    elif args.action == "apply":
        result = store.apply(args.id, args.token)
    elif args.action == "gc":
        if args.older_than < 0:
            parser.error("Age cannot be negative")
        result = store.gc(args.apply, args.older_than)
    else:
        row = store.load(args.id)
        with store.locked(row["checkout"]):
            row = store.load(args.id)
            if row["status"] not in ("terminal", "retired"):
                raise ValueError("Active or unknown run")
            if args.action == "finish":
                if args.verdict == "green" and row.get("process_exit") != 0:
                    raise ValueError("Nonzero/no process exit cannot become green")
                if args.verdict == "green" and row.get("tests", {}).get("state") in (
                    "tests_failed", "zero_tests_passed", "unrecognized_test_output"
                ):
                    raise ValueError("Failed/zero/unrecognized Cargo tests cannot become green")
                row.update(scenario=args.verdict, evidence_saved=True)
            else:
                row["keep"] = args.action == "keep"
            store.save(row)
        result = row
    print(json.dumps(result, indent=2))
    return 1 if args.action == "run" and result["scenario"] != "green" else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
