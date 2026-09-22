#[path = "agent_check_session.rs"]
mod checks;
pub use checks::*;
#[path = "agent_session.rs"]
mod agents;
pub use agents::*;
// Product-limited native session. No arbitrary operation, path, principal or grant IPC.
use crate::confirmation::{ConfirmationGate, ConfirmationPermit};
use crate::failure::DesktopFailure;
use crate::operation_observation::{IntentEvidence, SubmittedOperation};
use hiroute_application_api::*;
use hiroute_client_core::{Client, ObservationCursor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, atomic::AtomicBool};
mod prices;
pub use prices::{PriceConfirmation, PriceDisplayResult, PriceEditInput};
mod authoring;
pub use authoring::EditorInput;
mod model_save;
pub use model_save::*;
mod subscription;
pub use subscription::*;
mod worker_dependencies;

#[derive(Serialize)]
struct NativePlanApply {
    change: Value,
    accept_digest: CanonicalDigest,
    expected_revisions: RevisionSetV1,
    idempotency_key: String,
}
impl From<PlanContentApplyRequestV2> for NativePlanApply {
    fn from(request: PlanContentApplyRequestV2) -> Self {
        Self {
            change: serde_json::to_value(request.change).expect("typed change"),
            accept_digest: request.accept_digest,
            expected_revisions: request.expected_revisions,
            idempotency_key: request.idempotency_key,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    pub plan_id: String,
    pub display_name: String,
    pub language: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreInput {
    pub plan_id: String,
    pub language: String,
}
#[derive(Clone, Serialize)]
pub struct RestoreName {
    pub plan_id: String,
    pub previous_name: String,
}
#[derive(Serialize)]
pub struct DesktopSnapshot {
    pub service: ClientServiceStatusV1,
    pub catalog: AgentPlanCatalogViewV2,
    pub catalog_error: Option<DesktopFailure>,
    pub trusted_authority: bool,
    pub pending: Option<SubmittedOperation>,
    pub restore_names: Vec<RestoreName>,
}
#[derive(Serialize)]
pub struct MutationOutcome {
    pub state: String,
    pub operation: Option<ClientOperationViewV1>,
}
// Not serializable: a WebView can decide a temporary prompt but cannot reconstruct this context.
pub struct Confirmation {
    requires_confirmation: bool,
    permit: ConfirmationPermit,
    input: RenameInput,
    previous_name: String,
    request: NativePlanApply,
    message_override: Option<String>,
    action_label: String,
    intent: IntentEvidence,
}
impl Confirmation {
    /// The exact revision this apply was previewed against; never a displayed value.
    pub fn revision(&self) -> u64 {
        self.request.expected_revisions.target
    }
    pub fn requires_confirmation(&self) -> bool {
        self.requires_confirmation
    }
    pub fn action_label(&self) -> &str {
        &self.action_label
    }
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn test_key(&self) -> &str {
        &self.request.idempotency_key
    }
    pub fn message(&self) -> String {
        if let Some(message) = &self.message_override {
            return message.clone();
        }
        if self.input.language == "en" {
            format!(
                "Publish route name change\n{} → {}\nRouting and authorization stay unchanged.\n\nPublish this change?",
                self.previous_name, self.input.display_name
            )
        } else {
            format!(
                "发布路由名称修改\n{} → {}\n路由策略与授权保持不变。\n\n确认发布此变更？",
                self.previous_name, self.input.display_name
            )
        }
    }
    pub fn english(&self) -> bool {
        self.input.language == "en"
    }
}
pub struct Session {
    #[cfg(unix)]
    pub resident: crate::bootstrap::Resident,
    pub client: Client,
    hint: Option<SubmittedOperation>,
    subscription_hint: Option<SubmittedOperation>,
    confirmation: ConfirmationGate,
    restore_names: Vec<RestoreName>,
    cursor: ObservationCursor,
    observation_generation: u64,
    pending_model_inputs: BTreeMap<String, Vec<ComputeCandidateRefV2>>,
    subscription_close_requested: Arc<AtomicBool>,
    subscription_cancel_request: Option<OperationCancelRequestV1>,
}
impl Session {
    #[cfg(all(unix, feature = "desktop-runtime"))]
    pub(crate) async fn deletion_preview(
        &mut self,
        spec: SessionDeletionSpecV1,
    ) -> Result<SessionDeletionPreviewV2, DesktopFailure> {
        let status: ClientServiceStatusV1 = query(
            &self.client,
            "GetClientServiceStatus",
            &serde_json::json!({}),
        )
        .await?;
        let request = ObservationDeletePreviewRequestV2 { spec };
        let digest = CanonicalDigest::of(&request).map_err(|_| "REQUEST_INVALID")?;
        let token = self
            .resident
            .register_retention(&digest, &status.revisions, false)?;
        let result = self
            .client
            .observation_delete_preview(
                &crate::random_id()?,
                request,
                ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: token.to_string(),
                },
            )
            .await?;
        if result.error.is_some() {
            return Err("DELETION_PREVIEW_UNAVAILABLE".into());
        }
        result.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
    }
    #[cfg(all(unix, feature = "desktop-runtime"))]
    pub(crate) async fn deletion_apply(
        &mut self,
        preview: SessionDeletionPreviewV2,
    ) -> Result<SessionDeletionOutcomeV2, DesktopFailure> {
        let status: ClientServiceStatusV1 = query(
            &self.client,
            "GetClientServiceStatus",
            &serde_json::json!({}),
        )
        .await?;
        let request = ObservationDeleteApplyRequestV2 {
            accepted_digest: preview.change_digest.clone(),
            preview,
        };
        let digest = CanonicalDigest::of(&request).map_err(|_| "REQUEST_INVALID")?;
        let token = self
            .resident
            .register_retention(&digest, &status.revisions, true)?;
        let result = self
            .client
            .observation_delete_apply(
                &crate::random_id()?,
                request,
                ProtectedClientGrantV2 {
                    principal_kind: PrincipalKind::Desktop,
                    capability: token.to_string(),
                },
            )
            .await?;
        if result.error.is_some() {
            return Err("DELETION_NOT_CONFIRMED_REFRESH_REQUIRED".into());
        }
        result.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
    }

    pub async fn observation_read(
        &mut self,
        request: ObservationReadRequestV2,
    ) -> Result<Value, DesktopFailure> {
        #[cfg(unix)]
        {
            let status: ClientServiceStatusV1 = query(
                &self.client,
                "GetClientServiceStatus",
                &serde_json::json!({}),
            )
            .await?;
            let capability = self
                .resident
                .register_observation(&request, &status.revisions)?;
            let envelope: MachineEnvelopeV2<Value> = self
                .client
                .observation_read(
                    &crate::random_id()?,
                    request,
                    ProtectedClientGrantV2 {
                        principal_kind: PrincipalKind::Desktop,
                        capability: capability.to_string(),
                    },
                )
                .await?;
            if envelope.error.is_some() {
                return Err(DesktopFailure::backend(envelope));
            }
            envelope.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
        }
        #[cfg(not(unix))]
        {
            let _ = request;
            Err("TRUSTED_AUTHORITY_UNAVAILABLE".into())
        }
    }

    pub async fn test_classifier_decision(
        &mut self,
        classifier: hiroute_domain::ComplexityClassifierModeV1,
    ) -> Result<ClassifierDecisionTestResultV1, DesktopFailure> {
        #[cfg(unix)]
        {
            let request = ClassifierDecisionTestRequestV1 {
                schema: CLASSIFIER_DECISION_TEST_SCHEMA_V1.into(),
                classifier,
            };
            if !request.validate() {
                return Err("CLASSIFIER_TEST_INVALID".into());
            }
            let status: ClientServiceStatusV1 = query(
                &self.client,
                "GetClientServiceStatus",
                &serde_json::json!({}),
            )
            .await?;
            let capability = self
                .resident
                .register_classifier_diagnostic(&request, &status.revisions)?;
            let envelope: MachineEnvelopeV2<Value> = self
                .client
                .call_typed(LocalControlWireRequestV2 {
                    schema_version: LOCAL_CONTROL_SCHEMA_V2,
                    request_id: crate::random_id()?,
                    operation_id: "TestClassifierDecision".into(),
                    payload: serde_json::to_value(request)
                        .map_err(|_| "CLASSIFIER_TEST_INVALID")?,
                    protected_grant: Some(ProtectedClientGrantV2 {
                        principal_kind: PrincipalKind::Desktop,
                        capability: capability.to_string(),
                    }),
                })
                .await?;
            if envelope.error.is_some() {
                return Err(DesktopFailure::backend(envelope));
            }
            serde_json::from_value(envelope.data.ok_or("RESPONSE_DATA_MISSING")?)
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        #[cfg(not(unix))]
        {
            let _ = classifier;
            Err("TRUSTED_AUTHORITY_UNAVAILABLE".into())
        }
    }

    pub async fn save_classifier_header_secret(
        &mut self,
        secret_id: String,
        secret: zeroize::Zeroizing<String>,
    ) -> Result<ApplyResultV1, DesktopFailure> {
        #[cfg(unix)]
        {
            let candidate = self.resident.register_model_input(secret)?;
            let desired_state = match serde_json::to_value(ClassifierHeaderSecretInputV1 {
                secret_id,
                input_slot: candidate.candidate_ref.clone(),
                expected_generation: 0,
            }) {
                Ok(value) => value,
                Err(_) => {
                    let _ = self.resident.release_model_input(&candidate);
                    return Err("CLASSIFIER_SECRET_INVALID".into());
                }
            };
            let spec = ChangeSpecV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                command_id: "routing.classifier.secret.apply".into(),
                resource_id: Some("personal/default".into()),
                desired_state,
            };
            let preview: Result<PreviewResultV1, DesktopFailure> = query(
                &self.client,
                "ApplyClassifierHeaderSecret",
                &PreviewRequestV1::new(spec),
            )
            .await;
            let preview = match preview {
                Ok(preview) => preview,
                Err(error) => {
                    let _ = self.resident.release_model_input(&candidate);
                    return Err(error);
                }
            };
            let idempotency_id = match crate::random_id() {
                Ok(value) => value,
                Err(error) => {
                    let _ = self.resident.release_model_input(&candidate);
                    return Err(error.into());
                }
            };
            let request = ApplyRequestV1 {
                schema_version: CHANGE_SPEC_SCHEMA_V1,
                spec: preview.normalized_spec,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key: format!("classifier-secret-{idempotency_id}"),
                apply_capability: None,
            };
            let payload = match serde_json::to_value(request) {
                Ok(value) => value,
                Err(_) => {
                    let _ = self.resident.release_model_input(&candidate);
                    return Err("CLASSIFIER_SECRET_INVALID".into());
                }
            };
            let request_id = match crate::random_id() {
                Ok(value) => value,
                Err(error) => {
                    let _ = self.resident.release_model_input(&candidate);
                    return Err(error.into());
                }
            };
            let envelope: Result<MachineEnvelopeV2<Value>, _> = self
                .client
                .call_typed(LocalControlWireRequestV2 {
                    schema_version: LOCAL_CONTROL_SCHEMA_V2,
                    request_id,
                    operation_id: "ApplyClassifierHeaderSecret".into(),
                    payload,
                    protected_grant: None,
                })
                .await;
            // Application runs this transaction synchronously. Once the call returns,
            // the protected input is no longer needed, including transport and terminal
            // failures.
            let release = self.resident.release_model_input(&candidate);
            let envelope = match envelope {
                Ok(envelope) => envelope,
                Err(error) => {
                    let _ = release;
                    return Err(error.into());
                }
            };
            release?;
            if envelope.error.is_some() {
                return Err(DesktopFailure::backend(envelope));
            }
            serde_json::from_value(envelope.data.ok_or("RESPONSE_DATA_MISSING")?)
                .map_err(|_| "RESPONSE_DATA_INVALID".into())
        }
        #[cfg(not(unix))]
        {
            let _ = (secret_id, secret);
            Err("TRUSTED_AUTHORITY_UNAVAILABLE".into())
        }
    }

    #[cfg(unix)]
    pub fn new(resident: crate::bootstrap::Resident) -> Self {
        let mut cursor = ObservationCursor::default();
        let observation_generation = cursor.restart();
        Self {
            client: resident.client.clone(),
            resident,
            hint: None,
            subscription_hint: None,
            confirmation: ConfirmationGate::default(),
            restore_names: vec![],
            cursor,
            observation_generation,
            pending_model_inputs: BTreeMap::new(),
            subscription_close_requested: Arc::new(AtomicBool::new(false)),
            subscription_cancel_request: None,
        }
    }
    pub fn invalidate_confirmation(&mut self) {
        self.confirmation.invalidate();
    }
    pub fn stop_observing(&mut self) {
        self.cursor.stop();
    }
    pub fn resume_observing(&mut self) {
        self.observation_generation = self.cursor.restart();
    }
    pub async fn snapshot(&mut self) -> Result<DesktopSnapshot, DesktopFailure> {
        let service = query(
            &self.client,
            "GetClientServiceStatus",
            &ClientEmptyRequestV1 {},
        )
        .await?;
        let (catalog, catalog_error) = match read_plan_catalog(&self.client).await {
            Ok(catalog) => (catalog, None),
            Err(error) => (
                AgentPlanCatalogViewV2 {
                    next_cursor: None,
                    schema: "hiroute.agent-plan-catalog/v2".into(),
                    plans: vec![],
                    drafts: vec![],
                },
                Some(error),
            ),
        };
        // The successful status query came over Client Core's verified owner-only Local Control
        // peer. Configuration writes use that same transport, not the separate check grant pipe.
        #[cfg(unix)]
        let trusted_authority = true;
        #[cfg(not(unix))]
        let trusted_authority = false;
        Ok(DesktopSnapshot {
            service,
            catalog,
            catalog_error,
            trusted_authority,
            pending: self.hint.clone(),
            restore_names: self.restore_names.clone(),
        })
    }
    pub async fn preview_restore(
        &mut self,
        input: RestoreInput,
    ) -> Result<Confirmation, DesktopFailure> {
        let previous_name = self
            .restore_names
            .iter()
            .find(|item| item.plan_id == input.plan_id)
            .ok_or("PREVIOUS_NAME_UNAVAILABLE")?
            .previous_name
            .clone();
        self.preview(RenameInput {
            plan_id: input.plan_id,
            language: input.language,
            display_name: previous_name,
        })
        .await
    }
    pub async fn preview(&mut self, input: RenameInput) -> Result<Confirmation, DesktopFailure> {
        if self.confirmation.is_open() {
            return Err("CONFIRMATION_ALREADY_OPEN".into());
        }
        if !matches!(input.language.as_str(), "zh" | "en") {
            return Err("INVALID_LANGUAGE".into());
        }
        let intent = IntentEvidence::rename(&input.plan_id, &input.display_name);
        let retry = self.retry_key(&intent).await?;
        let snapshot = self.snapshot().await?;
        if !snapshot.trusted_authority || !snapshot.service.mutation_available {
            return Err("TRUSTED_AUTHORITY_UNAVAILABLE".into());
        }
        if let Some(error) = snapshot.catalog_error {
            return Err(error);
        }
        let plan = snapshot
            .catalog
            .plans
            .into_iter()
            .find(|p| p.agent_plan_id.as_str() == input.plan_id)
            .ok_or("PLAN_NOT_FOUND")?;
        let previous_name = serde_json::to_value(&plan.desired.display_name)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .ok_or("PLAN_INVALID")?;
        if previous_name == input.display_name {
            return Err("NAME_UNCHANGED".into());
        }
        let mut editor = plan
            .desired
            .editor(Some(plan.model_alias.as_str().into()))
            .map_err(|_| "PLAN_INVALID")?;
        editor.display_name = input.display_name.clone();
        let change = PlanContentChangeV2 {
            schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
            target: PlanContentTargetV2::Update {
                plan_id: plan.agent_plan_id,
                expected_head_revision: plan.head.head_revision,
            },
            editor,
            consumed_draft: None,
        };
        let preview: PlanContentPreviewV2 = query(
            &self.client,
            "PreviewAgentPlanChange",
            &PlanContentPreviewRequestV2 {
                change: change.clone(),
            },
        )
        .await?;
        if preview.schema != PLAN_CONTENT_PREVIEW_SCHEMA_V2
            || preview.plan_version.compiled.body.materialized_route_digest
                != plan.editable_route_digest
        {
            return Err("RENAME_WOULD_CHANGE_ROUTE".into());
        }
        let idempotency_key = match retry {
            Some(key) => key,
            None => crate::random_id()?,
        };
        let permit = self.confirmation.begin()?;
        Ok(Confirmation {
            requires_confirmation: false,
            action_label: if input.language == "en" {
                "Publish"
            } else {
                "发布"
            }
            .into(),
            permit,
            intent,
            input,
            previous_name,
            message_override: None,
            request: PlanContentApplyRequestV2 {
                change,
                accept_digest: preview.change_digest,
                expected_revisions: preview.expected_revisions,
                idempotency_key,
            }
            .into(),
        })
    }
    // Native bridge accepts an explicit ordinary submission or a risk dialog callback.
    pub async fn finish_native_confirmation(
        &mut self,
        context: Confirmation,
        accepted: bool,
    ) -> Result<MutationOutcome, DesktopFailure> {
        if !self.confirmation.finish(&context.permit, accepted)? {
            return Ok(MutationOutcome {
                state: "cancelled_before_apply".into(),
                operation: None,
            });
        }
        let hint = SubmittedOperation {
            plan_id: context.input.plan_id,
            principal_kind: PrincipalKind::InteractiveUser,
            operation_kind: "ApplyAgentPlanChange".into(),
            idempotency_key: context.request.idempotency_key.clone(),
            accepted_digest: context.request.accept_digest.clone(),
            operation_id: None,
            after_sequence: 0,
            intent: Some(context.intent),
            latest_edit_not_applied: false,
        };

        self.restore_names
            .retain(|item| item.plan_id != hint.plan_id);
        if !context.previous_name.is_empty() {
            self.restore_names.push(RestoreName {
                plan_id: hint.plan_id.clone(),
                previous_name: context.previous_name,
            });
        }
        self.hint = Some(hint);
        let request = LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: crate::random_id()?,
            operation_id: "ApplyAgentPlanChange".into(),
            payload: serde_json::to_value(context.request).map_err(|_| "REQUEST_INVALID")?,
            protected_grant: None,
        };
        #[cfg(all(test, target_os = "macos"))]
        if crate::fault_tests::take(crate::fault_tests::Fault::LoseBeforeApply) {
            return Err(hiroute_client_core::ClientFailure {
                code: hiroute_client_core::FailureCode::TransportUnavailable,
                submission: hiroute_client_core::SubmissionState::MayHaveReachedServer,
            }
            .into());
        }
        let response = self.client.call_wire(request).await;
        #[cfg(all(test, target_os = "macos"))]
        if crate::fault_tests::take(crate::fault_tests::Fault::LoseApplyResponseAndDisconnect) {
            // Fault boundary: the real daemon processed the request, but the native
            // session receives neither its response nor a subsequent lookup result.
            return Err(hiroute_client_core::ClientFailure {
                code: hiroute_client_core::FailureCode::TransportUnavailable,
                submission: hiroute_client_core::SubmissionState::MayHaveReachedServer,
            }
            .into());
        }
        self.reconcile_apply_response(response).await
    }
    async fn reconcile_apply_response(
        &mut self,
        response: Result<MachineEnvelopeV2<Value>, hiroute_client_core::ClientFailure>,
    ) -> Result<MutationOutcome, DesktopFailure> {
        self.resume_observing();
        let operation = match response {
            Ok(envelope) if envelope.error.is_some() && envelope.operation.is_none() => {
                if envelope
                    .error
                    .as_ref()
                    .is_some_and(|error| error.code == ErrorCode::IdempotencyKeyReused)
                {
                    // This key may have won concurrently with a different digest. The
                    // exact lookup is necessary to identify the winning Operation and
                    // mark this edit as unapplied; it is not an unconditional post-Apply
                    // lookup after every explicit response.
                    match self.find_pending().await? {
                        Some(winner) => Some(winner),
                        None => {
                            self.hint = None;
                            return Err(DesktopFailure::backend(envelope));
                        }
                    }
                } else {
                    // Ordinary pre-admission rejection has no Operation. Preserve its
                    // exact error instead of letting a recovery query obscure it.
                    self.hint = None;
                    return Err(DesktopFailure::backend(envelope));
                }
            }
            Ok(envelope) => {
                let hint = self.hint.as_mut().ok_or("RESPONSE_OPERATION_MISSING")?;
                Some(accepted_operation_view(envelope, hint)?)
            }
            // Only a lost response needs an identity lookup. If that lookup cannot
            // find an Operation yet, retain the request identity for a later retry.
            Err(_) => self.find_pending().await?,
        };
        if operation.is_none() {
            return Ok(MutationOutcome {
                state: "response_unknown".into(),
                operation: None,
            });
        }
        Ok(MutationOutcome {
            state: if self
                .hint
                .as_ref()
                .is_some_and(|h| h.latest_edit_not_applied)
            {
                "original_operation_restored"
            } else {
                "submitted"
            }
            .into(),
            operation,
        })
    }
    // Request identity belongs to this interaction, never to a global write gate.
    // A different explicit edit uses its own fresh preview and server revision checks.
    async fn retry_key(
        &mut self,
        intent: &IntentEvidence,
    ) -> Result<Option<String>, DesktopFailure> {
        if !self
            .hint
            .as_ref()
            .is_some_and(|hint| hint.intent.as_ref() == Some(intent))
        {
            return Ok(None);
        }
        let known = self
            .hint
            .as_ref()
            .and_then(|hint| hint.operation_id.clone());
        let operation = self.find_pending().await?;
        if self
            .hint
            .as_ref()
            .is_some_and(|hint| hint.latest_edit_not_applied && hint.operation_id != known)
        {
            return Err("LATEST_EDIT_NOT_APPLIED".into());
        }
        match operation {
            Some(operation) if terminal(&operation.state) => Ok(None),
            Some(_) => Err("OPERATION_IN_PROGRESS".into()),
            None => Ok(self.hint.as_ref().map(|hint| hint.idempotency_key.clone())),
        }
    }

    async fn find_subscription(&mut self) -> Result<Option<ClientOperationViewV1>, DesktopFailure> {
        crate::operation_observation::lookup(&self.client, &mut self.subscription_hint).await
    }

    async fn find_pending(&mut self) -> Result<Option<ClientOperationViewV1>, DesktopFailure> {
        let result = crate::operation_observation::lookup(&self.client, &mut self.hint).await?;
        if result
            .as_ref()
            .is_some_and(|operation| terminal(&operation.state))
            && self.hint.as_ref().is_some_and(|hint| {
                self.pending_model_inputs
                    .contains_key(&hint.idempotency_key)
            })
        {
            let idempotency_key = self
                .hint
                .as_ref()
                .expect("checked model save hint")
                .idempotency_key
                .clone();
            #[cfg(unix)]
            self.release_pending_model_inputs(&idempotency_key)?;
        }
        Ok(result)
    }

    pub async fn observe(&mut self) -> Result<Option<ClientOperationViewV1>, DesktopFailure> {
        let operation = self.find_pending().await?;
        if let Some(op) = &operation
            && !self.cursor.accept(self.observation_generation, op.sequence)
        {
            return Err("OBSERVATION_STOPPED".into());
        }
        Ok(operation)
    }
}
async fn read_plan_catalog(client: &Client) -> Result<AgentPlanCatalogViewV2, DesktopFailure> {
    let mut request = AgentPlanCatalogQueryV2 {
        limit: 128,
        cursor: None,
    };
    let mut combined = AgentPlanCatalogViewV2 {
        schema: "hiroute.agent-plan-catalog/v2".into(),
        plans: vec![],
        drafts: vec![],
        next_cursor: None,
    };
    for _ in 0..128 {
        let page: AgentPlanCatalogViewV2 = query(client, "ListAgentPlanCatalog", &request).await?;
        if page.schema != combined.schema || page.plans.len() + page.drafts.len() > request.limit {
            return Err("CATALOG_PAGE_INVALID".into());
        }
        combined.plans.extend(page.plans);
        combined.drafts.extend(page.drafts);
        let Some(cursor) = page.next_cursor else {
            return Ok(combined);
        };
        if request.cursor.as_ref().is_some_and(|previous| {
            previous.snapshot_digest != cursor.snapshot_digest || previous.offset >= cursor.offset
        }) {
            return Err("CATALOG_PAGE_INVALID".into());
        }
        request.cursor = Some(cursor);
    }
    Err("CATALOG_LIMIT_EXCEEDED".into())
}

pub(crate) async fn query<P: Serialize, R: serde::de::DeserializeOwned>(
    client: &Client,
    operation: &str,
    payload: &P,
) -> Result<R, DesktopFailure> {
    let envelope: MachineEnvelopeV2<Value> = client
        .query(operation, &crate::random_id()?, payload)
        .await?;
    if envelope.error.is_some() {
        return Err(DesktopFailure::backend(envelope));
    }
    serde_json::from_value(envelope.data.ok_or("RESPONSE_DATA_MISSING")?)
        .map_err(|_| "RESPONSE_DATA_INVALID".into())
}
fn terminal(state: &str) -> bool {
    matches!(state, "succeeded" | "rolled_back" | "needs_attention")
}

fn accepted_operation_view(
    envelope: MachineEnvelopeV2<Value>,
    hint: &mut SubmittedOperation,
) -> Result<ClientOperationViewV1, DesktopFailure> {
    let reference = envelope.operation.ok_or("RESPONSE_OPERATION_MISSING")?;
    let result: ApplyResultV1 =
        serde_json::from_value(envelope.data.ok_or("RESPONSE_DATA_MISSING")?)
            .map_err(|_| "RESPONSE_DATA_INVALID")?;
    if reference.operation_id != result.operation_id
        || reference.state != result.state
        || result.accepted_digest != hint.accepted_digest
    {
        return Err("RESPONSE_OPERATION_MISMATCH".into());
    }
    hint.operation_id = Some(reference.operation_id.clone());
    hint.after_sequence = reference.sequence;
    Ok(ClientOperationViewV1 {
        operation_id: reference.operation_id,
        state: reference.state,
        sequence: reference.sequence,
        cancellable: reference.cancellable,
        accepted_digest: result.accepted_digest,
        safe_error_code: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admitted_apply_response_has_operation_identity_without_a_follow_up_lookup() {
        let digest = CanonicalDigest::of_bytes(b"accepted agent edit");
        let mut hint = SubmittedOperation {
            plan_id: "plan/test".into(),
            principal_kind: PrincipalKind::InteractiveUser,
            operation_kind: "ApplyAgentPlanChange".into(),
            idempotency_key: "save-once".into(),
            accepted_digest: digest.clone(),
            operation_id: None,
            after_sequence: 0,
            intent: None,
            latest_edit_not_applied: false,
        };
        let mut response = MachineEnvelopeV2::accepted(
            serde_json::to_value(ApplyResultV1 {
                operation_id: "operation/accepted".into(),
                accepted_digest: digest.clone(),
                state: "succeeded".into(),
            })
            .unwrap(),
            Some("request/save-once".into()),
        );
        response.operation = Some(OperationReferenceV1 {
            operation_id: "operation/accepted".into(),
            state: "succeeded".into(),
            sequence: 7,
            cancellable: false,
        });
        let view = accepted_operation_view(response.clone(), &mut hint).unwrap();
        assert_eq!(view.operation_id, "operation/accepted");
        assert_eq!(view.sequence, 7);
        assert_eq!(hint.operation_id.as_deref(), Some("operation/accepted"));
        assert_eq!(hint.after_sequence, 7);

        let mut mismatched = response;
        mismatched.operation.as_mut().unwrap().operation_id = "operation/other".into();
        assert!(accepted_operation_view(mismatched, &mut hint).is_err());
        assert_eq!(hint.operation_id.as_deref(), Some("operation/accepted"));
    }

    #[test]
    fn webview_cannot_supply_approval_principal_grant_or_arbitrary_wire_action() {
        for extra in [
            "confirmed",
            "principal_kind",
            "capability",
            "operation_id",
            "path",
        ] {
            let mut input =
                serde_json::json!({"plan_id":"plan/test", "display_name":"Name", "language":"zh"});
            input[extra] = serde_json::json!(true);
            assert!(
                serde_json::from_value::<RenameInput>(input).is_err(),
                "{extra}"
            );
            let mut agent = serde_json::json!({"spec":{"schema_version":{"major":2,"minor":0},"context_id":"agent-context/codex/test","model":{"intent":"restore","restore_point_ref":"model-restore/test"}},"language":"zh"});
            agent[extra] = serde_json::json!(true);
            assert!(
                serde_json::from_value::<AgentSettingsInput>(agent).is_err(),
                "{extra}"
            );
        }
        assert!(serde_json::from_value::<RestoreInput>(serde_json::json!({"plan_id":"plan/test", "language":"en", "previous_name":"forged"})).is_err());
    }
}
