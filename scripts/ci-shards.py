#!/usr/bin/env python3
"""Partition a full hosted backend plan without weakening its Cargo coverage."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parent.parent
FULL_TEST = [
    "cargo", "test", "--locked", "--workspace", "--exclude", "hiroute-desktop",
    "--all-features",
]

# Integration targets are explicit so a new target cannot silently land in an
# arbitrary shard. Targets with the same Cargo name stay together because a
# workspace-level `--test NAME` selects every matching package target.
INTEGRATION_SHARDS = {
    "integration-foundation": {
        ("hiroute-application-api", "pre_gateway_compute_routing"),
        ("hiroute-daemon", "pre_gateway_compute_routing"),
        ("hiroute-product-e2e", "pre_gateway_compute_routing"),
        ("hiroute-domain", "canonical_digest"),
        ("hiroute-domain", "compute"),
        ("hiroute-client-core", "transport"),
        ("hiroute-diagnostics", "files_safety"),
        ("hiroute-diagnostics", "worker_probe_progress"),
        ("hiroute-diagnostics", "writer_bounds"),
        ("hiroute-cli", "contracts"),
        ("hiroute-cli", "desktop_locator"),
        ("hiroute-cli", "work_plans"),
        ("hiroute-cli", "worker_dependencies"),
        ("hiroute-integrations", "agent_profiles"),
        ("hiroute-integrations", "current_model_metadata"),
        ("hiroute-integrations", "model_connections"),
        ("hiroute-integrations", "pre_gateway_compute"),
        ("hiroute-integrations", "release_facts"),
        ("hiroute-local-storage", "canonical_compat"),
    },
    "integration-daemon": {
        ("hiroute-daemon", "discovered_model_product"),
        ("hiroute-daemon", "local_worker_platform"),
        ("hiroute-daemon", "publication_process"),
        ("hiroute-daemon", "subscription_management_product"),
    },
    "integration-gateway": {
        ("hiroute-gateway-core", "attempt_exchange"),
        ("hiroute-gateway-core", "body_memory"),
        ("hiroute-gateway-core", "body_memory_and_framing"),
        ("hiroute-gateway-core", "component_boundaries"),
        ("hiroute-gateway-core", "filter_state_machine"),
        ("hiroute-gateway-core", "gateway_lifecycle"),
        ("hiroute-gateway-core", "gateway_stability"),
        ("hiroute-gateway-core", "publication_and_configuration"),
        ("hiroute-gateway-core", "routing_contracts"),
        ("hiroute-gateway-core", "sse_streaming"),
        ("hiroute-gateway-core", "telemetry_and_privacy"),
        ("hiroute-gateway-core", "transport_loopback"),
        ("hiroute-e2e", "case_shards"),
        ("hiroute-e2e", "p0_gateway_commit_boundary"),
        ("hiroute-e2e", "p0_gateway_context_hold"),
        ("hiroute-e2e", "p0_gateway_matrix_coverage"),
        ("hiroute-e2e", "p0_gateway_observation"),
        ("hiroute-e2e", "p0_gateway_oracle"),
        ("hiroute-e2e", "p0_gateway_planner"),
        ("hiroute-e2e", "p0_gateway_privacy"),
        ("hiroute-e2e", "p0_gateway_protocol"),
        ("hiroute-e2e", "p0_gateway_replay"),
        ("hiroute-e2e", "p0_gateway_request_authority"),
        ("hiroute-e2e", "p0_gateway_runtime"),
    },
    "integration-product": {
        ("hiroute-product-e2e", "agent_connection"),
        ("hiroute-product-e2e", "bundled_release_data"),
        ("hiroute-product-e2e", "compute_pool"),
        ("hiroute-product-e2e", "control_shell"),
        ("hiroute-product-e2e", "cpa_connector"),
        ("hiroute-product-e2e", "gateway_adapter_contract"),
        ("hiroute-product-e2e", "local_observation"),
        ("hiroute-product-e2e", "product_oracle"),
        ("hiroute-product-e2e", "routing_plans"),
        ("hiroute-product-e2e", "transaction_recovery"),
        ("hiroute-product-e2e", "worker_delegation"),
        ("hiroute-product-e2e", "worker_read"),
    },
    "integration-smoke": {
        ("hiroute-product-e2e", "smoke_cli"),
    },
}

FULL_SHARDS = [
    "quality",
    "unit",
    "integration-foundation",
    "integration-daemon",
    "integration-gateway",
    "integration-product",
    "integration-smoke",
    "gates",
]


def load_test_plan():
    path = Path(__file__).with_name("test-plan.py")
    spec = importlib.util.spec_from_file_location("hiroute_test_plan", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def cargo_metadata(root=ROOT):
    output = subprocess.check_output(
        ["cargo", "metadata", "--format-version=1", "--no-deps", "--locked"],
        cwd=root,
        text=True,
    )
    return json.loads(output)


def integration_targets(metadata):
    members = set(metadata["workspace_members"])
    targets = set()
    unsupported = []
    for package in metadata["packages"]:
        if package["id"] not in members or package["name"] == "hiroute-desktop":
            continue
        for target in package["targets"]:
            kinds = set(target["kind"])
            if "test" in kinds:
                targets.add((package["name"], target["name"]))
            elif target.get("test") and not kinds & {"lib", "proc-macro", "bin"}:
                unsupported.append((package["name"], target["name"], sorted(kinds)))
    if unsupported:
        raise ValueError("test-enabled targets need an explicit coverage rule: " + repr(unsupported))
    return targets


def verify_assignments(metadata):
    actual = integration_targets(metadata)
    owners = {}
    for shard, entries in INTEGRATION_SHARDS.items():
        for entry in entries:
            if entry in owners:
                raise ValueError(f"duplicate integration target assignment: {entry!r}")
            owners[entry] = shard
    expected = set(owners)
    missing = sorted(actual - expected)
    extra = sorted(expected - actual)
    if missing or extra:
        raise ValueError(f"integration target assignments differ: missing={missing!r}, extra={extra!r}")

    by_name = {}
    for (package, name), shard in owners.items():
        by_name.setdefault(name, set()).add(shard)
    split_names = {name: sorted(shards) for name, shards in by_name.items() if len(shards) != 1}
    if split_names:
        raise ValueError("same-name workspace targets cannot span shards: " + repr(split_names))
    return {shard: sorted({name for _, name in entries})
            for shard, entries in INTEGRATION_SHARDS.items()}


def matrix(plan):
    if not plan.get("rust"):
        return {"include": []}
    shards = FULL_SHARDS if plan.get("mode") == "full" else ["affected"]
    return {"include": [{"shard": shard} for shard in shards]}


def full_commands(plan, metadata):
    commands = plan.get("commands")
    if not isinstance(commands, list) or commands.count(FULL_TEST) != 1:
        raise ValueError("full plan no longer has exactly one canonical workspace test command")
    names = verify_assignments(metadata)
    quality = [command for command in commands
               if command[:2] in (["cargo", "fmt"], ["cargo", "clippy"])]
    remainder = [command for command in commands if command != FULL_TEST and command not in quality]
    if not quality or not remainder:
        raise ValueError("full plan no longer has both quality and explicit gate commands")

    result = {
        "quality": quality,
        "unit": [[*FULL_TEST, "--lib", "--bins"]],
        "gates": [[*FULL_TEST, "--doc"], *remainder],
    }
    for shard, targets in names.items():
        result[shard] = [[*FULL_TEST, *[arg for name in targets for arg in ("--test", name)]]]
    if set(result) != set(FULL_SHARDS):
        raise ValueError("full shard command set is incomplete")
    return result


def plan_for_args(base=None, full=False):
    planner = load_test_plan()
    paths = planner.changed_paths(base) if base else []
    result = planner.select(paths, full=full)
    result["candidate"] = planner.revision("HEAD")
    result["base"] = planner.revision(base) if base else None
    return result


def run_commands(shard, commands):
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise ValueError("shard execution is restricted to GitHub Actions")
    runner = Path(__file__).with_name("ci-run.py")
    for index, command in enumerate(commands):
        argv = [sys.executable, str(runner), "--name", f"{shard}-{index}", "--phase", "final"]
        if command[:2] == ["cargo", "test"]:
            argv.append("--require-tests")
        completed = subprocess.run([*argv, "--", *command])
        if completed.returncode:
            return completed.returncode
    return 0


def write_matrix(value, github_output=False):
    encoded = json.dumps(value, separators=(",", ":"))
    print(encoded)
    if github_output:
        output = os.environ.get("GITHUB_OUTPUT")
        if not output:
            raise ValueError("GITHUB_OUTPUT is required with --github-output")
        with open(output, "a") as stream:
            stream.write("matrix=" + encoded + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="action", required=True)
    check = subparsers.add_parser("check")
    check.add_argument("--root", type=Path, default=ROOT)
    matrix_parser = subparsers.add_parser("matrix")
    matrix_parser.add_argument("--plan", type=Path, required=True)
    matrix_parser.add_argument("--github-output", action="store_true")
    run = subparsers.add_parser("run")
    run.add_argument("--shard", required=True)
    scope = run.add_mutually_exclusive_group(required=True)
    scope.add_argument("--base")
    scope.add_argument("--full", action="store_true")
    args = parser.parse_args()

    try:
        if args.action == "check":
            metadata = cargo_metadata(args.root)
            names = verify_assignments(metadata)
            print(json.dumps({"integration_targets": len(integration_targets(metadata)),
                              "selector_names": sum(map(len, names.values())),
                              "shards": {key: len(value) for key, value in names.items()}},
                             sort_keys=True))
            return 0
        if args.action == "matrix":
            write_matrix(matrix(json.loads(args.plan.read_text())), args.github_output)
            return 0

        plan = plan_for_args(args.base, args.full)
        selected = [entry["shard"] for entry in matrix(plan)["include"]]
        if args.shard not in selected:
            raise ValueError(f"shard {args.shard!r} is not selected; expected {selected!r}")
        if args.shard == "affected":
            commands = plan["commands"]
        else:
            commands = full_commands(plan, cargo_metadata())[args.shard]
        return run_commands(args.shard, commands)
    except (OSError, ValueError, subprocess.CalledProcessError, json.JSONDecodeError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    raise SystemExit(main())
