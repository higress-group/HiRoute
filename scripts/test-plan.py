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
WORKER_FIXTURE_TOOLING = {"scripts/test-agent-product-support.py",
                          "scripts/test-native-context-product.py",
                          "scripts/test-native-context-boundaries.py",
                          "scripts/test-qoder-product.py"}
WORKER_BOOTSTRAP_TEST = "crates/daemon/src/delegation/profile/claude_adapter_bootstrap.test.mjs"
INTEGRATION_SKILL = ".agents/skills/hiroute-integrate/SKILL.md"
WEBSITE_TOOLING = {".github/workflows/website.yml", ".github/workflows/release.yml"}
WEBSITE_PREFIXES = ("apps/website/", ".github/scripts/")
RELEASE_CONTRACT_TOOLING = {"scripts/release-contracts.py", "scripts/test-release-contracts.py",
                          "scripts/test-release-contract-pr.py", ".github/scripts/release-contract-pr.sh",
                          ".github/workflows/release.yml"}
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

# Known shared Worker contracts consumed by the feature-gated Tauri bridge and
# protected Session. Keep this bounded: shared-library location is not sufficient
# to require Desktop compilation. Review new consumers when a contract moves.
DESKTOP_WORKER_CONTRACTS = {
    "crates/application-api/src/worker.rs",
    "crates/domain/src/delegation/mod.rs",
    "crates/domain/src/delegation/installation.rs",
}


def desktop_compile_checks(paths):
    triggers = sorted({path for path in paths if path in DESKTOP_WORKER_CONTRACTS
                       or path.startswith("apps/desktop/src-tauri/")})
    if not triggers:
        return []
    return [{
        "id": "desktop.compile", "platform": "macos", "trigger_paths": triggers,
        "guide": "docs/validation-routing.md#native-consumer-compilation",
        # --frontend-dist uses the existing Pilot TAURI_CONFIG, whose ACL requires
        # desktop-pilot. That feature also enables desktop-runtime and its bridge.
        "command_template": ["python3", "scripts/validation.py",
                             "--frontend-dist", "<candidate-frontend-dist>", "desktop", "run",
                             "--ref", "<pushed-branch-ref>", "--sha", "<candidate-sha>",
                             "--plan", "<feature-plan>", "--phase", "focused", "--cargo-only", "--",
                             "cargo", "check", "--locked", "-p", "hiroute-desktop",
                             "--features", "desktop-pilot", "--all-targets"],
        "required_inputs": {"pushed-branch-ref": "Advertised refs/heads/... containing the candidate",
                            "candidate-sha": "Exact committed candidate being validated",
                            "candidate-frontend-dist": "Same-candidate built frontend directory on the Mac",
                            "feature-plan": "Current validation plan; include related runs when applicable"},
        "evidence_limit": "Compile production and test consumers on the configured Mac; "
                          "does not execute tests or prove native product behavior.",
    }]


