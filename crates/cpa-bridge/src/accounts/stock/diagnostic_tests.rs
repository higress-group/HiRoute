use super::*;
use hiroute_diagnostics::event::{CpaControlPhase, CpaStageOutcome, ProcessRole};
use hiroute_diagnostics::level::DiagnosticLevel;
use hiroute_diagnostics::record::Component;
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

fn diagnostics(root: &Path) -> DiagnosticRuntime {
    DiagnosticRuntime::start(RuntimeConfig {
        root: root.to_owned(),
        role: ProcessRole::Daemon,
        component: Component::Cpa,
        parent_session_id: None,
        level_override: Some(DiagnosticLevel::Debug),
    })
}

#[test]
fn each_unmet_pin_condition_is_reported_without_per_poll_logging() {
    for (fault, field, expected) in [
        (1, "account_models_nonempty", serde_json::json!(false)),
        (2, "account_models_prefixed", serde_json::json!(false)),
        (3, "catalog_missing_count", serde_json::json!(1)),
        (4, "account_index_matches", serde_json::json!(false)),
        (5, "account_retry_zero", serde_json::json!(false)),
        (6, "persisted_prefix_matches", serde_json::json!(false)),
        (7, "persisted_retry_zero", serde_json::json!(false)),
        (8, "persisted_cooling_disabled", serde_json::json!(false)),
        (9, "account_model_ids_valid", serde_json::json!(false)),
    ] {
        let fixture = Fixture::new();
        fixture.fault.store(fault, Ordering::SeqCst);
        let path = fixture.root.path().join("codex-a.json");
        let mut controls: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match fault {
            6 => controls["prefix"] = serde_json::json!("wrong"),
            7 => controls["request_retry"] = serde_json::json!(1),
            8 => controls["disable_cooling"] = serde_json::json!(false),
            _ => (),
        }
        fs::write(path, controls.to_string()).unwrap();
        let logs = crate::private_test_support::tempdir().unwrap();
        let runtime = diagnostics(logs.path());
        let identity = ManagedAccountIdentity {
            account_kind: CpaAccountKind::Codex,
            stock_file_name: "codex-a.json".into(),
            account_digest: "a".repeat(64),
            generation: 1,
            client_version: None,
        };
        let result = StockCpaControlPlane.discover_and_pin(
            fixture.address,
            fixture.root.path(),
            &[identity],
            &InstanceSecrets::generate().unwrap(),
            "8.0.4-hiroute.4",
            Duration::from_millis(150),
            false,
            &runtime.port().handle().context(),
        );
        // The shared deadline may expire between polls or during the last local
        // read. The observed unmet condition must survive either terminal error.
        assert!(result.is_err(), "fault {fault}: {result:?}");
        runtime.shutdown();
        let text = fs::read_to_string(logs.path().join("daemon/current.jsonl")).unwrap();
        let events: Vec<Value> = text
            .lines()
            .filter_map(|line| {
                let value: Value = serde_json::from_str(line).unwrap();
                value["event"]["cpa_control"].is_object().then_some(value)
            })
            .collect();
        let failed = events
            .iter()
            .find(|event| event["event"]["cpa_control"]["outcome"]["failed"].is_object())
            .unwrap();
        let control = &failed["event"]["cpa_control"];
        assert_eq!(
            control["checks"][field], expected,
            "fault {fault}: {control}"
        );
        assert_eq!(
            control["phase"],
            if fault == 3 {
                "catalog_pin"
            } else {
                "account_pin"
            }
        );
        assert!(control["polls"].as_u64().unwrap() > 1);
        assert_eq!(failed["level"], "warn");
        assert!(events.len() <= 10, "per-poll logging: {}", events.len());
        assert!(
            events
                .iter()
                .all(|event| event["span_id"] == failed["span_id"])
        );
        for forbidden in [
            "codex-a.json",
            "idx-a",
            "future-model",
            "other-prefix",
            "Bearer",
        ] {
            assert!(!text.contains(forbidden), "diagnostic leaked {forbidden}");
        }
        if fault == 1 {
            assert_eq!(control["checks"]["account_index_matches"], true);
            assert_eq!(control["checks"]["account_retry_zero"], true);
            for field in [
                "persisted_prefix_matches",
                "persisted_retry_zero",
                "persisted_cooling_disabled",
            ] {
                assert_eq!(control["checks"][field], true);
            }
            assert_eq!(fixture.patches.load(Ordering::SeqCst), 0);
        }
        if fault == 9 {
            assert_eq!(control["checks"]["account_models_nonempty"], true);
            assert_eq!(control["checks"]["account_models_prefixed"], true);
        }
    }
}

