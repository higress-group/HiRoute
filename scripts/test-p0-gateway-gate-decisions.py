#!/usr/bin/env python3
"""Focused adversarial checks for the P0 Gateway Gate decision scripts."""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from types import ModuleType, SimpleNamespace


ROOT = Path(__file__).resolve().parent
REPOSITORY = ROOT.parent
REVISION = "a" * 40
HIROUTED_SHA256 = "b" * 64


def load_module(name: str, filename: str) -> ModuleType:
    specification = importlib.util.spec_from_file_location(name, ROOT / filename)
    if specification is None or specification.loader is None:
        raise RuntimeError(f"could not load {filename}")
    module = importlib.util.module_from_spec(specification)
    specification.loader.exec_module(module)
    return module


STABILITY = load_module("p0_gateway_stability", "check-p0-gateway-stability-result.py")
SUMMARY = load_module("p0_gateway_summary", "summarize-p0-gateway-benchmarks.py")


def green_stability_report() -> dict[str, object]:
    return {
        "schema_version": "hiroute.p0-gateway-production-stability/v2",
        "semantic_status": "green",
        "scenario": "sixty-second",
        "expected_duration_seconds": 60,
        "revision": REVISION,
        "hirouted": {
            "product_subprocess": True,
            "pid": 1234,
            "sha256": HIROUTED_SHA256,
            "path": "/fixed-runner/hirouted",
        },
        "provider_requests": 2,
        "response_status": 200,
        "long_stream_seconds": 60,
        "rss": {
            "baseline_kib": 1024,
            "peak_kib": 1024 + STABILITY.RSS_DELTA_CEILING_KIB,
            "delta_kib": STABILITY.RSS_DELTA_CEILING_KIB,
            "ceiling_kib": STABILITY.RSS_DELTA_CEILING_KIB,
        },
        "replay": {
            "peak_bytes": STABILITY.REPLAY_RETAINED_CAPACITY_CEILING_BYTES,
            "ceiling_bytes": STABILITY.REPLAY_RETAINED_CAPACITY_CEILING_BYTES,
            "terminal_bytes": 0,
        },
        "errors": [],
    }


def product_rows(variance: float, ci: float) -> str:
    rows: list[str] = []
    for variant, replay_peak in (("memory-retained", 0), ("disk-spill", 4096)):
        for round_number in range(1, 6):
            rows.append(
                "\t".join(
                    (
                        "gateway_product",
                        variant,
                        str(round_number),
                        "5",
                        "100.0",
                        "10",
                        "20",
                        "30",
                        "40",
                        str(variance),
                        str(ci),
                        ";".join(
                            (
                                "copy_bytes=500",
                                "scan_bytes=500",
                                "peak_rss_delta_kib=32",
                                f"peak_replay_bytes={replay_peak}",
                                "terminal_replay_bytes=0",
                                f"hirouted_sha256={HIROUTED_SHA256}",
                            )
                        ),
                    )
                )
            )
    return "\n".join(rows) + "\n"


def core_rows() -> str:
    rows: list[str] = []
    for protocol in ("h1", "h2"):
        for round_number in range(1, 6):
            rows.append(
                "\t".join(
                    (
                        protocol,
                        str(round_number),
                        "gateway-production-driver",
                        "100.0",
                        "10",
                        "20",
                        "30",
                        "40",
                        "1.0",
                        "1.0",
                    )
                )
            )
    return "\n".join(rows) + "\n"


def replay_rows() -> str:
    rows: list[str] = []
    for variant, spill_peak in (("memory-retained", 0), ("disk-spill", 4096)):
        for round_number in range(1, 6):
            rows.append(
                "\t".join(
                    (
                        "gateway_replay",
                        variant,
                        str(round_number),
                        "500",
                        "1000",
                        "200",
                        ";".join(
                            (
                                "copy_bytes=500",
                                "scan_bytes=500",
                                "peak_retained_bytes=500",
                                f"peak_spill_bytes={spill_peak}",
                                f"disk_backed={'true' if spill_peak else 'false'}",
                            )
                        ),
                    )
                )
            )
    return "\n".join(rows) + "\n"


