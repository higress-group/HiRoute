#!/usr/bin/env python3
"""Install or remove the verified HiRoute standalone candidate for the current user."""

import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import http.client
import ipaddress
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.parse
import urllib.request

sys.dont_write_bytecode = True

PACKAGE_SCHEMA = "hiroute.standalone-package/v1"
MARKER_SCHEMA = "hiroute.standalone-install/v1"
MANAGED_TEXT = "Managed by HiRoute standalone installer"
SERVICE_LABEL = "ai.hiroute.cli"
MAX_MANIFEST = 1024 * 1024
MAX_ARCHIVE = 1024 * 1024 * 1024
DOWNLOAD_CHUNK_SIZE = 8 * 1024 * 1024
DOWNLOAD_ATTEMPTS = 3
PORTABLE = re.compile(r"^[A-Za-z0-9._-]+$")
HEX_SHA256 = re.compile(r"^[a-fA-F0-9]{64}$")
EXPECTED_FILES = {
    "bin/hiroute",
    "bin/hirouted",
    "libexec/cliproxyapi",
    "licenses/CLIProxyAPI-LICENSE",
    "licenses/HiRoute-LICENSE",
    "licenses/THIRD-PARTY-LICENSES.txt",
    "licenses/third-party-licenses.json",
    "skills/hiroute-management/SKILL.md",
    "docs/standalone-cli.md",
}


def run_systemctl(*arguments):
    """Run the optional per-user systemd integration when it exists."""
    if platform.system() != "Linux":
        return None
    executable = shutil.which("systemctl")
    if executable is None:
        return None
    return subprocess.run([executable, "--user", *arguments], check=False)


def load_json_bytes(value):
    def unique(pairs):
        result = {}
        for key, item in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = item
        return result

    return json.loads(value, object_pairs_hook=unique)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def host_target():
    machine = {"AMD64": "x86_64", "arm64": "aarch64"}.get(
        platform.machine(), platform.machine()
    )
    suffix = {"Linux": "unknown-linux-gnu", "Darwin": "apple-darwin"}.get(
        platform.system()
    )
    if not suffix:
        raise ValueError("standalone installation supports only Linux and macOS")
    return f"{machine}-{suffix}"


def layout(home):
    home = home.resolve(strict=True)
    if platform.system() == "Darwin":
        state = home / "Library/Application Support/ai.hiroute.cli"
        runtime = state / "run"
    else:
        state_value = os.environ.get("XDG_STATE_HOME")
        runtime_value = os.environ.get("XDG_RUNTIME_DIR")
        state_base = Path(state_value) if state_value and Path(state_value).is_absolute() else home / ".local/state"
        state = state_base / "hiroute"
        runtime = Path(runtime_value) if runtime_value and Path(runtime_value).is_absolute() else state / "run"
    data = home / ".local/share/hiroute"
    return {
        "home": home,
        "bin": home / ".local/bin",
        "lib": home / ".local/lib/hiroute",
        "data": data,
        "state": state,
        "runtime": runtime,
        "marker": data / "standalone.json",
    }


class IncompleteDownload(OSError):
    pass


def https_response(url, headers):
    request = urllib.request.Request(url, headers={"User-Agent": "HiRoute-Installer/1", **headers})
    try:
        response = urllib.request.urlopen(request, timeout=30)
    except urllib.error.HTTPError:
        raise
    except (urllib.error.URLError, OSError) as error:
        raise IncompleteDownload("HTTPS connection interrupted") from error
    final = urllib.parse.urlparse(response.geturl())
    if final.scheme != "https" or not final.netloc or final.username or final.password:
        response.close()
        raise ValueError("download redirect left HTTPS")
    return response


def https_chunks(response, limit):
    total = 0
    while True:
        try:
            chunk = response.read(1024 * 1024)
        except (http.client.HTTPException, OSError) as error:
            raise IncompleteDownload("HTTPS transfer interrupted") from error
        if not chunk:
            break
        total += len(chunk)
        if total > limit:
            raise ValueError("download exceeds the bounded size")
        yield chunk
    declared = response.headers.get("Content-Length")
    if declared is not None and total != int(declared):
        raise IncompleteDownload("HTTPS transfer ended before Content-Length")


def fetch_whole_https(url, destination, limit, expected_size=None):
    for attempt in range(DOWNLOAD_ATTEMPTS):
        try:
            with https_response(url, {}) as response, destination.open("wb") as output:
                total = 0
                for chunk in https_chunks(response, limit):
                    output.write(chunk)
                    total += len(chunk)
                if expected_size is not None and total != expected_size:
                    raise IncompleteDownload("HTTPS transfer ended before the manifest size")
            return
        except IncompleteDownload:
            if attempt + 1 == DOWNLOAD_ATTEMPTS:
                raise


