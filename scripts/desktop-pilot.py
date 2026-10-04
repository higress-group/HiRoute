#!/usr/bin/env python3
"""Launch and stop one isolated HiRoute Desktop tauri-pilot instance."""

import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import sys
import tempfile
import time
from typing import Optional


IDENTIFIER = "ai.hiroute.desktop"
SOCKET_NAME = f"tauri-pilot-{IDENTIFIER}.sock"
SESSION_FILE = "session.json"
LOCAL_RUST_ROOT = Path.home() / ".cache/hiroute/local-rust"
BUILD_RUN_PATTERN = re.compile(r"[a-f0-9]{32}")
# `persisted` is a launcher selection, not a fifth diagnostic level: it launches without an
# override so the isolated root's saved value applies.
DIAGNOSTIC_LEVELS = ("error", "warn", "info", "debug")
DIAGNOSTIC_LEVEL_SELECTIONS = (*DIAGNOSTIC_LEVELS, "persisted")
DIAGNOSTIC_LEVEL_ENV = "HIROUTE_DIAGNOSTIC_LEVEL_OVERRIDE"
DIAGNOSTIC_RECORD_SCHEMA = "hiroute.diagnostic-event/v1"
DIAGNOSTIC_ROLES = ("desktop", "daemon")
# One role may hold four rotated files plus the current one; each file is capped by the
# product, and the Pilot refuses anything larger instead of reading it.
MAX_DIAGNOSTIC_RECORD_BYTES = 4096
MAX_DIAGNOSTIC_FILE_BYTES = 4 * 1024 * 1024
MAX_DEGRADED_REASONS = 8
DIAGNOSTIC_FILES = (
    "current.jsonl",
    *[f"previous-{index}.jsonl" for index in range(1, 5)],
)
ALLOWED_DIAGNOSTIC_NAMES = frozenset(
    [
        *DIAGNOSTIC_FILES,
        "settings.json",
        "settings.lock",
        "correlation.key",
        "writer.lock",
    ]
)


def private_directory(path: Path, *, create: bool = False) -> Path:
    if create:
        path.mkdir(parents=True, exist_ok=False, mode=0o700)
    path = path.resolve(strict=True)
    info = path.lstat()
    if path.is_symlink() or not path.is_dir() or info.st_uid != os.geteuid():
        raise ValueError(f"not an owned real directory: {path}")
    if stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError(f"directory must have mode 0700: {path}")
    return path


def new_root() -> Path:
    system = Path(tempfile.gettempdir()).resolve(strict=True)
    probe = system / "hrp-xxxxxxxx" / "runtime" / SOCKET_NAME
    if len(os.fsencode(probe)) >= 100:
        system = Path("/tmp").resolve(strict=True)
    root = Path(tempfile.mkdtemp(prefix="hrp-", dir=system)).resolve(strict=True)
    os.chmod(root, 0o700)
    return private_directory(root)


def make_subdirectory(root: Path, name: str) -> Path:
    path = root / name
    path.mkdir(mode=0o700)
    return private_directory(path)


def select_data_root(root: Path, existing: Optional[str]) -> Path:
    if existing is None:
        return make_subdirectory(root, "data")
    supplied = Path(existing)
    if supplied.is_symlink():
        raise ValueError(f"existing data root must not be a symlink: {supplied}")
    data = private_directory(supplied)
    if data == root or data in root.parents or root in data.parents:
        raise ValueError("existing data root must be outside the Pilot session root")
    return data


def launch_environment(
    runtime: Path,
    temporary: Path,
    data: Path,
    process_home: Optional[Path],
    diagnostic_override: Optional[str] = None,
) -> dict[str, str]:
    environment = os.environ.copy()
    # A foreign or stale override must never leak into the isolated instance; only this
    # launcher decides the level, and `persisted` deliberately sets none.
    environment.pop(DIAGNOSTIC_LEVEL_ENV, None)
    environment.update(
        XDG_RUNTIME_DIR=str(runtime),
        TMPDIR=str(temporary),
        HIROUTE_DESKTOP_TEST_ROOT=str(data),
    )
    if process_home is not None:
        # The explicit synthetic context must not authenticate through a caller's
        # inherited provider variables. Leave ordinary non-isolated launches alone.
        for key in list(environment):
            if key.startswith(("OPENAI_", "ANTHROPIC_", "AZURE_", "AWS_", "GOOGLE_", "VERTEX_", "QODER_")) or key in (
                "CODEX_CONFIG", "CODEX_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "GEMINI_API_KEY",
                "OPENROUTER_API_KEY", "HIROUTE_RUN_TOKEN", "CLAUDE_CODE_USE_BEDROCK",
                "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY",
            ):
                environment.pop(key)
        environment["HOME"] = str(process_home)
        # Workers borrow the product instance's native context. An isolated HOME must
        # not retain native config locators pointing into the caller's daily account.
        environment["CODEX_HOME"] = str(process_home / ".codex")
        environment["CLAUDE_CONFIG_DIR"] = str(process_home / ".claude")
        environment["QODER_CONFIG_DIR"] = str(process_home / ".qoder")
    if diagnostic_override is not None:
        environment[DIAGNOSTIC_LEVEL_ENV] = diagnostic_override
    return environment