def resource_rows(dynamic_values: list[int]) -> str:
    rows: list[str] = ["resource\tvariant\titerations\telapsed_ns\tns_per_operation\tdetail"]
    for dynamic_value in dynamic_values:
        rows.append(
            "\t".join(
                (
                    "executor",
                    "Compute-depth-1",
                    "10",
                    "1000",
                    "100",
                    ";".join(
                        (
                            f"queue_mean_ns={dynamic_value}",
                            f"run_mean_ns={dynamic_value}",
                            f"resume_mean_ns={dynamic_value}",
                            "static_work_units=10",
                        )
                    ),
                )
            )
        )
    return "\n".join(rows) + "\n"


def summary_arguments(directory: Path) -> SimpleNamespace:
    return SimpleNamespace(
        base_revision=REVISION,
        candidate_revision=REVISION,
        harness_revision=REVISION,
        base_core=directory / "base-core.tsv",
        candidate_core=directory / "candidate-core.tsv",
        base_resource=directory / "base-resource.tsv",
        candidate_resource=directory / "candidate-resource.tsv",
        base_replay=directory / "base-replay.tsv",
        candidate_replay=directory / "candidate-replay.tsv",
        base_product=directory / "base-product.tsv",
        candidate_product=directory / "candidate-product.tsv",
        base_hirouted_sha256=HIROUTED_SHA256,
        candidate_hirouted_sha256=HIROUTED_SHA256,
    )


def write_valid_summary_inputs(
    directory: Path,
    *,
    base_product: str,
    candidate_product: str,
    base_resource: str | None = None,
    candidate_resource: str | None = None,
) -> SimpleNamespace:
    for label in ("base", "candidate"):
        (directory / f"{label}-core.tsv").write_text(core_rows(), encoding="utf-8")
        (directory / f"{label}-replay.tsv").write_text(replay_rows(), encoding="utf-8")
    (directory / "base-product.tsv").write_text(base_product, encoding="utf-8")
    (directory / "candidate-product.tsv").write_text(candidate_product, encoding="utf-8")
    (directory / "base-resource.tsv").write_text(
        base_resource or resource_rows([100, 100, 100, 100, 100]), encoding="utf-8"
    )
    (directory / "candidate-resource.tsv").write_text(
        candidate_resource or resource_rows([100, 100, 100, 100, 100]), encoding="utf-8"
    )
    return summary_arguments(directory)


