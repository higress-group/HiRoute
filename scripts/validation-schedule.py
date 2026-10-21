#!/usr/bin/env python3
"""Bounded backend scheduling; preparation never substitutes for scenario evidence."""
from concurrent.futures import FIRST_COMPLETED, ThreadPoolExecutor, wait
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import sys
import threading
import time


SMOKE = "default_two_domain_smoke"
CASE_LINE = re.compile(r"^test (\S+)(?: - should panic)? \.\.\. (?:ok|FAILED|ignored)(?:[ \t,].*)?$", re.M)
CASE_START = re.compile(r"\btest (\S+)(?: - should panic)? \.\.\.")


def commands(cargo):
    expected = ["cargo", "test", "--locked", "--workspace", "--exclude", "hiroute-desktop", "--all-features"]
    base = [arg for arg in cargo if arg != "--no-fail-fast"]
    if base != expected or cargo.count("--no-fail-fast") > 1:
        raise ValueError("workspace-smoke requires exactly: " + shlex.join(expected) + " [--no-fail-fast]")
    return {
        "compile": [*cargo, "--no-run"],
        "list": [*cargo, "--", "--list"],
        "prepare": [*cargo, "--test", "smoke_cli", "prepare_default_smoke_builds", "--", "--ignored", "--exact", "--nocapture"],
        "smoke": [*cargo, "--test", "smoke_cli", SMOKE, "--", "--exact"],
        "remainder": [*cargo, "--", "--skip", SMOKE],
    }


def listed_tests(log):
    names = re.findall(r"^(.+): test$", log, re.M)
    counts = re.findall(r"^(\d+) tests?, (\d+) benchmarks?$", log, re.M)
    if (not counts or sum(int(n) for n, _ in counts) != len(names)
            or any(int(b) for _, b in counts)
            or [name for name in names if SMOKE in name] != [SMOKE]):
        raise ValueError("Cannot prove an exact, non-overlapping smoke/remainder test partition")
    return len(names)


def compiled_smoke(log, checkout):
    """Select exactly the full-workspace smoke test binary built by --no-run."""
    paths = re.findall(r"^\s+Executable tests/smoke_cli\.rs \((target/debug/deps/smoke_cli-[0-9a-f]+)\)$", log, re.M)
    if len(paths) != 1:
        raise ValueError("Full workspace build did not emit exactly one smoke test executable")
    target = (Path(checkout) / "target").resolve(strict=True)
    executable = (Path(checkout) / paths[0]).resolve(strict=True)
    try:
        executable.relative_to(target)
    except ValueError:
        raise ValueError("Smoke executable is outside this checkout's default target") from None
    if not executable.is_file() or executable.parent != target / "debug/deps":
        raise ValueError("Smoke executable is outside this checkout's default target")
    return executable


def file_digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def snapshot_copy(source, destination):
    if sys.platform == "linux":
        subprocess.run(["cp", "--reflink=auto", str(source), str(destination)], check=True)
    else:
        shutil.copy2(source, destination)


def preparation_receipt(binding, target, package):
    """Read only the receipt bound by this preparation, never a digest cache search."""
    if (not isinstance(binding, dict) or set(binding) != {"path", "sha256"}
            or not isinstance(binding["path"], str)):
        raise ValueError("Build preparation has no exact receipt binding: " + package)
    path = Path(binding["path"])
    resolved = path.resolve(strict=True)
    build_root = target / "smoke/builds"
    try:
        relative = resolved.relative_to(build_root)
    except ValueError:
        raise ValueError("Prepared receipt escapes this checkout: " + package) from None
    if (not path.is_absolute() or path != resolved or len(relative.parts) != 3
            or not re.fullmatch(r"[0-9a-f]{64}", relative.parts[0])
            or relative.parts[1:] != (package, "receipt.json")):
        raise ValueError("Prepared receipt has an invalid cache path: " + package)
    if "sha256:" + file_digest(resolved) != binding["sha256"]:
        raise ValueError("Prepared receipt changed: " + package)
    return json.loads(resolved.read_text()), resolved.parent


def prepared_binary(value, directory, binary, digest):
    path = Path(value).resolve(strict=True)
    if path != directory / binary:
        raise ValueError("Product artifact escapes selected receipt directory")
    if "sha256:" + file_digest(path) != digest:
        raise ValueError("Prepared product binary changed")
    return path


