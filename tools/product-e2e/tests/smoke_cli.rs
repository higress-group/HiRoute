#![cfg(unix)]

use hiroute_product_e2e::smoke::{self, Execution, State};
use serde_json::Value;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, atomic::AtomicBool};

fn smoke_checkout_test_lock() -> MutexGuard<'static, ()> {
    // These named tests are shardable only by giving each remote invocation its
    // own checkout. A single checkout deliberately has one smoke writer so
    // private recovery artifacts and the production runner cannot overlap.
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn run_smoke(args: &[&str]) -> (bool, Value, smoke::Report) {
    run_smoke_with_preparation(args, false)
}

fn run_smoke_with_preparation(args: &[&str], prepared: bool) -> (bool, Value, smoke::Report) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hiroute-smoke"));
    command.arg("run").args(args);
    if prepared {
        command.env("HIROUTE_SMOKE_REQUIRE_PREPARED", "1");
    }
    let output = command.output().unwrap();
    let success = output.status.success();
    assert!(
        !output.stdout.is_empty(),
        "no structured smoke result: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let report: smoke::Report = serde_json::from_value(value["result"].clone()).unwrap();
    report.verify().unwrap();
    (success, value, report)
}

fn assert_executed_green(report: &smoke::Report, expected: usize) {
    assert_eq!(report.actual_case_count, expected);
    assert!(
        report
            .cases
            .iter()
            .all(|case| case.execution == Execution::Executed && case.state == Some(State::Green))
    );
}

#[test]
#[ignore = "build-only preparation for managed workspace scheduling"]
fn prepare_default_smoke_builds() {
    let _lock = smoke_checkout_test_lock();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = smoke::prepare(&root, Arc::new(AtomicBool::new(false))).unwrap();
    let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(value["scenarios_executed"], 0);
    assert!(value.get("scenario_state").is_none());
    println!("build preparation={}", path.display());
}