def canonical_digest(value: object) -> str:
    encoded = json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")
    return "sha256:" + hashlib.sha256(encoded).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def final_gate_fixture(root: Path) -> tuple[Path, dict[str, object], str]:
    """Build a small independently sealed v1 result fixture for checker tests."""

    e2e_root = root / "e2e"
    seal_revision = "c" * 40
    aggregate_port_digest = "sha256:" + "d" * 64
    check_ids = [
        "launch_production",
        "readiness_exact",
        "listener_client",
        "native_provider",
        "lifecycle_stream",
        "execution_stream",
        "content_stream",
        "otel_stream",
        "privacy_resource",
        "aggregate_port",
    ]
    checkpoint_evidence = {
        "listener_client": check_ids[:3],
        "native_provider": ["native_provider"],
        "semantic_observation": check_ids[4:8] + ["aggregate_port"],
        "privacy_resource": ["privacy_resource"],
    }
    digest_schema = {"type": "string", "minLength": 71, "maxLength": 71}
    check_schema = {
        "type": "object",
        "additionalProperties": False,
        "required": ["id", "status", "expected_digest", "actual_digest", "detail"],
        "properties": {
            "id": {"type": "string", "minLength": 1},
            "status": {"enum": ["passed", "failed"]},
            "expected_digest": digest_schema,
            "actual_digest": digest_schema,
            "detail": {"type": "string", "minLength": 1},
        },
    }
    checkpoint_schema = {
        "type": "object",
        "additionalProperties": False,
        "required": ["id", "status", "evidence"],
        "properties": {
            "id": {"enum": list(checkpoint_evidence)},
            "status": {"enum": ["passed", "failed"]},
            "evidence": {"type": "array", "minItems": 1, "uniqueItems": True},
        },
    }
    result_properties = {
        "schema_version": {"const": "hiroute.e2e.production-result/v1"},
        "oracle_version": {"const": "p0-gateway-production-oracle-v1"},
        "contract_digest": digest_schema,
        "aggregate_port_digest": digest_schema,
        "schema_digests": {"type": "object", "minProperties": 1},
        "evidence_digest": digest_schema,
        "result_payload_digest": digest_schema,
        "process_exit": {
            "type": "object",
            "additionalProperties": False,
            "required": ["test_process_code", "sut_termination", "sut_reaped"],
            "properties": {
                "test_process_code": {"enum": [0, 1]},
                "sut_termination": {"const": "harness_initiated_after_independent_terminals"},
                "sut_reaped": {"const": True},
            },
        },
        "scenario_state": {"enum": ["green", "red"]},
        "run_nonce": {"type": "string", "minLength": 32},
        "freshness_challenge": {"type": "string", "minLength": 32},
        "launcher": {"type": "object", "minProperties": 3},
        "readiness": {"type": "object", "minProperties": 1},
        "native_provider": {"type": "object", "minProperties": 1},
        "client_output": {"type": "object", "minProperties": 1},
        "observations": {"type": "object", "minProperties": 1},
        "privacy": {"type": "object", "minProperties": 4},
        "checks": {
            "type": "array",
            "minItems": len(check_ids),
            "maxItems": len(check_ids),
            "uniqueItems": True,
            "items": check_schema,
        },
        "checkpoints": {
            "type": "array",
            "minItems": len(checkpoint_evidence),
            "maxItems": len(checkpoint_evidence),
            "uniqueItems": True,
            "items": checkpoint_schema,
        },
    }
    schemas = {
        "schema/p0-production-manifest.schema.json": {
            "type": "object",
            "additionalProperties": False,
            "required": ["schema_version", "oracle_version", "contract_digest", "artifacts"],
            "properties": {
                "schema_version": {"const": "hiroute.e2e.production-manifest/v1"},
                "oracle_version": {"const": "p0-gateway-production-oracle-v1"},
                "contract_digest": digest_schema,
                "artifacts": {"type": "array", "minItems": 1, "uniqueItems": True},
            },
        },
        "schema/p0-gateway-profile.schema.json": {
            "type": "object",
            "additionalProperties": False,
            "required": ["sut_source_revision", "aggregate_port_digest"],
            "properties": {
                "sut_source_revision": {"const": seal_revision},
                "aggregate_port_digest": {"const": aggregate_port_digest},
            },
        },
        "schema/p0-gateway-scenario.schema.json": {
            "type": "object",
            "additionalProperties": False,
            "required": ["coverage", "checkpoints"],
            "properties": {
                "coverage": {"type": "array", "minItems": 1, "maxItems": 1},
                "checkpoints": {"type": "array", "minItems": 4, "maxItems": 4},
            },
        },
        "schema/p0-gateway-result.schema.json": {
            "type": "object",
            "additionalProperties": False,
            "required": list(result_properties),
            "properties": result_properties,
        },
        "schema/p0-production-launcher.schema.json": {
            "type": "object",
            "additionalProperties": False,
            "required": ["sut_source_revision", "executable_sha256", "build_attestation"],
            "properties": {
                "sut_source_revision": {"const": seal_revision},
                "executable_sha256": digest_schema,
                "build_attestation": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["source_revision", "executable_sha256"],
                    "properties": {
                        "source_revision": {"const": seal_revision},
                        "executable_sha256": digest_schema,
                    },
                },
            },
        },
        "schema/p0-production-readiness.schema.json": {
            "type": "object",
            "additionalProperties": False,
            "required": ["product"],
            "properties": {
                "product": {
                    "type": "object",
                    "additionalProperties": False,
                    "required": ["executable_sha256"],
                    "properties": {"executable_sha256": digest_schema},
                },
            },
        },
        "schema/p0-production-collector.schema.json": {
            "type": "object",
            "additionalProperties": False,
            "required": [
                "independent_streams",
                "content_terminal_independent",
                "lifecycle",
                "execution_fact",
                "conversation_content",
                "otel",
            ],
            "properties": {
                "independent_streams": {"const": True},
                "content_terminal_independent": {"const": True},
                "lifecycle": {"type": "object", "minProperties": 1},
                "execution_fact": {"type": "object", "minProperties": 1},
                "conversation_content": {"type": "object", "minProperties": 1},
                "otel": {"type": "object", "minProperties": 1},
            },
        },
        "schema/p0-production-fixture.schema.json": {"type": "object", "minProperties": 1},
    }
    profile = {
        "sut_source_revision": seal_revision,
        "aggregate_port_digest": aggregate_port_digest,
    }
    scenario = {
        "coverage": [{"id": "exact", "evidence": check_ids}],
        "checkpoints": [
            {"id": key, "evidence": evidence}
            for key, evidence in checkpoint_evidence.items()
        ],
    }
    fixture = {
        "case": {
            "providers": [
                {
                    "id": "sealed-provider",
                    "protocol": "responses",
                    "expected_calls": 1,
                    "expected_request": {
                        "path": "/v1/responses",
                        "body": {
                            "model": "sealed-model",
                            "input": "${RUN_CHALLENGE}",
                            "stream": False,
                        },
                    },
                }
            ]
        },
        "expected_client": {"status": 200, "challenge": "${RUN_CHALLENGE}"},
    }
    write_json(e2e_root / "fixtures/p0-oracle/production-smoke.json", fixture)
    write_json(e2e_root / "profiles/gateway-isolated.json", profile)
    write_json(e2e_root / "scenarios/p0-gateway.json", scenario)
    for relative, schema in schemas.items():
        write_json(e2e_root / relative, schema)

    artifact_schema_bindings = {
        "fixtures/p0-oracle/production-smoke.json": "schema/p0-production-fixture.schema.json",
        "profiles/gateway-isolated.json": "schema/p0-gateway-profile.schema.json",
        "scenarios/p0-gateway.json": "schema/p0-gateway-scenario.schema.json",
        **{relative: None for relative in schemas},
    }
    artifacts = []
    for relative, schema in artifact_schema_bindings.items():
        value = json.loads((e2e_root / relative).read_text(encoding="utf-8"))
        artifacts.append({"path": relative, "schema": schema, "sha256": canonical_digest(value)})
    contract_digest = canonical_digest(
        {
            "schema_version": "hiroute.e2e.production-contract-digest/v1",
            "aggregate_port_digest": aggregate_port_digest,
            "artifacts": [{"path": item["path"], "sha256": item["sha256"]} for item in artifacts],
        }
    )
    manifest = {
        "schema_version": "hiroute.e2e.production-manifest/v1",
        "oracle_version": "p0-gateway-production-oracle-v1",
        "contract_digest": contract_digest,
        "artifacts": artifacts,
    }
    write_json(e2e_root / "schema/p0-production-manifest.json", manifest)
    schema_digests = {
        relative: canonical_digest(json.loads((e2e_root / relative).read_text(encoding="utf-8")))
        for relative in schemas
    }
    check_digest = "sha256:" + "e" * 64
    challenge = "f" * 32
    native_body = {
        "model": "sealed-model",
        "input": challenge,
        "stream": False,
    }
    native_provider = {
        "entries": [
            {
                "authorization": "exact",
                "body_digest": canonical_digest(native_body),
                "body_semantics": {
                    "content_digest": canonical_digest(challenge),
                    "instructions_digest": None,
                    "model": "sealed-model",
                    "reasoning_digest": None,
                    "stream": False,
                    "tool_choice_digest": None,
                    "tools_digest": None,
                },
                "content_type": "application/json",
                "method": "POST",
                "ordinal": 1,
                "path": "/v1/responses",
                "protocol": "responses",
                "provider_id": "sealed-provider",
                "body": native_body,
            }
        ],
        "accepted": 1,
        "active": 0,
        "parse_failed": 0,
        "aborted": 0,
        "infrastructure_failures": [],
    }
    report: dict[str, object] = {
        "schema_version": "hiroute.e2e.production-result/v1",
        "oracle_version": "p0-gateway-production-oracle-v1",
        "contract_digest": contract_digest,
        "aggregate_port_digest": aggregate_port_digest,
        "schema_digests": schema_digests,
        "evidence_digest": "",
        "result_payload_digest": "",
        "process_exit": {
            "test_process_code": 0,
            "sut_termination": "harness_initiated_after_independent_terminals",
            "sut_reaped": True,
        },
        "scenario_state": "green",
        "run_nonce": "r" * 32,
        "freshness_challenge": challenge,
        "launcher": {
            "sut_source_revision": seal_revision,
            "executable_sha256": check_digest,
            "build_attestation": {
                "source_revision": seal_revision,
                "executable_sha256": check_digest,
            },
        },
        "readiness": {"product": {"executable_sha256": check_digest}},
        "native_provider": native_provider,
        "client_output": {"status": 200, "challenge": challenge},
        "observations": {
            "independent_streams": True,
            "content_terminal_independent": True,
            "lifecycle": {"records": 7},
            "execution_fact": {"records": 14},
            "conversation_content": {"records": 7},
            "otel": {"records": 2},
        },
        "privacy": {
            "private_root_mode": "owner_only",
            "observation_root_mode": "owner_only",
            "process_root_mode": "owner_only",
            "sensitive_occurrences": 0,
        },
        "checks": [
            {
                "id": identifier,
                "status": "passed",
                "expected_digest": check_digest,
                "actual_digest": check_digest,
                "detail": identifier,
            }
            for identifier in check_ids
        ],
        "checkpoints": [
            {"id": identifier, "status": "passed", "evidence": evidence}
            for identifier, evidence in checkpoint_evidence.items()
        ],
    }
    refresh_final_report(report)
    return e2e_root, report, seal_revision


