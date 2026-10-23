#!/usr/bin/env python3
"""Derived validation diagnostics; never an attestation or an automatic test waiver."""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import sys

VERSION = 1
SAFE_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}\Z")
HEADER = re.compile(r"^\s*Running (.+?) \(([^)]+)\)\s*$")
SUMMARY = re.compile(r"^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored(?:;.*)?$")
CASE = re.compile(r"^test (\S+) \.\.\. (ok|FAILED|ignored)(?:\s.*)?$")
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def add_arguments(parser):
    parser.add_argument("--plan", help="This feature's validation group (not a global latest-run lookup)")
    parser.add_argument("--phase", choices=("diagnostic", "focused", "final"), default="diagnostic")
    parser.add_argument("--related-run", action="append", default=[], metavar="ID",
                        help="Explicit earlier run in the same plan and execution-host run store")


def options(args):
    plan = getattr(args, "plan", None)
    related = list(dict.fromkeys(getattr(args, "related_run", []) or []))
    phase = getattr(args, "phase", "diagnostic")
    if phase not in ("diagnostic", "focused", "final"):
        raise ValueError("Unknown validation phase")
    if plan is not None and not SAFE_ID.fullmatch(plan):
        raise ValueError("Invalid validation plan ID")
    if related and not plan:
        raise ValueError("--related-run requires --plan")
    if any(not SAFE_ID.fullmatch(key) for key in related):
        raise ValueError("Invalid related run ID")
    return dict(plan=plan, phase=phase, related_runs=related)


def related_reports(root, metadata):
    reports = []
    for key in metadata.get("related_runs", []):
        if not SAFE_ID.fullmatch(key) or not metadata.get("plan"):
            raise ValueError("Invalid related run/plan")
        path = Path(root) / key / "validation-report.json"
        report = json.loads(path.read_text())
        if report.get("version") != VERSION or report.get("plan") != metadata["plan"]:
            raise ValueError("Related run belongs to another plan or unsupported report version: " + key)
        if report.get("process_exit") is None:
            raise ValueError("Related run has not executed: " + key)
        reports.append(report)
    return reports


def preflight(record, related):
    """A visible reminder before paying for an explicitly linked duplicate final run."""
    repeated = [r.get("run_id") for r in related if record.get("phase") == r.get("phase") == "final"
                and record.get("plan") == r.get("plan") and record.get("sha", record.get("revision")) == r.get("sha")
                and record.get("command") == r.get("context", {}).get("command")]
    if repeated:
        record["validation_advice"] = "Repeated final command for the same SHA; record the rerun reason. Related runs: " + ", ".join(str(r) for r in repeated)
        print(record["validation_advice"], file=sys.stderr, flush=True)


def seconds(value):
    parts = re.findall(r"([\d.]+)(ms|s|m|h)", value)
    return sum(float(n) * {"ms": .001, "s": 1, "m": 60, "h": 3600}[unit] for n, unit in parts) if parts else None


def parse_log(log):
    """Use the terminal summary of each Cargo target, not sums of child summaries.

    A missing terminal summary or repeated target identity remains ambiguous. Cargo
    output is diagnostic text, not structured proof of per-function execution.
    """
    modules, builds, unassigned = [], [], []
    current = None
    open_harnesses = 0
    for number, raw in enumerate(log.splitlines(), 1):
        line = ANSI.sub("", raw)
        header = HEADER.match(line)
        if header or line.strip().startswith("Doc-tests "):
            if header:
                label, binary = header.groups()
                executable = Path(binary).name
                stem = re.sub(r"-[0-9a-f]{8,}$", "", executable)
                key = label + " [" + executable + "]"
                identity = label + " [" + stem + "]"
            else:
                label = line.strip()
                key = identity = label
            current = dict(module=key, identity=identity, label=label, line=number, cases={}, summaries=[],
                           seconds=None, counts=None, result=None, complete=False, expected_count=None)
            modules.append(current)
            open_harnesses = 0
        running = re.fullmatch(r"running (\d+) tests?", line)
        if running and current is not None:
            open_harnesses += 1
            if current["expected_count"] is None:
                current["expected_count"] = int(running[1])
        summary = SUMMARY.match(line)
        if summary:
            state, passed, failed, ignored = summary.groups()
            duration = re.search(r"finished in ([\d.]+)s", line)
            row = dict(result=state, counts=dict(passed=int(passed), failed=int(failed), ignored=int(ignored)),
                       seconds=float(duration[1]) if duration else None, summary_line=number)
            if current is not None:
                # A same-sized child cannot close a still-running parent harness.
                open_harnesses = max(0, open_harnesses - 1)
                current["summaries"].append(row)
                current.update(row)
                current["complete"] = (open_harnesses == 0 and current["expected_count"] is not None
                                       and sum(row["counts"].values()) == current["expected_count"])
            else:
                unassigned.append(row)
        case = CASE.match(line)
        if case and current is not None:
            name, state = case.groups()
            previous = current["cases"].get(name)
            current["cases"][name] = state if previous in (None, state) else "conflicting_child_output"
        if re.match(r"\s*Finished .* profile .* in ", line):
            builds.append(dict(seconds=seconds(line.rsplit(" in ", 1)[-1]), line=number,
                               scope="nested_or_later" if current else "outer"))
    duplicates = Counter(module["module"] for module in modules)
    identities = Counter(module["identity"] for module in modules)
    for module in modules:
        module["child_summaries"] = max(0, len(module.pop("summaries")) - 1)
        if duplicates[module["module"]] > 1:
            module["complete"] = False
            module["ambiguity"] = "repeated_target_identity"
        module["comparison_identity_unique"] = identities[module["identity"]] == 1
        module["failures"] = sorted(name for name, state in module["cases"].items() if state == "FAILED")
        if module["child_summaries"] or (module["counts"] and len(module["failures"]) != module["counts"]["failed"]):
            module["case_attribution"] = "incomplete_or_nested"
        else:
            module["case_attribution"] = "observed"
    return dict(modules=modules, build_events=builds, unassigned_summaries=unassigned)


