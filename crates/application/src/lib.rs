#![forbid(unsafe_code)]

//! Application is the only future owner of product behavior.
//!
//! All clients enter through this one dispatcher. Adapters are dependency-injected by the
//! daemon; the CLI and a future Desktop cannot reach storage, discovery, or observation ports.

use hiroute_application_api::{
    AgentCheckRequestV1, ApplyRequestV1, ApplyResultV1, CanonicalDigest, ClientEmptyRequestV1,
    CommandLifecycle, ErrorCode, ErrorV1, LocalControlRequestV2, MachineEnvelopeV2,
    OperationCancelRequestV1, OperationLookupV1, OperationReferenceV1, PreviewRequestV1,
    RevisionSetV1, SetupApplyRequestV1, SetupRequestV1, WORKER_SETTINGS_GET_OPERATION_V1,
    WORKER_SETTINGS_SET_OPERATION_V1, command_by_id, command_by_operation, release_manifest,
    reserved_operation,
};
use hiroute_domain::{ObservationQueryError, WorkspaceId};
use serde_json::{Value, json};

pub mod agent_connection;
pub mod client_access;
pub mod compiler;
pub mod compute_management;
pub mod control;
mod control_plane;
pub mod delegation;
pub mod model_catalog;
pub mod observation_query;
pub mod prices;
pub mod publication;
pub mod routing;
pub mod subscriptions;

pub use change::{
    ChangePreparationError, ConnectionOptionAuthorizationPort, ConnectionOptionAuthorizationV1,
    ProtectedInputPort, RegisteredComputeSourceMaterializationV1,
};
pub use operations::{
    AcceptedApply, PreparedTransactionV1, TransactionCoordinator, TransactionError,
    TransactionRuntime, VerifiedPrincipal, settings_service_completion_is_current,
};

// Typed planners remain private; the coordinator and its narrow ports are exported only so the
// production daemon can compose the same Application path used by CLI and Desktop clients.
#[allow(dead_code)]
mod change;
#[allow(dead_code)]
mod operations;
#[allow(dead_code)]
mod settings;
#[allow(dead_code)]
mod setup;

#[derive(Clone, Default)]
pub struct ApplicationService {
    ports: Option<control::ApplicationPorts>,
}

impl ApplicationService {
    pub fn new(ports: control::ApplicationPorts) -> Self {
        Self { ports: Some(ports) }
    }

