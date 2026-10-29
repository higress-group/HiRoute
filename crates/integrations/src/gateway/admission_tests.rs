use super::*;

fn admission() -> Value {
    json!({
        "kind": "agent_turn_admitted", "version": 1,
        "agent_turn_id": "turn/admitted", "branch_id": "history/branch",
        "revision": 2, "ordinal": 3, "opening_cause": "no_comparable_history"
    })
}

#[test]
fn admission_transcodes_without_a_route_decision_or_invented_source_evidence() {
    let fact = admission();
    let mut wire = execution_payload(fact.clone(), 1, false);
    wire["correlation"]["correlation_provenance"] = json!("unproven");
    let projected = project_execution_payload(&serde_json::to_vec(&wire).unwrap()).unwrap();
    projected.validate().unwrap();
    let projected = serde_json::to_value(projected).unwrap();
    assert_eq!(projected["fact"], fact);
    assert_eq!(projected["correlation"], wire["correlation"]);
    assert_eq!(
        projected["producer"]["stream"]["producer_epoch"],
        wire["producer"]["producer_epoch"]
    );
    assert_eq!(projected["trust"]["route"], wire["route"]);
    assert_eq!(projected["event_id"], wire["event_id"]);
    assert_eq!(
        projected["schema_digest"],
        hiroute_domain::EXECUTION_FACT_PORT_DIGEST_V2
    );
    assert!(projected["attempt_id"].is_null());
    assert!(projected.get("pricing").is_none());
}

#[test]
fn live_admission_rejects_old_gateway_descriptor_and_invalid_payloads() {
    let mut old = execution_payload(admission(), 1, false);
    old["schema_digest"] =
        json!("sha256:c3ee57843d51aaa7dad46c7e5cfde2fa4b884b32742412d2c9030fae4253014a");
    assert_eq!(
        project_execution_payload(&serde_json::to_vec(&old).unwrap()),
        Err(GatewayProjectionError::UnsupportedSchema)
    );
    for (field, value) in [
        ("version", json!(2)),
        ("branch_id", json!("")),
        ("revision", json!(0)),
        ("ordinal", json!(0)),
        ("agent_turn_id", json!("../invalid")),
    ] {
        let mut invalid = execution_payload(admission(), 1, false);
        invalid["fact"][field] = value;
        assert_eq!(
            project_execution_payload(&serde_json::to_vec(&invalid).unwrap()),
            Err(GatewayProjectionError::InvalidOutput)
        );
    }
    for field in ["segment_id", "complete", "source_proof"] {
        let mut invalid = execution_payload(admission(), 1, false);
        invalid["fact"][field] = json!(true);
        assert!(project_execution_payload(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    let attempt_scoped = execution_payload(admission(), 1, true);
    assert_eq!(
        project_execution_payload(&serde_json::to_vec(&attempt_scoped).unwrap()),
        Err(GatewayProjectionError::InvalidOutput)
    );
}