def diagnostic_override(selection: str) -> Optional[str]:
    """Map one launcher selection to the process override; `persisted` means no override."""
    if selection == "persisted":
        return None
    if selection not in DIAGNOSTIC_LEVELS:
        raise ValueError(f"unknown diagnostic level selection: {selection}")
    return selection


def parse_diagnostic_record(raw: bytes) -> Optional[dict]:
    """One versioned record, or None for a record still being appended."""
    try:
        row = json.loads(raw)
    except ValueError:
        return None
    if not isinstance(row, dict) or row.get("schema") != DIAGNOSTIC_RECORD_SCHEMA:
        raise ValueError("diagnostic record schema is unexpected")
    return row


def owned_records(role_directory: Path) -> list:
    """The parsed complete records of this role directory, newest log file first."""
    files = []
    for name in DIAGNOSTIC_FILES:
        path = role_directory / name
        try:
            info = path.lstat()
        except FileNotFoundError:
            files.append([])
            continue
        if path.is_symlink() or not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid():
            raise ValueError(f"diagnostic log is not an owned real file: {name}")
        if info.st_size > MAX_DIAGNOSTIC_FILE_BYTES:
            raise ValueError(f"diagnostic log exceeds its bound: {name}")
        rows = []
        for line in path.read_bytes().splitlines(keepends=True):
            if not line.endswith(b"\n"):
                # The writer may be appending this record right now.
                break
            if len(line) > MAX_DIAGNOSTIC_RECORD_BYTES:
                raise ValueError(f"diagnostic record exceeds its bound: {name}")
            row = parse_diagnostic_record(line)
            if row is None:
                raise ValueError(f"diagnostic record is not valid JSON: {name}")
            rows.append(row)
        files.append(rows)
    return files


def last_boot_event(files: list, boot_id: str, event_name: str) -> Optional[dict]:
    """The last record of one event the given boot wrote, from the newest file that has one.

    Rotation moves a boot's earlier lines into `previous-*`, so the newest file alone is not
    the whole story; but a record of an older boot is never a stand-in for the current one."""
    for rows in files:
        last = None
        for row in rows:
            if row.get("boot_id") != boot_id:
                continue
            event = row.get("event")
            if isinstance(event, dict) and isinstance(event.get(event_name), dict):
                last = event[event_name]
        if last is not None:
            return last
    return None


def read_level_evidence(role_directory: Path) -> dict:
    """What the role's own logs state about the process that wrote them: the level the newest
    boot applied, and the last writer counters that same boot observed. Rotated files are
    searched for that boot; a previous boot's history is never reported as the current level
    or current counters, and a missing record stays unknown. Degradations are counted over
    every owned record. Bounded records are read at collection time; nothing here is treated
    as readiness, and no log content or free text is copied into the report."""
    evidence = {
        "records": 0,
        "level_applied": None,
        "degraded_events": 0,
        "degraded_reasons": [],
        "write_failures": None,
        "lost_at_shutdown": None,
    }
    reasons = set()
    files = owned_records(role_directory)
    for rows in files:
        evidence["records"] += len(rows)
        for row in rows:
            event = row.get("event")
            if not isinstance(event, dict):
                continue
            degraded = event.get("diagnostics_degraded")
            if isinstance(degraded, dict):
                evidence["degraded_events"] += 1
                reason = degraded.get("reason")
                if isinstance(reason, str) and len(reasons) < MAX_DEGRADED_REASONS:
                    reasons.add(reason)
    # The newest complete record belongs to the process this directory currently describes.
    current_boot = next((rows[-1].get("boot_id") for rows in files if rows), None)
    if isinstance(current_boot, str) and current_boot:
        applied = last_boot_event(files, current_boot, "level_applied")
        if applied is not None:
            evidence["level_applied"] = {
                "level": applied.get("level"),
                "revision": applied.get("revision"),
                "source": applied.get("source"),
            }
        stats = last_boot_event(files, current_boot, "writer_stats")
        if stats is not None:
            evidence["write_failures"] = stats.get("write_failures")
            evidence["lost_at_shutdown"] = stats.get("lost_at_shutdown")
    evidence["degraded_reasons"] = sorted(reasons)
    return evidence


