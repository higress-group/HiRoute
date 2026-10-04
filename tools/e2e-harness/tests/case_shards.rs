use serde_json::Value;
use std::{path::PathBuf, process::Command};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn validate(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hiroute-e2e"))
        .args(arguments)
        .env_remove("HIROUTE_E2E_CPA_BIN")
        .env_remove("HIROUTE_E2E_SUT_BIN")
        .output()
        .unwrap()
}

#[test]
fn core_routing_validation_covers_the_complete_unsharded_scenario() {
    let root = root();
    let scenario = root.join("e2e/scenarios/core-routing.json");
    let profile = root.join("e2e/profiles/local-process.json");
    let declared: Value = serde_json::from_slice(&std::fs::read(&scenario).unwrap()).unwrap();
    let output = validate(&[
        "validate",
        "--scenario",
        scenario.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["scenario"], "dual-protocol-rule-routing");
    assert_eq!(report["profile"], "local-process");
    assert!(report.get("selected_case").is_none());
    assert_eq!(report["steps"], 17);
    assert_eq!(report["case_shards"].as_array().unwrap().len(), 10);
    assert_eq!(report["case_shards"], declared["case_shards"]);
    assert_eq!(
        report["protocols"],
        serde_json::json!(["messages", "responses"])
    );
    assert_eq!(
        report["sources"],
        serde_json::json!(["claude-compatible", "codex-chatgpt"])
    );
    assert_eq!(
        report["executable_environment_required"],
        serde_json::json!(["HIROUTE_E2E_CPA_BIN", "HIROUTE_E2E_SUT_BIN"])
    );

    // A semantic error in the final step must not disappear behind a default shard
    // or successful JSON parsing. Validation never starts CPA or SUT processes.
    let mut invalid = declared;
    invalid["steps"].as_array_mut().unwrap().last_mut().unwrap()["request"]["stream"] =
        Value::Bool(false);
    let temporary = tempfile::tempdir().unwrap();
    let invalid_path = temporary.path().join("invalid-core-routing.json");
    std::fs::write(&invalid_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
    let rejected = validate(&[
        "validate",
        "--scenario",
        invalid_path.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
    ]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("streaming response"));
    println!("{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn generic_case_validation_selects_one_declared_partition() {
    let root = root();
    let scenario = root.join("e2e/scenarios/core-routing.json");
    let profile = root.join("e2e/profiles/local-process.json");
    let output = validate(&[
        "validate",
        "--scenario",
        scenario.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
        "--case",
        "responses-complex-continuation",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["selected_case"], "responses-complex-continuation");
    assert_eq!(report["steps"], 2);
    assert_eq!(
        report["case_shards"],
        serde_json::json!([{
            "id": "responses-complex-continuation",
            "steps": [
                "responses-complex-to-codex",
                "responses-tool-continuation-affinity-hit"
            ]
        }])
    );

    let unknown = validate(&[
        "validate",
        "--scenario",
        scenario.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
        "--case",
        "not-a-case",
    ]);
    assert!(!unknown.status.success());
}

#[test]
fn sealed_production_scenario_rejects_case_selection() {
    let root = root();
    let schema = root.join("e2e/schema");
    let scenario = root.join("e2e/scenarios/p0-gateway.json");
    let profile = root.join("e2e/profiles/gateway-isolated.json");
    let output = validate(&[
        "validate",
        "--schema",
        schema.to_str().unwrap(),
        "--scenario",
        scenario.to_str().unwrap(),
        "--profile",
        profile.to_str().unwrap(),
        "--case",
        "normal_hirouted_listener_exact_evidence",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--case"));
}