def context(record):
    command = record.get("command", [])
    before = command[:command.index("--")] if "--" in command else command
    selected, features, build = [], [], []
    for index, arg in enumerate(before):
        if arg in ("-p", "--package", "--exclude") and index + 1 < len(before):
            selected.extend([arg, before[index + 1]])
        elif arg == "--workspace" or arg.startswith(("--package=", "--exclude=")):
            selected.append(arg)
        elif arg in ("--features", "-F") and index + 1 < len(before):
            features.append(before[index + 1])
        elif arg in ("--all-features", "--no-default-features") or arg.startswith("--features="):
            features.append(arg)
        elif arg in ("--profile", "--target") and index + 1 < len(before):
            build.extend([arg, before[index + 1]])
        elif arg == "--release" or arg.startswith(("--profile=", "--target=")):
            build.append(arg)
    return dict(package_scope=selected, requested_features=features,
                platform=record.get("platform", record.get("runner_os")),
                architecture=record.get("architecture", record.get("runner_arch")),
                rustc=record.get("rustc"), cargo=record.get("cargo"),
                profile="release" if "--release" in command else "default",
                build_options=build, build_jobs=record.get("build_jobs"),
                harness_options=command[command.index("--") + 1:] if "--" in command else [],
                harness_concurrency=record.get("harness_concurrency", harness_concurrency(command)),
                command=command)


def harness_concurrency(command, environment=None, policy=None):
    """Record launch settings, never infer measured concurrency from host CPUs.

    Cargo config, custom harnesses and test-created threads can change runtime
    behavior. Even an explicit libtest limit is configuration, not a measurement.
    """
    if command[:2] != ["cargo", "test"]:
        return None
    arguments = command[command.index("--") + 1:] if "--" in command else []
    requested = []
    for index, argument in enumerate(arguments):
        if argument == "--test-threads":
            requested.append(arguments[index + 1] if index + 1 < len(arguments) else None)
        elif argument.startswith("--test-threads="):
            requested.append(argument.split("=", 1)[1])
    inherited = environment.get("RUST_TEST_THREADS") if environment is not None else None
    source = "argument" if requested else ("environment" if inherited is not None else
                                           "default" if environment is not None else "unknown")
    selected = requested[0] if len(requested) == 1 else inherited if not requested else None
    limit = int(selected) if isinstance(selected, str) and re.fullmatch(r"[0-9]+", selected) else None
    if limit is not None and limit < 1:
        limit = None
    return dict(requested_threads=requested, environment_threads=inherited,
                environment_recorded=environment is not None, configured_thread_limit=limit,
                configuration_source=source, effective_threads=None, effective_state="unknown",
                policy=policy)