def fetch_https(url, destination, limit, expected_size=None):
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise ValueError("download URL must be credential-free HTTPS")
    if expected_size is None:
        return fetch_whole_https(url, destination, limit)
    if type(expected_size) is not int or not 0 < expected_size <= limit:
        raise ValueError("archive size exceeds the bounded size")
    destination.write_bytes(b"")
    offset, etag = 0, None
    while offset < expected_size:
        end = min(offset + DOWNLOAD_CHUNK_SIZE, expected_size) - 1
        headers = {"Range": f"bytes={offset}-{end}"}
        if etag is not None:
            headers["If-Match"] = etag
        for attempt in range(DOWNLOAD_ATTEMPTS):
            try:
                with https_response(url, headers) as response:
                    if response.status == 200 and offset == 0:
                        response.close()
                        return fetch_whole_https(url, destination, expected_size, expected_size)
                    if (
                        response.status != 206
                        or response.headers.get("Content-Range") != f"bytes {offset}-{end}/{expected_size}"
                    ):
                        raise ValueError("download returned an invalid byte range")
                    identity = response.headers.get("ETag")
                    if etag is not None and identity != etag:
                        raise ValueError("download object identity changed")
                    if offset == 0 and end + 1 < expected_size and (
                        identity is None or not re.fullmatch(r'"[\x21\x23-\x7e\x80-\xff]*"', identity)
                    ):
                        response.close()
                        return fetch_whole_https(url, destination, expected_size, expected_size)
                    value = b"".join(https_chunks(response, end - offset + 1))
                    if len(value) != end - offset + 1:
                        raise IncompleteDownload("HTTPS byte range ended early")
                break
            except IncompleteDownload:
                if attempt + 1 == DOWNLOAD_ATTEMPTS:
                    raise
        with destination.open("ab") as output:
            output.write(value)
        etag, offset = identity, end + 1


def acquire(args, temporary):
    if bool(args.manifest_url) == bool(args.manifest):
        raise ValueError("choose exactly one local manifest or HTTPS manifest URL")
    if args.manifest_url:
        manifest_path = temporary / "manifest.json"
        fetch_https(args.manifest_url, manifest_path, MAX_MANIFEST)
        manifest = load_json_bytes(manifest_path.read_bytes())
        validate_manifest_shape(manifest)
        archive_path = temporary / manifest["archive"]["filename"]
        archive_url = urllib.parse.urljoin(args.manifest_url, manifest["archive"]["filename"])
        fetch_https(archive_url, archive_path, MAX_ARCHIVE, manifest["archive"]["size"])
    else:
        if args.archive is None:
            raise ValueError("a local manifest requires --archive")
        manifest_path = args.manifest.resolve(strict=True)
        archive_path = args.archive.resolve(strict=True)
        if manifest_path.stat().st_size > MAX_MANIFEST or archive_path.stat().st_size > MAX_ARCHIVE:
            raise ValueError("local package exceeds the bounded size")
        manifest = load_json_bytes(manifest_path.read_bytes())
    return manifest, archive_path


def validate_manifest_shape(manifest):
    if not isinstance(manifest, dict):
        raise ValueError("package manifest is invalid for this host")
    expected_top = {
        "schema", "version", "revision", "target", "archive", "files", "cpa",
        "distribution",
    }
    archive = manifest.get("archive")
    files = manifest.get("files")
    cpa = manifest.get("cpa")
    if (
        set(manifest) != expected_top
        or manifest.get("schema") != PACKAGE_SCHEMA
        or manifest.get("target") != host_target()
        or not isinstance(manifest.get("version"), str)
        or len(manifest["version"]) > 64
        or not PORTABLE.fullmatch(manifest["version"])
        or not isinstance(manifest.get("revision"), str)
        or len(manifest["revision"]) > 128
        or not PORTABLE.fullmatch(manifest["revision"])
        or manifest.get("distribution") != "integration-candidate"
        or not isinstance(archive, dict)
        or set(archive) != {"filename", "sha256", "size"}
        or not isinstance(archive.get("filename"), str)
        or not archive["filename"]
        or len(archive["filename"]) > 240
        or archive["filename"] in {".", ".."}
        or Path(archive["filename"]).name != archive["filename"]
        or not archive["filename"].endswith(".tar.gz")
        or not isinstance(archive.get("sha256"), str)
        or not HEX_SHA256.fullmatch(archive["sha256"])
        or type(archive.get("size")) is not int
        or not 0 < archive["size"] <= MAX_ARCHIVE
        or not isinstance(files, dict)
        or not valid_inventory(files)
        or not isinstance(cpa, dict)
        or set(cpa) != {"version", "sha256"}
        or not isinstance(cpa.get("version"), str)
        or not cpa["version"]
        or len(cpa["version"]) > 128
        or any(ord(value) < 0x20 for value in cpa["version"])
        or not isinstance(cpa.get("sha256"), str)
        or not HEX_SHA256.fullmatch(cpa["sha256"])
    ):
        raise ValueError("package manifest is invalid for this host")
    total = 0
    for name in files:
        facts = files[name]
        if (
            not isinstance(facts, dict)
            or set(facts) != {"sha256", "size", "mode"}
            or not isinstance(facts.get("sha256"), str)
            or not HEX_SHA256.fullmatch(facts["sha256"])
            or type(facts.get("size")) is not int
            or facts["size"] < 0
            or facts.get("mode")
            != ("executable" if name.startswith(("bin/", "libexec/")) else "read_only")
        ):
            raise ValueError("package manifest file inventory is invalid")
        total += facts["size"]
    if total > MAX_ARCHIVE or cpa["sha256"] != files["libexec/cliproxyapi"]["sha256"]:
        raise ValueError("package manifest payload identity is invalid")