#[test]
#[ignore = "explicit cache-miss regression; never overlap a real checkout smoke"]
fn prepared_candidate_miss_fails_without_building() {
    let _lock = smoke_checkout_test_lock();
    let output = Command::new(env!("CARGO_BIN_EXE_hiroute-smoke"))
        .args(["run", "--case", "control.status-discovery"])
        .env("HIROUTE_SMOKE_REQUIRE_PREPARED", "1")
        // A fresh recipe with an otherwise inert Cargo-prefixed variable ensures
        // this check works even if the candidate already has prepared artifacts.
        .env(
            "CARGO_HIROUTE_MISSING_RECEIPT_TEST",
            std::process::id().to_string(),
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    let report: smoke::Report = serde_json::from_value(value["result"].clone()).unwrap();
    report.verify().unwrap();
    assert_eq!(report.cases[0].state, Some(State::Red));
    assert_eq!(
        report.cases[0].reason.as_deref(),
        Some("control_build:prepared_build_missing")
    );
    assert!(report.artifacts.is_empty());
}

#[test]
fn default_two_domain_smoke() {
    let _lock = smoke_checkout_test_lock();
    // The remote workbench runs this exact test after building the committed candidate.
    // No fixture fallback: a missing/failed production case fails the Rust process too.
    let (quick_success, value, report) = run_smoke(&[]);
    // Keep semantic verdicts visible alongside the outer Rust test process exit.
    for case in &report.cases {
        println!(
            "smoke case={} state={:?} scope={}",
            case.id, case.state, case.scope
        );
    }
    println!("smoke report={}", value["report"]);
    assert_executed_green(&report, 3);
    let discovery = &report.cases[1];
    assert_eq!(discovery.id, "control.status-discovery");
    assert_eq!(
        discovery.scope,
        "production_cli_released_commands/staged_local_control/synthetic_agent_discovery"
    );
    assert_eq!(
        discovery
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>(),
        [
            "control_build",
            "control_storage_prepare",
            "control_ready",
            "system_status",
            "client_service_status",
            "operation_idempotency_lookup",
            "agents_scan",
            "agents_list",
            "control_cleanup",
        ]
    );
    let embedded = &report.cases[2];
    assert_eq!(embedded.id, "control.embedded-catalog");
    assert_eq!(
        embedded.scope,
        "production_cli_released_commands/embedded_catalog/storage_tampering_ignored"
    );
    assert_eq!(
        embedded
            .steps
            .iter()
            .map(|step| step.id.as_str())
            .collect::<Vec<_>>(),
        [
            "control_build",
            "control_storage_prepare",
            "embedded_catalog_ready",
            "storage_catalog_ignored",
            "embedded_catalog_cleanup",
        ]
    );
    assert_eq!(report.cases[0].id, "gateway.responses.controlled");
    assert_eq!(
        report.cases[0].scope,
        "production_gateway/controlled_upstream/protocol_client"
    );
    assert_eq!(
        report
            .cases
            .iter()
            .map(|c| c.domain.as_str())
            .collect::<Vec<_>>(),
        ["gateway", "control", "control"]
    );
    for case in &report.cases {
        if !case.timing_complete {
            assert_eq!(case.execution_ms, 0);
            assert!(case.unclassified_ms > 0);
        } else {
            assert_eq!(case.unclassified_ms, 0);
        }
    }
    // Both control journeys must actually finish cold setup through the verified
    // daemon before the ordinary startup/CLI steps. Keep its cost visible without
    // adding a field to the strict report schema or classifying it as compilation.
    let private = std::path::Path::new(value["report"].as_str().unwrap())
        .parent()
        .unwrap()
        .join("private");
    let daemon = report
        .artifacts
        .iter()
        .find(|a| a.binary == "hirouted")
        .unwrap();
    for case in [&report.cases[1], &report.cases[2]] {
        let bytes =
            std::fs::read(private.join(&case.id).join("control-storage-prepare.json")).unwrap();
        let prepared: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(prepared["role"], "control");
        assert_eq!(prepared["executable_sha256"], daemon.sha256);
        assert_eq!(prepared["outcome"], "completed");
        assert!(prepared["reason"].is_null());
        assert_eq!(prepared["timeout_ms"], 45_000);
        let elapsed = prepared["elapsed_ms"].as_u64().unwrap();
        assert!(elapsed < 45_000);
        assert!(u128::from(elapsed) <= case.execution_ms);
        use sha2::{Digest, Sha256};
        assert_eq!(
            case.steps[1].evidence_digest,
            format!("sha256:{:x}", Sha256::digest(&bytes))
        );
    }
    assert_eq!(report.scenario_timeout_ms, 30_000);
    assert_eq!(report.step_timeout_ms, 10_000);
    // A real report loses structural validity when required observed steps disappear.
    let mut incomplete = report.clone();
    incomplete.cases[1].steps.pop();
    assert!(incomplete.verify().is_err());
    let mut missing_preparation = report.clone();
    missing_preparation.cases[1].steps.remove(1);
    assert!(missing_preparation.verify().is_err());
    let mut wrong_scope = report.clone();
    wrong_scope.cases[0].scope = "desktop".into();
    assert!(wrong_scope.verify().is_err());
    assert!(
        quick_success,
        "production smoke contains a red/unexecuted case; see per-scenario reports"
    );
}

/// Explicit benchmark of one exact candidate in one managed checkout. The normal
/// short set still runs the three product cases only once.
#[test]
#[ignore = "run cold and warm production smoke in one retained candidate checkout"]
fn repeated_candidate_reuses_attested_builds_and_keeps_fresh_scenarios() {
    let _lock = smoke_checkout_test_lock();
    let (cold_ok, cold_value, cold) = run_smoke(&[]);
    let (warm_ok, warm_value, warm) = run_smoke_with_preparation(&[], true);
    assert!(cold_ok && warm_ok);
    assert_executed_green(&cold, 3);
    assert_executed_green(&warm, 3);
    assert_ne!(cold.run_id, warm.run_id);
    assert_eq!(cold.source_revision, warm.source_revision);
    assert_eq!(cold.artifacts.len(), 3);
    assert_eq!(cold.artifacts.len(), warm.artifacts.len());
    for (first, second) in cold.artifacts.iter().zip(&warm.artifacts) {
        assert_eq!(
            (first.package.as_str(), first.binary.as_str()),
            (second.package.as_str(), second.binary.as_str())
        );
        assert_eq!(first.sha256, second.sha256);
        assert_eq!(
            first.build_argv, second.build_argv,
            "a new random build nonce means this artifact was rebuilt"
        );
    }
    let gateway_build = |value: &Value| {
        let report = std::path::Path::new(value["report"].as_str().unwrap());
        let detail = report
            .parent()
            .unwrap()
            .join("private/gateway.responses.controlled/gateway-current.json");
        let detail: Value = serde_json::from_slice(&std::fs::read(detail).unwrap()).unwrap();
        detail["report"]["launcher"]["build_attestation"]["build_nonce"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        gateway_build(&cold_value),
        gateway_build(&warm_value),
        "Gateway SUT was rebuilt instead of reusing its verified binary"
    );
    println!(
        "cold_build_ms={} warm_build_ms={}",
        cold.cases.iter().map(|case| case.build_ms).sum::<u128>(),
        warm.cases.iter().map(|case| case.build_ms).sum::<u128>()
    );
    // A changed staged binary must fail, even when its recipe receipt still exists.
    let smoke_root = std::path::Path::new(warm_value["report"].as_str().unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let staged = std::fs::read_dir(smoke_root.join("builds"))
        .unwrap()
        .filter_map(|entry| {
            let receipt = entry.ok()?.path().join("hiroute-cli/receipt.json");
            let value: Value = serde_json::from_slice(&std::fs::read(receipt).ok()?).ok()?;
            if value["artifact"]["sha256"]
                == warm
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.package == "hiroute-cli")?
                    .sha256
            {
                value["path"].as_str().map(std::path::PathBuf::from)
            } else {
                None
            }
        })
        .next()
        .expect("CLI build receipt must exist");
    struct Restore(std::path::PathBuf, Vec<u8>);
    impl Drop for Restore {
        fn drop(&mut self) {
            std::fs::write(&self.0, &self.1).unwrap();
        }
    }
    let original = Restore(staged.clone(), std::fs::read(&staged).unwrap());
    std::fs::write(&staged, b"changed after the attested build").unwrap();
    let (changed_ok, _, changed) = run_smoke(&["--case", "control.status-discovery"]);
    drop(original);
    assert!(!changed_ok);
    assert_eq!(changed.cases[0].state, Some(State::Red));
    assert!(
        changed.cases[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("artifact_changed")
    );
    // A scheduled execution must not silently rebuild a missing prepared artifact.
    struct RestoreDirectory(std::path::PathBuf, std::path::PathBuf);
    impl Drop for RestoreDirectory {
        fn drop(&mut self) {
            std::fs::rename(&self.1, &self.0).unwrap();
        }
    }
    let original = staged.parent().unwrap().to_path_buf();
    let held = original.with_file_name("held-cli-build");
    std::fs::rename(&original, &held).unwrap();
    let restore = RestoreDirectory(original, held);
    let (missing_ok, _, missing) =
        run_smoke_with_preparation(&["--case", "control.status-discovery"], true);
    drop(restore);
    assert!(!missing_ok);
    assert_eq!(missing.cases[0].state, Some(State::Red));
    assert!(
        missing.cases[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("prepared_build_missing")
    );
}

#[test]
fn unknown_case_is_nonzero_and_validation_never_claims_product_green() {
    let output = Command::new(env!("CARGO_BIN_EXE_hiroute-smoke"))
        .args(["run", "--case", "control.stauts"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_hiroute-smoke"))
        .args(["validate", "--domain", "control"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["validation_only"], true);
    assert_eq!(value["cases"].as_array().unwrap().len(), 2);
    assert!(value.get("scenario_state").is_none());
    for id in [
        "gateway.responses.controlled",
        "control.status-discovery",
        "control.embedded-catalog",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_hiroute-smoke"))
            .args(["validate", "--case", id])
            .output()
            .unwrap();
        assert!(output.status.success());
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["validation_only"], true);
        assert_eq!(value["cases"].as_array().unwrap().len(), 1);
        assert_eq!(value["cases"][0]["id"], id);
    }
}

#[test]
fn absent_agent_adapter_is_unexecuted_and_cannot_be_promoted_by_result_editing() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("target")).unwrap();
    let (_, mut report) = smoke::run(
        temp.path(),
        vec!["agent".into()],
        vec![],
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(report.tool_process_exit, 1);
    assert_eq!(report.actual_case_count, 0);
    assert!(
        report
            .cases
            .iter()
            .all(|c| c.execution == Execution::NotExecuted && c.state.is_none())
    );
    report.verify().unwrap();
    let original = report.clone();
    report.cases[0].state = Some(State::ExpectedRed);
    assert!(report.verify().is_err());
    report = original.clone();
    report.cases.pop();
    assert!(report.verify().is_err());
    report = original.clone();
    report.cases[1] = report.cases[0].clone();
    assert!(report.verify().is_err());
    report = original;
    report.tool_process_exit = 0;
    assert!(report.verify().is_err());
}
