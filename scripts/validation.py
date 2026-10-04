#!/usr/bin/env python3
"""Route build/test tools using ~/.config/hiroute/validation.json."""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import sys

CONFIG = Path.home() / ".config/hiroute/validation.json"
LEGACY = Path.home() / ".config/hiroute/remote-rust.json"
REPO = Path(__file__).resolve().parent.parent
TARGETS = ("backend", "desktop", "frontend")


def ssh_host(value):
    if not isinstance(value, str) or not value or value.startswith("-") or any(c.isspace() for c in value):
        raise ValueError("host must be an SSH host/alias, not options or a command")
    return value


def configuration(path=CONFIG, platform=None, legacy=LEGACY):
    platform = platform or sys.platform
    defaults = {"backend": {"transport": "local"}, "frontend": {"transport": "local"}}
    if platform == "darwin":
        defaults["desktop"] = {"transport": "local"}
    configured = {}
    if path.exists():
        data = json.loads(path.read_text())
        if (not isinstance(data, dict) or set(data) - {"version", "targets"}
                or type(data.get("version")) is not int or data["version"] != 1
                or not isinstance(data.get("targets"), dict)):
            raise ValueError("Expected validation.json version 1 and targets object")
        if set(data["targets"]) - set(TARGETS):
            raise ValueError("Unknown validation target")
        configured = data["targets"]
    if platform == "darwin" and "backend" not in configured and legacy.exists():
        defaults["backend"] = {"transport": "ssh", "host": ssh_host(json.loads(legacy.read_text())["host"])}
    defaults.update(configured)
    for name, target in defaults.items():
        if not isinstance(target, dict) or set(target) - {"transport", "host", "repo", "path", "jobs", "pilot_cli"}:
            raise ValueError("Unknown fields in target " + name)
        if target.get("transport") not in ("local", "ssh"):
            raise ValueError("Expected transport local/ssh for " + name)
        if target["transport"] == "ssh":
            ssh_host(target.get("host"))
            if name != "backend" and not target.get("repo"):
                raise ValueError("SSH " + name + " requires repo on that host")
        elif "host" in target:
            raise ValueError("Local target must not specify host")
        if "jobs" in target and (type(target["jobs"]) is not int or target["jobs"] < 1):
            raise ValueError("jobs must be a positive integer")
        for key in ("repo", "pilot_cli"):
            if key in target and (not isinstance(target[key], str) or not target[key] or "\0" in target[key]):
                raise ValueError(key + " must be a nonempty path")
        if "path" in target and (not isinstance(target["path"], list) or any(
                not isinstance(p, str) or not p or "\0" in p or ":" in p for p in target["path"])):
            raise ValueError("path must be a list of directories")
        if name == "backend" and target["transport"] == "ssh" and set(target) - {"transport", "host"}:
            raise ValueError("SSH backend uses the existing workbench; configure only transport and host")
        if name == "frontend" and set(target) & {"jobs", "pilot_cli"}:
            raise ValueError("frontend does not use jobs/pilot_cli")
    return defaults


def with_candidate(arguments, repo):
    arguments = list(arguments)
    if arguments[:1] in (["run"], ["acquire"]):
        boundary = arguments.index("--") if "--" in arguments else len(arguments)
        options = arguments[:boundary]
        if not any(a == "--sha" or a.startswith("--sha=") for a in options):
            sha = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
            arguments[1:1] = ["--sha", sha]
    return arguments


def dispatch(targets, action, arguments, frontend_dist=None, dry_run=False):
    name = "desktop" if action in ("pilot", "pilot-cli", "pilot-build") else action
    if action == "doctor":
        if len(arguments) != 1 or arguments[0] not in TARGETS:
            raise ValueError("doctor requires backend, desktop or frontend")
        name = arguments[0]
        arguments = []
    if name not in targets:
        raise ValueError("Configure a " + name + " target in ~/.config/hiroute/validation.json")
    target = targets[name]
    if frontend_dist and not (action == "desktop" and arguments[:1] == ["run"]):
        raise ValueError("--frontend-dist is only for desktop run")
    if action in ("backend", "desktop", "pilot-build"):
        arguments = with_candidate(arguments, REPO)
    if name == "backend" and target["transport"] == "ssh":
        if action == "doctor":
            # This is read-only; no worker submission or sccache server startup.
            payload = dict(target=target, action="doctor", name=name, arguments=[], repo="~/git/HiRoute")
        else:
            argv = [sys.executable, str(REPO / "scripts/remote-rust.py"), "--host", target["host"], *arguments]
            if dry_run:
                print(json.dumps({"target": target, "argv": argv}, indent=2))
                return 0
            return subprocess.call(argv, cwd=REPO)
    else:
        repo = target.get("repo", str(REPO))
        payload = dict(target=target, action=action, name=name, arguments=arguments, repo=repo,
                       frontend_dist=frontend_dist)
    executor = (REPO / "scripts/validation-host.py").read_text()
    if dry_run:
        print(json.dumps(payload, indent=2))
        return 0
    if action in ("pilot", "pilot-build"):
        payload["bundle"] = {name: (REPO / "scripts" / name).read_text() for name in
                             ("pilot-builds.py", "local-rust.py", "remote-rust.py", "desktop-pilot.py", "validation-report.py")}
    if target["transport"] == "local":
        argv = [sys.executable, "-c", executor]
    else:
        argv = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", target["host"],
                "python3 -c " + shlex.quote(executor)]
    return subprocess.run(argv, input=json.dumps(payload), text=True).returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=CONFIG)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--frontend-dist", help="Frontend path on the Desktop execution host")
    parser.add_argument("action", choices=("show", "doctor", *TARGETS, "pilot", "pilot-cli", "pilot-build"))
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.config != CONFIG and not args.config.is_file():
        parser.error("Explicit --config does not exist")
    targets = configuration(args.config)
    if args.action == "show":
        if args.arguments or args.frontend_dist:
            parser.error("show takes no extra arguments")
        print(json.dumps({"config": str(args.config), "targets": targets}, indent=2))
        return 0
    return dispatch(targets, args.action, args.arguments, args.frontend_dist, args.dry_run)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