def validate_manifest(manifest, archive):
    validate_manifest_shape(manifest)
    if (
        archive.name != manifest["archive"]["filename"]
        or archive.stat().st_size != manifest["archive"]["size"]
        or digest(archive) != manifest["archive"]["sha256"]
    ):
        raise ValueError("archive checksum verification failed")
    return manifest


def valid_inventory(files):
    extra = set(files) - EXPECTED_FILES
    return (EXPECTED_FILES <= set(files) and len(extra) <= 2048
            and (not extra or "console/index.html" in extra)
            and all(len(name) <= 240 and name.startswith("console/")
                    and all(re.fullmatch(r"[A-Za-z0-9_@.-]+", part) and part not in {".", ".."}
                            for part in name.split("/")) for name in extra))


def extract_verified(manifest, archive, staging):
    seen = set()
    inventory = set(manifest["files"])
    with tarfile.open(archive, "r:gz") as bundle:
        for member in bundle:
            if len(seen) >= len(inventory):
                raise ValueError("archive inventory is unsafe or differs from manifest")
            relative = Path(member.name)
            if (
                member.name not in inventory
                or member.name in seen
                or not member.isfile()
                or relative.is_absolute()
                or ".." in relative.parts
                or member.size != manifest["files"][member.name].get("size")
            ):
                raise ValueError("archive inventory is unsafe or differs from manifest")
            stream = bundle.extractfile(member)
            if stream is None:
                raise ValueError("archive entry is unreadable")
            destination = staging / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            value = hashlib.sha256()
            with destination.open("wb") as output:
                remaining = member.size
                while remaining:
                    chunk = stream.read(min(1024 * 1024, remaining))
                    if not chunk:
                        raise ValueError("archive entry ended early")
                    remaining -= len(chunk)
                    value.update(chunk)
                    output.write(chunk)
            if stream.read(1):
                raise ValueError("archive entry exceeds its declared size")
            if value.hexdigest() != manifest["files"][member.name].get("sha256"):
                raise ValueError("archive file checksum verification failed")
            mode = 0o755 if manifest["files"][member.name].get("mode") == "executable" else 0o644
            destination.chmod(mode)
            seen.add(member.name)
    if seen != inventory:
        raise ValueError("archive is incomplete")


def desktop_conflict(paths):
    home = paths["home"]
    apps = [Path("/Applications/HiRoute.app"), home / "Applications/HiRoute.app"]
    desktop_socket = home / "Library/Application Support/ai.hiroute.desktop/run/hiroute/control.sock"
    return any(path.exists() or path.is_symlink() for path in apps) or desktop_socket.exists()


def skill_destinations(home):
    return [
        home / ".agents/skills/hiroute-management",
        home / ".claude/skills/hiroute-management",
    ]


def prepare_skill_parent(home, destination):
    """Create only the known Agent Skill parents and keep the runtime write boundary private."""
    try:
        relative = destination.parent.relative_to(home)
    except ValueError as error:
        raise ValueError("standalone Skill destination escaped HOME") from error
    if relative.parts not in ((".agents", "skills"), (".claude", "skills")):
        raise ValueError("standalone Skill destination is invalid")
    current = home
    for part in relative.parts:
        current = current / part
        if current.exists() or current.is_symlink():
            metadata = current.lstat()
            if current.is_symlink() or not current.is_dir():
                raise ValueError(f"Agent Skill parent is unsafe: {current}")
            continue
        current.mkdir(mode=0o700)
        current.chmod(0o700)


def service_bytes(paths, daemon, cpa, cpa_digest):
    home = paths["home"]
    # Native Agent discovery needs the same conventional user/system install locations
    # after a service-manager start. Do not inherit shell functions or an ambient PATH.
    native_path = f"{home}/.local/bin:/usr/local/bin:/usr/bin:/bin"
    if platform.system() == "Linux":
        quote = lambda value: '"' + str(value).replace("\\", "\\\\").replace('"', '\\"') + '"'
        text = (
            f"# {MANAGED_TEXT}\n"
            "[Unit]\nDescription=HiRoute standalone user service\nAfter=network.target\n\n"
            "[Service]\nType=simple\n"
            f"ExecStart={quote(daemon)} --role all --standalone --cpa-binary {quote(cpa)} --cpa-sha256 {cpa_digest}\n"
            "Restart=on-failure\nRestartSec=2\nTimeoutStopSec=45\n"
            f"Environment=HOME={quote(home)}\nEnvironment={quote('PATH=' + native_path)}\n"
            f"Environment={quote('XDG_STATE_HOME=' + str(paths['state'].parent))}\n"
            f"Environment={quote('XDG_RUNTIME_DIR=' + str(paths['runtime']))}\n"
            "NoNewPrivileges=true\nPrivateTmp=true\n\n[Install]\nWantedBy=default.target\n"
        )
        path = home / ".config/systemd/user/ai.hiroute.cli.service"
        return path, text.encode()
    logs = paths["state"] / "logs"
    value = {
        "Label": SERVICE_LABEL,
        "ProgramArguments": [
            str(daemon), "--role", "all", "--standalone", "--cpa-binary", str(cpa),
            "--cpa-sha256", cpa_digest,
        ],
        "RunAtLoad": False,
        "KeepAlive": {"SuccessfulExit": False},
        "ProcessType": "Background",
        "EnvironmentVariables": {"HOME": str(home), "PATH": native_path},
        "StandardOutPath": str(logs / "hirouted.log"),
        "StandardErrorPath": str(logs / "hirouted-error.log"),
    }
    return paths["data"] / "service/ai.hiroute.cli.plist", plistlib.dumps(value, sort_keys=True)