def compare(report, related):
    findings = []
    if report["phase"] != "final":
        return findings
    for previous in related:
        common = dict(related_run=previous.get("run_id"), original_sha=previous.get("sha"))
        if not report.get("plan") or previous.get("plan") != report["plan"]:
            findings.append(dict(common, kind="unrelated_plan", action="Do not reuse this run."))
            continue
        if previous.get("sha") != report["sha"]:
            findings.append(dict(common, kind="different_revision", action="Review intervening changes; earlier success is not same-candidate evidence."))
            continue
        if previous.get("phase") == "final" and previous.get("context", {}).get("command") == report["context"]["command"]:
            findings.append(dict(common, kind="repeated_final", action="Explain why this exact final command needed another run."))
        if report["process_exit"] in (None, 0) or previous.get("phase") != "focused" or not previous.get("observed_tests_passed"):
            continue
        differences = [key for key in ("package_scope", "requested_features", "platform", "architecture", "rustc", "cargo", "profile", "build_options", "build_jobs", "harness_options", "harness_concurrency")
                       if previous.get("context", {}).get(key) != report["context"].get(key)]
        unknown = [key for key in ("platform", "architecture", "rustc", "cargo")
                   if not report["context"].get(key) or not previous.get("context", {}).get(key)]
        for module in report["modules"]:
            if module["case_attribution"] != "observed":
                continue
            for name in module["failures"]:
                matches = [m for m in previous.get("modules", []) if m["identity"] == module["identity"]]
                observed = "not_observed" if module["comparison_identity_unique"] else "ambiguous_target_identity"
                if module["comparison_identity_unique"] and len(matches) == 1 and matches[0]["complete"]:
                    observed = (matches[0]["cases"].get(name, "not_observed") if matches[0]["case_attribution"] == "observed"
                                else "ambiguous_child_output")
                findings.append(dict(common, kind="focused_full_gap", module=module["module"], test=name,
                                     focused_observation=observed, context_differences=differences, unknown_context=unknown,
                                     action="Diagnose coverage, selection, build context or environment; add the smallest reliable regression/mapping fix. Matching flags do not prove equal resolved features."))
        if not any(m["failures"] and m["case_attribution"] == "observed" for m in report["modules"]):
            findings.append(dict(common, kind="full_failure_unattributed", action="Inspect compiler/setup/timeout diagnostics and ambiguous target output; do not invent a missing unit test."))
    return findings


def smoke_timings(log, checkout, sha, started=None, finished=None):
    """Copy only timing/verdict facts from explicitly logged, checkout-owned reports."""
    if not checkout:
        return []
    root = Path(checkout).resolve()
    collected = []
    paths = {Path(value.strip()) for value in re.findall(r"^smoke report=(.+)$", log, re.M)}
    # Passing Rust tests capture stdout. Discover only this command's persisted smoke
    # reports, never an arbitrary old same-SHA report in a reused CI checkout.
    if started is not None and finished is not None:
        paths.update((root / "target/smoke").glob("*/report.json"))
    for path in sorted(paths):
        if not path.is_absolute():
            path = root / path
        try:
            resolved = path.resolve(strict=True)
            resolved.relative_to(root)
            if resolved.stat().st_size > 4 * 1024 * 1024:
                continue
            data = json.loads(resolved.read_text())
            if data.get("schema") != "hiroute.smoke.result/v1" or data.get("source_revision") != sha:
                continue
            if started is not None and finished is not None and not (
                    started * 1000 <= data.get("started_unix_ms", -1) <= data.get("finished_unix_ms", -1) <= finished * 1000):
                continue
            collected.append(dict(run_id=data.get("run_id"), source_revision=sha, cases=[
                {key: case.get(key) for key in ("id", "execution", "state", "build_ms", "execution_ms", "integrity_ms", "unclassified_ms", "timing_complete")}
                for case in data["cases"]]))
        except (OSError, ValueError, KeyError, TypeError):
            continue
    return collected


def elapsed(record, start, end):
    a, b = record.get(start), record.get(end)
    return round(b - a, 3) if isinstance(a, (int, float)) and isinstance(b, (int, float)) and b >= a else None


def build_report(record, log, related=()):
    parsed = parse_log(log)
    report = dict(version=VERSION, run_id=record.get("id"), plan=record.get("plan"),
                  phase=record.get("phase", "diagnostic"), sha=record.get("sha", record.get("revision")),
                  process_exit=record.get("process_exit"), scenario=record.get("scenario", record.get("scenario_state", "not_assessed")),
                  context=context(record), **parsed)
    report["timing"] = dict(total_seconds=record.get("end_to_end_seconds"),
                            command_seconds=elapsed(record, "command_started_at", "command_finished_at"),
                            queue_seconds=record.get("queue_seconds"),
                            execution_seconds=record.get("execution_seconds"),
                            end_to_end_seconds=record.get("end_to_end_seconds"),
                            checkout_seconds=elapsed(record, "checkout_started_at", "checkout_finished_at"),
                            setup_seconds=elapsed(record, "capacity_acquired_at", "command_started_at"))
    # Even with all observed targets terminal, a fail-fast command may omit later targets.
    report["complete"] = (bool(report["modules"]) and not report["unassigned_summaries"] and
                          all(m["complete"] for m in report["modules"]) and record.get("process_exit") == 0)
    report["timing_complete"] = report["complete"] and all(m["seconds"] is not None for m in report["modules"])
    report["observed_tests_passed"] = (report["complete"] and record.get("status") != "failed" and
        record.get("tests", {}).get("state") not in ("tests_failed", "zero_tests_passed", "unrecognized_test_output") and
        sum(m["counts"]["passed"] for m in report["modules"] if m["counts"]) > 0 and
        not any(m["counts"] and m["counts"]["failed"] for m in report["modules"]))
    report["top_modules"] = sorted((dict(module=m["module"], seconds=m["seconds"]) for m in report["modules"]
                                    if m["complete"] and m["seconds"] is not None), key=lambda m: m["seconds"], reverse=True)[:3]
    report["schedule"] = record.get("schedule_result")
    report["smoke"] = smoke_timings(log, record.get("checkout"), report["sha"],
                                    record.get("command_started_at"), record.get("command_finished_at"))
    report["findings"] = compare(report, related)
    report["follow_up"] = []
    if report["phase"] == "final":
        if report["process_exit"] != 0:
            report["follow_up"].append("Classify full failures against related focused runs; repair coverage/selection/context before retrying affected checks.")
        if not related:
            report["follow_up"].append("No related run evidence supplied; do not infer that focused tests covered this candidate.")
        report["follow_up"].append("At feature handoff assess Top 3: optimize / defer / insufficient evidence, with cost, smallest change and preserved assertions. Do not automatically refactor the frozen candidate.")
    report["note"] = "Derived Cargo target diagnostics, not per-function profiling or product acceptance. Nested build/child times are not additive. Unobserved cases and resolved dependency features remain unknown."
    return report