def diagnostics_index(data_root: Path) -> dict:
    """Bounded index of this instance's safe diagnostics: which role files exist, their sizes
    and the level/gap evidence each process wrote into its own log. Never log contents; the
    raw desktop-and-daemon development log stays a separate private, non-sharable artifact."""
    root = data_root / "diagnostics"
    roles = {}
    complete = True
    for role in DIAGNOSTIC_ROLES:
        directory = root / role
        view = {
            "state": "missing",
            "files": [],
            "log_bytes": 0,
            "unexpected_entries": 0,
            "evidence": None,
        }
        try:
            info = directory.lstat()
        except FileNotFoundError:
            info = None
        if (
            info is not None
            and stat.S_ISDIR(info.st_mode)
            and not directory.is_symlink()
            and info.st_uid == os.geteuid()
        ):
            view["state"] = "present"
            for entry in sorted(directory.iterdir(), key=lambda item: item.name):
                try:
                    entry_info = entry.lstat()
                except FileNotFoundError:
                    continue
                if (
                    entry.name in ALLOWED_DIAGNOSTIC_NAMES
                    and not entry.is_symlink()
                    and stat.S_ISREG(entry_info.st_mode)
                ):
                    view["files"].append({"name": entry.name, "bytes": entry_info.st_size})
                    if entry.name.endswith(".jsonl"):
                        view["log_bytes"] += entry_info.st_size
                else:
                    view["unexpected_entries"] += 1
            try:
                view["evidence"] = read_level_evidence(directory)
            except (OSError, ValueError) as error:
                view["evidence"] = {"error": str(error)}
        if view["state"] != "present" or not any(
            item["name"] == "current.jsonl" for item in view["files"]
        ):
            complete = False
        roles[role] = view
    return {"root": str(root), "roles": roles, "complete": complete}


def write_private_json(path: Path, value: object) -> None:
    temporary = path.with_suffix(".tmp")
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        temporary.replace(path)
    finally:
        if temporary.exists():
            temporary.unlink()


def read_session(value: str) -> tuple[Path, dict]:
    supplied = Path(value).resolve(strict=True)
    session_path = supplied / SESSION_FILE if supplied.is_dir() else supplied
    root = private_directory(session_path.parent)
    info = session_path.lstat()
    if session_path.is_symlink() or not session_path.is_file() or info.st_uid != os.geteuid():
        raise ValueError("session metadata is not an owned real file")
    if stat.S_IMODE(info.st_mode) & 0o077:
        raise ValueError("session metadata must not be accessible by group or other users")
    row = json.loads(session_path.read_text(encoding="utf-8"))
    if Path(row["root"]).resolve(strict=True) != root or row["owner_uid"] != os.geteuid():
        raise ValueError("session metadata does not match its owned root")
    return session_path, row


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def executable_artifact(path: Path) -> dict:
    supplied = path
    path = path.resolve(strict=True)
    info = path.lstat()
    if supplied.is_symlink() or path.is_symlink() or not path.is_file():
        raise ValueError(f"executable must be a real regular file: {path}")
    if info.st_uid != os.geteuid() or not os.access(path, os.X_OK):
        raise ValueError(f"executable must be owned by this user and runnable: {path}")
    return {
        "path": str(path),
        "device": info.st_dev,
        "inode": info.st_ino,
        "size": info.st_size,
        "mtime_ns": info.st_mtime_ns,
        "sha256": file_sha256(path),
    }


def artifact_is_unchanged(expected: dict) -> None:
    current = executable_artifact(Path(expected["path"]))
    if current != expected:
        raise ValueError(f"recorded executable was replaced or modified: {expected['path']}")


def directory_identity(path: Path) -> list[int]:
    path = path.resolve(strict=True)
    info = path.lstat()
    if path.is_symlink() or not path.is_dir() or info.st_uid != os.geteuid():
        raise ValueError(f"not an owned real directory: {path}")
    return [info.st_dev, info.st_ino]


def git_output(checkout: Path, *arguments: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(checkout), *arguments], text=True, errors="replace"
    ).strip()


