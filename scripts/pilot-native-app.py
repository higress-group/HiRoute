#!/usr/bin/env python3
"""Private, source-attested macOS App entry for one reusable Pilot lease."""
import contextlib
import hashlib
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import platform
import shlex
import shutil
import stat
import subprocess
import sys
import time

sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("desktop_pilot", Path(__file__).with_name("desktop-pilot.py"))
pilot = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pilot)


@contextlib.contextmanager
def control(session):
    """Serialize reopen, observation and owner shutdown, without launching on observation."""
    pilot.read_session(str(session))
    path = session.parent / "native-control.lock"
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX)
        yield
    finally:
        os.close(descriptor)


def file_identity(path):
    info = path.lstat()
    if path.is_symlink() or not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
        raise ValueError("Native App entry must contain owned regular files")
    return dict(path=str(path), device=info.st_dev, inode=info.st_ino,
                mode=stat.S_IMODE(info.st_mode), size=info.st_size,
                mtime_ns=info.st_mtime_ns, sha256=pilot.file_sha256(path))


def staged_cpa(build):
    """Preserve an explicitly staged CPA only when the compiled manifest pins it."""
    source = Path(build["artifacts"]["daemon"]["path"]).with_name("cliproxyapi")
    if not source.exists() and not source.is_symlink():
        return None
    artifact = pilot.executable_artifact(source)
    result = Path(build["result"])
    record = json.loads(result.read_text())
    manifest_bytes = result.with_name("cpa-manifest.json").read_bytes()
    pinned = record.get("pilot_reuse", {}).get("recipe", {}).get("cpa_manifest", {}).get("sha256")
    if hashlib.sha256(manifest_bytes).hexdigest() != pinned:
        raise ValueError("Staged CPA manifest does not match the compiled build input")
    manifest = json.loads(manifest_bytes)
    system = "macos" if sys.platform == "darwin" else sys.platform
    machine = platform.machine().lower()
    arch = {"arm64": "aarch64", "amd64": "x86_64"}.get(machine, machine)
    matches = [item for item in manifest.get("artifacts", [])
               if item.get("os") == system and item.get("arch") == arch]
    if manifest.get("schema") != "hiroute.desktop.cpa-artifacts/v1" or len(matches) != 1:
        raise ValueError("Staged CPA manifest has no unique native target")
    selected = matches[0]
    if (selected.get("binary_name") != "cliproxyapi" or selected.get("sha256") != artifact["sha256"]
            or selected.get("size") != artifact["size"]):
        raise ValueError("Staged CPA does not match the compiled manifest hash/size")
    return artifact


def verify(row, session):
    native = row["native_app"]
    if native["state"] != "enabled":
        raise ValueError("Native App entry was retired; acquire a new lease")
    for path, identity in native["directories"].items():
        supplied = Path(path)
        if supplied.is_symlink() or pilot.directory_identity(supplied) != identity:
            raise ValueError("Native App directory was replaced")
        pilot.private_directory(supplied)
    for expected in native["files"]:
        if file_identity(Path(expected["path"])) != expected:
            raise ValueError("Native App launch artifact was replaced or modified")
    for expected in native["source_artifacts"].values():
        pilot.artifact_is_unchanged(expected)
    for expected in row["artifacts"].values():
        pilot.artifact_is_unchanged(expected)
    build = pilot.verify_managed_pilot_build(
        row["build_run"], native["source_artifacts"]["app"]["path"], row["source_sha"])
    if build["artifacts"] != native["source_artifacts"]:
        raise ValueError("Native App source build changed")
    if native["source_cpa"] is not None and staged_cpa(build) != native["source_cpa"]:
        raise ValueError("Native App staged CPA was replaced or modified")
    record = json.loads(Path(row["build_result"]).read_text())
    lease = record.get("pilot_leases", {}).get(native["lease"], {})
    if lease.get("state") != "open" or not any(
            item.get("session") == str(session) for item in lease.get("sessions", {}).values()):
        raise ValueError("Native App no longer belongs to an open registered lease")
    # Managed reuse records the actual merged configuration used by the compiler.
    checkout = Path(build["checkout"])
    overlay = json.loads((checkout / "apps/desktop/src-tauri/tauri.pilot.conf.json").read_text())
    production = json.loads((checkout / "apps/desktop/src-tauri/tauri.conf.json").read_text())
    windows = production["app"]["windows"]
    for window in windows:
        window["incognito"] = True
    overlay["app"]["windows"] = windows
    overlay["build"] = {"frontendDist": str(checkout / "apps/desktop/dist")}
    expected = hashlib.sha256(json.dumps(overlay, separators=(",", ":"), sort_keys=True).encode()).hexdigest()
    if record.get("pilot_frontend", {}).get("tauri_config_sha256") != expected:
        raise ValueError("Native App needs an attested non-persistent WebView configuration")


