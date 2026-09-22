use std::collections::{BTreeSet, HashMap};

use hiroute_application_api::{CommandLifecycle, command_by_id};
use hiroute_domain::{MAX_AGENT_PURPOSE_CHARS, MAX_CATALOG_ENTRIES, MAX_ROUTING_OVERLAY_BYTES};
use serde::Deserialize;

#[path = "../src/agents/mod.rs"]
mod agents;

use agents::{
    AgentConnectionContractV1, PRODUCTION_EVIDENCE, PRODUCTION_EVIDENCE_OWNER, ScenarioState,
};

const SCENARIO: &str =
    include_str!("../../../e2e/product/scenarios/agents/agent-connection-contract.v1.json");
const FIXTURE: &str = include_str!("../../../e2e/product/fixtures/agents/profile-matrix.v1.json");
const GOLDEN: &str =
    include_str!("../../../e2e/product/golden/agents/agent-connection-boundary.v1.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileMatrixFixture {
    schema: String,
    config_precedence: Vec<String>,
    profiles: Vec<ProfileFixture>,
    spawn_guidance_marker: String,
    catalog_limits: CatalogLimits,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFixture {
    kind: String,
    profile_id: String,
    exact_version: String,
    protocol: String,
    dynamic_catalog: bool,
    static_catalog_fallback: bool,
    native_subagent_routing: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogLimits {
    entries: usize,
    overlay_bytes: usize,
    purpose_chars: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundaryGolden {
    scenario_id: String,
    profile_grant_catalog_contract: String,
    adapter_emulator_contract: String,
    field_owned_restore_contract: String,
    production_cli_agent_connection: String,
    production_evidence: String,
    composition_owner: String,
    codex_protocol: String,
    claude_protocol: String,
    unknown_version_behavior: String,
}

#[test]
fn agent_connection_internal_contracts_and_public_subprocess_are_green() {
    let scenario: AgentConnectionContractV1 = serde_json::from_str(SCENARIO).unwrap();
    scenario.validate().unwrap();
    for id in [
        "profile-grant-catalog-contract",
        "adapter-emulator-contract",
        "field-owned-restore-contract",
    ] {
        assert_eq!(
            scenario
                .scenario_states
                .iter()
                .find(|state| state.scenario_id == id)
                .unwrap()
                .state,
            ScenarioState::Green
        );
    }
    let public = scenario
        .scenario_states
        .iter()
        .find(|state| state.scenario_id == "production-cli-agent-connection")
        .unwrap();
    assert_eq!(public.state, ScenarioState::Green);
    assert_eq!(public.evidence_owner, PRODUCTION_EVIDENCE_OWNER);
    assert!(public.blocker.is_none());

    for command_id in [
        "agents.connect.preview",
        "agents.connect.apply",
        "agents.restore.preview",
        "agents.restore.apply",
    ] {
        assert_eq!(
            command_by_id(command_id).unwrap().lifecycle,
            CommandLifecycle::Released,
            "the installed standalone product evidence requires this public command"
        );
    }
}

#[test]
fn agent_connection_profile_matrix_freezes_exact_protocol_and_config_precedence() {
    let fixture: ProfileMatrixFixture = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(fixture.schema, "hiroute.agent-profile-matrix-fixture/v1");
    assert_eq!(
        fixture.config_precedence,
        ["process", "launch", "project", "user", "managed"]
    );
    let profiles = fixture
        .profiles
        .iter()
        .map(|profile| (profile.kind.as_str(), profile))
        .collect::<HashMap<_, _>>();
    let codex = profiles["codex"];
    assert_eq!(codex.profile_id, "codex-responses-v1");
    assert_eq!(codex.exact_version, "0.116.0");
    assert_eq!(codex.protocol, "responses");
    assert!(codex.dynamic_catalog && codex.static_catalog_fallback);
    assert!(codex.native_subagent_routing);
    let claude = profiles["claude_code"];
    assert_eq!(claude.profile_id, "claude-messages-v1");
    assert_eq!(claude.exact_version, "2.1.0");
    assert_eq!(claude.protocol, "messages");
    assert!(!claude.dynamic_catalog && !claude.static_catalog_fallback);
    assert!(!claude.native_subagent_routing);
    assert_eq!(
        fixture.spawn_guidance_marker,
        "Available model overrides (optional; inherited parent model is preferred):"
    );
    assert_eq!(fixture.catalog_limits.entries, MAX_CATALOG_ENTRIES);
    assert_eq!(
        fixture.catalog_limits.overlay_bytes,
        MAX_ROUTING_OVERLAY_BYTES
    );
    assert_eq!(
        fixture.catalog_limits.purpose_chars,
        MAX_AGENT_PURPOSE_CHARS
    );
}

#[test]
fn agent_connection_boundary_golden_records_public_completion() {
    let golden: BoundaryGolden = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(golden.scenario_id, "process-25005-agent-connection");
    assert_eq!(golden.profile_grant_catalog_contract, "green");
    assert_eq!(golden.adapter_emulator_contract, "green");
    assert_eq!(golden.field_owned_restore_contract, "green");
    assert_eq!(golden.production_cli_agent_connection, "green");
    assert_eq!(golden.production_evidence, PRODUCTION_EVIDENCE);
    assert_eq!(golden.composition_owner, PRODUCTION_EVIDENCE_OWNER);
    assert_eq!(golden.codex_protocol, "responses");
    assert_eq!(golden.claude_protocol, "messages");
    assert_eq!(golden.unknown_version_behavior, "capability_based");

    let states = BTreeSet::from([
        golden.profile_grant_catalog_contract,
        golden.adapter_emulator_contract,
        golden.field_owned_restore_contract,
    ]);
    assert_eq!(states, BTreeSet::from(["green".to_owned()]));
}