# One registration per explicit product entry. The ordinary commands only compile ignored
# targets; these obligations require real installations and separate business verdicts.
WORKER_PRODUCT_CHECKS = {
    "worker_native_context": {
        "id": "worker.native-context", "harnesses": ["codex", "claude"],
        "guide": "tools/product-e2e/tests/WORKER_NATIVE_CONTEXT.md",
        "business_cases": ["worker.context." + name for name in (
            "native-skills", "concurrent-routing", "run-authority", "exact-continue",
            "cancel-owned-work", "retention-ownership")],
        "automated_cases": ["worker.context.native-skills", "worker.context.exact-continue",
                            "worker.context.concurrent-routing", "worker.context.cancel-owned-work"],
    },
    "worker_delegation": {
        "id": "worker.lifecycle", "harnesses": ["codex", "claude"],
        "guide": "docs/code-map/worker-context.md",
    },
    "worker_read": {
        "id": "worker.read-continue", "harnesses": ["codex", "claude"],
        "guide": "docs/code-map/worker-context.md",
    },
    "qoder_delegation": {
        "id": "qoder.delegation", "harnesses": ["qoder"],
        "guide": "tools/product-e2e/tests/QODER_DELEGATION.md",
        "required_tests": [
            "qoder_main_agent_uses_installed_user_skill_to_delegate_real_work",
            "qoder_worker_uses_native_skills_and_continues_the_frozen_task",
            "qoder_workers_route_independently_and_cancel_only_owned_work",
            "qoder_worker_compaction_keeps_the_frozen_managed_route",
            "qoder_main_agent_uses_persisted_additional_model_routes"],
        "required_environment": ["HIROUTE_PRODUCT_CANDIDATE_SHA", "HIROUTE_WORKER_QODER_BINARY",
                                 "HIROUTE_QODER_CONTEXT_HOME", "HIROUTE_QODER_CONFIG_DIR",
                                 "HIROUTE_QODER_MODEL_CONTEXT_HOME", "HIROUTE_QODER_MODEL_CONFIG_DIR"],
        "missing_environment": "fail",
    },
    "pi_delegation": {
        "id": "pi.delegation", "harnesses": ["pi"],
        "guide": "tools/product-e2e/tests/PI_INTEGRATION.md",
        "required_tests": [
            "pi_worker_uses_native_skills_and_continues_the_frozen_task",
            "pi_workers_route_independently_and_reject_missing_or_corrupt_history",
            "pi_agent_saved_model_routes_preserve_defaults_and_restore_independently",
            "pi_agent_uses_installed_user_skill_to_delegate_through_public_cli",
            "pi_worker_compaction_uses_frozen_route_and_continues_exact_history",
            "pi_static_api_discovery_imports_effective_source_and_rejects_stale_save"],
        "required_environment": ["HIROUTE_PRODUCT_CANDIDATE_SHA", "HIROUTE_WORKER_PI_BINARY", "HIROUTE_WORKER_NODE"],
        "missing_environment": "fail",
    },
}
OPT_IN_WORKER_TARGETS = set(WORKER_PRODUCT_CHECKS)
PRODUCT_GUIDES = {entry["guide"] for entry in WORKER_PRODUCT_CHECKS.values()} | {
    "crates/daemon/tests/support/NATIVE_CONTEXT_BOUNDARIES.md"}

# Qoder-only leaves and the shared settings/Skill boundaries consumed by its main-Agent
# journey. Model-specific Codex/Claude settings do not select unrelated native journeys.
QODER_PRODUCT_PREFIXES = (
    "crates/integrations/src/agents/qoder", "crates/daemon/src/delegation/profile/qoder",
    "crates/daemon/src/control/runtime/settings_facts/qoder",
    "crates/application/src/agent_connection/skill",
    "crates/application/src/agent_connection/qoder",
    "crates/daemon/src/control/runtime/qoder",
    "crates/domain/src/agents/qoder",
    "crates/application/src/agent_connection/settings/",
)
QODER_PRODUCT_PATHS = {
    "crates/daemon/src/control/runtime.rs",
    "crates/application-api/src/agent_settings.rs",
    "crates/domain/src/agents/profile.rs",
    "crates/integrations/src/agents/registry.rs",
    "crates/daemon/src/control/runtime/agent_connection.rs",
    "crates/daemon/src/control/runtime/settings_facts.rs",
    "crates/daemon/src/control/runtime/settings_status.rs",
    "crates/daemon/src/control/runtime/collaboration_installation.rs",
    "crates/daemon/src/control/runtime/collaboration_status.rs",
    "crates/daemon/src/control/runtime/settings_entry_qoder_tests.rs",
    "crates/application/src/agent_connection/settings.rs",
    "crates/application/src/agent_connection/settings_input.rs",
    "crates/application/src/control_plane/agent_settings.rs",
    "crates/integrations/src/agents/filesystem.rs",
    "crates/integrations/src/agents/settings_discovery.rs",
    "crates/integrations/src/agents/executable.rs",
}
PI_PRODUCT_PREFIXES = (
    "crates/integrations/src/agents/pi", "crates/daemon/src/delegation/profile/pi",
    "crates/daemon/src/control/runtime/model_connections/pi",
    "crates/integrations/src/agents/filesystem_tests/pi",
)
ADDITIONAL_MODEL_PREFIXES = (
    "crates/integrations/src/agents/additional_native",
    "crates/application/src/agent_connection/additional_model",
    "crates/domain/src/agents/additional_model",
    "crates/daemon/src/control/runtime/native_additional_model",
    "crates/daemon/src/control/runtime/additional_model",
    "crates/daemon/src/control/runtime/settings_facts/additional_model",
)
ADDITIONAL_NATIVE_FIXTURE_PREFIXES = (
    "crates/daemon/tests/support/additional_model_",
    "crates/daemon/tests/support/collaboration_",
    "crates/daemon/tests/support/native_compaction_",
)
# These helpers also serve publication/model journeys. Add the known real Worker gates
# without pretending their complete ordinary-test dependency graph is bounded here.
SHARED_WORKER_FIXTURES = {
    "crates/daemon/tests/support/delegation_product.py",
    "crates/daemon/tests/support/publication_product.py",
}