def refresh_final_report(report: dict[str, object]) -> None:
    observations = report["observations"]
    assert isinstance(observations, dict)
    aggregate_port_digest = report["aggregate_port_digest"]
    check_values = {
        "launch_production": (report["launcher"], report["launcher"]),
        "readiness_exact": (report["readiness"], report["readiness"]),
        "listener_client": (report["client_output"], report["client_output"]),
        "native_provider": (report["native_provider"], report["native_provider"]),
        "lifecycle_stream": (observations["lifecycle"], observations["lifecycle"]),
        "execution_stream": (observations["execution_fact"], observations["execution_fact"]),
        "content_stream": (observations["conversation_content"], observations["conversation_content"]),
        "otel_stream": (observations["otel"], observations["otel"]),
        "privacy_resource": (
            {
                "private_root_mode": "owner_only",
                "observation_root_mode": "owner_only",
                "process_root_mode": "owner_only",
                "sensitive_occurrences": 0,
            },
            report["privacy"],
        ),
        "aggregate_port": (
            {
                "port_digest": aggregate_port_digest,
                "independent_streams": True,
                "content_terminal_independent": True,
            },
            {
                "port_digest": aggregate_port_digest,
                "independent_streams": observations["independent_streams"],
                "content_terminal_independent": observations["content_terminal_independent"],
            },
        ),
    }
    checks = report["checks"]
    assert isinstance(checks, list)
    for check in checks:
        assert isinstance(check, dict)
        expected, actual = check_values[check["id"]]
        check["expected_digest"] = canonical_digest(expected)
        check["actual_digest"] = canonical_digest(actual)
    report["evidence_digest"] = canonical_digest(
        {
            key: report[key]
            for key in (
                "launcher",
                "readiness",
                "native_provider",
                "client_output",
                "observations",
                "privacy",
            )
        }
    )
    report["result_payload_digest"] = ""
    report["result_payload_digest"] = canonical_digest(report)