def prepare_service_directory(home):
    """Make the fixed proxy snapshot path safe before any recursive mkdir uses umask."""
    metadata = home.lstat()
    if not stat.S_ISDIR(metadata.st_mode):
        raise ValueError("standalone service HOME is unsafe")
    current = home
    for part in (".local", "share", "hiroute", "service"):
        current = current / part
        if current.exists() or current.is_symlink():
            metadata = current.lstat()
            if not stat.S_ISDIR(metadata.st_mode):
                raise ValueError(f"standalone service parent is unsafe: {current}")
        else:
            current.mkdir(mode=0o700)


def prepare_state_directory(paths):
    """Create Linux state parents privately; preserve safe existing ancestors."""
    if platform.system() != "Linux":
        return
    state = paths["state"]
    if not state.is_absolute() or ".." in state.parts:
        raise ValueError("standalone state parent is unsafe")
    current = Path(state.anchor)
    for part in state.parts[1:]:
        current = current / part
        if current.exists() or current.is_symlink():
            metadata = current.lstat()
            if not stat.S_ISDIR(metadata.st_mode):
                raise ValueError(f"standalone state parent is unsafe: {current}")
        else:
            current.mkdir(mode=0o700)


def current_owned_link(path, owned_root, expected_name):
    if not path.is_symlink():
        return False
    target = Path(os.readlink(path))
    if not target.is_absolute():
        target = path.parent / target
    try:
        target.relative_to(owned_root)
    except ValueError:
        return False
    return target.name == expected_name


def validate_marker(paths, marker):
    if not isinstance(marker, dict):
        raise ValueError("standalone ownership marker is invalid")
    expected_keys = {
        "schema_version", "version", "target", "install_root", "service_definition",
        "installed_skills", "cpa_binary", "cpa_sha256",
    }
    version = marker.get("version")
    installed_skills = marker.get("installed_skills")
    if (
        set(marker) != expected_keys
        or marker.get("schema_version") != MARKER_SCHEMA
        or marker.get("target") != host_target()
        or not isinstance(version, str)
        or len(version) > 64
        or not PORTABLE.fullmatch(version)
        or not isinstance(installed_skills, list)
        or any(not isinstance(value, str) for value in installed_skills)
        or len(installed_skills) != len(set(installed_skills))
        or not HEX_SHA256.fullmatch(marker.get("cpa_sha256", ""))
    ):
        raise ValueError("standalone ownership marker is invalid")
    install_root = paths["lib"] / version
    resource_root = paths["data"] / version
    expected_service = (
        paths["home"] / ".config/systemd/user/ai.hiroute.cli.service"
        if platform.system() == "Linux"
        else paths["data"] / "service/ai.hiroute.cli.plist"
    )
    allowed_skills = {
        str(paths["home"] / ".agents/skills/hiroute-management"),
        str(paths["home"] / ".claude/skills/hiroute-management"),
    }
    if (
        marker.get("install_root") != str(install_root)
        or marker.get("service_definition") != str(expected_service)
        or marker.get("cpa_binary") != str(resource_root / "libexec/cliproxyapi")
        or set(installed_skills) != allowed_skills
    ):
        raise ValueError("standalone ownership marker paths are invalid")
    return install_root, resource_root, expected_service


def read_marker(paths):
    marker_path = paths["marker"]
    if marker_path.is_symlink() or not marker_path.is_file() or marker_path.stat().st_size > MAX_MANIFEST:
        raise ValueError("standalone ownership marker is invalid")
    marker = load_json_bytes(marker_path.read_bytes())
    validate_marker(paths, marker)
    return marker


def staged_file(path, content, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        temporary.chmod(mode)
        return temporary
    except BaseException:
        if temporary.exists() or temporary.is_symlink():
            temporary.unlink()
        raise


def staged_link(path, target):
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", dir=path.parent
    )
    os.close(descriptor)
    temporary = Path(temporary_name)
    temporary.unlink()
    temporary.symlink_to(target)
    return temporary


def remove_path(path):
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)


@contextmanager
def installation_lock(paths):
    identity = hashlib.sha256(str(paths["home"]).encode()).hexdigest()[:16]
    lock_path = Path(tempfile.gettempdir()) / (
        f"hiroute-standalone-{os.geteuid()}-{identity}.lock"
    )
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(lock_path, flags, 0o600)
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError("standalone install lock is unsafe")
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError("another standalone install or uninstall is running") from error
        try:
            yield
        finally:
            fcntl.flock(descriptor, fcntl.LOCK_UN)
    finally:
        os.close(descriptor)


