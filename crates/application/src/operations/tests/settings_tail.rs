use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{
    AGENT_SETTINGS_SCHEMA_V2, AgentAccessGrantMutationV1, AgentAccessGrantScopeV1,
    AgentConnectionControlIntentV1, AgentConnectionTransactionSubjectV1, AgentFacetIntent,
    AgentModelDefaultSelectionV2, AgentModelGrantV2, AgentModelRouteV2, AgentModelSelectionV2,
    AgentPlanId, AgentSettingsSpecV2, CHANGE_SPEC_SCHEMA_V1, IdempotencyScopeV1,
    SETTINGS_SERVICE_COMPLETION_SCHEMA, SettingsServiceCompletionV1, TransactionPlanV1,
    is_settings_managed_configuration, is_settings_publication, settings_model_publication_intent,
};
use serde_json::json;

use super::*;
use crate::agent_connection::{
    CodexModelFileAction, settings_codex_model_file_intent, settings_login_item_intent,
};

const CONTEXT_ID: &str = "agent-context/settings-tail";
const MODEL_FILE_EFFECT: &str = "agent-connection-managed-configuration";
const PUBLICATION_EFFECT: &str = "agent-connection-grant-publication";

fn settings_spec() -> AgentSettingsSpecV2 {
    let plan_id = AgentPlanId::parse("plan/tail").unwrap();
    AgentSettingsSpecV2 {
        schema_version: AGENT_SETTINGS_SCHEMA_V2,
        context_id: CONTEXT_ID.to_owned(),
        model: AgentFacetIntent::Configure {
            settings: AgentModelSelectionV2::CodexDefault {
                native_model_mode: hiroute_domain::CodexNativeModelModeV2::HirouteOnly,
                fixed_models: Vec::new(),
                allowed_plan_ids: BTreeSet::from([plan_id.clone()]),
                default_selection: AgentModelDefaultSelectionV2::Plan { plan_id },
            },
        },
        collaboration: AgentFacetIntent::Keep,
        restore_native_model: None,
        protected_native_model_ids: Vec::new(),
        access_token: hiroute_domain::AgentAccessTokenIntentV1::Keep,
    }
}

fn settings_plan() -> TransactionPlanV1 {
    settings_plan_with_login_item(false)
}

fn settings_plan_with_login_item(host_login_item: bool) -> TransactionPlanV1 {
    let spec = hiroute_domain::ChangeSpecV1 {
        schema_version: CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".to_owned(),
        resource_id: Some(CONTEXT_ID.to_owned()),
        desired_state: serde_json::to_value(settings_spec()).unwrap(),
    };
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent_codex_default",
        "codex-responses-v1",
        "builtin/codex-responses/v1",
    )
    .unwrap();
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        subject,
        &spec,
        false,
        &json!({"fixture": "settings-tail"}),
    )
    .unwrap();
    let grant = AgentModelGrantV2::seal(
        hiroute_domain::AgentIngressProtocolV1::Responses,
        BTreeMap::from([(
            "hiroute-tail".to_owned(),
            AgentModelRouteV2::Plan {
                plan_id: AgentPlanId::parse("plan/tail").unwrap(),
                alias: hiroute_domain::ModelAlias::parse_custom("hiroute-tail").unwrap(),
                revision: 1,
                semantic_digest: CanonicalDigest::of_bytes(b"plan-tail"),
            },
        )]),
    )
    .unwrap();
    let scope =
        AgentAccessGrantScopeV1::new(format!("agent-connection/{CONTEXT_ID}"), grant).unwrap();
    let mutation = AgentAccessGrantMutationV1::ensure(WorkspaceId::DEFAULT, scope, 0).unwrap();
    let model_file = settings_codex_model_file_intent(
        &control,
        CONTEXT_ID,
        CanonicalDigest::of_bytes(b"model-file-content"),
        None,
        CodexModelFileAction::Configure {
            previous_operation: None,
            provider_id: "hiroute".to_owned(),
            endpoint: "http://127.0.0.1:8787/v1".to_owned(),
            model: None,
            model_catalog: None,
        },
    )
    .unwrap();
    let publication = settings_model_publication_intent(
        &control,
        CONTEXT_ID,
        CanonicalDigest::of_bytes(b"publication-source"),
        None,
    )
    .unwrap();
    let mut external = Vec::new();
    if host_login_item {
        external.push(
            settings_login_item_intent(
                &control,
                CONTEXT_ID,
                &hiroute_application_api::AgentLoginItemDeclarationV2 {
                    before: hiroute_application_api::AgentLoginItemStatusV2::NotRegistered,
                    after: hiroute_application_api::AgentLoginItemStatusV2::Enabled,
                    created: true,
                },
            )
            .unwrap(),
        );
    }
    external.extend([model_file, publication]);
    TransactionPlanV1::from_agent_connection_planner_with_agent_access_grants(
        spec,
        control,
        vec![mutation],
        external,
    )
    .unwrap()
}

