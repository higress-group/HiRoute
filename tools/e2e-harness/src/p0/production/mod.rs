mod attestation;
mod candidate;
pub use candidate::{
    CandidateRunReport, prepare_current_candidate, run_current_candidate, write_current_report,
};
mod collector;
mod contract;
mod listener;
mod record;
mod report;
mod runtime;
mod types;

pub use collector::verify_collector_evidence;
pub use contract::{ProductionBundle, ProductionValidationSummary};
pub use report::{
    read_report, verify_launcher_evidence, verify_native_evidence, verify_readiness_evidence,
    verify_report, write_report,
};
pub use runtime::run as run_production_oracle;
pub use types::{
    ChannelEvidence, CollectorEvidence, ContentTerminal, ExpectedObservation, LauncherRecord,
    ProductReady, ProductionError, ProductionRunOptions, ProductionRunReport, ReadinessEvidence,
    ScenarioState, SutBuildAttestation, VerifiedProductionRun,
};

mod current_inputs;

mod legacy_inputs;

#[cfg(test)]
mod tests {
    use super::types::{CURRENT_AGGREGATE_PORT_DIGEST, CURRENT_EXECUTION_SCHEMA_DIGEST};
    use hiroute_gateway::server::core_runtime::observation::{
        EXECUTION_FACT_PORT_DIGEST, GATEWAY_PORT_SET_DIGEST,
    };
    use serde_json::Value;

    #[test]
    fn current_oracle_pins_match_producer_and_json_schemas() {
        assert_eq!(CURRENT_EXECUTION_SCHEMA_DIGEST, EXECUTION_FACT_PORT_DIGEST);
        assert_eq!(CURRENT_AGGREGATE_PORT_DIGEST, GATEWAY_PORT_SET_DIGEST);
        let schema_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/schema");
        let read = |name| -> Value {
            serde_json::from_slice(&std::fs::read(schema_root.join(name)).unwrap()).unwrap()
        };
        assert_eq!(
            read("current-production-collector.schema.json")["properties"]["execution_fact"]["properties"]
                ["records"]["items"]["properties"]["schema_digest"]["const"],
            CURRENT_EXECUTION_SCHEMA_DIGEST
        );
        assert_eq!(
            read("current-gateway-result.schema.json")["properties"]["aggregate_port_digest"]["const"],
            CURRENT_AGGREGATE_PORT_DIGEST
        );
    }
}