def replace_components(components):
    """Replace exact component leaves and restore prior values on ordinary failure."""
    applied = []
    cleanup_backups = True
    try:
        for staged, destination in components:
            destination.parent.mkdir(parents=True, exist_ok=True)
            backup_root = Path(
                tempfile.mkdtemp(prefix=".hiroute-update-backup.", dir=destination.parent)
            )
            backup = backup_root / "previous"
            had_previous = destination.exists() or destination.is_symlink()
            if had_previous:
                os.replace(destination, backup)
            applied.append((destination, backup if had_previous else None, backup_root))
            if staged is not None:
                os.replace(staged, destination)
    except BaseException:
        rollback_error = None
        for destination, backup, _backup_root in reversed(applied):
            try:
                if destination.exists() or destination.is_symlink():
                    remove_path(destination)
                if backup is not None and (backup.exists() or backup.is_symlink()):
                    os.replace(backup, destination)
            except BaseException as error:
                rollback_error = rollback_error or error
        if rollback_error is not None:
            cleanup_backups = False
            raise OSError("standalone update failed and rollback was incomplete") from rollback_error
        raise
    finally:
        if cleanup_backups:
            for _destination, _backup, backup_root in reversed(applied):
                shutil.rmtree(backup_root, ignore_errors=True)


def console_desired(args):
    """Validate explicit Linux exposure before acquiring the install write lock."""
    mode = getattr(args, "console_mode", None)
    listen = getattr(args, "console_listen", None)
    port = getattr(args, "console_port", None)
    advertise = getattr(args, "console_advertise_address", None)
    plaintext = getattr(args, "allow_plaintext_network", False)
    if all(value is None for value in (mode, listen, port, advertise)) and not plaintext:
        return None
    if platform.system() != "Linux":
        raise ValueError("console installation options are supported only on Linux")
    def address(value):
        if value is None:
            return None
        if "%" in value:
            raise ValueError("console addresses must be IP literals without a scope identifier")
        return ipaddress.ip_address(value)
    ip, advertised = address(listen), address(advertise)
    mode = mode or ("network" if ip and not ip.is_loopback else "loopback")
    if port is not None and not 1 <= port <= 65535:
        raise ValueError("console port must be between 1 and 65535")
    if mode == "network":
        if not plaintext:
            raise ValueError("network console requires --allow-plaintext-network")
        if ip is None or ip.is_loopback or ip.is_multicast:
            raise ValueError("network console requires a non-loopback --console-listen IP")
        advertised = advertised or ip
        if (advertised.is_unspecified or advertised.is_loopback or advertised.is_multicast
                or str(advertised) == "255.255.255.255" or advertised.version != ip.version):
            raise ValueError("network console requires a concrete --console-advertise-address of the same address family")
    elif mode not in ("off", "loopback") or (ip is not None and str(ip) != "127.0.0.1") or advertised:
        raise ValueError("off/loopback console accepts only 127.0.0.1 and no advertised address")
    desired = {"mode": mode, "port_mode": "explicit" if port is not None else "automatic", "allow_plaintext_network": plaintext}
    if ip is not None:
        desired["address"] = str(ip)
    if advertise is not None:
        desired["advertise_address"] = str(advertised)
    if port is not None:
        desired["port"] = port
    return desired


def console_component(paths, desired):
    if desired is None:
        return None
    destination = paths["state"] / "console-listener.json"
    config = {"schema": "hiroute.console-listener/v1", "applied_address": None, "failure_code": None}
    try:
        descriptor = os.open(destination, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except FileNotFoundError:
        pass
    else:
        with os.fdopen(descriptor, "rb") as source:
            metadata = os.fstat(source.fileno())
            if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid()
                    or metadata.st_mode & 0o077 or metadata.st_nlink != 1):
                raise ValueError("existing console configuration is not a private single-link file")
            content = source.read(16 * 1024 + 1)
        if len(content) > 16 * 1024:
            raise ValueError("existing console configuration exceeds its size bound")
        config = load_json_bytes(content)
        if (not isinstance(config, dict) or config.get("schema") != "hiroute.console-listener/v1"
                or set(config) - {"schema", "desired", "applied_address", "failure_code"}):
            raise ValueError("existing console configuration has an unsupported schema")
    config["desired"] = desired
    return staged_file(destination, (json.dumps(config, sort_keys=True) + "\n").encode(), 0o600), destination


