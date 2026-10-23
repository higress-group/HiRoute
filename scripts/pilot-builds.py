#!/usr/bin/env python3
"""Attested Pilot build reuse and explicit owner/session lifecycle (execution host)."""
import argparse
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import time
import types
import uuid

sys.dont_write_bytecode = True


def module(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


local = module("local-rust")
pilot = module("desktop-pilot")
COMMAND = ["cargo", "build", "--locked", "-p", "hiroute-desktop", "-p", "hiroute-daemon",
           "-p", "hiroute-cli", "--features", "hiroute-desktop/desktop-pilot",
           "--bin", "hiroute-desktop", "--bin", "hirouted", "--bin", "hiroute"]
DRIVERS = ("pilot-builds.py", "local-rust.py", "remote-rust.py", "desktop-pilot.py", "pilot-native-app.py", "validation-report.py")
VERDICTS = ("green", "red", "expected_red", "not_executed")
DEFAULT_CPA_MANIFEST = Path("apps/desktop/src-tauri/development-cpa-artifacts.v1.json")


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def tree_digest(root):
    """Content hash, rejecting links/special files instead of following them."""
    root = Path(root)
    local.identity(root)
    entries = []
    for path in sorted(root.rglob("*")):
        if path.is_symlink() or not (path.is_dir() or path.is_file()):
            raise ValueError("Frontend contains a link or special file")
        if path.is_file():
            entries.append([str(path.relative_to(root)), pilot.file_sha256(path)])
    if not entries or not (root / "index.html").is_file():
        raise ValueError("Frontend output is empty or has no index.html")
    return digest(entries)


def validate_cpa_manifest(contents):
    def reject_constant(value):
        raise ValueError("Non-finite JSON constant: " + value)

    try:
        value = json.loads(contents.decode("utf-8"), parse_constant=reject_constant)
    except (UnicodeDecodeError, ValueError):
        raise ValueError("CPA manifest must be a UTF-8 JSON object") from None
    if not isinstance(value, dict):
        raise ValueError("CPA manifest must be a UTF-8 JSON object")


def read_cpa_manifest(path):
    """Read one stable regular file without following a final-component link."""
    path = Path(path)
    try:
        before = path.lstat()
        if path.is_symlink() or not stat.S_ISREG(before.st_mode):
            raise ValueError("CPA manifest must be a real regular file")
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(descriptor, "rb") as source:
            opened = os.fstat(source.fileno())
            if not stat.S_ISREG(opened.st_mode) or (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
                raise ValueError("CPA manifest changed while opening")
            contents = source.read()
            after = os.fstat(source.fileno())
        final = path.lstat()
    except OSError:
        raise ValueError("CPA manifest is missing or unreadable") from None
    identity = lambda item: (item.st_dev, item.st_ino, item.st_mode, item.st_size,
                             item.st_mtime_ns, item.st_ctime_ns)
    if path.is_symlink() or not stat.S_ISREG(final.st_mode) or not (
            identity(before) == identity(opened) == identity(after) == identity(final)):
        raise ValueError("CPA manifest changed while reading")
    validate_cpa_manifest(contents)
    return contents


def candidate_cpa_manifest(repo, sha):
    relative = DEFAULT_CPA_MANIFEST.as_posix()
    try:
        entry = subprocess.check_output(
            ["git", "-C", str(repo), "ls-tree", sha, "--", relative], text=True, timeout=30).strip()
        fields = entry.split(None, 3)
        if len(fields) != 4 or fields[:2] not in (["100644", "blob"], ["100755", "blob"]):
            raise ValueError
        contents = subprocess.check_output(
            ["git", "-C", str(repo), "show", sha + ":" + relative], timeout=30)
    except (OSError, ValueError, subprocess.SubprocessError):
        raise ValueError("Candidate default CPA manifest is missing or not a regular file") from None
    validate_cpa_manifest(contents)
    return contents


def select_cpa_manifest(repo, sha):
    selected = os.environ.get("HIROUTE_CPA_MANIFEST")
    if selected is None:
        return candidate_cpa_manifest(repo, sha)
    try:
        supplied = Path(selected)
        path = supplied if supplied.is_absolute() else repo / DEFAULT_CPA_MANIFEST.parent / supplied
        path = Path(os.path.abspath(path))
    except (OSError, TypeError, ValueError):
        raise ValueError("HIROUTE_CPA_MANIFEST does not identify a valid CPA manifest") from None
    return read_cpa_manifest(path)


def build_recipe(repo, sha, jobs, cpa_manifest=None):
    # These are the only inherited knobs supported by this fixed build profile.
    # Values may contain private paths/flags; persist their digest, not plaintext.
    knobs = {k: v for k, v in os.environ.items() if
             k.startswith(("CARGO_", "RUST", "VITE_", "npm_config_", "NPM_CONFIG_", "CC_", "CXX_",
                           "CMAKE_", "PKG_CONFIG_", "OPENSSL_"))
             or k in ("PATH", "SDKROOT", "MACOSX_DEPLOYMENT_TARGET", "CC", "CXX", "CFLAGS",
                      "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "ARCHFLAGS", "NODE_ENV", "NODE_OPTIONS")}
    for forbidden in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "TAURI_CONFIG"):
        if forbidden in os.environ:
            raise ValueError("Remove " + forbidden + " before acquiring a reusable Pilot build")
    toolchain = local.git(repo, "show", sha + ":rust-toolchain.toml")
    if (repo / "rust-toolchain.toml").read_text().strip() != toolchain:
        raise ValueError("Execution checkout toolchain differs from candidate; select a matching checkout")
    versions = {}
    for command in (["cargo", "--version"], ["rustc", "-vV"], ["node", "--version"], ["npm", "--version"],
                    ["cc", "--version"], ["cmake", "--version"], ["sccache", "--version"]):
        versions[command[0]] = subprocess.check_output(command, cwd=repo, text=True, timeout=30).strip()
    if sys.platform == "darwin":
        versions["sdk"] = subprocess.check_output(["xcrun", "--show-sdk-version"], text=True, timeout=30).strip()
        versions["sdk_path"] = subprocess.check_output(["xcrun", "--show-sdk-path"], text=True, timeout=30).strip()
    if cpa_manifest is None:
        cpa_manifest = select_cpa_manifest(repo, sha)
    return dict(sha=sha, origin=local.git(repo, "remote", "get-url", "origin"), command=COMMAND,
                profile="debug", jobs=jobs, toolchain=toolchain, tools=versions,
                platform=[sys.platform, platform.machine(), platform.release()], environment=digest(knobs),
                cpa_manifest={"sha256": hashlib.sha256(cpa_manifest).hexdigest()},
                drivers={name: pilot.file_sha256(Path(__file__).with_name(name)) for name in DRIVERS})


def run_step(argv, checkout, env, log, timeout, descriptors):
    with log.open("a") as output:
        process = local.remote.start_locked_command(argv, checkout, env, output, descriptors)
        try:
            code = process.wait(timeout=timeout)
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait()
        if code:
            raise RuntimeError("Frontend step failed (%s); inspect %s" % (code, log))


class Builds:
    def __init__(self, store=None):
        self.store = store or local.Store()
        pilot.LOCAL_RUST_ROOT = self.store.root
        self.root = self.store.root / "pilot-builds"
        self.root.mkdir(exist_ok=True, mode=0o700)
        local.identity(self.root)
        for name in ("keys", "leases"):
            (self.root / name).mkdir(exist_ok=True, mode=0o700)
            local.identity(self.root / name)

    @contextlib.contextmanager
    def locked(self, key):
        if not re.fullmatch(r"[a-f0-9]{64}", key):
            raise ValueError("Invalid build key")
        with (self.root / "keys" / (key + ".lock")).open("a") as handle:
            local.remote.fcntl.flock(handle, local.remote.fcntl.LOCK_EX | local.remote.fcntl.LOCK_NB)
            yield

    def index(self, key):
        return self.root / "keys" / (key + ".json")

    def locate(self, lease):
        if not re.fullmatch(r"[a-f0-9]{32}", lease):
            raise ValueError("Invalid lease ID")
        link = json.loads((self.root / "leases" / (lease + ".json")).read_text())
        row = self.store.load(link["run"])
        if row["pilot_reuse"]["key"] != link["key"] or lease not in row["pilot_leases"]:
            raise ValueError("Lease/build identity mismatch")
        return row

    def prepare(self, timeout, manifest):
        def frontend(checkout, env, row, descriptors):
            run = self.store.record_path(row["id"]).parent
            # Cargo consumes these verified bytes even if the borrowed source changes
            # while npm/Cargo run. Only the build-owned snapshot survives ingestion.
            snapshot = run / "cpa-manifest.json"
            with snapshot.open("xb") as output:
                os.chmod(snapshot, 0o600)
                output.write(manifest)
            env["HIROUTE_CPA_MANIFEST"] = str(snapshot)
            log = run / "frontend.log"
            env["PYTHONDONTWRITEBYTECODE"] = "1"
            for argv in (["npm", "--prefix", "apps/desktop", "ci"],
                         ["npm", "--prefix", "apps/desktop", "run", "build"]):
                run_step(argv, checkout, env, log, timeout, descriptors)
            dist = checkout / "apps/desktop/dist"
            row["pilot_frontend"] = {"sha256": tree_digest(dist)}
            saved = run / "frontend-dist"
            shutil.copytree(dist, saved)
            row["pilot_frontend"]["path"] = str(saved)
            env["TAURI_CONFIG"] = subprocess.check_output(
                [sys.executable, str(checkout / "scripts/desktop-pilot.py"), "config",
                 "--frontend-dist", str(dist)], cwd=checkout, env=env, text=True, timeout=30).strip()
            row["pilot_frontend"]["tauri_config_sha256"] = hashlib.sha256(env["TAURI_CONFIG"].encode()).hexdigest()
            row["pilot_frontend"]["generated"] = {
                str(path.relative_to(checkout)): local.identity(path)
                for path in (checkout / "apps/desktop/node_modules", dist) if path.exists()}
        return frontend

    def discard_generated(self, row):
        checkout = Path(row["checkout"])
        if local.identity(checkout) != row["identity"] or local.git(checkout, "status", "--porcelain", "--untracked-files=no"):
            raise ValueError("Build checkout changed; generated files retained")
        generated = dict(row["pilot_frontend"]["generated"])
        native = checkout / "apps/desktop/src-tauri/gen"
        if native.exists():
            generated[str(native.relative_to(checkout))] = local.identity(native)
        for relative, expected in generated.items():
            if relative not in ("apps/desktop/node_modules", "apps/desktop/dist", "apps/desktop/src-tauri/gen"):
                raise ValueError("Unexpected generated directory")
            path = checkout / relative
            if local.identity(path) != expected or local.git(checkout, "ls-files", relative):
                raise ValueError("Generated directory identity changed or contains tracked files")
            shutil.rmtree(path)  # Owned generated root; rmtree does not follow contained npm links.
        row["pilot_frontend"]["generated_removed"] = True

    def verify(self, row, recipe):
        if row["pilot_reuse"]["recipe"] != recipe or row["command"] != COMMAND:
            raise ValueError("Build inputs differ")
        snapshot = self.store.record_path(row["id"]).parent / "cpa-manifest.json"
        if hashlib.sha256(read_cpa_manifest(snapshot)).hexdigest() != recipe["cpa_manifest"]["sha256"]:
            raise ValueError("Compiled CPA manifest snapshot changed")
        self.store.verify(row)
        build = pilot.verify_managed_pilot_build(row["id"], str(Path(row["checkout"]) / "target/debug/hiroute-desktop"), row["sha"])
        if build["artifacts"] != row["pilot_reuse"]["artifacts"]:
            raise ValueError("Reusable executable identity/content changed")
        frontend = row["pilot_frontend"]
        expected = self.store.record_path(row["id"]).parent / "frontend-dist"
        if Path(frontend["path"]) != expected or tree_digest(expected) != frontend["sha256"]:
            raise ValueError("Reusable frontend content changed")
        return build

    def acquire(self, args):
        cases = sorted(set(args.case))
        if not cases or len(cases) != len(args.case) or not args.owner.strip() or any(not c.strip() or "=" in c for c in cases):
            raise ValueError("Declare a nonempty owner and distinct required cases")
        if args.jobs < 1 or args.timeout <= 0:
            raise ValueError("jobs/timeout must be positive")
        repo = Path(args.repo).resolve(strict=True)
        local.remote.validate(args.ref, args.sha, COMMAND)
        os.environ["PATH"] = local.toolchain_environment()
        ref = "refs/hiroute-pilot/" + uuid.uuid4().hex
        try:
            local.git(repo, "fetch", "--no-tags", "origin", args.ref + ":" + ref)
            local.git(repo, "merge-base", "--is-ancestor", args.sha, ref)
        finally:
            local.git(repo, "update-ref", "-d", ref)
        manifest = select_cpa_manifest(repo, args.sha)
        recipe = build_recipe(repo, args.sha, args.jobs, manifest)
        key = digest(recipe)
        with self.locked(key):
            index = self.index(key)
            row = self.store.load(json.loads(index.read_text())["run"]) if index.exists() else None
            reused = bool(row and not row.get("removed"))
            if reused:
                if not row.get("pilot_reuse"):
                    raise ValueError("Previous build failed/interrupted; inspect retained run " + row["id"])
                with self.store.locked(row["checkout"]):
                    self.verify(row, recipe)
            else:
                run_args = types.SimpleNamespace(repo=str(repo), ref=args.ref, sha=args.sha, source="origin",
                                                jobs=args.jobs, command=COMMAND, keep=True, cargo_only=False, timeout=args.timeout)
                row = local.run(self.store, run_args, prepare=self.prepare(args.timeout, manifest),
                                on_record=lambda record: local.remote.save(index, {"run": record["id"], "key": key}))
                if row.get("process_exit") != 0 or row.get("error") or row.get("scenario") != "unassessed":
                    raise ValueError("Pilot build failed/unknown; retained run " + row["id"])
                with self.store.locked(row["checkout"]):
                    self.discard_generated(row)
                    self.store.save(row)
                    build = pilot.verify_managed_pilot_build(row["id"], str(Path(row["checkout"]) / "target/debug/hiroute-desktop"), args.sha)
                    row["pilot_reuse"] = {"key": key, "recipe": recipe, "artifacts": build["artifacts"]}
                    row["pilot_leases"] = {}
                    row["pilot_cleanup_ready"] = False
                    self.store.save(row)
                    self.verify(row, recipe)
            with self.store.locked(row["checkout"]):
                # An open lease is durable ownership, not inferred from a process scan.
                for lease, owner in row["pilot_leases"].items():
                    if owner["owner"] == args.owner and owner["state"] == "open":
                        if owner["cases"] != cases:
                            raise ValueError("Owner already acquired this build with different cases")
                        break
                else:
                    lease = uuid.uuid4().hex
                    row["pilot_leases"][lease] = dict(owner=args.owner, cases=cases, sessions={}, state="open")
                row.update(keep=True, evidence_saved=False, pilot_cleanup_ready=False)
                self.store.save(row)
                local.remote.save(self.root / "leases" / (lease + ".json"), {"run": row["id"], "key": key})
            return dict(lease=lease, build_run=row["id"], reused=reused, source_sha=row["sha"],
                        checkout=row["checkout"], cases=cases, scenario="unassessed")

    def start(self, args):
        if getattr(args, "native_app", False) and sys.platform != "darwin":
            raise ValueError("--native-app requires macOS LaunchServices")
        row = self.locate(args.lease)
        with self.locked(row["pilot_reuse"]["key"]), self.store.locked(row["checkout"]):
            row = self.store.load(row["id"])
            owner = row["pilot_leases"][args.lease]
            if owner["state"] != "open" or args.case not in owner["cases"]:
                raise ValueError("Closed lease or undeclared case")
            build = self.verify(row, row["pilot_reuse"]["recipe"])
            if getattr(args, "native_app", False):
                pilot.native_app_module().staged_cpa(build)
            if args.case in owner["sessions"]:
                raise ValueError("Case already started; inspect its recorded session instead of duplicating it")
            root = Path("/tmp").resolve() / ("hrp-" + uuid.uuid4().hex[:12])
            entry = {"session": str(root / "session.json"), "state": "starting"}
            owner["sessions"][args.case] = entry
            self.store.save(row)  # Crash during launch leaves durable unknown ownership.
            launch = pilot.build_parser().parse_args([
                "start", "--app", str(Path(row["checkout"]) / "target/debug/hiroute-desktop"),
                "--build-run", row["id"], "--source-sha", row["sha"], "--root", str(root),
                "--timeout", str(args.timeout),
                *(["--native-app"] if getattr(args, "native_app", False) else []),
                *(["--data-root", args.data_root] if args.data_root else []),
                *(["--process-home", args.process_home] if args.process_home else [])])
            launch.managed_lease = args.lease
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                pilot.start(launch)
            entry["state"] = "ready"
            self.store.save(row)
            return json.loads(output.getvalue())

    def finish(self, args):
        values = {}
        for assessment in args.case:
            case, separator, verdict = assessment.partition("=")
            if not separator or case in values or verdict not in VERDICTS:
                raise ValueError("Expected distinct CASE=green/red/expected_red/not_executed assessments")
            values[case] = verdict
        if not args.evidence_saved:
            raise ValueError("Confirm unique debug evidence saved before releasing ownership")
        row = self.locate(args.lease)
        with self.locked(row["pilot_reuse"]["key"]):
            with self.store.locked(row["checkout"]):
                row = self.store.load(row["id"])
                owner = row["pilot_leases"][args.lease]
                if set(values) != set(owner["cases"]):
                    raise ValueError("Assess every declared case exactly once")
                if owner["state"] == "closed":
                    if values != owner["verdicts"]:
                        raise ValueError("Finished evidence cannot be rewritten")
                else:
                    for case, verdict in values.items():
                        entry = owner["sessions"].get(case)
                        if verdict == "green" and (not entry or entry["state"] != "ready"):
                            raise ValueError("Unstarted/unknown case cannot be green")
                        if entry:
                            _, session = pilot.read_session(entry["session"])
                            if session["build_run"] != row["id"] or session["source_sha"] != row["sha"]:
                                raise ValueError("Session belongs to another build")
                            pilot.stop_session(Path(entry["session"]), session, revoke=True)
                            diagnostics = pilot.diagnostics_index(Path(session["data_root"]))
                            if verdict == "green":
                                for role in ("desktop", "daemon"):
                                    applied = diagnostics["roles"][role].get("evidence", {}).get("level_applied")
                                    if not applied or applied.get("level") != "debug":
                                        raise ValueError("Green case needs both roles' actual Debug evidence")
                            entry["diagnostics"] = diagnostics
                    owner.update(state="closed", verdicts=values, evidence_saved=True, finished_at=time.time())
                    self.store.save(row)
                active = [lease for lease, value in row["pilot_leases"].items() if value["state"] != "closed"]
                if active:
                    return dict(build_run=row["id"], lease=args.lease, cleanup="retained", active_leases=active)
                outcomes = [v for value in row["pilot_leases"].values() for v in value["verdicts"].values()]
                scenario = ("red" if "red" in outcomes else "unassessed" if "not_executed" in outcomes
                            else "expected_red" if "expected_red" in outcomes else "green")
                row.update(scenario=scenario, evidence_saved=True, keep=False, pilot_cleanup_ready=True)
                self.store.save(row)
            if row.get("removed"):
                return dict(build_run=row["id"], lease=args.lease, cleanup="removed", scenario=row["scenario"])
            candidate = self.store.preview_one(row)
            if candidate["state"] != "candidate":
                return dict(build_run=row["id"], lease=args.lease, cleanup="retained", reason=candidate.get("reason"))
            result = self.store.apply(row["id"], candidate["token"])
            return dict(build_run=row["id"], lease=args.lease, cleanup=result["state"], scenario=row["scenario"])


def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="action", required=True)
    acquire = sub.add_parser("acquire")
    acquire.add_argument("--repo", default=".")
    acquire.add_argument("--ref", required=True)
    acquire.add_argument("--sha", required=True)
    acquire.add_argument("--owner", required=True)
    acquire.add_argument("--case", action="append", required=True)
    acquire.add_argument("--jobs", type=int, default=2)
    acquire.add_argument("--timeout", type=int, default=3600)
    start = sub.add_parser("start")
    start.add_argument("--lease", required=True)
    start.add_argument("--case", required=True)
    start.add_argument("--data-root")
    start.add_argument("--process-home")
    start.add_argument("--native-app", action="store_true")
    start.add_argument("--timeout", type=float, default=240)
    finish = sub.add_parser("finish")
    finish.add_argument("--lease", required=True)
    finish.add_argument("--case", action="append", required=True)
    finish.add_argument("--evidence-saved", action="store_true", required=True)
    sub.add_parser("status").add_argument("--lease", required=True)
    args = parser.parse_args()
    builds = Builds()
    result = builds.locate(args.lease) if args.action == "status" else getattr(builds, args.action)(args)
    print(json.dumps(result, indent=2))
    if args.action == "finish" and result["cleanup"] == "retained" and not result.get("active_leases"):
        return 1
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, KeyError, OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
