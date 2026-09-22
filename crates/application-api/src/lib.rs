#![forbid(unsafe_code)]

//! Versioned Local Control DTOs and the single command descriptor registry.
//!
//! Every client (the CLI today and a future Desktop) consumes these types. This crate has
//! no dependency on Application implementations, storage, Gateway internals, or an OS
//! transport.

mod agent_connection;
mod agent_grant_raw;
mod agent_settings;
mod classifier_diagnostic;
mod client_access;
mod work_plans;
pub use work_plans::*;
mod worker;
pub use worker::*;
mod worker_actions;
pub use worker_actions::*;
mod delegation_tasks;
pub use delegation_tasks::*;
mod commands;
pub use classifier_diagnostic::*;
pub use client_access::*;
mod compute_control;
mod compute_management;
mod compute_management_view;
mod generated;
mod model_catalog;
mod model_connections;
mod observation_v2;
pub use model_catalog::*;
pub use model_connections::*;
pub use observation_v2::*;
mod prices;
pub use prices::*;
mod protocol;
mod standalone_protected_input;

pub use agent_connection::*;
pub use agent_grant_raw::*;
pub use agent_settings::*;
pub use commands::{
    CommandDescriptorV1, CommandKind, CommandLifecycle, CommandManifestV1, CommandScope,
    CoverageState, PlannedScenarioV1, ReservedOperationV1, command_by_id, command_by_operation,
    descriptor_digest, planned_commands, planned_manifest, release_manifest, released_commands,
    reserved_operation, staged_control_commands,
};
pub use compute_control::*;
pub use compute_management::*;
pub use compute_management_view::*;
pub use generated::{GeneratedContractFile, generated_contract_files};
pub use hiroute_domain::delegation::{
    RunStateV1, WorkerHarnessV1, WorkerNetworkV1, WorkerPermissionPolicyV1, WorkerToolV1,
    WorkspaceAccessV1,
};
pub use hiroute_domain::{
    AgentActivationModeV1, AgentPlanId, CHANGE_SPEC_SCHEMA_V1, CanonicalDigest, ChangeSpecV1,
    ComputeCredentialSelectionError, ComputeCredentialSelectionV2, ContentMode,
    HIDDEN_AGENT_GRANT_HELPER_VERB_V1, MANAGED_CLAUDE_SETTING_SOURCES_ARGUMENT_V1,
    MANAGED_CLAUDE_SETTINGS_ARGUMENT_V1, ManagedClaudeLaunchDescriptorV2, ManagedLaunchProfileV1,
    MaterializationState, ModelAlias, ModelSwitchFilter, PRODUCT_CONTRACT_REVISION,
    PROPOSAL_MAP_REVISION, RevisionSetV1, SchemaVersion, SessionId, SessionListQueryV1,
    UpstreamProtocol, ValueGroupByV1, ValueQueryV1,
};
pub use protocol::{
    AgentCheckRequestV1, AgentCheckScopeV1, AgentCheckSuiteV1, AgentModelCheckTargetV2,
    ApplyRequestV1, ApplyResultV1, ClientHelloV1, ErrorCategory, ErrorCode, ErrorV1,
    FallbackPolicyV1, FreePoolModeV1, LOCAL_CONTROL_MAX_FRAME_BYTES, LOCAL_CONTROL_SCHEMA_V2,
    LocalControlRequestV2, LocalControlWireRequestV2, MACHINE_ENVELOPE_SCHEMA_V2,
    MachineEnvelopeV2, MachineErrorV1, MachineNextActionV1, MachineStatus, NextActionV1,
    OperationCancelRequestV1, OperationLookupV1, OperationReferenceV1, PreviewRequestV1,
    PreviewResultV1, PrincipalKind, PrincipalV1, ProtectedClientGrantV2, RoutingModeV1,
    SETUP_REQUEST_SCHEMA_V1, ServerHelloV1, SessionContentModeV1, SessionListPageV1,
    SessionListRequestV1, SessionLookupV1, SetupApplyRequestV1, SetupPreviewV1, SetupRequestV1,
    SetupSelectionV1, SystemStatusV1, ValuePeriodV1, ValueRequestV1, ValueScopeViewV1, WarningV1,
    negotiate_hello,
};
pub use standalone_protected_input::*;