    pub fn dispatch(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        if !matches!(request.schema_version.major, 1 | 2) || request.schema_version.minor != 0 {
            return failed(ErrorCode::SchemaIncompatible, request.request_id);
        }

        if reserved_operation(&request.operation_id).is_some() {
            return failed(ErrorCode::FeatureNotEnabled, request.request_id);
        }

        if compute_management::is_compute_management_operation(&request.operation_id) {
            return compute_management::dispatch_compute_management(self, request);
        }

        if matches!(
            request.operation_id.as_str(),
            WORKER_SETTINGS_GET_OPERATION_V1 | WORKER_SETTINGS_SET_OPERATION_V1
        ) {
            return delegation::tasks::dispatch(self, request);
        }

        let Some(descriptor) = command_by_operation(&request.operation_id) else {
            return failed(ErrorCode::UnknownCommand, request.request_id);
        };

        match descriptor.operation_id.as_str() {
            "ListSchemas" => MachineEnvelopeV2::succeeded(
                serde_json::to_value(release_manifest()).expect("manifest is serializable"),
                Some(request.request_id),
            ),
            "ShowSchema" => self.show_schema(request),
            "GetClientServiceStatus"
            | "FindOperationByIdempotency"
            | "ListAgentPlanCatalog"
            | "GetPlanEditorOptions"
            | "GetAgentPlanStatus" => client_access::dispatch(self, request),
            "GetEffectivePrices" | "PreviewPriceOverrideChange" | "ApplyPriceOverrideChange" => {
                prices::dispatch(self, request)
            }
            "ShowModel" => model_catalog::dispatch(self, request),
            "ListWorkPlans" => delegation::work_plans::dispatch(self, request),
            "SelectWorkerDependencies" => delegation::tasks::dispatch(self, request),
            "ListDelegations"
            | "GetDelegation"
            | "StartDelegation"
            | "WaitDelegation"
            | "ReadDelegationResult"
            | "CancelDelegation"
            | "ContinueDelegation"
            | "WorkerExecutorAvailability"
            | "WorkerDependenciesDiscover"
            | "WorkerPlans"
            | "WorkerExec"
            | "WorkerList"
            | "WorkerStatus"
            | "WorkerWait"
            | "WorkerResult"
            | "WorkerRead"
            | "WorkerContinue"
            | "WorkerCancel"
            | "WorkerConfirmResidual" => delegation::tasks::dispatch(self, request),
            "GetSystemStatus" => self.system_status(request),
            "PreviewSetup" => self.preview_setup(request),
            "ApplySetup" => self.apply_setup(request),
            "GetSetupStatus" => self.operation_query(request, true, false),
            "GetOperation" => self.operation_query(request, false, false),
            "WatchOperations" => self.operation_query(request, false, true),
            "CancelOperation" => self.cancel_operation(request),
            "CheckAgentConnection" => self.check_agent(request),
            "TestClassifierDecision" => self.test_classifier_decision(request),
            "ListDecisionServices" => {
                if serde_json::from_value::<hiroute_application_api::ClientEmptyRequestV1>(
                    request.payload.clone(),
                )
                .is_err()
                {
                    return failed(ErrorCode::InvalidArguments, request.request_id);
                }
                if request.protected_grant.is_some() {
                    return failed(ErrorCode::CapabilityDenied, request.request_id);
                }
                let Some(port) = self.ports.as_ref().and_then(|p| p.routing.as_deref()) else {
                    return failed(ErrorCode::DaemonUnavailable, request.request_id);
                };
                match port.decision_services(&WorkspaceId::default()) {
                    Ok(services) => {
                        succeeded(serde_json::json!({"services":services}), request.request_id)
                    }
                    Err(error) => failed(map_control_error(error), request.request_id),
                }
            }
            "ApplyDecisionService" if request.payload.get("accept_digest").is_some() => {
                self.apply_change(request, "decision.services.apply")
            }
            "ApplyDecisionService" => self.preview_change(request, "decision.services.apply"),
            "ScanAgents" | "ListAgents" => self.list_agents(request),
            "PreviewAgentConnectionChange" => {
                control_plane::dispatch_preview_agent_connection(self, request)
            }
            "ApplyAgentConnectionChange" => {
                control_plane::dispatch_apply_agent_connection(self, request)
            }
            "PreviewAgentConnectionRestore" => {
                control_plane::agent_settings::preview(self, request)
            }
            "ApplyAgentConnectionRestore" => control_plane::agent_settings::apply(self, request),
            "GetAgentConnectionStatus" => {
                control_plane::dispatch_status_agent_connection(self, request)
            }
            "GetManagedAgentLaunchDescriptor" => {
                control_plane::dispatch_launch_descriptor(self, request)
            }
            "ScanCompute" => control_plane::dispatch_scan_compute(self, request),
            "ListConnectionOptions" => control_plane::dispatch_connection_options(self, request),
            "PreviewComputeConnectionChange" => {
                control_plane::dispatch_preview_compute(self, request)
            }
            "ApplyComputeConnectionChange" => control_plane::dispatch_apply_compute(self, request),
            "AuthorizeComputeConnection" => {
                control_plane::dispatch_authorize_compute(self, request)
            }
            "TestComputeConnection" => control_plane::dispatch_test_compute(self, request),
            "ApplyCredentialAdd" if request.payload.get("accept_digest").is_some() => {
                self.apply_change(request, "compute.credential.add")
            }
            "ApplyCredentialAdd" => self.preview_change(request, "compute.credential.add"),
            "ApplyClassifierHeaderSecret" if request.payload.get("accept_digest").is_some() => {
                self.apply_change(request, "routing.classifier.secret.apply")
            }
            "ApplyClassifierHeaderSecret" => {
                self.preview_change(request, "routing.classifier.secret.apply")
            }
            "PreviewAgentPlanChange" => control_plane::dispatch_preview_routing(self, request),
            "ApplyAgentPlanChange" => control_plane::dispatch_apply_routing(self, request),
            "ListSessions" => self.observation(request, ObservationOperation::List),
            "GetSession" => self.observation(request, ObservationOperation::Show),
            "GetRoutingReceipt" => self.observation(request, ObservationOperation::Receipt),
            "GetObservationStatus" => self.observation(request, ObservationOperation::Status),
            "GetValue" => self.observation(request, ObservationOperation::Value),
            "GetPlanQualitySamples" => self.observation(request, ObservationOperation::PlanQuality),
            "PreviewSessionDeletion" => {
                self.observation(request, ObservationOperation::DeletePreview)
            }
            "ApplySessionDeletion" => self.observation(request, ObservationOperation::DeleteApply),
            _ => {
                debug_assert_eq!(descriptor.lifecycle, CommandLifecycle::Planned);
                failed(ErrorCode::NotImplemented, request.request_id)
            }
        }
    }