def prepared_products(log, checkout, sha, jobs):
    """Verify the exact product receipts and their captured Cargo environment."""
    paths = re.findall(r"^build preparation=(\S+)$", log, re.M)
    if len(paths) != 1:
        raise ValueError("Build preparation did not identify exactly one report")
    target = (Path(checkout) / "target").resolve(strict=True)
    if target != Path(checkout).resolve(strict=True) / "target":
        raise ValueError("Prepared target is outside this checkout")
    report = Path(paths[0]).resolve(strict=True)
    if (report.parent.parent != target / "smoke" or report.name != "preparation.json"
            or not report.parent.name.startswith("prepare-")):
        raise ValueError("Build preparation report is outside this checkout")
    data = json.loads(report.read_text())
    if (not isinstance(data, dict)
            or data.get("schema") != "hiroute.smoke.preparation/v1"
            or data.get("source_revision") != sha or data.get("scenarios_executed") != 0):
        raise ValueError("Build preparation report does not match the candidate")
    artifacts = data.get("artifacts")
    if (not isinstance(artifacts, list) or len(artifacts) != 3
            or {artifact["package"] for artifact in artifacts}
            != {"hiroute-e2e", "hiroute-cli", "hiroute-daemon"}):
        raise ValueError("Build preparation has incomplete artifacts")
    receipts = []
    products = {}
    for artifact in artifacts:
        package = artifact["package"]
        receipt, directory = preparation_receipt(artifact.get("receipt"), target, package)
        binary = {"hiroute-e2e": "hiroute-e2e", "hiroute-cli": "hiroute",
                  "hiroute-daemon": "hirouted"}[package]
        facts = {key: value for key, value in artifact.items() if key != "receipt"}
        recipe = receipt["recipe"]
        if (receipt["artifact"] != facts or facts.get("source_revision") != sha
                or facts.get("binary") != binary
                or recipe.get("source_revision") != sha or recipe.get("package") != package
                or recipe.get("binary") != binary):
            raise ValueError("Prepared product receipt does not match the candidate: " + package)
        products[package] = prepared_binary(receipt["path"], directory, binary, artifact["sha256"])
        receipts.append(receipt)
    environments = [item["recipe"]["environment"] for item in receipts]
    if not all(env == environments[0] for env in environments):
        raise ValueError("Build preparation receipts disagree on runtime environment")
    environment = environments[0]
    if (not isinstance(environment, dict)
            or not all(isinstance(k, str) and isinstance(v, str) for k, v in environment.items())
            or environment.get("CARGO_BUILD_JOBS") != str(jobs)
            or environment.get("CARGO_PKG_NAME") != "hiroute-product-e2e"
            or not environment.get("CARGO_BIN_EXE_hiroute-smoke")):
        raise ValueError("Build preparation has invalid Cargo test environment")
    return data, environment, products


def prepared_environment(log, checkout, sha, jobs=8):
    return prepared_products(log, checkout, sha, jobs)[1]


def lane_summary(parser, log):
    """Count terminal Cargo target summaries, excluding nested child output."""
    parsed = parser.parse_log(log)
    modules = parsed["modules"]
    counts = {key: sum((module.get("counts") or {}).get(key, 0) for module in modules)
              for key in ("passed", "failed", "ignored")}
    complete = (bool(modules) and not parsed["unassigned_summaries"]
                and all(module["complete"] and module["result"] == "ok" for module in modules))
    return dict(counts=counts, complete=bool(complete), modules=modules)