def command_features(command: list[str]) -> set[str]:
    values = []
    for index, token in enumerate(command):
        if token in ("--features", "-F") and index + 1 < len(command):
            values.append(command[index + 1])
        elif token.startswith("--features="):
            values.append(token.split("=", 1)[1])
    return {feature for value in values for feature in re.split(r"[ ,]+", value) if feature}


def command_packages(command: list[str]) -> set[str]:
    values = []
    for index, token in enumerate(command):
        if token in ("--package", "-p") and index + 1 < len(command):
            values.append(command[index + 1])
        elif token.startswith("--package="):
            values.append(token.split("=", 1)[1])
    return set(values)


def verify_managed_pilot_build(run_id: str, app_value: str, source_sha: Optional[str]) -> dict:
    if not BUILD_RUN_PATTERN.fullmatch(run_id):
        raise ValueError("--build-run must be a 32-character local-rust run ID")
    root = private_directory(LOCAL_RUST_ROOT)
    runs = private_directory(root / "runs")
    checkouts = private_directory(root / "checkouts")
    result_path = runs / run_id / "result.json"
    private_directory(result_path.parent)
    result_info = result_path.lstat()
    if result_path.is_symlink() or not result_path.is_file() or result_info.st_uid != os.geteuid():
        raise ValueError("local-rust result is not an owned real file")
    result_bytes = result_path.read_bytes()
    result = json.loads(result_bytes)
    checkout = (checkouts / run_id).resolve(strict=True)
    command = result.get("command")
    if (
        result.get("id") != run_id
        or result.get("kind") != "managed"
        or result.get("status") != "terminal"
        or result.get("process_exit") != 0
        or result.get("keep") is not True
        or result.get("scenario") not in ("unassessed", "green")
        or result.get("removed")
        or not isinstance(command, list)
    ):
        raise ValueError("local-rust result is not a successful retained managed build")
    if Path(result.get("checkout", "")).resolve(strict=True) != checkout:
        raise ValueError("local-rust result does not identify its managed checkout")
    if command[:2] != ["cargo", "build"] or "--locked" not in command:
        raise ValueError("local-rust result is not a locked Cargo build")
    if "--release" in command or any(token.startswith("--profile") for token in command):
        raise ValueError("release/custom-profile artifacts cannot start a Desktop Pilot session")
    if "hiroute-desktop/desktop-pilot" not in command_features(command):
        raise ValueError("local-rust build did not explicitly enable hiroute-desktop/desktop-pilot")
    packages = command_packages(command)
    if (
        not {"hiroute-desktop", "hiroute-daemon"}.issubset(packages)
        or "hiroute-gateway" in packages
        or "--workspace" in command
        or "--all" in command
    ):
        raise ValueError(
            "local-rust build must explicitly select hiroute-desktop and hiroute-daemon "
            "without the colliding hiroute-gateway bin"
        )
    bins = [command[index + 1] for index, token in enumerate(command[:-1]) if token == "--bin"]
    if not {"hiroute-desktop", "hirouted"}.issubset(bins):
        raise ValueError("local-rust build did not produce both required Desktop Pilot binaries")
    agent_cli_selected = "hiroute-cli" in packages or "hiroute" in bins
    if agent_cli_selected and not ({"hiroute-cli"}.issubset(packages) and "hiroute" in bins):
        raise ValueError(
            "Agent smoke builds must explicitly select hiroute-cli and the hiroute bin"
        )
    if result.get("identity") != directory_identity(checkout):
        raise ValueError("managed checkout identity changed after the recorded build")
    target = checkout / "target"
    debug = target / "debug"
    if result.get("target_identity") != directory_identity(target):
        raise ValueError("managed target identity changed after the recorded build")
    if result.get("debug_identity") != directory_identity(debug):
        raise ValueError("managed debug directory identity changed after the recorded build")
    recorded_sha = result.get("sha")
    if not isinstance(recorded_sha, str) or not re.fullmatch(r"[0-9a-f]{40}", recorded_sha):
        raise ValueError("local-rust result has no valid candidate SHA")
    if source_sha is not None:
        if not re.fullmatch(r"[0-9a-f]{40}", source_sha):
            raise ValueError("--source-sha must be a full lowercase 40-character Git SHA")
        if source_sha != recorded_sha:
            raise ValueError("--source-sha does not match the managed build result")
    if git_output(checkout, "rev-parse", "--show-toplevel") != str(checkout):
        raise ValueError("managed build checkout is no longer its recorded worktree")
    if git_output(checkout, "rev-parse", "HEAD") != recorded_sha:
        raise ValueError("managed build checkout no longer matches its recorded SHA")
    if git_output(checkout, "status", "--porcelain", "--untracked-files=no"):
        raise ValueError("managed build checkout has tracked changes after compilation")
    app = Path(app_value).resolve(strict=True)
    expected_app = debug / "hiroute-desktop"
    if app != expected_app:
        raise ValueError(f"--app must be the managed Pilot debug artifact: {expected_app}")
    daemon = debug / "hirouted"
    artifacts = {"app": executable_artifact(app), "daemon": executable_artifact(daemon)}
    if agent_cli_selected:
        artifacts["cli"] = executable_artifact(debug / "hiroute")
    return {
        "run_id": run_id,
        "reuse_managed": bool(result.get("pilot_reuse")),
        "result": str(result_path),
        "result_sha256": hashlib.sha256(result_bytes).hexdigest(),
        "source_sha": recorded_sha,
        "checkout": str(checkout),
        "command": command,
        "artifacts": artifacts,
    }