def e2e_consumers(path):
    """Bounded test/fixture ownership, not a guessed production dependency graph."""
    if path.startswith("tools/product-e2e/tests/worker_product_support/"):
        return [("hiroute-product-e2e", name) for name in WORKER_PRODUCT_CHECKS]
    if path.startswith("tools/product-e2e/tests/") and Path(path).stem in OPT_IN_WORKER_TARGETS:
        return [("hiroute-product-e2e", Path(path).stem)]
    if path.startswith("crates/daemon/tests/support/native_context_"):
        return [("hiroute-product-e2e", name) for name in ("worker_native_context", "qoder_delegation", "pi_delegation")]
    if path == "crates/daemon/tests/support/agent_product_support.py" or path.startswith(ADDITIONAL_NATIVE_FIXTURE_PREFIXES):
        return [("hiroute-product-e2e", name) for name in ("qoder_delegation", "pi_delegation")]
    if path.startswith("crates/daemon/tests/support/pi_"):
        return [("hiroute-product-e2e", "pi_delegation")]
    if path.startswith("crates/daemon/tests/support/qoder_"):
        return [("hiroute-product-e2e", "qoder_delegation")]
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


def worker_product_checks(paths):
    """Explicit opt-in obligations; ordinary Cargo success does not run ignored Agents."""
    selected = set()
    for path in paths:
        if path.endswith(".md"):
            continue
        selected.update(target for package, target in e2e_consumers(path)
                        if package == "hiroute-product-e2e" and target in OPT_IN_WORKER_TARGETS)
        if path.startswith(PI_PRODUCT_PREFIXES):
            selected.add("pi_delegation")
        elif path.startswith(ADDITIONAL_MODEL_PREFIXES):
            selected.update(("qoder_delegation", "pi_delegation"))
        elif path in QODER_PRODUCT_PATHS or path.startswith(QODER_PRODUCT_PREFIXES):
            selected.add("qoder_delegation")
            if path in QODER_PRODUCT_PATHS or path.startswith(("crates/application/src/agent_connection/settings/", "crates/application/src/agent_connection/skill")):
                selected.add("pi_delegation")
        elif path.startswith(("crates/daemon/src/delegation/", "crates/application/src/delegation/",
                              "crates/domain/src/delegation/")) or path in SHARED_WORKER_FIXTURES:
            selected.update(OPT_IN_WORKER_TARGETS)
    return [{**entry,
             "command": ["cargo", "test", "--locked", "-p", "hiroute-product-e2e",
                         "--test", target, "--", "--ignored", "--nocapture", "--test-threads=1"],
             "evidence_limit": "Real installed Harnesses with controlled upstream; native Desktop "
                               "and unexecuted component contracts need their own evidence."}
            for target, entry in WORKER_PRODUCT_CHECKS.items() if target in selected]


