#!/usr/bin/env python3
"""Execution-host adapter. Receives a JSON request from validation.py over stdin."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def tooling_bundle(sources):
    names = {"pilot-builds.py", "local-rust.py", "remote-rust.py", "desktop-pilot.py", "validation-report.py"}
    if not isinstance(sources, dict) or set(sources) != names or any(not isinstance(v, str) for v in sources.values()):
        raise ValueError("Incomplete Pilot tooling bundle")
    key = hashlib.sha256(json.dumps(sources, sort_keys=True).encode()).hexdigest()
    root = Path.home() / ".cache/hiroute/validation-tools"
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    def owned(path):
        if path.is_symlink() or not path.is_dir() or path.stat().st_uid != os.geteuid() or path.stat().st_mode & 0o077:
            raise ValueError("Tooling cache must be an owned private directory")
    owned(root)
    destination = root / key
    if not destination.exists():
        temporary = Path(tempfile.mkdtemp(prefix="bundle-", dir=root))
        try:
            for name, source in sources.items():
                (temporary / name).write_text(source)
                (temporary / name).chmod(0o600)
            try:
                temporary.rename(destination)
            except OSError:
                if not destination.is_dir():
                    raise
        finally:
            if temporary.exists():
                shutil.rmtree(temporary)
    owned(destination)
    for name, source in sources.items():
        path = destination / name
        if path.is_symlink() or path.read_text() != source:
            raise ValueError("Immutable tooling bundle changed")
    return destination


def gui_session():
    if sys.platform != "darwin":
        return {"available": False, "reason": "macOS required"}
    result = subprocess.run(["/bin/launchctl", "print", "gui/" + str(os.getuid())],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    owner = subprocess.check_output(["/usr/bin/stat", "-f", "%u", "/dev/console"], text=True).strip()
    return {"available": result.returncode == 0 and owner == str(os.getuid()),
            "domain_available": result.returncode == 0, "console_uid": owner}


def environment(target, stable_path=False):
    env = os.environ.copy()
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    # No login-shell dependence and no caller HOME/environment forwarding.
    paths = [str(Path(p).expanduser()) for p in target.get("path", [])]
    inherited = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] if stable_path and "path" in target else [env.get("PATH", "")]
    env["PATH"] = os.pathsep.join([*paths, *inherited, str(Path.home() / ".cargo/bin")])
    return env


def doctor(payload, repo, env):
    name = payload["name"]
    required = ["git", "python3", "node", "npm"] if name == "frontend" else ["git", "python3", "cargo", "rustc", "sccache", "cmake"]
    if name == "desktop":
        required += ["node", "npm"]
    found = {tool: shutil.which(tool, path=env["PATH"]) for tool in required}
    scripts = {}
    for filename in ("local-rust.py", "remote-rust.py", "desktop-pilot.py"):
        path = repo / "scripts" / filename
        scripts[filename] = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None
    revision = None
    if repo.is_dir():
        result = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], env=env, text=True, capture_output=True)
        if result.returncode == 0:
            revision = result.stdout.strip()
    report = dict(platform=sys.platform, repo=str(repo), revision=revision, tools=found, scripts=scripts,
                  free_gib=shutil.disk_usage(Path.home()).free // 1024**3)
    workbench = name == "backend" and payload["target"]["transport"] == "ssh"
    ready = all(found.values()) and (workbench or bool(revision))
    report["scope"] = "tools_only" if workbench else "repository_and_tools"
    if name == "desktop":
        report["gui"] = gui_session()
        value = payload["target"].get("pilot_cli", "tauri-pilot")
        report["pilot_cli"] = shutil.which(str(Path(value).expanduser()), path=env["PATH"])
        ready = ready and report["gui"]["available"] and bool(revision) and all(scripts.values())
        report["pilot_cli_available"] = report["pilot_cli"] is not None
        report["pilot_cli_version"] = None
        if report["pilot_cli_available"]:
            version = subprocess.run([report["pilot_cli"], "--version"], env=env,
                                     text=True, capture_output=True, timeout=10)
            if version.returncode == 0:
                report["pilot_cli_version"] = version.stdout.strip()
        report["pilot_prerequisites_ready"] = bool(ready and report["pilot_cli_version"] == "tauri-pilot 0.7.3")
    report["ready"] = bool(ready)
    print(json.dumps(report, indent=2))
    return 0 if ready else 2


def run_help(repo, action):
    """Derive wrapper help from the caller's actual runner argument parser."""
    spec = importlib.util.spec_from_file_location("local_rust", repo / "scripts/local-rust.py")
    runner = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runner)
    parser = runner.argument_parser()
    subcommands = next(item for item in parser._actions if isinstance(item, argparse._SubParsersAction))
    run = subcommands.choices["run"]
    run.prog = "validation.py " + action + " run"
    run.description = ("Desktop always fetches the exact pushed candidate from origin, including on a local Mac."
                       if action == "desktop" else
                       "Local backend selects committed source from the configured repository.")
    run.epilog = "Configure repository and compiler jobs in validation.json. Pass Cargo arguments after --."
    for option in run._actions:
        if option.dest in ("repo", "source", "jobs"):
            option.help = argparse.SUPPRESS
        elif option.dest == "sha":
            option.required = False
            option.help = "Exact candidate commit (defaults to the caller's HEAD)"
    run.print_help()
    return 0


