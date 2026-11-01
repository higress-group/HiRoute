use std::collections::BTreeSet;

use hiroute_application_api::{CommandLifecycle, command_by_id};
use hiroute_domain::{
    CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1, CONVERSATION_CONTENT_PORT_DIGEST_V2,
    CONVERSATION_CONTENT_SCHEMA_V2, CanonicalDigest, EXECUTION_FACT_PORT_DIGEST_V2,
    EXECUTION_FACT_SCHEMA_V2, OBSERVATION_ACK_SCHEMA_V2, OBSERVATION_NACK_SCHEMA_V1,
    OBSERVATION_RETENTION_SCHEMA_V1, SEVEN_DAYS_MILLIS,
};
use serde::Deserialize;

#[path = "../src/observation/mod.rs"]
mod observation;

use observation::{
    ObservationContractV2, PRODUCTION_EVIDENCE, PRODUCTION_EVIDENCE_OWNER, ScenarioState,
};

const SCENARIO: &str =
    include_str!("../../../e2e/product/scenarios/observation/local-observation-contract.v2.json");
const FIXTURE: &str =
    include_str!("../../../e2e/product/fixtures/observation/local-writer-query.v2.json");
const GOLDEN: &str =
    include_str!("../../../e2e/product/golden/observation/local-observation-boundary.v2.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalFixture {
    schema: String,
    fact_channel_capacity_bytes: usize,
    content_channel_capacity_bytes: usize,
    canonical_request: String,
    accepted_response: String,
    rejected_response: String,
    request_direction: String,
    response_direction: String,
    request_fork_id: String,
    response_fork_id: String,
    accepted_response_frame_id: String,
    canonical_response_media_type: String,
    search_query: String,
    value_calculation_basis: String,
    value_rollup_identity: BTreeSet<String>,
    delete_rollups_true_creates_tombstone: bool,
    value_cases: Vec<ValueCase>,
    forbidden_truth_inputs: BTreeSet<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ValueCase {
    agent_plan_id: String,
    currency: String,
    baseline_micros: Option<u64>,
    chosen_micros: Option<u64>,
    actual_micros: Option<u64>,
    routing_savings_micros: Option<i64>,
    entitlement_savings_micros: Option<i64>,
    estimated_total_savings_micros: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryGolden {
    scenario_id: String,
    local_writer_query_contract: String,
    production_cli_daemon_observation: String,
    production_evidence: String,
    composition_owner: String,
    execution_fact_schema: String,
    execution_fact_port_digest: String,
    conversation_content_schema: String,
    conversation_content_port_digest: String,
    acknowledgement_schema: String,
    negative_acknowledgement_schema: String,
    content_stream_identity: BTreeSet<String>,
    adapter_time_join: bool,
    activity_store: String,
    content_store: String,
    retention_schema: String,
    activity_schema_version: String,
    value_calculation_basis: String,
    value_rollup_identity: BTreeSet<String>,
    delete_rollups_true_creates_tombstone: bool,
    retention_millis: i64,
    ordinary_logs_are_truth: bool,
    writes_runtime_correctness_state: bool,
    implements_gateway_or_otel_mapping: bool,
}

#[test]
fn local_observation_contract_and_production_journey_are_green() {
    let contract: ObservationContractV2 = serde_json::from_str(SCENARIO).unwrap();
    contract.validate().unwrap();
    let local = contract
        .scenario_states
        .iter()
        .find(|state| state.scenario_id == "local-writer-query-contract")
        .unwrap();
    let production = contract
        .scenario_states
        .iter()
        .find(|state| state.scenario_id == "production-cli-daemon-observation")
        .unwrap();
    assert_eq!(local.state, ScenarioState::Green);
    assert_eq!(production.state, ScenarioState::Green);
    assert_eq!(production.evidence_owner, PRODUCTION_EVIDENCE_OWNER);
    assert!(production.blocker.is_none());
    for command_id in [
        "sessions.list",
        "sessions.show",
        "sessions.receipt",
        "sessions.status",
        "value.show",
    ] {
        assert_eq!(
            command_by_id(command_id).unwrap().lifecycle,
            CommandLifecycle::Released
        );
    }
}

#[test]
fn local_observation_boundary_golden_records_product_completion() {
    let golden: BoundaryGolden = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(golden.scenario_id, "process-25018-local-observation-v2");
    assert_eq!(golden.local_writer_query_contract, "green");
    assert_eq!(golden.production_cli_daemon_observation, "green");
    assert_eq!(golden.production_evidence, PRODUCTION_EVIDENCE);
    assert_eq!(golden.composition_owner, PRODUCTION_EVIDENCE_OWNER);
    assert_eq!(golden.execution_fact_schema, EXECUTION_FACT_SCHEMA_V2);
    // This completed journey predates budget diagnostics. Keep its original authenticated
    // descriptor and golden bytes; current ingestion uses the extended descriptor instead.
    let original =
        include_str!("../../../crates/domain/src/observation/fixtures/pre-context-contract.json");
    assert_eq!(
        golden.execution_fact_port_digest,
        CanonicalDigest::of_bytes(original.trim_end_matches('\n').as_bytes()).as_str()
    );
    assert_ne!(
        golden.execution_fact_port_digest,
        EXECUTION_FACT_PORT_DIGEST_V2
    );
    assert_eq!(
        golden.conversation_content_schema,
        CONVERSATION_CONTENT_SCHEMA_V2
    );
    assert_eq!(
        golden.conversation_content_port_digest,
        CONVERSATION_CONTENT_PORT_DIGEST_V2
    );
    assert_eq!(golden.acknowledgement_schema, OBSERVATION_ACK_SCHEMA_V2);
    assert_eq!(
        golden.negative_acknowledgement_schema,
        OBSERVATION_NACK_SCHEMA_V1
    );
    assert_eq!(
        golden.content_stream_identity,
        BTreeSet::from([
            "direction".to_owned(),
            "fork_id".to_owned(),
            "request_id".to_owned(),
        ])
    );
    assert!(!golden.adapter_time_join);
    assert_eq!(golden.activity_store, "activity.db");
    assert_eq!(golden.content_store, "conversation-content");
    assert_eq!(golden.retention_schema, OBSERVATION_RETENTION_SCHEMA_V1);
    assert_eq!(golden.activity_schema_version, "4");
    assert_eq!(
        golden.value_calculation_basis,
        hiroute_domain::VALUE_CALCULATION_BASIS_V1
    );
    assert_eq!(golden.value_rollup_identity, value_rollup_identity());
    assert!(!golden.delete_rollups_true_creates_tombstone);
    assert_eq!(golden.retention_millis, SEVEN_DAYS_MILLIS);
    assert!(!golden.ordinary_logs_are_truth);
    assert!(!golden.writes_runtime_correctness_state);
    assert!(!golden.implements_gateway_or_otel_mapping);
}

#[test]
fn local_observation_fixture_is_bounded_and_does_not_claim_product_execution() {
    let fixture: LocalFixture = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture.schema, "hiroute.local-observation-fixture/v2");
    assert_eq!(fixture.fact_channel_capacity_bytes, 128 * 1024);
    assert_eq!(fixture.content_channel_capacity_bytes, 128 * 1024);
    assert!(
        fixture
            .canonical_request
            .to_uppercase()
            .contains(&fixture.search_query)
    );
    assert_ne!(fixture.accepted_response, fixture.rejected_response);
    assert_eq!(fixture.request_direction, "request_input");
    assert_eq!(fixture.response_direction, "response_delivered");
    assert_eq!(fixture.request_fork_id, "fork-request");
    assert_eq!(fixture.response_fork_id, "fork-response");
    assert_eq!(fixture.accepted_response_frame_id, "frame-accepted-7");
    assert_eq!(
        fixture.canonical_response_media_type,
        CANONICAL_RESPONSE_CONTENT_MEDIA_TYPE_V1
    );
    assert_eq!(
        fixture.value_calculation_basis,
        hiroute_domain::VALUE_CALCULATION_BASIS_V1
    );
    assert_eq!(fixture.value_rollup_identity, value_rollup_identity());
    assert!(!fixture.delete_rollups_true_creates_tombstone);
    assert_eq!(fixture.value_cases.len(), 3);
    let identities = fixture
        .value_cases
        .iter()
        .map(|case| (&case.agent_plan_id, &case.currency))
        .collect::<BTreeSet<_>>();
    assert_eq!(identities.len(), 3);
    for case in &fixture.value_cases {
        assert_eq!(
            case.routing_savings_micros,
            difference(case.baseline_micros, case.chosen_micros)
        );
        assert_eq!(
            case.entitlement_savings_micros,
            difference(case.chosen_micros, case.actual_micros)
        );
        assert_eq!(
            case.estimated_total_savings_micros,
            difference(case.baseline_micros, case.actual_micros)
        );
    }
    assert!(fixture.value_cases.iter().any(|case| {
        case.estimated_total_savings_micros
            .is_some_and(|value| value < 0)
    }));
    assert!(
        fixture
            .value_cases
            .iter()
            .any(|case| case.estimated_total_savings_micros.is_none())
    );
    assert_eq!(
        fixture.forbidden_truth_inputs,
        BTreeSet::from([
            "current_agent_plan".to_owned(),
            "current_price_service".to_owned(),
            "gateway_runtime_state".to_owned(),
            "ordinary_log".to_owned(),
            "otel_log_record".to_owned(),
        ])
    );

    // The executable adapter-contract evidence lives in the Observation and Application suites;
    // the installed standalone scenario supplies the public CLI/daemon/Gateway product evidence.
    let contract: ObservationContractV2 = serde_json::from_str(SCENARIO).unwrap();
    assert_eq!(contract.process, "PROCESS-25018");
    assert!(
        contract
            .scenario_states
            .iter()
            .any(|state| state.scenario_id == "local-writer-query-contract"
                && state.evidence_owner == "PROCESS-25018")
    );
}

fn value_rollup_identity() -> BTreeSet<String> {
    ["agent_plan_id", "currency", "day_number", "workspace_id"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn difference(left: Option<u64>, right: Option<u64>) -> Option<i64> {
    Some(i64::try_from(left?).unwrap() - i64::try_from(right?).unwrap())
}