def markdown(report):
    lines = ["# Validation feedback", "", "Candidate: `" + str(report["sha"]) + "`; phase: " + report["phase"],
             "", "Process exit: " + str(report["process_exit"]) + "; observed target report complete: " + str(report["complete"])]
    harness = report["context"].get("harness_concurrency")
    if harness:
        lines += ["", "Harness concurrency (launch settings; runtime concurrency is unknown):",
                  "", "```json", json.dumps(harness, indent=2), "```"]
    lines += ["", "| Cargo target | Seconds | Passed / failed / ignored | Complete |", "| --- | ---: | --- | --- |"]
    for module in report["modules"]:
        counts = module["counts"]
        values = " / ".join(str(counts[k]) for k in ("passed", "failed", "ignored")) if counts else "unknown"
        lines.append("| " + module["module"].replace("|", "\\|") + " | " + str(module["seconds"] if module["seconds"] is not None else "unknown") + " | " + values + " | " + str(module["complete"]) + " |")
    schedule = report.get("schedule")
    if schedule:
        lines += ["", "## Scheduled phases", "",
                  "Phase durations overlap and must not be summed as wall-clock time.", "",
                  "Execution: " + str(report["timing"].get("execution_seconds"))
                  + "s; five-minute target met: " + str(schedule.get("performance_goal_met")) + ".", "",
                  "| Phase | Seconds | Compiler jobs | Process exit | Status |",
                  "| --- | ---: | ---: | ---: | --- |"]
        for phase in schedule.get("phases", []):
            lines.append("| " + phase["name"] + " | " + str(elapsed(phase, "started_at", "finished_at"))
                         + " | " + str(phase.get("build_jobs")) + " | " + str(phase.get("process_exit"))
                         + " | " + phase["status"] + " |")
        lines += ["", "Expected tests: " + str(schedule.get("expected_tests"))
                  + "; partition complete: " + str(schedule.get("complete")) + "."]
    lines += ["", "## Follow-up", ""] + ["- " + item for item in report["follow_up"]]
    for item in report["findings"]:
        lines.append("- `" + item["kind"] + "`: " + json.dumps(item, ensure_ascii=False))
    lines += ["", report["note"], "", "Original command: `" + " ".join(report["context"]["command"]).replace("`", "'") + "`", ""]
    return "\n".join(lines)


def write_report(run, record, related=()):
    """Record report completeness; the DAG runner requires a complete final report."""
    run = Path(run)
    try:
        log = (run / "command.log").read_text(errors="replace") if (run / "command.log").exists() else ""
        report = build_report(record, log, related)
        for name, value in (("validation-report.json", json.dumps(report, indent=2, ensure_ascii=False) + "\n"),
                            ("validation-report.md", markdown(report))):
            temporary = run / (name + ".tmp")
            temporary.write_text(value)
            temporary.replace(run / name)
        record["validation_report"] = dict(path=str(run / "validation-report.json"), markdown=str(run / "validation-report.md"),
                                           complete=report["complete"], top_modules=report["top_modules"],
                                           findings=report["findings"], follow_up=report["follow_up"])
    except Exception as error:
        record["validation_report"] = dict(complete=False, error=str(error), follow_up=["Validation report unavailable; inspect original run logs."])
    return record["validation_report"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path, help="Existing run directory; preserves process/scenario verdict")
    args = parser.parse_args()
    record = json.loads((args.run / "result.json").read_text())
    related = related_reports(args.run.parent, record)
    print(json.dumps(write_report(args.run, record, related), indent=2))


if __name__ == "__main__":
    main()