def select(paths, full=False):
    groups = set()
    frontend = full
    native_checks = desktop_compile_checks(paths)
    native = bool(native_checks)
    reasons = []
    selection_tooling = False
    worker_fixture_tooling = False
    validation_tooling = False
    release_contract_tooling = False
    targets = set()
    for path in sorted(set(paths)):
        if path in PRODUCT_GUIDES:
            continue  # Navigation-only guide; not embedded in a product binary.
        if path in WORKER_FIXTURE_TOOLING:
            worker_fixture_tooling = True
            continue
        if path in RELEASE_CONTRACT_TOOLING or path.startswith("contracts/releases/"):
            release_contract_tooling = True
            continue
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
            commands.append(["cargo", "test", "--locked", "-p", "hiroute-e2e", "--test", "case_shards",
                             "core_routing_validation_covers_the_complete_unsharded_scenario",
                             "--", "--exact", "--nocapture"])
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
            if package == "hiroute-product-e2e" and target in OPT_IN_WORKER_TARGETS:
                # Clippy above compiles all consumers. Running an ignored-only target
                # normally would produce zero executed tests, not product acceptance.
                continue
            flags = ["--lib"] if target == "lib" else ["--test", target]
            commands.append(["cargo", "test", "--locked", "-p", package, "--all-features", *flags] +
                            (feature_flags if target == "p0_gateway_runtime" else []) +
                            (["--", "--test-threads=1"] if target in PROCESS_TARGETS else []))
    rust = bool(commands)
    if full or any(path.startswith(("assets/agent-profiles/", "assets/release-facts/",
                                    "tools/release-facts/"))
                   or path == "crates/integrations/src/agents/registry.rs"
                   for path in paths):
        # Rust catalog tests use builtin profiles; the Python bundle producer
        # also consumes profile-seed.json. Check that path before integration.
        commands.append(["python3", "assets/release-facts/current/prepare-bundle.py", "--check"])
    if "diagnostics" in groups and not full:
        reasons.append("diagnostic implementation: expand if caller API, protocol, routing or Worker behavior changes")
        commands.append(["python3", "scripts/test-desktop-pilot.py"])
        if "crates/diagnostics/src/level.rs" in paths:
            commands.append(["cargo", "test", "--locked", "-p", "hiroute-diagnostics",
                             "--release", "--lib", "runtime::tests::unconfigured_runtime_uses_build_default_without_persisting",
                             "--", "--exact"])
    if selection_tooling:
        commands.append(["python3", "scripts/test-test-plan.py"])
    product_checks = worker_product_checks(paths)
    if full or worker_fixture_tooling or product_checks:
        commands.append(["python3", "scripts/test-agent-product-support.py"])
        commands.append(["python3", "scripts/test-native-context-product.py"])
        commands.append(["python3", "scripts/test-native-context-boundaries.py"])
    if full or worker_fixture_tooling or any(check["id"] == "qoder.delegation" for check in product_checks):
        commands.append(["python3", "scripts/test-qoder-product.py"])
    if full or any(path.startswith("crates/daemon/src/delegation/profile/") for path in paths):
        commands.append(["node", "--test", WORKER_BOOTSTRAP_TEST])
    if full or release_contract_tooling:
        commands.extend([["python3", "scripts/test-release-contracts.py"],
                         ["python3", "scripts/test-release-contract-pr.py"],
                         ["python3", "scripts/release-contracts.py", "check"]])
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
            "native_checks": native_checks,
            "product_checks": product_checks,
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
    storage_startup = any(path.startswith("crates/local-storage/src/migrations/")
                          or path == "crates/local-storage/src/lib.rs"
                          for path in plan["paths"])
    # These already-selected cheap tooling gates run before Rust preparation.
    commands = [c[:] for c in plan["commands"]
                if c[:1] == ["python3"] or c[:2] == ["node", "--test"]]
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
    if storage_startup:
        targets.update({"publication_process", "pre_gateway_compute_routing"})
        reasons.append("storage startup: exercise the production daemon initialization order")
        broad = next((c for c in context_plan["commands"] if c[:2] == ["cargo", "test"]
                      and "--test" not in c and "--lib" not in c), None)
        context = broad[:] if broad else ["cargo", "test", "--locked", "-p", "hiroute-daemon", "--all-features"]
        if "--workspace" not in context and "hiroute-daemon" not in context:
            context.extend(["-p", "hiroute-daemon"])
        if "--workspace" not in context and "hiroute-product-e2e" not in context:
            context.extend(["-p", "hiroute-product-e2e"])
        commands.append([*context, "--test", "publication_process", "--test", "pre_gateway_compute_routing"])
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