fn inject_settings_operation(ports: &MemoryPorts) -> OperationId {
    inject_settings_operation_with_plan(ports, settings_plan())
}

fn inject_settings_operation_with_plan(
    ports: &MemoryPorts,
    plan: TransactionPlanV1,
) -> OperationId {
    let workspace = WorkspaceId::default();
    let idempotency = IdempotencyScopeV1::new(
        "interactive-user",
        "agents.settings.apply",
        "settings-tail-1",
    )
    .unwrap();
    let request_digest = CanonicalDigest::of_bytes(b"settings-tail-request");
    let operation_id = OperationId::derive(&workspace, &idempotency, &request_digest);
    let operation = OperationV1::new(
        operation_id.clone(),
        workspace.clone(),
        idempotency.clone(),
        request_digest,
        CanonicalDigest::of_bytes(b"settings-tail-accepted"),
        RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        },
        plan,
    )
    .unwrap();
    let mut state = ports.state.borrow_mut();
    state.idempotency.insert(
        MemoryPorts::idempotency_key(&workspace, &idempotency),
        operation_id.as_str().to_owned(),
    );
    state
        .operations
        .insert(operation_id.as_str().to_owned(), operation);
    state.writer = Some(operation_id.as_str().to_owned());
    state.publication_revision = 7;
    operation_id
}

#[test]
fn failed_publication_with_host_login_item_rolls_back_without_blocking_other_writes() {
    let ports = MemoryPorts::default();
    let admission = TransactionRuntime::default();
    let operation_id =
        inject_settings_operation_with_plan(&ports, settings_plan_with_login_item(true));
    {
        let mut state = ports.state.borrow_mut();
        state.host_login_item_observed = true;
        state.fail_activation = Some(PUBLICATION_EFFECT.to_owned());
    }

    let failed = coordinator(&ports, &admission).run(&operation_id).unwrap();

    assert_eq!(failed.state, OperationState::RolledBack);
    assert_eq!(failed.safe_error_code.as_deref(), Some("ADAPTER_FAILURE"));
    assert!(
        failed
            .steps
            .iter()
            .all(|step| step.status == OperationStepStatus::Compensated)
    );
    assert_eq!(ports.state.borrow().writer, None);
}

#[test]
fn startup_rechecks_a_fully_compensated_host_login_item_rollback() {
    let ports = MemoryPorts::default();
    let admission = TransactionRuntime::default();
    let operation_id =
        inject_settings_operation_with_plan(&ports, settings_plan_with_login_item(true));
    {
        let mut state = ports.state.borrow_mut();
        state.host_login_item_observed = true;
        state.fail_activation = Some(PUBLICATION_EFFECT.to_owned());
    }
    let rolled_back = coordinator(&ports, &admission).run(&operation_id).unwrap();
    assert_eq!(rolled_back.state, OperationState::RolledBack);
    {
        // Exact journal shape left by the previous terminal check: all daemon effects were
        // compensated, but the host's pre-admission declaration kept NeedsAttention claimed.
        let mut state = ports.state.borrow_mut();
        state
            .operations
            .get_mut(operation_id.as_str())
            .unwrap()
            .state = OperationState::NeedsAttention;
        state.writer = Some(operation_id.as_str().to_owned());
    }

    let restarted = TransactionRuntime::default();
    let recovered = coordinator(&ports, &restarted)
        .reconcile_startup_and_open()
        .unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state, OperationState::RolledBack);
    assert_eq!(ports.state.borrow().writer, None);
    assert!(restarted.writes_open.load(Ordering::Acquire));
}

fn coordinator<'a>(
    ports: &'a MemoryPorts,
    admission: &'a TransactionRuntime,
) -> TransactionCoordinator<'a, MemoryPorts, MemoryPorts, MemoryPorts, MemoryPorts, MemoryPorts> {
    TransactionCoordinator::new(ports, ports, ports, ports, ports, admission)
}