def execute(runner, root, run, checkout, request, lease, output):
    selected = commands(request["command"])
    deadline = time.monotonic() + request["timeout"]
    records = []
    outcome = dict(process_exit=1, timed_out=False, phases=records, complete=False)
    smoke_executable = None
    smoke_digest = None
    smoke_environment = None

    def phase(name, exclusive=False):
        argv = ([str(smoke_executable), SMOKE, "--exact"] if name == "smoke"
                else selected[name])
        path = run / ("phase-" + name + ".log")
        record = dict(name=name, command=argv, status="waiting", process_exit=None,
                      submitted_at=time.time(), log=str(path))
        try:
            with runner.capacity(root, exclusive, deadline) as handles, path.open("w") as log:
                env = dict(os.environ, CARGO_BUILD_JOBS=str(
                    runner.EXCLUSIVE_BUILD_JOBS if name == "compile" else runner.SHARED_BUILD_JOBS))
                # Do not permit execution lanes to rebuild a missing smoke artifact.
                if name == "smoke":
                    env.update(smoke_environment)
                    env["HIROUTE_SMOKE_REQUIRE_PREPARED"] = "1"
                    if file_digest(smoke_executable) != smoke_digest:
                        raise ValueError("Compiled smoke test executable changed before execution")
                    log.write("Running tests/smoke_cli.rs (" + str(smoke_executable) + ")\n")
                    log.flush()
                else:
                    env.pop("HIROUTE_SMOKE_REQUIRE_PREPARED", None)
                record.update(started_at=time.time(), status="running", build_jobs=int(env["CARGO_BUILD_JOBS"]))
                cwd = Path(checkout) / "tools/product-e2e" if name == "smoke" else checkout
                process = runner.start_locked_command(argv, cwd, env, log, [*handles, lease])
                try:
                    record["process_exit"] = process.wait(timeout=max(0, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    record["process_exit"] = process.wait()
                    record["timed_out"] = True
                record["status"] = "completed" if record["process_exit"] == 0 else "failed"
        except Exception as error:
            record.update(status="failed", error=str(error))
        record["finished_at"] = time.time()
        runner.save(run / ("phase-" + name + ".json"), record)
        return record

    for name in ("compile", "list", "prepare"):
        record = phase(name, exclusive=True)
        records.append(record)
        if record["process_exit"] != 0:
            outcome["timed_out"] = record.get("timed_out", False)
            outcome["error"] = "Preparation failed: " + name
            return outcome
        if name == "list":
            try:
                outcome["expected_tests"] = listed_tests((run / "phase-list.log").read_text(errors="replace"))
            except ValueError as error:
                outcome["error"] = str(error)
                return outcome
            try:
                smoke_executable = compiled_smoke((run / "phase-compile.log").read_text(errors="replace"), checkout)
                smoke_digest = file_digest(smoke_executable)
            except (ValueError, OSError) as error:
                outcome["error"] = str(error)
                return outcome
        if name == "prepare":
            log = (run / "phase-prepare.log").read_text(errors="replace")
            summary = runner.test_summary(log, selected[name])
            if summary["counts"] != dict(passed=1, failed=0, ignored=0):
                outcome["error"] = "Required build preparation did not execute exactly once"
                return outcome
            try:
                smoke_environment = prepared_environment(log, checkout, request["sha"])
            except (ValueError, OSError, KeyError, TypeError, json.JSONDecodeError) as error:
                outcome["error"] = str(error)
                return outcome

    # The remainder Cargo command may relink its own copy of the test target.
    # Snapshot the already-built executable inside this checkout's default target
    # so Cargo cannot replace it while the two runtime lanes overlap.
    try:
        copied = Path(checkout) / "target/validation-schedule" / run.name / "smoke_cli"
        copied.parent.mkdir(parents=True, mode=0o700, exist_ok=False)
        if file_digest(smoke_executable) != smoke_digest:
            raise ValueError("Compiled smoke test executable changed during preparation")
        shutil.copy2(smoke_executable, copied)
        if file_digest(copied) != smoke_digest:
            raise ValueError("Copied smoke test executable does not match compilation")
        smoke_executable = copied
    except (ValueError, OSError) as error:
        outcome["error"] = str(error)
        return outcome

    # Both lanes are already admitted here. Cargo remains fail-fast within each;
    # the other admitted lane finishes and saves its evidence after a failure.
    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(phase, name) for name in ("smoke", "remainder")]
        lanes = [future.result() for future in futures]
    records.extend(lanes)
    passed = True
    counts = dict(passed=0, failed=0, ignored=0)
    for record in lanes:
        path = run / ("phase-" + record["name"] + ".log")
        log = path.read_text(errors="replace") if path.exists() else ""
        # Keep target output contiguous for the existing parser. Preparation has
        # separate evidence and is not counted as an additional product test.
        output.write(log + "\n")
        summary = lane_summary(runner.reporting(), log)
        modules = summary["modules"]
        lane_counts = summary["counts"]
        for key in counts:
            counts[key] += lane_counts[key]
        passed &= record["process_exit"] == 0 and summary["complete"]
        if record["name"] == "smoke":
            passed &= (len(modules) == 1 and modules[0]["label"] == "tests/smoke_cli.rs"
                       and lane_counts == dict(passed=1, failed=0, ignored=0))
            try:
                passed &= file_digest(smoke_executable) == smoke_digest
            except OSError:
                passed = False
        outcome["timed_out"] |= record.get("timed_out", False)
    passed &= sum(counts.values()) == outcome["expected_tests"]
    outcome.update(process_exit=0 if passed else 1, complete=bool(passed), counts=counts)
    if not passed:
        outcome["error"] = "Lane failure or incomplete test partition; inspect phase logs"
    return outcome


PRODUCT_TARGETS = frozenset({
    "publication_process", "control_shell", "pre_gateway_compute_routing",
    "p0_gateway_protocol", "p0_gateway_request_authority", "p0_gateway_runtime",
    "p0_gateway_privacy", "p0_gateway_context_hold", "p0_gateway_commit_boundary",
    "p0_gateway_replay", "p0_gateway_observation", "p0_gateway_native_sessions",
    "smoke_cli",
})


def dag_catalog(log, checkout, metadata):
    """Match Cargo's executable artifacts against independently declared test targets."""
    members = set(metadata["workspace_members"])
    workspace = {package["id"]: package for package in metadata["packages"]
                 if package["id"] in members and package["name"] != "hiroute-desktop"}
    if not workspace or len(workspace) != len(members) - sum(
            package["name"] == "hiroute-desktop" for package in metadata["packages"]
            if package["id"] in members):
        raise ValueError("Workspace test metadata is incomplete")
    packages = {identifier: Path(package["manifest_path"]).parent.resolve(strict=True)
                for identifier, package in workspace.items()}
    expected = {}
    for identifier, package in workspace.items():
        for declared in package["targets"]:
            if declared.get("test") is not True:
                continue
            key = (identifier, declared["name"], tuple(declared["kind"]))
            if key in expected:
                raise ValueError("Workspace metadata declares a duplicate test target")
            expected[key] = Path(declared["src_path"]).resolve(strict=True)
    if not expected:
        raise ValueError("Workspace metadata declares no test targets")
    target = (Path(checkout) / "target/debug/deps").resolve(strict=True)
    found = {}
    build_finished = 0
    for line in log.splitlines():
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError("Malformed Cargo JSON compilation event") from error
        if event.get("reason") == "build-finished":
            build_finished += 1
            if event.get("success") is not True:
                raise ValueError("Cargo JSON compilation did not finish successfully")
        if event.get("reason") != "compiler-artifact" or not event.get("profile", {}).get("test"):
            continue
        executable = event.get("executable")
        if not executable:
            continue
        package = event.get("package_id")
        key = (package, event["target"]["name"], tuple(event["target"]["kind"]))
        if key not in expected:
            raise ValueError("Test executable has no declared workspace test target")
        path = Path(executable).resolve(strict=True)
        if path.parent != target or not path.is_file():
            raise ValueError("Test executable escapes the checkout target")
        source = Path(event["target"]["src_path"]).resolve(strict=True)
        if source != expected[key]:
            raise ValueError("Test executable source differs from workspace metadata")
        source = source.relative_to(packages[package]).as_posix()
        kinds = event["target"]["kind"]
        label = ("tests/" + Path(source).name if "test" in kinds
                 else "unittests " + source)
        previous = found.get(key)
        row = dict(package=package, name=event["target"]["name"], kind=kinds,
                   label=label, cwd=str(packages[package]), test_source=str(source),
                   source=str(path),
                   sha256=file_digest(path))
        if previous and previous != row:
            raise ValueError("Test target produced ambiguous artifacts")
        found[key] = row
    if build_finished != 1 or set(found) != set(expected):
        missing = len(set(expected) - set(found))
        extra = len(set(found) - set(expected))
        raise ValueError("Compiled test artifact catalog is incomplete: "
                         f"build-finished={build_finished}, missing={missing}, extra={extra}")
    targets = list(found.values())
    if not targets or sum(item["name"] == "smoke_cli" for item in targets) != 1:
        raise ValueError("Compiled test artifact catalog is incomplete")
    return targets


def dag_list(binary, cwd, environment):
    result = subprocess.run([str(binary), "--list"], cwd=cwd, env=environment,
                            capture_output=True, text=True, timeout=45)
    if result.returncode:
        raise ValueError("Failed to list compiled test target: " + str(binary))
    names = re.findall(r"^(.+): test$", result.stdout, re.M)
    counts = re.findall(r"^(\d+) tests?, (\d+) benchmarks?$", result.stdout, re.M)
    if len(counts) != 1 or int(counts[0][0]) != len(names) or int(counts[0][1]):
        raise ValueError("Compiled test target has an incomplete listing")
    if len(set(names)) != len(names):
        raise ValueError("Compiled test target lists a duplicate case")
    return names


def dag_observed_cases(log):
    # Concurrent child stderr can splice bytes into a libtest result line.
    # Match the stable case-start marker even when that line is interleaved;
    # the parent terminal summary separately proves complete counts/status.
    # Child libtest lines are set-deduplicated, never added to parent counts.
    return set(CASE_START.findall(log))


def dag_exec_audit(trace):
    """Keep process classes, never command arguments or runtime secrets."""
    calls = re.findall(r'\bexecve(?:at)?\([^\n]*?"([^"\n]+)"', trace)
    if not calls:
        raise ValueError("Execution trace contains no started test process")
    blocked = [path for path in calls if Path(path).name in {"cargo", "rustc"}]
    return dict(schema="hiroute.execution-process-audit/v1",
                process_starts=len(calls), cargo_rustc_starts=len(blocked),
                blocked_classes=sorted({Path(path).name for path in blocked}))


def dag_snapshot(targets, checkout, run_id):
    directory = Path(checkout) / "target/validation-schedule" / run_id / "tests"
    directory.mkdir(parents=True, mode=0o700, exist_ok=False)
    for index, item in enumerate(targets):
        # The following Cargo preparation command selects smoke_cli again and
        # could relink that exact target. Other targets retain Cargo's stable
        # artifact path. Their frozen digest is checked before and after the
        # run, so a later writer cannot silently change the tested artifact.
        if item["name"] != "smoke_cli":
            item["executable"] = item["source"]
            continue
        path = directory / (str(index) + "-" + Path(item["source"]).name)
        # Copy just the target that preparation may rewrite. The workbench
        # filesystem has no reflink, so copying all 17 GiB adds ~80 seconds.
        if file_digest(Path(item["source"])) != item["sha256"]:
            raise ValueError("Test executable changed before snapshot")
        snapshot_copy(item["source"], path)
        if file_digest(path) != item["sha256"]:
            raise ValueError("Test executable changed during snapshot")
        item["executable"] = str(path)


def dag_daemon_fixture(log, checkout, metadata, run_id):
    """Freeze the daemon binary before product recipes reuse its output path."""
    package_id = next((p["id"] for p in metadata["packages"]
                       if p["name"] == "hiroute-daemon"), None)
    expected = (Path(checkout) / "target/debug/hirouted").resolve(strict=True)
    events = []
    for line in log.splitlines():
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (event.get("reason") == "compiler-artifact"
                and event.get("package_id") == package_id
                and event.get("target", {}).get("name") == "hirouted"
                and event.get("target", {}).get("kind") == ["bin"]
                and not event.get("profile", {}).get("test")):
            events.append(event)
    if (len(events) != 1 or "integration-test-hooks" not in events[0].get("features", [])
            or Path(events[0]["executable"]).resolve(strict=True) != expected):
        raise ValueError("Daemon test fixture has no exact feature-qualified artifact")
    destination = (Path(checkout) / "target/validation-schedule" / run_id
                   / "daemon-fixture/hirouted")
    destination.parent.mkdir(parents=True, mode=0o700, exist_ok=False)
    digest = file_digest(expected)
    snapshot_copy(expected, destination)
    if file_digest(destination) != digest:
        raise ValueError("Daemon test fixture changed during snapshot")
    return dict(path=str(destination), sha256=digest)


def dag_bind_daemon_fixture(targets, artifact):
    consumers = set()
    for item in targets:
        if not item["test_source"].startswith("tests/"):
            continue
        source = (Path(item["cwd"]) / item["test_source"]).read_text(errors="replace")
        if "discovered_model_product_support.rs" in source:
            if item["package_name"] != "hiroute-daemon":
                raise ValueError("Daemon fixture consumer has an unexpected package")
            item["environment"] = {"HIROUTE_VALIDATION_DAEMON_BIN": artifact["path"]}
            item["external_artifacts"] = [artifact]
            consumers.add(item["name"])
    if consumers != {"discovered_model_product", "subscription_management_product"}:
        raise ValueError("Daemon fixture consumer set changed")


def dag_bind_metadata(targets, artifact):
    consumers = {"product_oracle", "transaction_recovery"}
    found = set()
    for item in targets:
        if item["name"] not in consumers:
            continue
        if item["package_name"] != "hiroute-product-e2e":
            raise ValueError("Metadata consumer has an unexpected package")
        item.setdefault("environment", {})["HIROUTE_VALIDATION_METADATA_JSON"] = artifact["path"]
        item.setdefault("external_artifacts", []).append(artifact)
        found.add(item["name"])
    if found != consumers:
        raise ValueError("Metadata consumer set changed")


def dag_toolchain_context(runner, checkout, sha, run):
    """Capture version probes in the build lane, before runtime processes start."""
    cargo = Path(runner.command(["rustup", "which", "cargo"], checkout)).resolve(strict=True)
    rustc = cargo.with_name("rustc").resolve(strict=True)
    def record(path, args):
        version = subprocess.check_output([str(path), *args], cwd=checkout, text=True)
        return dict(path=str(path), sha256="sha256:" + file_digest(path), version=version)
    cargo_record = record(cargo, ["-vV"])
    rustc_record = record(rustc, ["-vV"])
    host = re.findall(r"^host: (\S+)$", rustc_record["version"], re.M)
    if len(host) != 1:
        raise ValueError("Prepared rustc host target is missing")
    wrapper = shutil.which("sccache")
    wrapper_record = record(Path(wrapper).resolve(strict=True), ["--version"]) if wrapper else None
    if wrapper_record:
        wrapper_record["version"] = wrapper_record["version"].strip()
        if not wrapper_record["version"].startswith("sccache "):
            raise ValueError("Prepared compiler wrapper is not sccache")
    data = dict(schema="hiroute.validation-toolchain/v1", sha=sha,
                cargo=cargo_record, rustc=rustc_record,
                target_triple=host[0], wrapper=wrapper_record)
    path = Path(run) / "dag-toolchain.json"
    runner.save(path, data)
    return dict(path=str(path), sha256=file_digest(path))


def dag_products(log, checkout, sha, run_id):
    report, environment, artifacts = prepared_products(log, checkout, sha, jobs=32)
    gateway = report["gateway"]
    receipt, directory = preparation_receipt(gateway.get("receipt"),
                                             Path(checkout).resolve() / "target", "hiroute-gateway")
    attestation = receipt["attestation"]
    if (gateway.get("schema") != "hiroute.e2e.current-preparation/v1"
            or gateway.get("prepared") is not True or gateway.get("source_revision") != sha
            or attestation["schema_version"] != "hiroute.e2e.sut-build-attestation/v3"
            or attestation["source_revision"] != sha
            or attestation["executable_sha256"] != gateway["executable_sha256"]
            or receipt["recipe"] != "sha256:" + directory.parent.name):
        raise ValueError("Prepared Gateway receipt does not match the candidate")
    gateway_path = prepared_binary(attestation["executable_path"], directory, "hirouted",
                                   gateway["executable_sha256"])
    directory = Path(checkout) / "target/validation-schedule" / run_id / "products"
    directory.mkdir(parents=True, mode=0o700, exist_ok=False)
    for package, name in (("hiroute-cli", "hiroute"), ("hiroute-daemon", "hirouted")):
        path = directory / name
        snapshot_copy(artifacts[package], path)
        if file_digest(path) != file_digest(artifacts[package]):
            raise ValueError("Prepared product copy changed")
    environment.update(HIROUTE_VALIDATION_EXECUTION="1",
                       HIROUTE_VALIDATION_PRODUCT_BIN_DIR=str(directory),
                       HIROUTE_VALIDATION_GATEWAY_BIN=str(gateway_path),
                       HIROUTE_SMOKE_REQUIRE_PREPARED="1")
    return environment


def dag_publication_daemon(log, checkout, metadata, run_id, environment, sha, run):
    """Keep the recovery-test daemon distinct from the production daemon."""
    package_id = next((p["id"] for p in metadata["packages"]
                       if p["name"] == "hiroute-daemon"), None)
    if package_id is None:
        raise ValueError("Daemon package absent from workspace metadata")
    expected = (Path(checkout) / "target/debug/hirouted").resolve(strict=True)
    events = []
    for line in log.splitlines():
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (event.get("reason") == "compiler-artifact"
                and event.get("package_id") == package_id
                and event.get("target", {}).get("name") == "hirouted"
                and event.get("target", {}).get("kind") == ["bin"]):
            events.append(event)
    if (len(events) != 1 or "integration-test-hooks" not in events[0].get("features", [])
            or Path(events[0]["executable"]).resolve(strict=True) != expected):
        raise ValueError("Publication daemon recipe has no exact feature-qualified artifact")
    directory = Path(checkout) / "target/validation-schedule" / run_id / "publication"
    directory.mkdir(parents=True, mode=0o700, exist_ok=False)
    copied = directory / "hirouted"
    digest = file_digest(expected)
    snapshot_copy(expected, copied)
    if file_digest(copied) != digest:
        raise ValueError("Publication daemon changed during snapshot")
    cli = Path(environment["HIROUTE_VALIDATION_PRODUCT_BIN_DIR"]) / "hiroute"
    cli_copy = directory / "hiroute"
    cli_digest = file_digest(cli)
    snapshot_copy(cli, cli_copy)
    if file_digest(cli_copy) != cli_digest:
        raise ValueError("Publication CLI changed during snapshot")
    receipt = dict(schema="hiroute.prepared-publication-daemon/v1", sha=sha,
                   package_id=package_id, features=events[0]["features"],
                   cargo_fresh=events[0].get("fresh"), path=str(copied), sha256=digest,
                   cli_path=str(cli_copy), cli_sha256=cli_digest)
    runner_path = Path(run) / "dag-publication-receipt.json"
    runner_path.write_text(json.dumps(receipt, indent=2) + "\n")
    return directory


def dag_run_targets(runner, root, run, targets, checkout, lease, environment, deadline,
                    slots=4, workers=4, fail_fast=True, reserved_handles=None,
                    stop_event=None):
    """Admit bounded target processes; preserve each Cargo-style target log."""
    results = []
    reservation = (contextlib.nullcontext(reserved_handles) if reserved_handles is not None
                   else runner.capacity(root, False, deadline, slots=slots))
    with reservation as handles:
        tracer = shutil.which("strace") if sys.platform == "linux" else None
        if not tracer:
            raise ValueError("DAG execution requires strace to audit Cargo/rustc starts")
        def run_one(item):
            name = item["name"]
            path = run / ("dag-test-" + str(item["index"]) + ".log")
            selected = item["executable"]
            if file_digest(Path(selected)) != item["sha256"]:
                raise ValueError("Compiled test executable changed before execution")
            for artifact in item.get("external_artifacts", []):
                if file_digest(Path(artifact["path"])) != artifact["sha256"]:
                    raise ValueError("Prepared external artifact changed before execution: " + name)
            env = dict(environment, CARGO_PKG_NAME=item["package_name"],
                       **item.get("environment", {}))
            if name == "smoke_cli":
                env.update(item.get("smoke_environment", {}))
                env.update(environment)
            args = [selected, "--test-threads=" + str(item.get(
                "test_threads", 1 if name == "smoke_cli" else 8))]
            trace = run / ("dag-exec-" + str(item["index"]) + ".log")
            traced = [tracer, "-f", "-e", "trace=execve,execveat", "-s", "0",
                      "-o", str(trace), *args]
            started = time.monotonic()
            started_at = time.time()
            with path.open("w") as output:
                output.write("     Running " + item["label"] + " (" + selected + ")\n")
                output.flush()
                process = runner.start_locked_command(traced, item["cwd"], env, output,
                                                      [*handles, lease])
                try:
                    code = process.wait(timeout=max(0, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    code = process.wait()
            audit = dag_exec_audit(trace.read_text(errors="replace"))
            trace.unlink()
            runner.save(run / ("dag-exec-audit-" + str(item["index"]) + ".json"), audit)
            if audit["cargo_rustc_starts"]:
                raise ValueError("Runtime Cargo/rustc start detected: " + name)
            if file_digest(Path(selected)) != item["sha256"]:
                raise ValueError("Compiled test executable changed after execution")
            for artifact in item.get("external_artifacts", []):
                if file_digest(Path(artifact["path"])) != artifact["sha256"]:
                    raise ValueError("Prepared external artifact changed after execution: " + name)
            text = path.read_text(errors="replace")
            summary = lane_summary(runner.reporting(), text)
            modules = summary["modules"]
            if (len(modules) != 1 or modules[0]["label"] != item["label"]
                    or sum(summary["counts"].values()) != len(item["cases"])
                    or dag_observed_cases(text) != set(item["cases"])):
                raise ValueError("Test target did not complete its listed cases: " + name)
            return dict(name=name, index=item["index"], log=str(path), process_exit=code,
                        complete=summary["complete"] and code == 0,
                        counts=summary["counts"], exec_audit=audit,
                        started_at=started_at, finished_at=time.time(),
                        seconds=round(time.monotonic() - started, 3))

        pending = iter(targets)
        active = {}
        with ThreadPoolExecutor(max_workers=workers) as pool:
            def admit():
                if stop_event is not None and stop_event.is_set():
                    return False
                try:
                    item = next(pending)
                except StopIteration:
                    return False
                active[pool.submit(run_one, item)] = item
                return True
            for _ in range(workers):
                admit()
            while active:
                done, _ = wait(active, return_when=FIRST_COMPLETED)
                for future in done:
                    item = active.pop(future)
                    try:
                        result = future.result()
                    except Exception as error:
                        result = dict(name=item["name"], index=item["index"],
                                      log=str(run / ("dag-test-" + str(item["index"]) + ".log")),
                                      complete=False, process_exit=1,
                                      error=str(error), counts=dict(passed=0, failed=0, ignored=0))
                    results.append(result)
                    if not result["complete"] and fail_fast and stop_event is not None:
                        stop_event.set()
                    if result["complete"] or not fail_fast:
                        admit()
    return results


def execute_dag(runner, root, run, checkout, request, lease, output, reserved_handles=None):
    """Build once, overlap prepared product work with independent test targets."""
    commands(request["command"])
    deadline = time.monotonic() + request["timeout"]
    base = [arg for arg in request["command"] if arg != "--no-fail-fast"]
    fail_fast = "--no-fail-fast" not in request["command"]
    phase_records = []
    outcome = dict(process_exit=1, timed_out=False, phases=phase_records, complete=False,
                   doctests_executed=False)
    targets, results = [], []
    expected = None

    def cargo_phase(name, argv, slots):
        path = run / ("dag-" + name + ".log")
        started = time.monotonic()
        started_at = time.time()
        reservation = (contextlib.nullcontext(reserved_handles) if reserved_handles is not None
                       else runner.capacity(root, slots == 8, deadline, slots=slots))
        with reservation as handles:
            env = dict(os.environ, CARGO_BUILD_JOBS=str(slots * 8))
            with path.open("w") as stream:
                process = runner.start_locked_command(argv, checkout, env, stream,
                                                      [*handles, lease])
                try:
                    code = process.wait(timeout=max(0, deadline - time.monotonic()))
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    code = process.wait()
        record = dict(name=name, command=argv, log=str(path), process_exit=code,
                      status="completed" if code == 0 else "failed",
                      build_jobs=slots * 8, started_at=started_at,
                      finished_at=time.time(),
                      seconds=round(time.monotonic() - started, 3))
        phase_records.append(record)
        if code:
            raise ValueError(name + " failed with process exit " + str(code))
        return path.read_text(errors="replace")

    try:
        compile_log = cargo_phase("compile", [*base, "--no-run", "--message-format=json"], 8)
        metadata = json.loads(runner.command(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], checkout))
        metadata_path = run / "dag-cargo-metadata.json"
        runner.save(metadata_path, metadata)
        metadata_artifact = dict(path=str(metadata_path), sha256=file_digest(metadata_path))
        toolchain_artifact = dag_toolchain_context(runner, checkout, request["sha"], run)
        targets = dag_catalog(compile_log, checkout, metadata)
        package_names = {p["id"]: p["name"] for p in metadata["packages"]}
        listing_environment = dict(os.environ)
        for index, item in enumerate(targets):
            item["index"] = index
            item["package_name"] = package_names[item["package"]]
            item["cases"] = dag_list(item["source"], item["cwd"], listing_environment)
        if sum("default_two_domain_smoke" in item["cases"] for item in targets) != 1:
            raise ValueError("Default smoke case missing or duplicated")
        expected = sum(len(item["cases"]) for item in targets)
        dag_snapshot(targets, checkout, run.name)
        daemon_fixture = dag_daemon_fixture(compile_log, checkout, metadata, run.name)
        dag_bind_daemon_fixture(targets, daemon_fixture)
        dag_bind_metadata(targets, metadata_artifact)
        for item in targets:
            item.setdefault("external_artifacts", []).append(toolchain_artifact)
        # CPA's child must finish its released-port handoff before other listeners.
        # Observation keeps its real 500 ms query deadlines and library parallelism,
        # without competing with preparation builds or other library processes.
        isolated = []
        for package, threads in (("hiroute-cpa-bridge", 1), ("hiroute-observation", 8)):
            selected = [item for item in targets
                        if item["package_name"] == package
                        and item["test_source"] == "src/lib.rs"]
            if len(selected) != 1:
                raise ValueError(package + " library target missing or duplicated in isolation lane")
            selected[0]["test_threads"] = threads
            isolated.extend(selected)
        runner.save(run / "dag-validation-plan.json", dict(
            schema="hiroute.validation-plan/v1", sha=request["sha"],
            command=request["command"], targets=targets, expected_tests=expected))

        independent = sorted((item for item in targets
                              if item not in isolated and item["name"] not in PRODUCT_TARGETS),
                             key=lambda item: (item["name"] != "hiroute_daemon", item["name"]))
        dependent = [item for item in targets if item["name"] in PRODUCT_TARGETS]
        stop_event = threading.Event() if fail_fast else None
        for item in independent:
            if (item["test_source"].startswith("tests/")
                    and "mod runtime_support;" in (Path(item["cwd"]) / item["test_source"]).read_text(errors="replace")):
                raise ValueError("Gateway runtime consumer lacks product preparation: " + item["name"])
        execution_env = dict(os.environ, HIROUTE_VALIDATION_EXECUTION="1",
                             HIROUTE_VALIDATION_TOOLCHAIN_JSON=toolchain_artifact["path"])
        results = dag_run_targets(runner, root, run, isolated, checkout, lease,
                                  execution_env, deadline, slots=4, workers=1,
                                  fail_fast=fail_fast, reserved_handles=reserved_handles,
                                  stop_event=stop_event)
        if stop_event is not None and stop_event.is_set():
            outcome.update(expected_tests=expected, targets=len(targets),
                           executed_targets=len(results), target_results=results,
                           error="Isolated test target failed; remaining targets and doctests were not admitted")
            return outcome
        with ThreadPoolExecutor(max_workers=2) as pool:
            prepared = pool.submit(cargo_phase, "prepare", commands(base)["prepare"], 4)
            ordinary = pool.submit(dag_run_targets, runner, root, run, independent, checkout,
                                   lease, execution_env,
                                   deadline, 4, 4, fail_fast, reserved_handles, stop_event)
            try:
                preparation_log = prepared.result()
                summary = runner.test_summary(preparation_log, commands(base)["prepare"])
                if summary["counts"] != dict(passed=1, failed=0, ignored=0):
                    raise ValueError("Build preparation did not execute exactly once")
                prepared_env = dag_products(preparation_log, checkout, request["sha"], run.name)
                prepared_env["HIROUTE_VALIDATION_TOOLCHAIN_JSON"] = toolchain_artifact["path"]
                hooks_command = ["cargo", "build", "--locked", "-p", "hiroute-daemon",
                                 "--bin", "hirouted", "--features", "integration-test-hooks",
                                 "--profile", "dev", "--message-format=json"]
                hooks_log = cargo_phase("publication-daemon", hooks_command, 4)
                hooks_dir = dag_publication_daemon(hooks_log, checkout, metadata, run.name,
                                                   prepared_env, request["sha"], run)
                for item in dependent:
                    if item["name"] == "publication_process":
                        item["environment"] = {
                            "HIROUTE_VALIDATION_PRODUCT_BIN_DIR": str(hooks_dir)}
                results.extend(dag_run_targets(runner, root, run, dependent, checkout, lease,
                                              dict(os.environ, **prepared_env), deadline,
                                              slots=4, workers=4, fail_fast=fail_fast,
                                              reserved_handles=reserved_handles,
                                              stop_event=stop_event))
            except Exception:
                if stop_event is not None:
                    stop_event.set()
                raise
            finally:
                # Preparation can fail after independent targets already finished.
                # Retain those results even when dependent admission never begins.
                results.extend(ordinary.result())
        if stop_event is not None and stop_event.is_set():
            outcome.update(expected_tests=expected, targets=len(targets),
                           executed_targets=len(results), target_results=results,
                           error="Test target failed; remaining targets and doctests were not admitted")
            return outcome
        outcome["doctests_executed"] = True
        doc_log = cargo_phase("doctest", [*base, "--doc"], 8)
        output.write(doc_log + "\n")
        doc_summary = lane_summary(runner.reporting(), doc_log)
        counts = {key: doc_summary["counts"][key] + sum(
            row["counts"][key] for row in results) for key in ("passed", "failed", "ignored")}
        all_complete = (len(results) == len(targets)
                        and all(row["complete"] for row in results)
                        and doc_summary["complete"] and counts["failed"] == 0
                        and sum(row["counts"][key] for row in results
                                for key in ("passed", "failed", "ignored")) == expected)
        outcome.update(process_exit=0 if all_complete else 1, complete=bool(all_complete),
                       expected_tests=expected, counts=counts,
                       targets=len(targets), executed_targets=len(results),
                       target_results=results,
                       doctest_modules=len(doc_summary["modules"]),
                       performance_seconds=round(time.monotonic() - (deadline - request["timeout"]), 3),
                       performance_goal_met=False)
        outcome["performance_goal_met"] = outcome["performance_seconds"] <= 300
        if not all_complete:
            outcome["error"] = "Missing, failed or incomplete test target; inspect DAG logs"
    except Exception as error:
        outcome["error"] = str(error)
        outcome["timed_out"] = time.monotonic() >= deadline
    finally:
        for result in sorted(results, key=lambda item: item.get("log", "")):
            if result.get("log") and Path(result["log"]).is_file():
                output.write(Path(result["log"]).read_text(errors="replace") + "\n")
        if expected is not None:
            executed = {row["index"] for row in results}
            outcome.update(expected_tests=expected, targets=len(targets),
                           executed_targets=len(results), target_results=results,
                           unexecuted_targets=[dict(index=item["index"], name=item["name"],
                                                    package=item["package_name"], cases=item["cases"])
                                               for item in targets if item["index"] not in executed])
            outcome.setdefault("counts", {key: sum(row["counts"][key] for row in results)
                                          for key in ("passed", "failed", "ignored")})
    return outcome