def process_rows() -> list[dict]:
    output = subprocess.check_output(
        ["ps", "-axo", "pid=,ppid=,pgid=,uid=,state=,command="],
        text=True,
        errors="replace",
    )
    rows = []
    for line in output.splitlines():
        fields = line.strip().split(None, 5)
        if len(fields) == 6 and not fields[4].startswith("Z"):
            rows.append(
                {
                    "pid": int(fields[0]),
                    "ppid": int(fields[1]),
                    "pgid": int(fields[2]),
                    "uid": int(fields[3]),
                    "state": fields[4],
                    "command": fields[5],
                }
            )
    return rows


def process_start_time(pid: int) -> str:
    if sys.platform == "darwin":
        class ProcBsdInfo(ctypes.Structure):
            _fields_ = [
                ("pbi_flags", ctypes.c_uint32),
                ("pbi_status", ctypes.c_uint32),
                ("pbi_xstatus", ctypes.c_uint32),
                ("pbi_pid", ctypes.c_uint32),
                ("pbi_ppid", ctypes.c_uint32),
                ("pbi_uid", ctypes.c_uint32),
                ("pbi_gid", ctypes.c_uint32),
                ("pbi_ruid", ctypes.c_uint32),
                ("pbi_rgid", ctypes.c_uint32),
                ("pbi_svuid", ctypes.c_uint32),
                ("pbi_svgid", ctypes.c_uint32),
                ("rfu_1", ctypes.c_uint32),
                ("pbi_comm", ctypes.c_char * 16),
                ("pbi_name", ctypes.c_char * 32),
                ("pbi_nfiles", ctypes.c_uint32),
                ("pbi_pgid", ctypes.c_uint32),
                ("pbi_pjobc", ctypes.c_uint32),
                ("e_tdev", ctypes.c_uint32),
                ("e_tpgid", ctypes.c_uint32),
                ("pbi_nice", ctypes.c_int32),
                ("pbi_start_tvsec", ctypes.c_uint64),
                ("pbi_start_tvusec", ctypes.c_uint64),
            ]

        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        info = ProcBsdInfo()
        size = ctypes.sizeof(info)
        if library.proc_pidinfo(pid, 3, 0, ctypes.byref(info), size) != size:
            raise ProcessLookupError(f"cannot resolve start identity for process {pid}")
        return f"darwin:{info.pbi_start_tvsec}:{info.pbi_start_tvusec}"
    proc_stat = Path("/proc") / str(pid) / "stat"
    if proc_stat.is_file():
        fields = proc_stat.read_text(encoding="utf-8").rsplit(")", 1)[1].split()
        if len(fields) > 19:
            return f"linux:{fields[19]}"
    raise ValueError(f"this platform cannot prove the start identity for process {pid}")


def process_executable(pid: int) -> Path:
    if sys.platform == "darwin":
        library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        buffer = ctypes.create_string_buffer(4096)
        if library.proc_pidpath(pid, buffer, len(buffer)) <= 0:
            raise ProcessLookupError(f"cannot resolve executable for process {pid}")
        return Path(os.fsdecode(buffer.value)).resolve(strict=True)
    proc_path = Path("/proc") / str(pid) / "exe"
    if proc_path.exists():
        return Path(os.readlink(proc_path)).resolve(strict=True)
    raise ValueError(f"this platform cannot prove the executable for process {pid}")


def capture_process_identity(row: dict, item: dict, role: str) -> dict:
    artifact = row["artifacts"][role]
    if item["uid"] != os.geteuid() or item["pgid"] != int(row["pgid"]):
        raise ValueError("process is outside the recorded owner or process group")
    actual = process_executable(item["pid"])
    if actual != Path(artifact["path"]):
        raise ValueError(f"process executable does not match the recorded {role} artifact")
    artifact_is_unchanged(artifact)
    return {
        "pid": item["pid"],
        "role": role,
        "start_time": process_start_time(item["pid"]),
        "executable": str(actual),
    }


