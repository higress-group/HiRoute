#!/usr/bin/env python3
"""Select conservative affected checks; print a plan or execute it in hosted CI."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


GROUPS = {
    "gateway": ("hiroute-gateway-core", "hiroute-gateway", "hiroute-e2e", "hiroute-integrations",
                "hiroute-cpa-bridge", "hiroute-local-storage", "hiroute-release-facts", "hiroute-daemon", "hiroute-cli"),
    "cli": ("hiroute-cli", "hiroute-client-core", "hiroute-daemon"),
    "integrations": ("hiroute-integrations", "hiroute-application", "hiroute-daemon", "hiroute-cli",
                     "hiroute-cpa-bridge", "hiroute-local-storage", "hiroute-release-facts"),
    "storage": ("hiroute-local-storage", "hiroute-observation", "hiroute-application", "hiroute-daemon", "hiroute-client-core", "hiroute-cli"),
    "observation": ("hiroute-observation", "hiroute-daemon", "hiroute-client-core", "hiroute-cli"),
    "daemon": ("hiroute-daemon", "hiroute-cli"),
    "cpa": ("hiroute-cpa-bridge", "hiroute-daemon", "hiroute-integrations", "hiroute-cli"),
    "diagnostics": ("hiroute-diagnostics",),
}
OWNERS = {
    "gateway-core": "gateway", "gateway": "gateway", "cli": "cli",
    "client-core": "cli", "integrations": "integrations", "local-storage": "storage",
    "observation": "observation", "daemon": "daemon", "cpa-bridge": "cpa",
    "diagnostics": "diagnostics",
}


# Observable diagnostic contracts still need cross-boundary review. Internal logging
# changes do not, by themselves, change routing, Worker or frontend behavior.
DIAGNOSTIC_CONTRACTS = (
    "crates/diagnostics/src/event/", "crates/diagnostics/src/record.rs",
    "crates/diagnostics/src/identity.rs", "crates/diagnostics/src/error.rs",
)
SELECTION_TOOLING = {"scripts/test-plan.py", "scripts/test-test-plan.py"}
INTEGRATION_SKILL = ".agents/skills/hiroute-integrate/SKILL.md"
WEBSITE_TOOLING = {".github/workflows/website.yml", ".github/workflows/release.yml"}
WEBSITE_PREFIXES = ("apps/website/", ".github/scripts/")
HOSTED_BACKEND_EXECUTION = {
    ".github/workflows/gateway-core.yml",
    "scripts/ci-run.py",
    "scripts/ci-shards.py",
}
VALIDATION_TOOLING = {"scripts/validation.py", "scripts/validation-host.py", "scripts/test-validation.py",
                      "scripts/validation-report.py", "scripts/test-validation-report.py",
                      "scripts/ci-run.py", "scripts/test-ci-run.py",
                      "scripts/ci-shards.py", "scripts/test-ci-shards.py",
                      "scripts/pilot-builds.py", "scripts/test-pilot-builds.py",
                      "scripts/desktop-pilot.py", "scripts/test-desktop-pilot.py",
                      "scripts/local-rust.py", "scripts/test-local-rust.py",
                      "scripts/remote-rust.py", "scripts/test-remote-rust.py",
                      "scripts/validation-schedule.py", "scripts/test-validation-schedule.py"}
PRODUCT_GROUPS = set(GROUPS) - {"diagnostics"}


def e2e_consumers(path):
    """Bounded test/fixture ownership, not a guessed production dependency graph."""
    if path == "tools/e2e-harness/tests/p0_gateway_runtime.rs" or path.startswith("tools/e2e-harness/tests/p0_gateway_runtime/"):
        return [("hiroute-e2e", "p0_gateway_runtime"), ("hiroute-e2e", "p0_gateway_protocol"),
                ("hiroute-e2e", "p0_gateway_matrix_coverage")]
    if path == "tools/e2e-harness/tests/p0_gateway_protocol.rs" or path.startswith("tools/e2e-harness/tests/p0_gateway_protocol/"):
        return [("hiroute-e2e", "p0_gateway_protocol"), ("hiroute-e2e", "p0_gateway_matrix_coverage")]
    if path in ("tools/e2e-harness/tests/p0_gateway_matrix_coverage.rs", "e2e/matrix/p0-gateway-coverage.json",
                "e2e/schema/p0-gateway-coverage.schema.json"):
        return [("hiroute-e2e", "p0_gateway_matrix_coverage")]
    if path == "tools/product-e2e/tests/routing_plans.rs" or path.startswith((
            "tools/product-e2e/src/routing/", "e2e/product/scenarios/routing/",
            "e2e/product/fixtures/routing/", "e2e/product/golden/routing/")):
        return [("hiroute-product-e2e", "routing_plans")]
    if path == "tools/e2e-harness/src/p0/client.rs":
        return [("hiroute-e2e", "lib"), ("hiroute-e2e", "p0_gateway_protocol"),
                ("hiroute-e2e", "p0_gateway_oracle"), ("hiroute-e2e", "p0_gateway_matrix_coverage"),
                ("hiroute-product-e2e", "smoke_cli")]
    return []


PROCESS_TARGETS = {"p0_gateway_runtime", "p0_gateway_protocol", "smoke_cli"}


def select(paths, full=False):
    groups = set()
    frontend = full
    native = False
    reasons = []
    selection_tooling = False
    validation_tooling = False
    targets = set()
    for path in sorted(set(paths)):
        if path in SELECTION_TOOLING or path == INTEGRATION_SKILL:
            selection_tooling = True
        elif path in WEBSITE_TOOLING or path.startswith(WEBSITE_PREFIXES):
            # Static site and OSS/release publication have their own Node/browser
            # workflow. They do not change Desktop or backend product behavior.
            continue
        elif path in HOSTED_BACKEND_EXECUTION:
            # These files decide what the hosted backend actually executes. Unit
            # tests validate their mapping, while one full run proves the changed
            # workflow and shard contract on its real entry path.
            full = True
            validation_tooling = True
            reasons.append("hosted backend execution contract: " + path)
        elif path in VALIDATION_TOOLING:
            validation_tooling = True
        elif e2e_consumers(path):
            targets.update(e2e_consumers(path))
            reasons.append("bounded E2E consumers: " + path)
        elif path.startswith(DIAGNOSTIC_CONTRACTS):
            full = True
            reasons.append("diagnostic event/identity contract: " + path)
        elif path.startswith("apps/desktop/src-tauri/"):
            native = True
            groups.add("cli")
            if path.endswith(("Cargo.toml", "Cargo.lock")):
                full = True
                reasons.append("native package dependency change: " + path)
            frontend = True
            reasons.append("native entry: add affected macOS checks; review downstream behavior")
        elif path.startswith("apps/desktop/"):
            frontend = True
        elif path.startswith("assets/") and "/BRAND_ASSETS" in path:
            frontend = True
        elif (path.endswith(".md") and not path.startswith(".agents/")):
            # Runtime Markdown embeds and product Skills must not be treated as docs-only.
            if path.startswith(("docs/", "issue-spec/specs/")) or path in ("README.md", "AGENTS.md"):
                continue
            full = True
            reasons.append("possible embedded Markdown: " + path)
        elif path.endswith(("Cargo.toml", "Cargo.lock")):
            full = True
            reasons.append("package dependency change: " + path)
        elif path.startswith("crates/") and len(path.split("/")) > 2:
            owner = path.split("/")[1]
            if owner in OWNERS:
                groups.add(OWNERS[owner])
            else:
                full = True
                reasons.append("shared or unclassified package: " + owner)
        else:
            full = True
            reasons.append("shared tooling, contract, dependency or unknown path: " + path)
    if full:
        frontend = True
    commands = []
    if full or groups:
        commands.append(["cargo", "fmt", "--check"])
        packages = sorted({p for group in groups for p in GROUPS[group]})
        flags = ["--workspace", "--exclude", "hiroute-desktop"] if full else [x for p in packages for x in ("-p", p)]
        commands.append(["cargo", "clippy", "--locked", *flags, "--all-targets", "--all-features", "--", "-D", "warnings"])
        commands.append(["cargo", "test", "--locked", *flags, "--all-features"])
        if not full and groups & PRODUCT_GROUPS:
            commands.append(["cargo", "test", "--locked", "-p", "hiroute-product-e2e", "--test", "smoke_cli"])
        if full or "gateway" in groups:
            commands.append(["cargo", "test", "--locked", "-p", "hiroute-gateway-core", "--no-default-features",
                             "--test", "transport_loopback", "loopback_plain_http_smoke_reuses_h1_connection_and_joins_server",
                             "--", "--exact"])
        if full or "gateway" in groups:
            commands.append(["cargo", "run", "--locked", "-p", "hiroute-e2e", "--", "validate",
                             "--scenario", "e2e/scenarios/core-routing.json", "--profile", "e2e/profiles/local-process.json"])
    if targets and not full:
        if not groups:
            commands.append(["cargo", "fmt", "--check"])
        covered = {p for group in groups for p in GROUPS[group]}
        pending = {(p, t) for p, t in targets if p not in covered and
                   not (t == "smoke_cli" and groups & PRODUCT_GROUPS)}
        # #94: ACP enables preserve_order in the full backend graph. Keep this
        # known fixture-sensitive combination in focused runtime checks too.
        runtime_context = ("hiroute-e2e", "p0_gateway_runtime") in pending
        feature_flags = ["--features", "serde_json/preserve_order"]
        if runtime_context:
            reasons.append("Gateway runtime: check serde_json/preserve_order from the full backend context (#94)")
        for package in sorted({p for p, _ in pending}):
            commands.append(["cargo", "clippy", "--locked", "-p", package, "--all-targets", "--all-features"] +
                            (feature_flags if runtime_context and package == "hiroute-e2e" else []) + ["--", "-D", "warnings"])
        for package, target in sorted(pending):
            flags = ["--lib"] if target == "lib" else ["--test", target]
            commands.append(["cargo", "test", "--locked", "-p", package, "--all-features", *flags] +
                            (feature_flags if target == "p0_gateway_runtime" else []) +
                            (["--", "--test-threads=1"] if target in PROCESS_TARGETS else []))
    rust = bool(commands)
    if "diagnostics" in groups and not full:
        reasons.append("diagnostic implementation: expand if caller API, protocol, routing or Worker behavior changes")
        commands.append(["python3", "scripts/test-desktop-pilot.py"])
        if "crates/diagnostics/src/level.rs" in paths:
            commands.append(["cargo", "test", "--locked", "-p", "hiroute-diagnostics",
                             "--release", "--lib", "runtime::tests::unconfigured_runtime_uses_build_default_without_persisting",
                             "--", "--exact"])
    if selection_tooling:
        commands.append(["python3", "scripts/test-test-plan.py"])
    if validation_tooling:
        commands.extend([["python3", "scripts/" + name] for name in
                         ("test-validation.py", "test-validation-report.py", "test-local-rust.py", "test-remote-rust.py", "test-validation-schedule.py",
                          "test-ci-run.py", "test-ci-shards.py", "test-desktop-pilot.py", "test-pilot-builds.py")])
    if full or paths:
        commands.append(["python3", "scripts/test-contract-convergence.py"])
        commands.append(["python3", "scripts/check-contract-convergence.py"])
    return {"mode": "full" if full else "affected", "paths": sorted(set(paths)),
            "groups": sorted(groups), "reasons": reasons, "rust": rust,
            "frontend": frontend, "native_required": native, "commands": commands,
            "execution": {"remote_exclusive": full,
                          "feature_context": "Package selection may resolve different dependency features than workspace; preserve failing context when diagnosing."},
            "note": "Native Desktop, real accounts and fixed-machine performance are separate evidence."}


def integration_preflight(plan, collect_failures=False, context_plan=None):
    """Early known boundary checks; never subtract obligations from the final plan."""
    context_plan = plan if context_plan is None else context_plan
    targets = set()
    reasons = []
    for path in plan["paths"]:
        consumers = {t for p, t in e2e_consumers(path) if p == "hiroute-e2e"}
        selected = consumers & {"p0_gateway_matrix_coverage", "p0_gateway_protocol", "p0_gateway_runtime"}
        # These production adapters and the shared model IR feed the protocol matrix.
        # This only adds early checks; their conservative final package scope stays intact.
        if path.startswith(("crates/gateway/src/adapters/", "crates/gateway/src/model_ir/")):
            selected.update({"p0_gateway_protocol", "p0_gateway_matrix_coverage"})
        if path == "tools/e2e-harness/src/p0/coverage.rs":
            selected.add("p0_gateway_matrix_coverage")
        if selected:
            targets.update(selected)
            reasons.append("early boundary checks: " + path)
    # These already-selected cheap tooling gates run before Rust preparation.
    commands = [c[:] for c in plan["commands"] if c[:1] == ["python3"]]
    if targets:
        # Keep the final package selection for broad plans: package selection can
        # change dependency features, even when both commands say --all-features.
        broad = next((c for c in context_plan["commands"] if c[:2] == ["cargo", "test"]
                      and "--test" not in c and "--lib" not in c), None)
        context = broad[:] if broad else ["cargo", "test", "--locked", "-p", "hiroute-e2e", "--all-features"]
        if "--workspace" not in context and "hiroute-e2e" not in context:
            context.extend(["-p", "hiroute-e2e"])
        if "--workspace" not in context and "p0_gateway_runtime" in targets:
            context.extend(["--features", "serde_json/preserve_order"])
        # First expose stale generated receipts, before even short protocol/runtime tests.
        if "p0_gateway_matrix_coverage" in targets:
            commands.append([*context, "--test", "p0_gateway_matrix_coverage"])
        behavior = sorted(targets - {"p0_gateway_matrix_coverage"})
        if behavior:
            commands.append([*context, *(["--no-fail-fast"] if collect_failures else []),
                             *[arg for target in behavior for arg in ("--test", target)],
                             "--", "--test-threads=1"])
    return {"diagnostic_only": True, "targets": sorted(targets), "reasons": reasons,
            "commands": commands, "remote_exclusive": bool(targets) and context_plan["execution"]["remote_exclusive"],
            "collect_failures": collect_failures,
            "note": "Review this bounded scope before execution. Collection continues selected Cargo test executables only; build failures and unexecuted commands remain incomplete. Final obligations are unchanged; preflight does not authorize skipping or automatically repeating product smoke."}


def revision(ref):
    return subprocess.check_output(["git", "rev-parse", "--verify", "--end-of-options", ref + "^{commit}"], text=True).strip()


def changed_paths(base, since=False):
    # --no-renames includes both sides of moves; base is an exact commit from CI.
    resolved = revision(base)
    if since and subprocess.call(["git", "merge-base", "--is-ancestor", resolved, "HEAD"]):
        raise ValueError("--since must be an ancestor of HEAD")
    parent = resolved if since else subprocess.check_output(["git", "merge-base", "HEAD", resolved], text=True).strip()
    tracked = subprocess.check_output(["git", "diff", "--no-renames", "--name-only", "-z", parent, "--"])
    untracked = subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard", "-z"])
    return sorted({p.decode() for p in (tracked + untracked).split(b"\0") if p})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="Comparison ref/commit; includes staged, unstaged and untracked changes")
    parser.add_argument("--full", action="store_true")
    parser.add_argument("--since", help="Iteration change checkpoint (ancestor of HEAD); diagnostic only, never waives final checks")
    parser.add_argument("--integration", action="store_true", help="Print early boundary checks alongside the unchanged final plan")
    parser.add_argument("--collect-failures", action="store_true", help="Explicit bounded collection in integration preflight only")
    parser.add_argument("--github-output", action="store_true")
    parser.add_argument("--run-ci", action="store_true", help="Hosted execution only; local Agents use validation.py")
    args = parser.parse_args()
    if not args.full and not args.base:
        parser.error("pass --base or explicitly request --full")
    if args.since and (args.run_ci or args.github_output):
        parser.error("--since is diagnostic only; hosted CI always uses the final plan")
    if args.collect_failures and not args.integration:
        parser.error("--collect-failures requires --integration and review of its bounded scope")
    if args.integration and (args.run_ci or args.github_output):
        parser.error("--integration is a local planning view; hosted CI uses the final plan")
    try:
        plan = select(changed_paths(args.base) if args.base else [], args.full)
        plan["candidate"] = revision("HEAD")
        plan["base"] = revision(args.base) if args.base else None
        if args.since:
            plan["iteration"] = select(changed_paths(args.since, since=True))
            plan["iteration"].update(since=revision(args.since), diagnostic_only=True)
        if args.integration:
            scope = plan.get("iteration", plan)
            plan["integration"] = integration_preflight(scope, args.collect_failures, context_plan=plan)
            plan["integration"]["scope"] = "iteration" if args.since else "base"
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.error(str(error))
    print(json.dumps(plan, indent=2), flush=True)
    if args.github_output:
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            for key in ("rust", "frontend", "native_required"):
                output.write(key + "=" + str(plan[key]).lower() + "\n")
    if args.run_ci:
        if os.environ.get("GITHUB_ACTIONS") != "true":
            parser.error("--run-ci is for Actions; submit the printed Cargo commands through validation.py")
        for index, command in enumerate(plan["commands"]):
            argv = [sys.executable, str(Path(__file__).with_name("ci-run.py")), "--name", "check-" + str(index),
                    "--phase", "final"]
            if command[:2] == ["cargo", "test"]:
                argv.append("--require-tests")
            result = subprocess.run([*argv, "--", *command])
            if result.returncode:
                return result.returncode
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