    fn system_status(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let workspace = WorkspaceId::default();
        let observation_available = ports.observation.get_status(&workspace).is_ok();
        match control::system_status(
            ports.control.as_ref(),
            ports.discovery.as_ref(),
            &workspace,
            observation_available,
            ports.role_all_ready,
        ) {
            Ok(status) => succeeded(status, request.request_id),
            Err(error) => failed(map_control_error(error), request.request_id),
        }
    }

    fn list_agents(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        if request.protected_grant.is_some() {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        if serde_json::from_value::<ClientEmptyRequestV1>(request.payload).is_err() {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        match ports.discovery.discover() {
            Ok(agents) => succeeded(json!({"agents": agents}), request.request_id),
            Err(error) => failed(map_control_error(error), request.request_id),
        }
    }

    fn preview_change(
        &self,
        request: LocalControlRequestV2,
        expected_command_id: &str,
    ) -> MachineEnvelopeV2<Value> {
        if request.protected_grant.is_some() {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        let Some(mutation) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.mutation.as_ref())
        else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(preview) = serde_json::from_value::<PreviewRequestV1>(request.payload) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if preview.spec.command_id != expected_command_id {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        match mutation.preview_change(preview) {
            Ok(result) => succeeded(result, request.request_id),
            Err(error) => failed(error.error_code(), request.request_id),
        }
    }

    fn apply_change(
        &self,
        request: LocalControlRequestV2,
        expected_command_id: &str,
    ) -> MachineEnvelopeV2<Value> {
        if request.protected_grant.is_some() {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        let Some(mutation) = self
            .ports
            .as_ref()
            .and_then(|ports| ports.mutation.as_ref())
        else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(apply) = serde_json::from_value::<ApplyRequestV1>(request.payload) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if apply.spec.command_id != expected_command_id || apply.apply_capability.is_some() {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        match mutation.apply_local_change(apply) {
            Ok(operation) => accepted_apply(operation, request.request_id),
            Err(error) => failed(error.error_code(), request.request_id),
        }
    }

    fn preview_setup(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let spec = request
            .payload
            .get("spec")
            .cloned()
            .unwrap_or_else(|| request.payload.clone());
        let Ok(spec) = serde_json::from_value::<SetupRequestV1>(spec) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        match setup::preview(
            ports.control.as_ref(),
            ports.discovery.as_ref(),
            &WorkspaceId::default(),
            spec,
        ) {
            Ok(preview) if preview.applicable => succeeded(preview, request.request_id),
            Ok(preview) => {
                let code = if preview
                    .blockers
                    .iter()
                    .any(|blocker| blocker == "NO_SUPPORTED_AGENT")
                {
                    ErrorCode::NoSupportedAgent
                } else {
                    ErrorCode::FeatureNotEnabled
                };
                failed_with_data(code, preview, request.request_id)
            }
            Err(setup::SetupPreviewError::InvalidInput) => {
                failed(ErrorCode::InvalidArguments, request.request_id)
            }
            Err(setup::SetupPreviewError::UnknownAgent) => {
                failed(ErrorCode::ResourceNotFound, request.request_id)
            }
            Err(setup::SetupPreviewError::Control(error)) => {
                failed(map_control_error(error), request.request_id)
            }
        }
    }

    fn apply_setup(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        let Some(grant) = request.protected_grant.as_ref() else {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        };
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(apply) = serde_json::from_value::<SetupApplyRequestV1>(request.payload.clone())
        else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if apply.idempotency_key.is_empty() {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let revisions = RevisionSetV1 {
            target: apply.expected_revision,
            dependencies: Default::default(),
        };
        if let Err(error) = ports.control.validate_protected_capability(
            &grant.capability,
            &WorkspaceId::default(),
            grant.principal_kind,
            "ApplySetup",
            &apply.accept_digest,
            &revisions,
        ) {
            return failed(map_control_error(error), request.request_id);
        }
        let preview = match setup::preview(
            ports.control.as_ref(),
            ports.discovery.as_ref(),
            &WorkspaceId::default(),
            apply.spec,
        ) {
            Ok(preview) => preview,
            Err(_) => return failed(ErrorCode::ChangePreviewStale, request.request_id),
        };
        if preview.expected_revision != apply.expected_revision {
            return failed(ErrorCode::RevisionConflict, request.request_id);
        }
        if preview.change_digest != apply.accept_digest {
            return failed(ErrorCode::ChangePreviewStale, request.request_id);
        }
        // A valid authority token is still not permission to invent the missing Gateway
        // publication target. No Operation is admitted and the one-shot token is not consumed.
        failed_with_data(ErrorCode::GatewayUnavailable, preview, request.request_id)
    }

    fn operation_query(
        &self,
        request: LocalControlRequestV2,
        setup_only: bool,
        watch: bool,
    ) -> MachineEnvelopeV2<Value> {
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(lookup) = serde_json::from_value::<OperationLookupV1>(request.payload) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        let result = if setup_only {
            operations::control::setup_status(ports.control.as_ref(), &lookup)
        } else if watch {
            operations::control::watch_snapshot(ports.control.as_ref(), &lookup)
        } else {
            operations::control::get(ports.control.as_ref(), &lookup)
        };
        match result {
            Ok(value) => succeeded(value, request.request_id),
            Err(error) => failed(map_control_error(error), request.request_id),
        }
    }

    fn cancel_operation(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        let Some(grant) = request.protected_grant.as_ref() else {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        };
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(cancel) = serde_json::from_value::<OperationCancelRequestV1>(request.payload) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if cancel.idempotency_key.is_empty() {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let digest = match CanonicalDigest::of(&cancel) {
            Ok(digest) => digest,
            Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
        let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
            Ok(snapshot) => snapshot.revisions,
            Err(error) => return failed(map_control_error(error), request.request_id),
        };
        if let Err(error) = ports.control.validate_protected_capability(
            &grant.capability,
            &WorkspaceId::default(),
            grant.principal_kind,
            "CancelOperation",
            &digest,
            &revisions,
        ) {
            return failed(map_control_error(error), request.request_id);
        }
        // Cancellation admission is owned by the final daemon composition. A validated token
        // cannot turn this staged negative handler into an unjournaled state transition.
        failed(ErrorCode::NotImplemented, request.request_id)
    }

    fn check_agent(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(check) = serde_json::from_value::<AgentCheckRequestV1>(request.payload) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        match agent_connection::control::check(ports.discovery.as_ref(), &check) {
            Ok(agent_connection::control::CheckDisposition::Configuration(data)) => {
                succeeded(data, request.request_id)
            }
            Ok(agent_connection::control::CheckDisposition::ModelConsentRequired) => {
                failed(ErrorCode::ActionRequired, request.request_id)
            }
            Ok(disposition @ (agent_connection::control::CheckDisposition::LiveProbeRequested
                | agent_connection::control::CheckDisposition::NativeAuthenticationRequested
                | agent_connection::control::CheckDisposition::CollaborationRequested)) => {
                let digest = match CanonicalDigest::of(&check) {
                    Ok(digest) => digest,
                    Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
                };
                let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
                    Ok(snapshot) => snapshot.revisions,
                    Err(error) => return failed(map_control_error(error), request.request_id),
                };
                if let Some(grant) = request.protected_grant.as_ref() {
                    if let Err(error) = ports.control.validate_protected_capability(
                        &grant.capability,
                        &WorkspaceId::default(),
                        grant.principal_kind,
                        "CheckAgentConnection",
                        &digest,
                        &revisions,
                    ) {
                        return failed(map_control_error(error), request.request_id);
                    }
                    if grant.principal_kind.is_collaboration() {
                        return failed(ErrorCode::CapabilityDenied, request.request_id);
                    }
                }
                if disposition == agent_connection::control::CheckDisposition::CollaborationRequested {
                    let Some(agent) = ports.agent_connection.as_deref() else {
                        return failed(ErrorCode::DaemonUnavailable, request.request_id);
                    };
                    return match agent.check_collaboration(&check.agent_id) {
                        Ok(()) => succeeded(serde_json::json!({
                            "schema":"hiroute.agent-check/v1", "scope":"collaboration",
                            "agent_id":check.agent_id, "skill_loading":"proven",
                            "trusted_cli_execution":"proven", "model_call":false,
                        }), request.request_id),
                        Err(error) => agent_connection::collaboration_check::failed(error, request.request_id),
                    };
                }
                if disposition == agent_connection::control::CheckDisposition::NativeAuthenticationRequested {
                    let Some(agent) = ports.agent_connection.as_deref() else {
                        return failed(ErrorCode::DaemonUnavailable, request.request_id);
                    };
                    return match agent.check_native_authentication(&check.agent_id) {
                        Ok(()) => succeeded(serde_json::json!({
                            "schema":"hiroute.agent-check/v1", "scope":"native_authentication",
                            "agent_id":check.agent_id, "native_authentication":"proven",
                            "model_call":false, "model_verified":false,
                        }), request.request_id),
                        Err(error) => failed(map_control_error(error), request.request_id),
                    };
                }
                let Some(grant) = request.protected_grant.as_ref() else {
                    // Only a live provider call needs the separately delivered one-shot grant.
                    // Local native and collaboration probes are bounded by same-UID Local Control.
                    return failed(ErrorCode::CapabilityDenied, request.request_id);
                };
                let Some(agent) = ports.agent_connection.as_deref() else {
                    return failed(ErrorCode::DaemonUnavailable, request.request_id);
                };
                if let Err(error) = agent.validate_live_check_target(&check) {
                    return failed(map_control_error(error), request.request_id);
                }
                if let Err(error) = ports.control.consume_agent_live_check_capability(
                    &grant.capability,
                    &WorkspaceId::default(),
                    grant.principal_kind,
                    &digest,
                    &revisions,
                ) {
                    return failed(map_control_error(error), request.request_id);
                }
                let record = match agent.execute_live_check(&check, &digest) {
                    Ok(record) => record,
                    Err(error) => return failed(map_control_error(error), request.request_id),
                };
                let Some(target) = check.target.as_ref() else {
                    return failed(ErrorCode::DaemonUnavailable, request.request_id);
                };
                if record.validate().is_err()
                    || record.context_id != target.context_id
                    || record.surface != target.surface
                    || record.applied_revision != target.expected_applied_revision
                    || record.check_request_digest != digest
                    || record.checked_model_ids.iter().any(|model| {
                        !target.client_model_ids.iter().any(|allowed| allowed == model)
                    })
                    || (record.state == hiroute_domain::AgentSurfaceCheckStateV1::Passed
                        && record.checked_model_ids != target.client_model_ids)
                {
                    return failed(ErrorCode::DaemonUnavailable, request.request_id);
                }
                match agent.save_live_check_result(&record) {
                    Ok(true) => {}
                    Ok(false) => {
                        return failed(ErrorCode::ChangePreviewStale, request.request_id);
                    }
                    Err(error) => return failed(map_control_error(error), request.request_id),
                }
                let checked_model_ids = record.checked_model_ids.clone();
                let call_count = checked_model_ids.len();
                succeeded(
                    serde_json::json!({
                        "schema":"hiroute.agent-check/v1",
                        "scope":"live",
                        "agent_id":check.agent_id,
                        "surface":target.surface,
                        "applied_revision":target.expected_applied_revision,
                        "checked_model_ids":checked_model_ids,
                        "requested_call_count":target.client_model_ids.len(),
                        "call_count":call_count,
                        "state":record.state,
                        "reason_code":record.reason_code,
                        "model_call":true,
                    }),
                    request.request_id,
                )
            }
            Err(error) => failed(map_control_error(error), request.request_id),
        }
    }

    fn test_classifier_decision(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        if request
            .protected_grant
            .as_ref()
            .is_some_and(|grant| grant.principal_kind.is_collaboration())
            || (request.protected_grant.is_none()
                && !request
                    .principal
                    .capabilities
                    .iter()
                    .any(|c| c == "same-os-user:query-preview"))
        {
            return failed(ErrorCode::CapabilityDenied, request.request_id);
        }
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let Ok(test) = serde_json::from_value::<
            hiroute_application_api::ClassifierDecisionTestRequestV1,
        >(request.payload) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        if !test.validate() {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        }
        let digest = match CanonicalDigest::of(&test) {
            Ok(digest) => digest,
            Err(_) => return failed(ErrorCode::InvalidArguments, request.request_id),
        };
        let revisions = match ports.control.snapshot(&WorkspaceId::default()) {
            Ok(snapshot) => snapshot.revisions,
            Err(error) => return failed(map_control_error(error), request.request_id),
        };
        if let Some(grant) = request.protected_grant.as_ref()
            && let Err(error) = ports.control.validate_protected_capability(
                &grant.capability,
                &WorkspaceId::default(),
                grant.principal_kind,
                "TestClassifierDecision",
                &digest,
                &revisions,
            )
        {
            return failed(map_control_error(error), request.request_id);
        }
        let Some(diagnostic) = ports.classifier_diagnostic.as_deref() else {
            return failed(ErrorCode::GatewayUnavailable, request.request_id);
        };
        match diagnostic.test_classifier_decision(&test) {
            Ok(result) => succeeded(result, request.request_id),
            Err(_) => failed(ErrorCode::GatewayUnavailable, request.request_id),
        }
    }

    fn observation(
        &self,
        request: LocalControlRequestV2,
        operation: ObservationOperation,
    ) -> MachineEnvelopeV2<Value> {
        let Some(ports) = self.ports.as_ref() else {
            return failed(ErrorCode::DaemonUnavailable, request.request_id);
        };
        let control = observation_query::control::ObservationControl::new(
            ports.observation.clone(),
            ports.control.clone(),
            ports.clock.clone(),
            ports.value_scope.clone(),
        );
        let result = match operation {
            ObservationOperation::List => control.list(
                request.principal.kind,
                request.protected_grant.as_ref(),
                request.payload,
            ),
            ObservationOperation::Show => control.show(
                request.principal.kind,
                request.protected_grant.as_ref(),
                request.payload,
            ),
            ObservationOperation::Receipt => {
                control.receipt(request.protected_grant.as_ref(), request.payload)
            }
            ObservationOperation::DeletePreview => {
                control.delete_preview(request.protected_grant.as_ref(), request.payload)
            }
            ObservationOperation::DeleteApply => {
                control.delete_apply(request.protected_grant.as_ref(), request.payload)
            }
            ObservationOperation::Status => control.status(request.protected_grant.as_ref()),
            ObservationOperation::Value => {
                control.value(request.protected_grant.as_ref(), request.payload)
            }
            ObservationOperation::PlanQuality => {
                control.plan_quality(request.protected_grant.as_ref(), request.payload)
            }
        };
        match result {
            Ok(value) => succeeded(value, request.request_id),
            Err(error) => failed(map_observation_error(error), request.request_id),
        }
    }

    fn show_schema(&self, request: LocalControlRequestV2) -> MachineEnvelopeV2<Value> {
        let Some(command_id) = request.payload.get("command_id").and_then(Value::as_str) else {
            return failed(ErrorCode::InvalidArguments, request.request_id);
        };
        let Some(descriptor) = command_by_id(command_id)
            .filter(|descriptor| descriptor.lifecycle == CommandLifecycle::Released)
        else {
            return failed(ErrorCode::ResourceNotFound, request.request_id);
        };
        MachineEnvelopeV2::succeeded(json!(descriptor), Some(request.request_id))
    }
}

#[derive(Clone, Copy)]
enum ObservationOperation {
    DeletePreview,
    DeleteApply,
    List,
    Show,
    Receipt,
    Status,
    Value,
    PlanQuality,
}

fn succeeded<T: serde::Serialize>(data: T, request_id: String) -> MachineEnvelopeV2<Value> {
    MachineEnvelopeV2::succeeded(
        serde_json::to_value(data).expect("Application response DTO is serializable"),
        Some(request_id),
    )
}

fn failed_with_data<T: serde::Serialize>(
    code: ErrorCode,
    data: T,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    MachineEnvelopeV2::failed_with_data(
        ErrorV1::new(code),
        serde_json::to_value(data).expect("Application response DTO is serializable"),
        Some(request_id),
    )
}

fn map_control_error(error: control::ControlReadError) -> ErrorCode {
    match error {
        control::ControlReadError::NotFound => ErrorCode::ResourceNotFound,
        control::ControlReadError::SnapshotChanged => ErrorCode::ChangePreviewStale,
        control::ControlReadError::Unavailable | control::ControlReadError::Corrupt => {
            ErrorCode::DaemonUnavailable
        }
        control::ControlReadError::Denied => ErrorCode::CapabilityDenied,
    }
}

fn map_observation_error(error: ObservationQueryError) -> ErrorCode {
    match error {
        ObservationQueryError::Unauthorized => ErrorCode::CapabilityDenied,
        ObservationQueryError::NotFound => ErrorCode::ResourceNotFound,
        ObservationQueryError::InvalidQuery => ErrorCode::InvalidArguments,
        ObservationQueryError::Unavailable | ObservationQueryError::Corrupt => {
            ErrorCode::ObservationUnavailable
        }
        ObservationQueryError::StalePreview => ErrorCode::ChangePreviewStale,
        ObservationQueryError::RevisionConflict => ErrorCode::RevisionConflict,
    }
}

fn accepted_apply(
    operation: hiroute_domain::OperationV1,
    request_id: String,
) -> MachineEnvelopeV2<Value> {
    let reference = OperationReferenceV1 {
        operation_id: operation.operation_id.to_string(),
        state: operation.state.as_str().to_owned(),
        sequence: operation.generation,
        cancellable: !operation.state.is_terminal(),
    };
    let result = ApplyResultV1 {
        operation_id: reference.operation_id.clone(),
        accepted_digest: operation.accepted_digest,
        state: reference.state.clone(),
    };
    let mut envelope = MachineEnvelopeV2::accepted(
        serde_json::to_value(result).expect("Apply result is serializable"),
        Some(request_id),
    );
    envelope.operation = Some(reference);
    envelope
}

fn failed(code: ErrorCode, request_id: String) -> MachineEnvelopeV2<Value> {
    MachineEnvelopeV2::failed(ErrorV1::new(code), Some(request_id))
}

#[cfg(test)]
mod tests {
    use hiroute_application_api::{
        LOCAL_CONTROL_SCHEMA_V2, PrincipalKind, PrincipalV1, SchemaVersion,
    };

    use super::*;

    fn request(operation_id: &str) -> LocalControlRequestV2 {
        LocalControlRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: "request-1".to_owned(),
            principal: PrincipalV1::interactive_user(),
            operation_id: operation_id.to_owned(),
            payload: json!({}),
            protected_grant: None,
        }
    }

    #[test]
    fn unknown_major_fails_closed_before_dispatch() {
        let mut request = request("ListSchemas");
        request.schema_version = SchemaVersion::new(3, 0);
        let response = ApplicationService::default().dispatch(request);
        assert_eq!(response.error.unwrap().code, ErrorCode::SchemaIncompatible);
    }

    #[test]
    fn staged_behavior_without_injected_ports_is_unavailable() {
        let response = ApplicationService::default().dispatch(request("PreviewSetup"));
        assert_eq!(response.error.unwrap().code, ErrorCode::DaemonUnavailable);
    }

    #[test]
    fn reserved_behavior_is_disabled_not_fake_success() {
        let response = ApplicationService::default().dispatch(request("SubmitACPDelegation"));
        assert_eq!(response.error.unwrap().code, ErrorCode::FeatureNotEnabled);
    }

    #[test]
    fn a_skill_has_no_ambient_write_capability() {
        let mut request = request("ApplySetup");
        request.principal.kind = PrincipalKind::Skill;
        request
            .principal
            .capabilities
            .push("same-os-user".to_owned());
        let response = ApplicationService::default().dispatch(request);
        assert_eq!(response.error.unwrap().code, ErrorCode::CapabilityDenied);
    }

    #[test]
    fn an_unstaged_p0_handler_still_fails_closed() {
        let response = ApplicationService::default().dispatch(request("PreviewSettingsChange"));
        assert_eq!(response.error.unwrap().code, ErrorCode::NotImplemented);
    }

    #[test]
    fn worker_settings_are_private_typed_operations_not_unknown_commands() {
        for operation in [
            WORKER_SETTINGS_GET_OPERATION_V1,
            WORKER_SETTINGS_SET_OPERATION_V1,
        ] {
            let response = ApplicationService::default().dispatch(request(operation));
            assert_eq!(
                response.error.unwrap().code,
                ErrorCode::CapabilityUnavailable
            );
        }
    }
}