fn stored(ports: &MemoryPorts, operation_id: &OperationId) -> OperationV1 {
    ports
        .state
        .borrow()
        .operations
        .get(operation_id.as_str())
        .cloned()
        .unwrap()
}

fn effect_applied(ports: &MemoryPorts, operation_id: &OperationId, effect_id: &str) -> bool {
    ports
        .state
        .borrow()
        .effects
        .get(&MemoryPorts::effect_key(operation_id, effect_id))
        .is_some_and(|(_, applied)| *applied)
}

fn effect_staged(ports: &MemoryPorts, operation_id: &OperationId, effect_id: &str) -> bool {
    ports
        .state
        .borrow()
        .effects
        .get(&MemoryPorts::effect_key(operation_id, effect_id))
        .is_some_and(|(_, applied)| !*applied)
}

#[test]
fn settings_publication_serves_before_the_client_file_switch() {
    let ports = MemoryPorts::default();
    let admission = TransactionRuntime::default();
    let operation_id = inject_settings_operation(&ports);

    let finished = coordinator(&ports, &admission).run(&operation_id).unwrap();

    assert_eq!(finished.state, OperationState::Succeeded);
    assert!(effect_applied(&ports, &operation_id, PUBLICATION_EFFECT));
    assert!(effect_applied(&ports, &operation_id, MODEL_FILE_EFFECT));
    let state = ports.state.borrow();
    let publication = state
        .activation_log
        .iter()
        .position(|effect| effect == PUBLICATION_EFFECT)
        .unwrap();
    let model_file = state
        .activation_log
        .iter()
        .position(|effect| effect == MODEL_FILE_EFFECT)
        .unwrap();
    assert!(publication < model_file);
    let receipt = SettingsServiceCompletionV1::parse(
        finished
            .step(OperationStepKind::Activate)
            .terminal_result
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(receipt.publication_revision, 7);
    assert_eq!(
        receipt.publication_digest,
        CanonicalDigest::of_bytes(PUBLICATION_EFFECT.as_bytes())
    );
}

#[test]
fn client_file_failure_parks_the_tail_without_rollback() {
    let ports = MemoryPorts::default();
    let admission = TransactionRuntime::default();
    let operation_id = inject_settings_operation(&ports);
    ports.state.borrow_mut().fail_activation = Some(MODEL_FILE_EFFECT.to_owned());

    let parked = coordinator(&ports, &admission).run(&operation_id).unwrap();

    assert_eq!(parked.state, OperationState::Activating);
    assert!(parked.safe_error_code.is_some());
    assert!(effect_applied(&ports, &operation_id, PUBLICATION_EFFECT));
    assert!(effect_staged(&ports, &operation_id, MODEL_FILE_EFFECT));
    assert!(
        SettingsServiceCompletionV1::parse(
            parked
                .step(OperationStepKind::Activate)
                .terminal_result
                .as_deref()
                .unwrap()
        )
        .is_some()
    );
    assert_eq!(ports.state.borrow().writer, None);

    ports.state.borrow_mut().fail_activation = None;
    let finished = coordinator(&ports, &admission).run(&operation_id).unwrap();

    assert_eq!(finished.state, OperationState::Succeeded);
    assert!(effect_applied(&ports, &operation_id, MODEL_FILE_EFFECT));
    let activations = ports
        .state
        .borrow()
        .activation_log
        .iter()
        .filter(|effect| effect == &PUBLICATION_EFFECT)
        .count();
    assert_eq!(activations, 1);
}

#[test]
fn startup_reports_a_pending_tail_without_touching_files() {
    let ports = MemoryPorts::default();
    let admission = TransactionRuntime::default();
    let operation_id = inject_settings_operation(&ports);
    ports.state.borrow_mut().fail_activation = Some(MODEL_FILE_EFFECT.to_owned());
    coordinator(&ports, &admission).run(&operation_id).unwrap();
    let activations = ports.state.borrow().activation_log.len();

    let startup_admission = TransactionRuntime::default();
    let startup =
        TransactionCoordinator::new(&ports, &ports, &ports, &ports, &ports, &startup_admission);
    let recovered = startup.reconcile_startup_and_open().unwrap();

    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].state, OperationState::Activating);
    assert_eq!(
        recovered[0].safe_error_code.as_deref(),
        Some("SETTINGS_TAIL_PENDING")
    );
    assert!(effect_staged(&ports, &operation_id, MODEL_FILE_EFFECT));
    assert_eq!(ports.state.borrow().activation_log.len(), activations);
    assert_eq!(ports.state.borrow().writer, None);
}