def candidate_preflight(program, storage, package):
    # Only our verified, short-lived helper may be killed. Never buffer arbitrary
    # child diagnostics or start a business service for a provisional check.
    try:
        child = subprocess.Popen([str(program), "--upgrade-preflight", "--storage-root", str(storage), "--recovery-package", str(package)], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        try:
            code = child.wait(timeout=15)
            body = child.stdout.read(16 * 1024 + 1)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            child.stdout.close()
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError("candidate upgrade preflight is unavailable; the installed program was not replaced. Preserve data and use the verified transition route or startup diagnostics") from error
    try:
        report = load_json_bytes(body)
    except (ValueError, UnicodeError) as error:
        raise ValueError("candidate cannot perform the required upgrade preflight; use the supported transition release") from error
    fields = {"schema", "history_cutover_required", "required_bytes", "available_bytes", "recovery"}
    if (len(body) > 16 * 1024 or not isinstance(report, dict) or set(report) != fields
            or report["schema"] != "hiroute.upgrade-preflight/v1"
            or type(report["history_cutover_required"]) is not bool
            or any(type(report[name]) is not int or not 0 <= report[name] <= 2**64 - 1 for name in ("required_bytes", "available_bytes"))):
        raise ValueError("candidate upgrade preflight is invalid; the installed program was not replaced. Preserve data and use the verified transition route")
    if code or report["recovery"] is not None:
        recovery = report["recovery"]
        cause = recovery.get("cause") if isinstance(recovery, dict) else "unknown_state"
        allowed = {"missing_package", "invalid_package", "insufficient_space", "access_denied", "locked", "missing_key", "unsupported_format", "namespace_unavailable", "migration_failed", "unknown_state"}
        raise ValueError("upgrade preflight refused: " + (cause if cause in allowed else "unknown_state") + "; the installed program was not replaced. Preserve data and old packages, correct the prerequisite before retrying")
    return report


@contextmanager
def retain_recovery_package(paths, current, target_daemon):
    """Preserve the complete installed program/resources before replacement.

    This handoff goes to the daemon through its private state directory, so
    systemd/launchd does not depend on inheriting the invoking shell's environment.
    """
    if current is None:
        yield None
        return
    old_program, old_resources, _service = validate_marker(paths, current)
    # This provisional call determines cutover before allocating any archive.
    # A cutover repeats preflight with the complete retained program/resources.
    report = candidate_preflight(target_daemon, paths["state"] / "storage", old_program)
    if not report["history_cutover_required"]:
        yield None
        return
    def footprint(path):
        metadata = path.lstat()
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError("recovery package contains an unsafe entry")
        if path.is_dir():
            return sum(footprint(entry) for entry in path.iterdir())
        return metadata.st_size
    required = footprint(old_program) + footprint(old_resources) + 256 * 1024 * 1024
    if shutil.disk_usage(paths["state"]).free < required:
        raise ValueError("Insufficient space on the service-state volume for the matching old package; free unrelated space before retrying")
    parent = paths["state"] / "upgrade-packages"
    parent.mkdir(mode=0o700, exist_ok=True)
    if parent.is_symlink() or not parent.is_dir():
        raise ValueError("unsafe recovery package directory")
    retained = Path(tempfile.mkdtemp(prefix="old-package-", dir=parent))
    retained_identity = retained.stat()
    handoff = None
    try:
        files = {}
        def copy_verified_tree(source, target, prefix):
            source_fd = os.open(source, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                target.mkdir(mode=0o700)
                for name in os.listdir(source_fd):
                    metadata = os.stat(name, dir_fd=source_fd, follow_symlinks=False)
                    relative = prefix + "/" + name
                    if stat.S_ISDIR(metadata.st_mode):
                        copy_verified_tree(source / name, target / name, relative)
                    elif stat.S_ISREG(metadata.st_mode):
                        fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW, dir_fd=source_fd)
                        try:
                            before = os.fstat(fd)
                            if (before.st_dev, before.st_ino) != (metadata.st_dev, metadata.st_ino):
                                raise ValueError("old package changed before preservation")
                            hashed = hashlib.sha256()
                            with os.fdopen(os.open(target / name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600), "wb") as output:
                                remaining = before.st_size
                                while remaining:
                                    chunk = os.read(fd, min(remaining, 64 * 1024))
                                    if not chunk:
                                        raise ValueError("old package was truncated during preservation")
                                    output.write(chunk)
                                    hashed.update(chunk)
                                    remaining -= len(chunk)
                                if os.read(fd, 1):
                                    raise ValueError("old package grew during preservation")
                                after = os.fstat(fd)
                                if (before.st_size, before.st_mtime_ns, before.st_mode) != (after.st_size, after.st_mtime_ns, after.st_mode):
                                    raise ValueError("old package changed during preservation")
                                os.fchmod(output.fileno(), stat.S_IMODE(before.st_mode))
                                output.flush()
                                os.fsync(output.fileno())
                            files[relative] = hashed.hexdigest()
                        finally:
                            os.close(fd)
                    else:
                        raise ValueError("old recovery package contains a link or special entry")
                    if len(files) > 6000:
                        raise ValueError("old recovery package has too many entries")
            finally:
                os.close(source_fd)
        for source, name in ((old_program, "program"), (old_resources, "resources")):
            copy_verified_tree(source, retained / name, name)
        locations = retained / "restore-locations.json"
        metadata = retained / "installation"
        metadata.mkdir(mode=0o700)
        restore_files = {}
        for source, name in ((paths["marker"], "standalone.json"), (_service, "service-definition")):
            if source.exists():
                target = metadata / name
                shutil.copy2(source, target)
                files["installation/" + name] = digest(target)
                restore_files["installation/" + name] = str(source)
        links = {str(paths["bin"] / name): str(old_program / name) for name in ("hiroute", "hirouted")}
        locations.write_text(json.dumps({"schema": "hiroute.standalone-program-restore/v1", "program": str(old_program), "resources": str(old_resources), "files": restore_files, "links": links}, sort_keys=True) + "\n")
        locations.chmod(0o600)
        files["restore-locations.json"] = digest(locations)
        record = {"schema": "hiroute.retained-recovery-package/v1", "storage_root": str(paths["state"] / "storage"), "package_root": str(retained), "target_daemon_sha256": digest(target_daemon), "files": files}
        candidate_preflight(target_daemon, paths["state"] / "storage", retained)
        handoff = staged_file(paths["state"] / "upgrade-package.json", (json.dumps(record, sort_keys=True) + "\n").encode(), 0o600)
        yield handoff
    finally:
        # A moved handoff may already have been consumed by a daemon or batch.
        # Retain it even after later transaction rollback. Delete only this
        # invocation's still-unpublished copy, never previous recovery material.
        if handoff is None or handoff.exists():
            metadata = retained.lstat()
            if (metadata.st_dev, metadata.st_ino) == (retained_identity.st_dev, retained_identity.st_ino):
                remove_path(retained)


def install(args):
    home = Path(os.environ.get("HOME", ""))
    if not home.is_absolute() or not home.is_dir():
        raise ValueError("HOME must identify the current user's absolute home directory")
    desired_console = console_desired(args)
    paths = layout(home)
    with installation_lock(paths), tempfile.TemporaryDirectory(
        prefix="hiroute-install-verify-"
    ) as temporary_name:
        temporary = Path(temporary_name)
        manifest, archive = acquire(args, temporary)
        validate_manifest(manifest, archive)
        staging = temporary / "payload"
        extract_verified(manifest, archive, staging)
        if desktop_conflict(paths):
            raise ValueError("HiRoute Desktop conflicts with standalone installation")
        version = manifest["version"]
        current = None
        if paths["marker"].exists() or paths["marker"].is_symlink():
            try:
                current = read_marker(paths)
            except (OSError, ValueError, KeyError, json.JSONDecodeError):
                # The marker itself is one fixed installer component. Do not trust invalid
                # path data from it, but allow the verified candidate to repair that leaf.
                current = None
        version_root = paths["lib"] / version
        resource_root = paths["data"] / version
        daemon = version_root / "hirouted"
        cpa = resource_root / "libexec/cliproxyapi"
        service_path, service_content = service_bytes(
            paths, daemon, cpa, manifest["cpa"]["sha256"]
        )
        skills = skill_destinations(paths["home"])

        version_parent = version_root.parent
        resource_parent = resource_root.parent
        prepare_service_directory(paths["home"])
        prepare_state_directory(paths)
        version_parent.mkdir(parents=True, exist_ok=True)
        resource_parent.mkdir(parents=True, exist_ok=True)
        staged_bin = Path(tempfile.mkdtemp(prefix=f".{version}.", dir=version_parent))
        staged_data = Path(tempfile.mkdtemp(prefix=f".{version}.", dir=resource_parent))
        staged_components = []
        try:
            configured_console = console_component(paths, desired_console)
            if configured_console is not None:
                staged_components.append(configured_console)
            shutil.copy2(staging / "bin/hiroute", staged_bin / "hiroute")
            shutil.copy2(staging / "bin/hirouted", staged_bin / "hirouted")
            if (staging / "console").is_dir():
                shutil.copytree(staging / "console", staged_bin / "console")
            for relative in ("libexec", "licenses", "docs", "skills"):
                shutil.copytree(staging / relative, staged_data / relative)
            staged_components.extend(
                [(staged_bin, version_root), (staged_data, resource_root)]
            )

            paths["state"].mkdir(parents=True, exist_ok=True, mode=0o700)
            paths["runtime"].mkdir(parents=True, exist_ok=True, mode=0o700)
            if platform.system() == "Darwin":
                (paths["state"] / "logs").mkdir(parents=True, exist_ok=True, mode=0o700)

            staged_components.append(
                (staged_file(service_path, service_content, 0o600), service_path)
            )
            for destination in skills:
                prepare_skill_parent(paths["home"], destination)
                temporary_skill = Path(
                    tempfile.mkdtemp(
                        prefix=".hiroute-management.", dir=destination.parent
                    )
                )
                shutil.copy2(
                    staging / "skills/hiroute-management/SKILL.md",
                    temporary_skill / "SKILL.md",
                )
                staged_components.append((temporary_skill, destination))
            for name in ("hiroute", "hirouted"):
                entry = paths["bin"] / name
                staged_components.append(
                    (staged_link(entry, version_root / name), entry)
                )

            with retain_recovery_package(paths, current, staged_bin / "hirouted") as handoff:
                if handoff is not None:
                    staged_components.insert(0, (handoff, paths["state"] / "upgrade-package.json"))
                if current is not None:
                    old_install_root, old_resource_root, _old_service = validate_marker(
                        paths, current
                    )
                    if old_install_root != version_root:
                        staged_components.append((None, old_install_root))
                    if old_resource_root != resource_root:
                        staged_components.append((None, old_resource_root))

                marker = {
                    "schema_version": MARKER_SCHEMA,
                    "version": version,
                    "target": manifest["target"],
                    "install_root": str(version_root),
                    "service_definition": str(service_path),
                    "installed_skills": [str(path) for path in skills],
                    "cpa_binary": str(cpa),
                    "cpa_sha256": manifest["cpa"]["sha256"],
                }
                marker_content = (json.dumps(marker, indent=2, sort_keys=True) + "\n").encode()
                staged_components.append(
                    (staged_file(paths["marker"], marker_content, 0o600), paths["marker"])
                )
                replace_components(staged_components)
        finally:
            for staged_component, _destination in staged_components:
                if staged_component is not None and (
                    staged_component.exists() or staged_component.is_symlink()
                ):
                    remove_path(staged_component)
            if staged_bin.exists() or staged_bin.is_symlink():
                remove_path(staged_bin)
            if staged_data.exists() or staged_data.is_symlink():
                remove_path(staged_data)
    run_systemctl("daemon-reload")
    print(json.dumps({"installed": True, "version": manifest["version"], "marker": str(paths["marker"])}, sort_keys=True))


def owned_skill(path, expected):
    return (
        path.is_dir()
        and not path.is_symlink()
        and (path / "SKILL.md").is_file()
        and not (path / "SKILL.md").is_symlink()
        and expected.is_file()
        and (path / "SKILL.md").read_bytes() == expected.read_bytes()
    )


def uninstall(_args):
    home = Path(os.environ.get("HOME", ""))
    if not home.is_absolute() or not home.is_dir():
        raise ValueError("HOME must identify an absolute home directory")
    paths = layout(home)
    with installation_lock(paths):
        uninstall_owned(paths)


def uninstall_owned(paths):
    home = paths["home"]
    marker = read_marker(paths)
    install_root, resource_root, service = validate_marker(paths, marker)
    _, expected_service = service_bytes(
        paths,
        install_root / "hirouted",
        resource_root / "libexec/cliproxyapi",
        marker["cpa_sha256"],
    )

    # Resolve every owned target before the first external effect or deletion. A partially
    # modified installation is evidence to stop, not permission to remove nearby user files.
    for name in ("hiroute", "hirouted"):
        entry = paths["bin"] / name
        if (entry.exists() or entry.is_symlink()) and not current_owned_link(
            entry, paths["lib"], name
        ):
            raise ValueError(f"owned entry changed; refusing to remove: {entry}")
    expected_skill = resource_root / "skills/hiroute-management/SKILL.md"
    for value in marker["installed_skills"]:
        skill = Path(value)
        if (skill.exists() or skill.is_symlink()) and not owned_skill(skill, expected_skill):
            raise ValueError(f"owned Skill changed; refusing to remove: {skill}")
    if service.exists() or service.is_symlink():
        if service.is_symlink() or not service.is_file() or service.read_bytes() != expected_service:
            raise ValueError("owned service definition changed; refusing to remove")
    for directory, parent in (
        (install_root, paths["lib"]),
        (resource_root, paths["data"]),
    ):
        if directory.exists() or directory.is_symlink():
            if directory.is_symlink() or not directory.is_dir() or directory.parent != parent:
                raise ValueError(f"owned installation directory changed: {directory}")

    if platform.system() == "Linux":
        run_systemctl("disable", "--now", SERVICE_LABEL)
    else:
        domain = f"gui/{os.getuid()}/{SERVICE_LABEL}"
        subprocess.run(["/bin/launchctl", "bootout", domain], check=False)
        login = home / "Library/LaunchAgents/ai.hiroute.cli.plist"
        if login.is_symlink() and Path(os.readlink(login)) == service:
            login.unlink()
    for name in ("hiroute", "hirouted"):
        entry = paths["bin"] / name
        if entry.exists() or entry.is_symlink():
            entry.unlink()
    for value in marker["installed_skills"]:
        skill = Path(value)
        if skill.exists():
            shutil.rmtree(skill)
    if service.exists() or service.is_symlink():
        service.unlink()
    if install_root.is_dir():
        shutil.rmtree(install_root)
    if resource_root.is_dir():
        shutil.rmtree(resource_root)
    paths["marker"].unlink()
    run_systemctl("daemon-reload")
    print(json.dumps({"installed": False, "data_preserved": True}, sort_keys=True))


def parser():
    result = argparse.ArgumentParser()
    sub = result.add_subparsers(dest="command", required=True)
    add = sub.add_parser("install")
    add.add_argument("--manifest", type=Path)
    add.add_argument("--archive", type=Path)
    add.add_argument("--manifest-url")
    add.add_argument("--console-mode", choices=("off", "loopback", "network"))
    add.add_argument("--console-listen", help="console bind IP; defaults to loopback when enabled")
    add.add_argument("--console-port", type=int, help="explicit console port (otherwise prefer 5838, then a free port)")
    add.add_argument("--console-advertise-address", help="reachable IP for a wildcard network bind")
    add.add_argument("--allow-plaintext-network", action="store_true")
    sub.add_parser("uninstall")
    return result


def main():
    args = parser().parse_args()
    try:
        install(args) if args.command == "install" else uninstall(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError, tarfile.TarError) as error:
        print(f"install failed: {error}", file=sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