def overrides_option(arguments, option):
    # The runner accepts unambiguous argparse long-option abbreviations. Reject
    # every prefix here, including ambiguous ones, so none can bypass ownership.
    return any(value.startswith("--") and option.startswith(value.split("=", 1)[0])
               for value in arguments)


def execute(payload):
    target = payload["target"]
    action = payload["action"]
    name = payload["name"]
    repo = Path(payload["repo"]).expanduser().resolve()
    env = environment(target, stable_path=action == "pilot-build")
    arguments = list(payload["arguments"])
    if action == "doctor":
        return doctor(payload, repo, env)
    if not repo.is_dir():
        raise ValueError("Execution repository does not exist: " + str(repo))
    if name == "desktop" and sys.platform != "darwin":
        raise ValueError("Desktop target requires macOS; refusing a platform substitution")
    if action == "pilot-build":
        if not arguments:
            raise ValueError("pilot-build requires acquire/start/status/finish")
        if arguments[0] == "start" and not gui_session()["available"]:
            raise ValueError("Desktop GUI login session unavailable for the execution user")
        if arguments[0] == "acquire":
            if any(a in ("--repo", "--jobs") or a.startswith(("--repo=", "--jobs=")) for a in arguments):
                raise ValueError("Configure repo/jobs in validation.json")
            arguments[1:1] = ["--repo", str(repo), "--jobs", str(target.get("jobs", 2))]
        bundle = tooling_bundle(payload.get("bundle"))
        argv = [sys.executable, str(bundle / "pilot-builds.py"), *arguments]
    elif action in ("backend", "desktop"):
        if not arguments:
            raise ValueError(action + " requires a local-rust action")
        if arguments[0] == "run":
            boundary = arguments.index("--") if "--" in arguments else len(arguments)
            options = arguments[:boundary]
            if overrides_option(options, "--source"):
                if action == "desktop":
                    raise ValueError("Desktop always fetches the exact pushed candidate from origin, including on a local Mac; omit --source")
                raise ValueError("Local backend selects committed source from the configured repository; omit --source")
            if any(overrides_option(options, option) for option in ("--repo", "--jobs")):
                raise ValueError("Configure repo/jobs in validation.json; omit these command-line overrides")
            additions = ["--repo", str(repo)]
            if action == "backend":
                additions += ["--source", "local"]
                if "--cargo-only" not in options:
                    additions += ["--cargo-only"]
            if "jobs" in target:
                additions += ["--jobs", str(target["jobs"])]
            arguments[1:1] = additions
        script = repo / "scripts/local-rust.py"
        if payload.get("frontend_dist"):
            env["TAURI_CONFIG"] = subprocess.check_output(
                [sys.executable, str(repo / "scripts/desktop-pilot.py"), "config", "--frontend-dist",
                 str(Path(payload["frontend_dist"]).expanduser())], cwd=repo, env=env, text=True).strip()
        argv = [sys.executable, str(script), *arguments]
    elif action == "frontend":
        if arguments not in (["ci"], ["build"], ["build", "--console-assets"], ["test"]):
            raise ValueError("frontend expects ci, build, build --console-assets or test")
        if arguments == ["build", "--console-assets"]:
            spec = importlib.util.spec_from_file_location("console_assets", repo / "scripts/console-assets.py")
            console = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(console)
            return console.build(repo, env)
        argv = ["npm", "--prefix", "apps/desktop", *(arguments if arguments == ["ci"] else ["run", *arguments])]
    elif action == "pilot":
        if not arguments:
            raise ValueError("pilot requires config/start/status/stop")
        if arguments[0] == "start" and not gui_session()["available"]:
            raise ValueError("Desktop GUI login session unavailable for the execution user")
        if arguments[0] == "config":
            if any(a == "--repo" or a.startswith("--repo=") for a in arguments):
                raise ValueError("Configure repo in validation.json")
            arguments[1:1] = ["--repo", str(repo)]
        bundle = tooling_bundle(payload.get("bundle"))
        argv = [sys.executable, str(bundle / "desktop-pilot.py"), *arguments]
    elif action == "pilot-cli":
        if "--socket" not in arguments and not any(a.startswith("--socket=") for a in arguments):
            raise ValueError("pilot-cli requires an explicit execution-host --socket")
        cli = str(Path(target.get("pilot_cli", "tauri-pilot")).expanduser())
        argv = [cli, *arguments]
    else:
        raise ValueError("Unknown host action")
    return subprocess.run(argv, cwd=repo, env=env).returncode


if __name__ == "__main__":
    try:
        sys.exit(execute(json.load(sys.stdin)))
    except (ValueError, KeyError, OSError, subprocess.SubprocessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