def prepare(args, build, root, runtime, temporary, data, working, log, process_home):
    if not build["reuse_managed"] or not getattr(args, "managed_lease", None):
        raise ValueError("Native App entries require pilot-build start with an owner lease")
    process_home = process_home or pilot.make_subdirectory(root, "home")
    session = root / pilot.SESSION_FILE
    bundle = pilot.make_subdirectory(root, "HiRoute Pilot.app")
    contents = pilot.make_subdirectory(bundle, "Contents")
    binaries = pilot.make_subdirectory(contents, "MacOS")
    resources = pilot.make_subdirectory(contents, "Resources")
    artifacts = {}
    cpa = staged_cpa(build)
    sources = dict(build["artifacts"])
    if cpa is not None:
        sources["cpa"] = cpa
    for role, source in sources.items():
        pilot.artifact_is_unchanged(source)
        destination = binaries / Path(source["path"]).name
        # macOS proc_pidpath can report another name for a shared executable
        # inode. Each App needs its own inode as well as the exact signed bytes.
        shutil.copy2(source["path"], destination)
        deployed = pilot.executable_artifact(destination)
        pilot.artifact_is_unchanged(source)
        if (deployed["sha256"] != source["sha256"] or deployed["size"] != source["size"]
                or (deployed["device"], deployed["inode"]) == (source["device"], source["inode"])):
            raise ValueError("Native App needs a distinct copy of the exact source artifact")
        artifacts[role] = deployed
    for name in ("desktop-pilot.py", "pilot-native-app.py"):
        shutil.copyfile(Path(__file__).with_name(name), resources / name)
        os.chmod(resources / name, 0o600)
    launcher = binaries / "pilot-launch"
    launcher.write_text("#!/bin/sh\nexec " + shlex.join([
        sys.executable, "-I", "-B", str(resources / "pilot-native-app.py"), str(session)]) + "\n")
    os.chmod(launcher, 0o700)
    identifier = "ai.hiroute.pilot." + root.name.removeprefix("hrp-")
    with (contents / "Info.plist").open("wb") as output:
        plistlib.dump(dict(CFBundleIdentifier=identifier, CFBundleExecutable="pilot-launch",
                          CFBundleName="HiRoute Pilot", CFBundleDisplayName="HiRoute Pilot",
                          CFBundlePackageType="APPL", CFBundleVersion="1",
                          NSHighResolutionCapable=True), output)
    os.chmod(contents / "Info.plist", 0o600)
    # Only explicit, non-authentication ambient values survive a normal OS reopen.
    environment = {key: os.environ[key] for key in ("PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ") if key in os.environ}
    environment.update(HOME=str(process_home), XDG_RUNTIME_DIR=str(runtime), TMPDIR=str(temporary),
                       HIROUTE_DESKTOP_TEST_ROOT=str(data), CODEX_HOME=str(process_home / ".codex"),
                       CLAUDE_CONFIG_DIR=str(process_home / ".claude"), QODER_CONFIG_DIR=str(process_home / ".qoder"),
                       PI_CODING_AGENT_DIR=str(process_home / ".pi/agent"), DSH_HOME=str(process_home / ".dsh"))
    override = pilot.diagnostic_override(args.diagnostic_level)
    if override is not None:
        environment[pilot.DIAGNOSTIC_LEVEL_ENV] = override
    row = dict(schema="hiroute.desktop-pilot-session/v2", root=str(root), owner_uid=os.geteuid(),
               created_at_unix_ms=int(time.time() * 1000), source_sha=build["source_sha"],
               build_run=build["run_id"], build_result=build["result"], build_result_sha256=build["result_sha256"],
               artifacts=artifacts, identifier=pilot.IDENTIFIER, socket=str(runtime / pilot.SOCKET_NAME),
               data_root=str(data), process_home=str(process_home), runtime_root=str(runtime),
               temporary_directory=str(temporary), working_directory=str(working), log=str(log),
               app=artifacts["app"]["path"], daemon=artifacts["daemon"]["path"], pid=0, pgid=0,
               process_identities=[], diagnostic_level_selection=args.diagnostic_level,
               diagnostic_level_override=override)
    directories = (root, runtime, temporary, data, working, process_home, log.parent,
                   bundle, contents, binaries, resources)
    row["native_app"] = dict(bundle=str(bundle), bundle_identifier=identifier, state="enabled",
                             lease=args.managed_lease, generation=0, environment=environment,
                             source_artifacts=build["artifacts"], source_cpa=cpa,
                             directories={str(path): pilot.directory_identity(path) for path in directories},
                             files=[file_identity(path) for path in (launcher, contents / "Info.plist",
                                    resources / "desktop-pilot.py", resources / "pilot-native-app.py")])
    pilot.write_private_json(session, row)
    verify(row, session)
    return session, row