#[cfg(test)]
mod protocol_tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_machine_status_has_one_stable_exit_semantic() {
        let cases = [
            (MachineStatus::Succeeded, 0),
            (MachineStatus::Accepted, 0),
            (MachineStatus::UsageError, 2),
            (MachineStatus::Conflict, 3),
            (MachineStatus::Denied, 4),
            (MachineStatus::NotFound, 5),
            (MachineStatus::Unavailable, 6),
            (MachineStatus::ActionRequired, 7),
            (MachineStatus::NeedsAttention, 8),
            (MachineStatus::InternalError, 1),
        ];
        for (status, exit) in cases {
            assert_eq!(status.exit_code(), exit);
        }
    }

    #[test]
    fn v2_negotiates_only_as_a_matched_wire_and_machine_pair() {
        let hello = ClientHelloV1 {
            api_version: SchemaVersion::new(2, 0),
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "contract-test".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let negotiated = negotiate_hello(&hello).unwrap();
        assert_eq!(negotiated.api_version, LOCAL_CONTROL_SCHEMA_V2);
        assert_eq!(
            negotiated.machine_schema_version,
            MACHINE_ENVELOPE_SCHEMA_V2
        );

        let mixed = ClientHelloV1 {
            machine_schema_version: SchemaVersion::new(1, 0),
            ..hello
        };
        assert_eq!(
            negotiate_hello(&mixed).unwrap_err().code,
            ErrorCode::MachineSchemaIncompatible
        );
    }

    #[test]
    fn client_name_is_not_an_authority_assertion() {
        let hello = ClientHelloV1 {
            api_version: LOCAL_CONTROL_SCHEMA_V2,
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "claims-to-be-hiroute-cli".to_owned(),
            client_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        assert!(negotiate_hello(&hello).is_ok());
        assert_eq!(PrincipalV1::ambient_local_peer().kind, PrincipalKind::Skill);
    }

    #[test]
    fn legacy_request_and_unknown_current_fields_are_rejected() {
        let old = serde_json::json!({
            "schema_version": {"major": 1, "minor": 0},
            "request_id": "old-client",
            "principal": {"kind": "interactive_user", "capabilities": []},
            "operation_id": "ListSchemas",
            "payload": {},
        });
        assert!(serde_json::from_value::<LocalControlWireRequestV2>(old).is_err());

        let current_with_unknown = serde_json::json!({
            "schema_version": {"major": 2, "minor": 0},
            "request_id": "new-client",
            "operation_id": "ListSchemas",
            "payload": {},
            "unexpected": true,
        });
        assert!(serde_json::from_value::<LocalControlWireRequestV2>(current_with_unknown).is_err());
    }

    #[test]
    fn canonical_content_spelling_round_trips_from_generated_schema_to_typed_wire() {
        let schema = generated_contract_files()
            .into_iter()
            .find(|file| file.relative_path == "session-lookup.v1.schema.json")
            .unwrap();
        assert!(schema.contents.contains("messages-and-tools"));
        assert!(!schema.contents.contains("messages_and_tools"));
        let lookup: SessionLookupV1 = serde_json::from_value(serde_json::json!({
            "id": "session-1",
            "content": "messages-and-tools",
        }))
        .unwrap();
        assert_eq!(lookup.content, Some(SessionContentModeV1::MessagesAndTools));
        assert_eq!(
            serde_json::to_value(lookup).unwrap()["content"],
            "messages-and-tools"
        );
    }

    #[test]
    fn agent_check_help_discloses_each_live_suite_model_and_effect_boundary() {
        let help = command_by_id("agents.check").unwrap().help_document();
        for disclosure in [
            "Configuration, native-authentication, and collaboration are bounded same-UID local checks",
            "Live creates bounded connectivity-probe runtime state",
            "Live quick sends one fixed inference request",
            "tool sends a fixed inference request",
            "conformance may send a bounded protocol matrix",
            "may consume real provider quota",
        ] {
            assert!(help.contains(disclosure), "missing {disclosure}");
        }
        assert!(!help.contains("never sends a model inference request"));
    }

    #[test]
    fn managed_launch_public_contract_exposes_child_stream_semantics_not_hidden_helper() {
        let descriptor = command_by_id("agent.launch").unwrap();
        assert_eq!(descriptor.effect, "managed_local_process");
        assert_eq!(
            descriptor.network,
            "local_control_then_numeric_loopback_gateway"
        );
        let help = descriptor.help_document();
        assert!(help.contains("native OsString"));
        assert!(help.contains("launched Agent streams"));
        assert!(!help.contains(HIDDEN_AGENT_GRANT_HELPER_VERB_V1));
        assert!(
            !serde_json::to_string(&release_manifest())
                .unwrap()
                .contains(HIDDEN_AGENT_GRANT_HELPER_VERB_V1)
        );
    }

    #[test]
    fn worker_dependency_help_is_a_complete_discover_select_and_replay_contract() {
        let discover = command_by_id("worker.dependencies.discover").unwrap();
        assert!(discover.help.examples[0].contains("--harness codex_cli --output json"));
        assert!(discover.help.automation.contains("selection_revisions"));
        assert!(discover.help.output.contains("omits the CAS revisions"));

        let select = command_by_id("worker.dependencies.select").unwrap();
        assert_eq!(select.stdin_channels, ["request_stdin"]);
        assert_eq!(select.idempotency, "deterministic_request_replay");
        let help = select.help_document();
        for required in [
            "harness",
            "adapter_path",
            "cli_path",
            "node_path",
            "expected_selection_revision",
            "replay that same document unchanged",
        ] {
            assert!(help.contains(required), "missing {required}");
        }
        assert!(!help.contains("<worker-dependencies-select-request-json>"));
        assert!(!help.contains("capability-fd"));
    }

    #[test]
    fn p0_registry_and_staging_invariants_remain_closed() {
        let commands = planned_commands();
        assert_eq!(commands.len(), 83);
        assert_eq!(
            commands
                .iter()
                .map(|command| &command.command_id)
                .collect::<BTreeSet<_>>()
                .len(),
            83
        );
        assert_eq!(
            commands
                .iter()
                .map(|command| &command.path)
                .collect::<BTreeSet<_>>()
                .len(),
            83
        );
        assert_eq!(
            commands
                .iter()
                .map(|command| &command.operation_id)
                .collect::<BTreeSet<_>>()
                .len(),
            83
        );
        for operation in ["GetClientServiceStatus", "ListWorkPlans"] {
            let command = command_by_operation(operation).unwrap();
            assert_eq!(command.lifecycle, CommandLifecycle::Planned);
            assert!(
                !release_manifest()
                    .commands
                    .iter()
                    .any(|c| c.operation_id == operation)
            );
        }
        assert_eq!(
            command_by_operation("FindOperationByIdempotency")
                .unwrap()
                .lifecycle,
            CommandLifecycle::Released
        );
        assert_eq!(
            command_by_operation("GetPlanEditorOptions")
                .unwrap()
                .lifecycle,
            CommandLifecycle::Released
        );
        let release = release_manifest();
        assert_eq!(release.commands.len(), 45);
        assert!(release.commands.iter().all(|command| {
            command.lifecycle == CommandLifecycle::Released
                && command.positive.state == CoverageState::Executable
                && command.negative.state == CoverageState::Executable
        }));
        assert_eq!(staged_control_commands().len(), 17);
        assert!(
            staged_control_commands()
                .iter()
                .all(|command| command.lifecycle == CommandLifecycle::Planned)
        );
    }

    #[test]
    fn every_help_contract_contains_all_required_sections() {
        for descriptor in planned_commands() {
            let help = descriptor.help_document();
            for heading in [
                "Purpose",
                "Usage",
                "Arguments/Options",
                "Effects",
                "Network and model use",
                "Automation",
                "Output",
                "Exit status",
                "Examples",
                "See also",
            ] {
                assert!(help.contains(&format!("{heading}\n")));
            }
        }
    }
}

mod plan_authoring;
pub use plan_authoring::*;