def verify_process_identity(row: dict, item: dict, expected: dict) -> None:
    if item["uid"] != os.geteuid():
        raise ValueError("process group contains a process owned by another user")
    role = expected.get("role")
    if role not in ("app", "daemon"):
        raise ValueError("session contains an unknown process role")
    artifact = row.get("artifacts", {}).get(role)
    if not artifact:
        raise ValueError("session has no exact executable artifact identity")
    if process_start_time(item["pid"]) != expected.get("start_time"):
        raise ValueError(f"PID {item['pid']} was reused; refusing to signal")
    actual = process_executable(item["pid"])
    if str(actual) != expected.get("executable") or str(actual) != artifact.get("path"):
        raise ValueError(f"PID {item['pid']} executable identity changed; refusing to signal")
    artifact_is_unchanged(artifact)


def record_started_processes(row: dict) -> list[int]:
    members = [item for item in process_rows() if item["pgid"] == int(row["pgid"])]
    recorded = {int(item["pid"]): item for item in row.get("process_identities", [])}
    if not members:
        return []
    leader = next((item for item in members if item["pid"] == int(row["pid"])), None)
    if leader is None:
        for item in members:
            expected = recorded.get(item["pid"])
            if expected is None:
                raise ValueError(
                    "Desktop leader disappeared with an unrecorded group member; refusing cleanup"
                )
            verify_process_identity(row, item, expected)
        return [item["pid"] for item in members]
    if leader["pid"] not in recorded:
        identity = capture_process_identity(row, leader, "app")
        row.setdefault("process_identities", []).append(identity)
        recorded[leader["pid"]] = identity
    else:
        verify_process_identity(row, leader, recorded[leader["pid"]])
    for item in members:
        if item["pid"] == leader["pid"]:
            continue
        if item["pid"] in recorded:
            verify_process_identity(row, item, recorded[item["pid"]])
            continue
        if item["ppid"] != int(row["pid"]):
            raise ValueError("unrecorded process is not a direct child of the launched Desktop")
        identity = capture_process_identity(row, item, "daemon")
        row["process_identities"].append(identity)
        recorded[item["pid"]] = identity
    return [item["pid"] for item in members]


def owned_process_group(row: dict, *, stopping: bool = False) -> list[int]:
    pid = int(row["pid"])
    pgid = int(row["pgid"])
    snapshot = process_rows()
    recorded = {int(item["pid"]): item for item in row.get("process_identities", [])}
    exited = set()
    while True:
        if any(item["pid"] in exited for item in snapshot):
            raise ValueError("an exited PID reappeared during stop; refusing to signal")
        members = [item for item in snapshot if item["pgid"] == pgid]
        if not members:
            return []
        if pgid != pid:
            raise ValueError("recorded process group is not led by the launched Desktop PID")
        if not recorded:
            raise ValueError("session has no process start identities; refusing to signal")
        for item in members:
            expected = recorded.get(item["pid"])
            if expected is None:
                raise ValueError(f"process group contains unrecorded PID {item['pid']}; refusing to signal")
            try:
                verify_process_identity(row, item, expected)
            except ProcessLookupError:
                if not stopping:
                    raise
                latest = process_rows()
                if any(current["pid"] == item["pid"] for current in latest):
                    raise
                # TERM/KILL can win between ps and the native identity query. Only
                # confirmed absence is normal exit; revalidate the complete fresh
                # snapshot before another signal, including any newly seen member.
                # Each retry removes a different recorded PID, so this is bounded.
                exited.add(item["pid"])
                snapshot = latest
                break
        else:
            return [item["pid"] for item in members]


def stop_group(row: dict, timeout: float = 12.0) -> list[int]:
    members = owned_process_group(row)
    if not members:
        return []
    pgid = int(row["pgid"])
    os.killpg(pgid, signal.SIGTERM)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not owned_process_group(row, stopping=True):
            return members
        time.sleep(0.1)
    remaining = owned_process_group(row, stopping=True)
    if remaining:
        os.killpg(pgid, signal.SIGKILL)
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline and owned_process_group(row, stopping=True):
            time.sleep(0.05)
        remaining = owned_process_group(row, stopping=True)
        if remaining:
            raise RuntimeError(f"process group still has verified members after SIGKILL: {remaining}")
    return members