#[test]
fn displaced_publication_expires_the_tail_with_zero_file_writes() {
    let ports = MemoryPorts::default();
    let admission = TransactionRuntime::default();
    let operation_id = inject_settings_operation(&ports);
    ports.state.borrow_mut().fail_activation = Some(MODEL_FILE_EFFECT.to_owned());
    coordinator(&ports, &admission).run(&operation_id).unwrap();
    let activations = ports.state.borrow().activation_log.len();
    ports.state.borrow_mut().publication_displaced = true;

    let expired = coordinator(&ports, &admission).run(&operation_id).unwrap();

    assert_eq!(expired.state, OperationState::Activating);
    assert_eq!(
        expired.safe_error_code.as_deref(),
        Some("SETTINGS_TAIL_EXPIRED")
    );
    assert!(effect_staged(&ports, &operation_id, MODEL_FILE_EFFECT));
    assert_eq!(ports.state.borrow().activation_log.len(), activations);
    assert_eq!(ports.state.borrow().writer, None);
}

#[test]
fn tampered_receipt_digest_rolls_the_operation_back() {
    let ports = MemoryPorts::default();
    let admission = TransactionRuntime::default();
    let operation_id = inject_settings_operation(&ports);
    ports.state.borrow_mut().fail_activation = Some(MODEL_FILE_EFFECT.to_owned());
    coordinator(&ports, &admission).run(&operation_id).unwrap();
    {
        let mut state = ports.state.borrow_mut();
        let operation = state
            .operations
            .get_mut(operation_id.as_str())
            .expect("parked operation");
        let mut receipt = SettingsServiceCompletionV1::parse(
            operation
                .step(OperationStepKind::Activate)
                .terminal_result
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        receipt.completed_effects_digest = CanonicalDigest::of_bytes(b"tampered");
        operation
            .step_mut(OperationStepKind::Activate)
            .terminal_result = Some(serde_json::to_string(&receipt).unwrap());
    }

    let rolled_back = coordinator(&ports, &admission).run(&operation_id).unwrap();

    assert_eq!(
        rolled_back.safe_error_code.as_deref(),
        Some("SETTINGS_RECEIPT_INVALID")
    );
    assert_ne!(rolled_back.state, OperationState::Activating);
    assert!(rolled_back.state.is_terminal());
}

#[test]
fn settings_discriminators_accept_only_the_settings_transaction_facets() {
    let plan = settings_plan();
    let model_file = plan
        .external()
        .iter()
        .find(|intent| intent.effect_id() == MODEL_FILE_EFFECT)
        .unwrap();
    let publication = plan
        .external()
        .iter()
        .find(|intent| intent.effect_id() == PUBLICATION_EFFECT)
        .unwrap();

    assert!(is_settings_managed_configuration(model_file));
    assert!(!is_settings_managed_configuration(publication));
    assert!(is_settings_publication(publication));
    assert!(!is_settings_publication(model_file));
}

#[test]
fn settings_receipt_parse_accepts_only_the_registered_schema() {
    let receipt = SettingsServiceCompletionV1 {
        schema: SETTINGS_SERVICE_COMPLETION_SCHEMA.to_owned(),
        publication_revision: 3,
        publication_digest: CanonicalDigest::of_bytes(b"publication"),
        completed_effects_digest: CanonicalDigest::of_bytes(b"effects"),
    };
    let encoded = serde_json::to_string(&receipt).unwrap();
    assert_eq!(
        SettingsServiceCompletionV1::parse(&encoded),
        Some(receipt.clone())
    );

    let mut wrong_schema = receipt.clone();
    wrong_schema.schema = "hiroute.settings-service-completion/v2".to_owned();
    assert_eq!(
        SettingsServiceCompletionV1::parse(&serde_json::to_string(&wrong_schema).unwrap()),
        None
    );
    assert_eq!(SettingsServiceCompletionV1::parse("applied"), None);
    assert_eq!(SettingsServiceCompletionV1::parse("{}"), None);
}