def run_final_checker(root: Path, e2e_root: Path, report: dict[str, object], revision: str) -> subprocess.CompletedProcess[str]:
    result = root / "result.json"
    write_json(result, report)
    return subprocess.run(
        [
            str(ROOT / "check-p0-gateway-final-result.sh"),
            "--result",
            str(result),
            "--expected-revision",
            revision,
            "--e2e-root",
            str(e2e_root),
        ],
        check=False,
        capture_output=True,
        text=True,
    )


class GateDecisionTests(unittest.TestCase):
    def test_stability_rejects_forged_resource_ceiling(self) -> None:
        arguments = SimpleNamespace(
            scenario="sixty-second",
            revision=REVISION,
            hirouted_sha256=HIROUTED_SHA256,
            driver_process_exit=0,
        )
        self.assertEqual(STABILITY.validate(arguments, green_stability_report()), [])

        forged = copy.deepcopy(green_stability_report())
        forged["rss"] = {
            "baseline_kib": 1024,
            "peak_kib": 100_001_024,
            "delta_kib": 100_000_000,
            "ceiling_kib": 100_000_000,
        }
        forged["replay"] = {
            "peak_bytes": 1_000_000_000,
            "ceiling_bytes": 1_000_000_000,
            "terminal_bytes": 0,
        }
        errors = STABILITY.validate(arguments, forged)
        self.assertTrue(any("trusted 64 MiB" in error for error in errors))
        self.assertTrue(any("trusted 8 MiB" in error for error in errors))

    def test_product_dispersion_is_a_semantic_gate(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            payload = SUMMARY.run(
                write_valid_summary_inputs(
                    directory,
                    base_product=product_rows(1.0, 1.0),
                    candidate_product=product_rows(1000.0, 1000.0),
                )
            )
        self.assertEqual(payload["semantic_status"], "red")
        self.assertTrue(
            any("request_sample_variance_ns2" in error for error in payload["errors"])
        )
        self.assertTrue(any("request_mean_ci95_ns" in error for error in payload["errors"]))

    def test_resource_scheduler_timing_is_not_compared_as_exact_static_data(self) -> None:
        # These are two five-run slices of one unchanged ten-run scheduler sample.
        # Their means differ slightly, which the former exact-equality check rejected.
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            payload = SUMMARY.run(
                write_valid_summary_inputs(
                    directory,
                    base_product=product_rows(1.0, 1.0),
                    candidate_product=product_rows(1.0, 1.0),
                    base_resource=resource_rows([100, 101, 99, 100, 100]),
                    candidate_resource=resource_rows([101, 100, 102, 100, 101]),
                )
            )
        self.assertEqual(payload["semantic_status"], "green")
        scheduler = payload["resource_paths"]["comparison"]["executor:Compute-depth-1"]["detail"][
            "queue_mean_ns"
        ]
        self.assertGreater(scheduler["observed_regression_pct"], 0.0)
        self.assertLess(scheduler["confidence_aware_regression_pct"], SUMMARY.THRESHOLD_PERCENT)

    def test_final_checker_accepts_a_complete_sealed_production_result_v1(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            e2e_root, report, seal_revision = final_gate_fixture(directory)
            checked = run_final_checker(directory, e2e_root, report, seal_revision)
        self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)
        self.assertEqual(json.loads(checked.stdout)["semantic_status"], "green")

    def test_final_checker_rejects_wrong_schema_digest_revision_and_zero_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            e2e_root, valid, seal_revision = final_gate_fixture(directory)

            wrong_schema = copy.deepcopy(valid)
            wrong_schema["schema_version"] = "hiroute.e2e.legacy-result/v1"
            refresh_final_report(wrong_schema)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, wrong_schema, seal_revision).returncode,
                0,
            )

            wrong_digest = copy.deepcopy(valid)
            wrong_digest["evidence_digest"] = "sha256:" + "0" * 64
            refresh_final_report(wrong_digest)
            wrong_digest["evidence_digest"] = "sha256:" + "0" * 64
            wrong_digest["result_payload_digest"] = ""
            wrong_digest["result_payload_digest"] = canonical_digest(wrong_digest)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, wrong_digest, seal_revision).returncode,
                0,
            )

            wrong_revision = copy.deepcopy(valid)
            launcher = wrong_revision["launcher"]
            assert isinstance(launcher, dict)
            launcher["sut_source_revision"] = REVISION
            attestation = launcher["build_attestation"]
            assert isinstance(attestation, dict)
            attestation["source_revision"] = REVISION
            refresh_final_report(wrong_revision)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, wrong_revision, seal_revision).returncode,
                0,
            )

            zero_evidence = copy.deepcopy(valid)
            zero_evidence["checks"] = []
            zero_evidence["checkpoints"] = []
            refresh_final_report(zero_evidence)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, zero_evidence, seal_revision).returncode,
                0,
            )

            zero_native = copy.deepcopy(valid)
            zero_native["native_provider"] = {
                "entries": [],
                "accepted": 0,
                "active": 0,
                "parse_failed": 0,
                "aborted": 0,
                "infrastructure_failures": [],
            }
            refresh_final_report(zero_native)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, zero_native, seal_revision).returncode,
                0,
            )

            fabricated_client = copy.deepcopy(valid)
            fabricated_client["client_output"] = {
                "status": 0,
                "challenge": "not-the-fresh-sealed-challenge",
            }
            refresh_final_report(fabricated_client)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, fabricated_client, seal_revision).returncode,
                0,
            )

    def test_final_checker_rejects_filtered_and_skipped_production_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            directory = Path(temporary_directory)
            e2e_root, valid, seal_revision = final_gate_fixture(directory)

            filtered = copy.deepcopy(valid)
            checks = filtered["checks"]
            assert isinstance(checks, list)
            filtered["checks"] = checks[:-1]
            refresh_final_report(filtered)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, filtered, seal_revision).returncode,
                0,
            )

            skipped = copy.deepcopy(valid)
            skipped_checks = skipped["checks"]
            assert isinstance(skipped_checks, list)
            first = skipped_checks[0]
            assert isinstance(first, dict)
            first["status"] = "skipped"
            refresh_final_report(skipped)
            self.assertNotEqual(
                run_final_checker(directory, e2e_root, skipped, seal_revision).returncode,
                0,
            )

    def test_hosted_workflow_join_requires_all_current_results(self) -> None:
        final_workflow = (REPOSITORY / ".github/workflows/p0-gateway-final-gates.yml").read_text(
            encoding="utf-8"
        )
        portable_workflow = (REPOSITORY / ".github/workflows/p0-gateway-gates.yml").read_text(
            encoding="utf-8"
        )
        dedicated_workflow = (
            REPOSITORY / ".github/workflows/gateway-core-dedicated.yml"
        ).read_text(encoding="utf-8")
        # The current hosted suite is separate from the sealed historical Gate.
        # Keep the real sealed-result rejection cases above, and bind this join
        # to the three workflows documented in github-actions-validation.md.
        self.assertIn("workflow_call:", portable_workflow)
        self.assertIn("workflow_call:", dedicated_workflow)
        for workflow in ("gateway-core.yml", "p0-gateway-gates.yml", "gateway-core-dedicated.yml"):
            self.assertIn("uses: ./.github/workflows/" + workflow, final_workflow)
        self.assertIn("needs: [backend, gateway, stability]", final_workflow)
        self.assertIn("if: always()", final_workflow)
        for job in ("backend", "gateway", "stability"):
            self.assertIn("needs." + job + ".result", final_workflow)
            self.assertIn('test "$' + job.upper() + '" = success', final_workflow)
        self.assertIn("--test p0_gateway_replay", portable_workflow)
        self.assertIn("--test p0_gateway_observation", portable_workflow)
        self.assertIn("--test p0_gateway_privacy", portable_workflow)
        self.assertIn("--scenario sixty-second", dedicated_workflow)
        self.assertIn("--scenario ten-minute", dedicated_workflow)
        self.assertIn("HIROUTE_CONNECTION_STRESS_SECONDS: '600'", dedicated_workflow)
        self.assertIn("HIROUTE_STRESS_CONNECTIONS: 1000,10000", dedicated_workflow)
        self.assertIn("dedicated_1k_10k_sse_connection_churn_for_ten_minutes", dedicated_workflow)
        self.assertIn("--require-tests", dedicated_workflow)
        self.assertIn("--ignored --exact --nocapture", dedicated_workflow)


if __name__ == "__main__":
    unittest.main()