def pilot_config(args: argparse.Namespace) -> int:
    frontend = Path(args.frontend_dist).resolve(strict=True)
    if not frontend.is_dir() or not (frontend / "index.html").is_file():
        raise ValueError(f"frontend dist must contain index.html: {frontend}")
    repository = Path(args.repo).resolve(strict=True) if args.repo else Path(__file__).resolve().parent.parent
    config_path = repository / "apps/desktop/src-tauri/tauri.pilot.conf.json"
    production_path = repository / "apps/desktop/src-tauri/tauri.conf.json"
    config = json.loads(config_path.read_text(encoding="utf-8"))
    production = json.loads(production_path.read_text(encoding="utf-8"))
    windows = production["app"]["windows"]
    for window in windows:
        window["incognito"] = True
    config["app"]["windows"] = windows
    config["build"] = {"frontendDist": str(frontend)}
    print(json.dumps(config, separators=(",", ":"), sort_keys=True))
    return 0


def start(args: argparse.Namespace) -> int:
    override = diagnostic_override(args.diagnostic_level)
    build = verify_managed_pilot_build(args.build_run, args.app, args.source_sha)
    if build["reuse_managed"] and not getattr(args, "managed_lease", None):
        raise ValueError("Reusable Pilot builds must start through pilot-build start with an owner lease")
    app = Path(build["artifacts"]["app"]["path"])
    daemon = Path(build["artifacts"]["daemon"]["path"])
    process_home = None
    if args.process_home:
        supplied_home = Path(args.process_home)
        if supplied_home.is_symlink():
            raise ValueError(f"process home must not be a symlink: {supplied_home}")
        process_home = private_directory(supplied_home)

    # Artifact attestation is deliberately complete before any session directory or
    # process is created. A release or arbitrary executable must have zero launch side effects.
    if args.root:
        root_arg = Path(args.root)
        root_arg.mkdir(parents=False, exist_ok=False, mode=0o700)
        root = private_directory(root_arg)
    else:
        root = new_root()
    runtime = make_subdirectory(root, "runtime")
    data = select_data_root(root, args.data_root)
    temporary = make_subdirectory(root, "tmp")
    working = make_subdirectory(root, "agent-input")
    logs = make_subdirectory(root, "logs")
    socket = runtime / SOCKET_NAME
    if len(os.fsencode(socket)) >= 100:
        raise ValueError(f"socket path is too long for a portable Unix-domain socket: {socket}")

    log_path = logs / "desktop-and-daemon.log"
    log_descriptor = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    environment = launch_environment(runtime, temporary, data, process_home, override)
    with os.fdopen(log_descriptor, "ab", buffering=0) as output:
        process = subprocess.Popen(
            [str(app)],
            cwd=working,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=output,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )

    row = {
        "schema": "hiroute.desktop-pilot-session/v2",
        "root": str(root),
        "owner_uid": os.geteuid(),
        "created_at_unix_ms": int(time.time() * 1000),
        "source_sha": build["source_sha"],
        "build_run": build["run_id"],
        "build_result": build["result"],
        "build_result_sha256": build["result_sha256"],
        "artifacts": build["artifacts"],
        "identifier": IDENTIFIER,
        "socket": str(socket),
        "data_root": str(data),
        "process_home": str(process_home) if process_home is not None else environment.get("HOME"),
        "runtime_root": str(runtime),
        "temporary_directory": str(temporary),
        "working_directory": str(working),
        "log": str(log_path),
        "app": str(app),
        "daemon": str(daemon),
        "pid": process.pid,
        "pgid": process.pid,
        "process_identities": [],
        "diagnostic_level_selection": args.diagnostic_level,
        "diagnostic_level_override": override,
    }
    session_path = root / SESSION_FILE
    write_private_json(session_path, row)

    try:
        record_started_processes(row)
        write_private_json(session_path, row)
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(
                    f"Desktop exited with {process.returncode} before the Pilot socket "
                    f"appeared; diagnostics: {json.dumps(diagnostics_index(data), sort_keys=True)}; "
                    f"log: {log_path}"
                )
            try:
                socket_info = socket.lstat()
            except FileNotFoundError:
                time.sleep(0.1)
                continue
            if not stat.S_ISSOCK(socket_info.st_mode):
                raise RuntimeError(f"Pilot path is not a socket: {socket}")
            if socket_info.st_uid != os.geteuid() or stat.S_IMODE(socket_info.st_mode) != 0o600:
                raise RuntimeError(f"Pilot socket must be owned by this user with mode 0600: {socket}")
            record_started_processes(row)
            write_private_json(session_path, row)
            if not any(item.get("role") == "daemon" for item in row["process_identities"]):
                time.sleep(0.05)
                continue
            print(
                json.dumps(
                    {
                        "state": "ready",
                        "session": str(session_path),
                        "diagnostics": diagnostics_index(data),
                        **row,
                    },
                    sort_keys=True,
                )
            )
            return 0
        raise TimeoutError(
            f"Pilot socket did not appear within {args.timeout:.1f}s; "
            f"diagnostics: {json.dumps(diagnostics_index(data), sort_keys=True)}; log: {log_path}"
        )
    except BaseException as error:
        try:
            record_started_processes(row)
            write_private_json(session_path, row)
            stop_group(row)
        except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as cleanup_error:
            raise RuntimeError(
                f"{error}; automatic cleanup refused because process ownership could not be "
                f"proved: {cleanup_error}; inspect {session_path} manually"
            ) from error
        raise