#[test]
fn slow_patch_is_distinct_from_unmet_pin_checks() {
    let fixture = Fixture::new();
    fixture.patch_delay_ms.store(300, Ordering::SeqCst);
    let logs = crate::private_test_support::tempdir().unwrap();
    let runtime = diagnostics(logs.path());
    let identity = ManagedAccountIdentity {
        account_kind: CpaAccountKind::Codex,
        stock_file_name: "codex-a.json".into(),
        account_digest: "a".repeat(64),
        generation: 1,
        client_version: None,
    };
    let result = StockCpaControlPlane.discover_and_pin(
        fixture.address,
        fixture.root.path(),
        &[identity],
        &InstanceSecrets::generate().unwrap(),
        "8.0.4-hiroute.4",
        Duration::from_millis(100),
        true,
        &runtime.port().handle().context(),
    );
    assert!(result.is_err());
    runtime.shutdown();
    let log = fs::read_to_string(logs.path().join("daemon/current.jsonl")).unwrap();
    let failed: Value = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| event["event"]["cpa_control"]["outcome"]["failed"].is_object())
        .unwrap();
    assert_eq!(failed["event"]["cpa_control"]["phase"], "controls_patch");
    assert_eq!(
        failed["event"]["cpa_control"]["outcome"]["failed"]["code"],
        "control_timeout"
    );
    assert_eq!(failed["event"]["cpa_control"]["polls"], 0);
    assert!(
        failed["event"]["cpa_control"]["checks"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}

#[test]
#[ignore = "requires a private HIROUTE_CPA_PIN_SMOKE manifest and an isolated real CPA"]
fn live_managed_cpa_discovery_diagnostics() {
    let manifest = std::env::var("HIROUTE_CPA_PIN_SMOKE").expect("smoke manifest required");
    let value: Value = serde_json::from_slice(&fs::read(manifest).unwrap()).unwrap();
    let field = |key: &str| value[key].as_str().unwrap();
    let runtime = diagnostics(Path::new(field("diagnostics_root")));
    let identity = ManagedAccountIdentity {
        account_kind: CpaAccountKind::Codex,
        stock_file_name: field("stock_file_name").into(),
        account_digest: "a".repeat(64),
        generation: 1,
        client_version: value["client_version"].as_str().map(str::to_owned),
    };
    let result = StockCpaControlPlane.discover_and_pin(
        field("address").parse().unwrap(),
        Path::new(field("auth_dir")),
        &[identity],
        &InstanceSecrets::read(Path::new(field("capability_file"))).unwrap(),
        field("version"),
        crate::MANAGED_CPA_CONTROL_TIMEOUT,
        value["refresh_models"].as_bool().unwrap(),
        &runtime.port().handle().context(),
    );
    runtime.shutdown();
    let log = fs::read_to_string(Path::new(field("diagnostics_root")).join("daemon/current.jsonl"))
        .unwrap();
    let events: Vec<hiroute_diagnostics::record::DiagnosticRecordV1> = log
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    if value["expect_empty"].as_bool().unwrap() {
        assert!(result.is_err());
        assert!(events.iter().any(|event| matches!(&event.event,
            hiroute_diagnostics::event::DiagnosticEvent::CpaControl(control)
            if control.phase == CpaControlPhase::AccountPin
                && matches!(control.outcome, CpaStageOutcome::Failed { .. })
                && control.checks.account_models_nonempty == Some(false)
                && control.checks.account_index_matches == Some(true)
                && control.checks.account_retry_zero == Some(true)
        )));
    } else {
        let accounts = result.unwrap();
        assert_eq!(accounts.len(), 1);
        assert!(!accounts[0].observed_model_ids.is_empty());
    }
}