def launch(session):
    """LaunchServices executes this entry on every ordinary native opening."""
    with control(session):
        _, row = pilot.read_session(str(session))
        verify(row, session)
        descriptor = os.open(session.parent / "native-instance.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            os.close(descriptor)
            return 0  # Another invocation already owns this exact isolated instance.
        if row["pid"] and pilot.owned_process_group(row):
            os.close(descriptor)
            raise ValueError("Previous native instance still has owned processes")
        socket = Path(row["socket"])
        if socket.exists():
            info = socket.lstat()
            if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.geteuid():
                raise ValueError("Refusing an unowned or non-socket Pilot path")
            socket.unlink()
        if os.getpgrp() != os.getpid():
            os.setsid()
        row.update(pid=os.getpid(), pgid=os.getpid(), process_identities=[dict(
            pid=os.getpid(), role="app", start_time=pilot.process_start_time(os.getpid()), executable=row["app"])])
        row["native_app"]["generation"] += 1
        pilot.write_private_json(session, row)
        # Held by Desktop (and any inheriting children) until this launch exits.
        os.set_inheritable(descriptor, True)
        environment = row["native_app"]["environment"]
        os.chdir(row["working_directory"])
        output = os.open(row["log"], os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW)
        os.dup2(output, 1)
        os.dup2(output, 2)
        os.close(output)
        null = os.open(os.devnull, os.O_RDONLY)
        os.dup2(null, 0)
        os.close(null)
        # The control descriptor closes on exec. Readers cannot observe the
        # short Python-to-Desktop transition as an executable identity mismatch.
        os.execve(row["app"], [row["app"]], environment)


def observe(session):
    with control(session):
        _, row = pilot.read_session(str(session))
        if not row["pid"]:
            return row, []
        members = pilot.record_started_processes(row)
        pilot.write_private_json(session, row)
        return row, members


def stop(session, *, timeout=12.0, revoke=False):
    with control(session):
        _, row = pilot.read_session(str(session))
        if revoke:
            row["native_app"]["state"] = "retired"
            pilot.write_private_json(session, row)
        stopped = []
        if row["pid"]:
            pilot.record_started_processes(row)
            pilot.write_private_json(session, row)
            stopped = pilot.stop_group(row, timeout)
        if revoke:
            # Retired bundles keep metadata/logs, not private executable copies
            # after the last lease's managed checkout cleanup.
            for artifact in row["artifacts"].values():
                path = Path(artifact["path"])
                if path.exists() or path.is_symlink():
                    pilot.artifact_is_unchanged(artifact)
                    path.unlink()
            row["native_app"]["deployment_removed"] = True
            pilot.write_private_json(session, row)
        return stopped


def start(args, build, root, runtime, temporary, data, working, log, process_home):
    session, row = prepare(args, build, root, runtime, temporary, data, working, log, process_home)
    try:
        subprocess.run(["/usr/bin/open", "-n", row["native_app"]["bundle"]], check=True, timeout=30)
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            row, members = observe(session)
            socket = Path(row["socket"])
            if members and socket.exists():
                info = socket.lstat()
                if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o600:
                    raise ValueError("Pilot socket must be owned by this user with mode 0600")
                if any(item["role"] == "daemon" for item in row["process_identities"]):
                    print(json.dumps(dict(row, state="ready", session=str(session),
                                          diagnostics=pilot.diagnostics_index(data)), sort_keys=True))
                    return 0
            time.sleep(0.1)
        raise TimeoutError(f"Native App did not become ready; inspect {session} and {log}")
    except BaseException:
        stop(session, revoke=True)
        raise


if __name__ == "__main__":
    try:
        sys.exit(launch(Path(sys.argv[1])))
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