def status(args: argparse.Namespace) -> int:
    session_path, row = read_session(args.session)
    members = owned_process_group(row)
    socket = Path(row["socket"])
    result = {
        "state": "running" if members else "stopped",
        "session": str(session_path),
        "source_sha": row.get("source_sha"),
        "build_run": row.get("build_run"),
        "identifier": row["identifier"],
        "socket": str(socket),
        "socket_present": socket.exists(),
        "processes": members,
        "log": row["log"],
        "data_root": row["data_root"],
        "diagnostic_level_selection": row.get("diagnostic_level_selection"),
        "diagnostics": diagnostics_index(Path(row["data_root"])),
    }
    print(json.dumps(result, sort_keys=True))
    return 0 if members else 1


def stop(args: argparse.Namespace) -> int:
    session_path, row = read_session(args.session)
    stopped = stop_group(row, args.timeout)
    remaining = owned_process_group(row)
    if remaining:
        raise RuntimeError(f"verified process group is still running after stop: {remaining}")
    result = {
        "state": "stopped",
        "session": str(session_path),
        "stopped_processes": stopped,
        "log": row["log"],
        "data_root": row["data_root"],
        "artifacts_retained": True,
    }
    print(json.dumps(result, sort_keys=True))
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    configure = subparsers.add_parser(
        "config", allow_abbrev=False,
        help="print the isolated TAURI_CONFIG merge for an already-built frontend"
    )
    configure.add_argument("--frontend-dist", required=True)
    configure.add_argument("--repo", help="repository containing the Desktop config; defaults to this script's checkout")
    configure.set_defaults(function=pilot_config)
    launch = subparsers.add_parser("start", help="start one isolated instrumented Desktop")
    launch.add_argument("--app", required=True, help="absolute or relative path to hiroute-desktop")
    launch.add_argument(
        "--build-run", required=True, help="successful retained local-rust Pilot build run ID"
    )
    launch.add_argument(
        "--source-sha", help="optional full Git SHA that must match the managed build result"
    )
    launch.add_argument("--root", help="new, non-existing session root; defaults to a short OS temp path")
    launch.add_argument(
        "--data-root",
        help=(
            "existing owned 0700 offline HIROUTE_DESKTOP_TEST_ROOT to reuse without copying; "
            "defaults to a new directory inside the session root"
        ),
    )
    launch.add_argument(
        "--process-home",
        help="existing owned 0700 HOME used only by the launched Desktop and daemon",
    )
    launch.add_argument(
        "--diagnostic-level",
        choices=DIAGNOSTIC_LEVEL_SELECTIONS,
        default="debug",
        help=(
            "isolated-instance diagnostic level: an explicit error/warn/info/debug sets the "
            "pilot-build process override (default debug); `persisted` launches without an "
            "override so the isolated root's saved value applies"
        ),
    )
    launch.add_argument("--timeout", type=float, default=240.0)
    launch.set_defaults(function=start)
    inspect = subparsers.add_parser("status", help="inspect one recorded session")
    inspect.add_argument("--session", required=True, help="session.json or its parent directory")
    inspect.set_defaults(function=status)
    terminate = subparsers.add_parser("stop", help="stop only the recorded owned process group")
    terminate.add_argument("--session", required=True, help="session.json or its parent directory")
    terminate.add_argument("--timeout", type=float, default=12.0)
    terminate.set_defaults(function=stop)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    return args.function(args)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError, TimeoutError, subprocess.SubprocessError) as error:
        print(f"desktop-pilot: {error}", file=sys.stderr)
        raise SystemExit(2)
