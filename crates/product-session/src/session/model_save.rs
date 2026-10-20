use super::{Session, terminal};
use crate::authority::HostAuthority;
use crate::confirmation::ConfirmationPermit;
use crate::failure::DesktopFailure;
use crate::operation_observation::{IntentEvidence, SubmittedOperation};
use hiroute_application_api::*;
use serde::Serialize;

// Backend-only handoff. The WebView cannot construct a confirmation permit or change the request
// after the shared confirmation host has displayed the backend prompt.
pub struct ModelSaveConfirmation {
    requires_confirmation: bool,
    permit: ConfirmationPermit,
    request: ComputeConnectionApplyRequestV1,
    input_candidates: Vec<ComputeCandidateRefV2>,
    language: String,
    message: String,
}
impl ModelSaveConfirmation {
    pub fn revision(&self) -> u64 {
        self.request.expected_revisions.target
    }
    pub fn requires_confirmation(&self) -> bool {
        self.requires_confirmation
    }
    pub fn message(&self) -> String {
        self.message.clone()
    }
    pub fn english(&self) -> bool {
        self.language == "en"
    }
}
#[derive(Serialize)]
pub struct ModelSaveAccepted {
    pub result: ApplyResultV1,
    pub operation: OperationReferenceV1,
}
impl Session {
    pub async fn prepare_model_save_confirmation(
        &mut self,
        request: ComputeConnectionApplyRequestV1,
        language: String,
    ) -> Result<ModelSaveConfirmation, DesktopFailure> {
        if !matches!(language.as_str(), "zh" | "en") {
            return Err("REQUEST_INVALID".into());
        }
        // The daemon seals and revalidates the original Preview at writer admission. A
        // second Preview here races ordinary control-head changes and can reject an otherwise
        // valid first save before Apply has a chance to perform its authoritative CAS.
        let change =
            hiroute_application_api::saved_compute_change(request.spec.desired_state.clone())
                .map_err(|_| "REQUEST_INVALID")?;
        change.validate_shape().map_err(|_| "REQUEST_INVALID")?;
        let input_candidates = change
            .key_edits
            .iter()
            .filter_map(|edit| match edit {
                ComputeKeyEditV2::Add { input_candidate }
                | ComputeKeyEditV2::Replace {
                    input_candidate, ..
                } => Some(input_candidate.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        // Adding a credential or changing its order is an ordinary save. Replacing,
        // removing, or changing the enabled state of an existing credential keeps
        // the single risk confirmation required by the shared access policy.
        let requires_confirmation = change.key_edits.iter().any(|edit| {
            matches!(
                edit,
                ComputeKeyEditV2::Replace { .. }
                    | ComputeKeyEditV2::Remove { .. }
                    | ComputeKeyEditV2::SetEnabled { .. }
            )
        });
        let mut message = if language == "en" {
            "Change existing source credentials?\nThis will replace, remove or change the availability/order of the credentials selected in the editor. Future calls will use the saved settings."
        } else {
            "修改既有来源凭据？\n将替换、移除或调整编辑器中选定凭据的可用性或顺序。后续调用将使用保存后的设置。"
        }.to_owned();
        if requires_confirmation {
            let masks = self.confirmation_key_masks(&change).await;
            message.push_str(&confirmation_key_details(
                &change,
                masks.as_ref(),
                language == "en",
            ));
        }
        Ok(ModelSaveConfirmation {
            requires_confirmation,
            permit: self.confirmation.begin()?,
            request,
            input_candidates,
            language,
            message,
        })
    }

    async fn confirmation_key_masks(
        &self,
        change: &ComputeManagementChangeV2,
    ) -> Option<ComputeKeyMasksViewV1> {
        let ComputeManagementSubjectV2::SavedSource { source_id } = &change.subject else {
            return None;
        };
        let snapshot = self
            .client
            .compute_management_snapshot_v3(
                &crate::random_id().ok()?,
                ComputeManagementQueryV2::default(),
            )
            .await
            .ok()?
            .data?;
        let source = snapshot
            .sources
            .iter()
            .find(|source| &source.source_id == source_id)?;
        super::query(
            &self.client,
            GET_COMPUTE_KEY_MASKS_OPERATION_V1,
            &ComputeKeyMasksQueryV1 {
                source_id: source_id.clone(),
                expected_source_revision: source.revision,
            },
        )
        .await
        .ok()
    }

    pub async fn finish_model_save_confirmation(
        &mut self,
        context: ModelSaveConfirmation,
        accepted: bool,
    ) -> Result<ModelSaveAccepted, DesktopFailure> {
        if !self.confirmation.finish(&context.permit, accepted)? {
            return Err("MODEL_SAVE_CANCELLED".into());
        }
        let hint = SubmittedOperation {
            plan_id: context
                .request
                .spec
                .resource_id
                .clone()
                .unwrap_or_else(|| "compute-management".into()),
            principal_kind: PrincipalKind::InteractiveUser,
            operation_kind: APPLY_COMPUTE_SAVE_OPERATION_V2.into(),
            idempotency_key: context.request.idempotency_key.clone(),
            accepted_digest: context.request.accept_digest.clone(),
            operation_id: None,
            after_sequence: 0,
            intent: Some(IntentEvidence::model_save(&context.request)),
            latest_edit_not_applied: false,
        };

        self.hint = Some(hint);
        let model_save_key = context.request.idempotency_key.clone();
        self.pending_model_inputs
            .insert(model_save_key.clone(), context.input_candidates.clone());
        let response = self
            .client
            .apply_compute_save(&crate::random_id()?, context.request)
            .await;
        if let Ok(envelope) = &response
            && let Some(operation) = &envelope.operation
            && let Some(hint) = &mut self.hint
        {
            hint.operation_id = Some(operation.operation_id.clone());
            hint.after_sequence = operation.sequence;
        }
        let (result, operation) = match response {
            Ok(envelope) if envelope.operation.is_some() || envelope.error.is_none() => {
                let operation = match envelope.operation {
                    Some(operation) => operation,
                    None => self
                        .find_pending()
                        .await?
                        .as_ref()
                        .map(operation_reference)
                        .ok_or("RESPONSE_OPERATION_MISSING")?,
                };
                let result = envelope.data.unwrap_or_else(|| ApplyResultV1 {
                    operation_id: operation.operation_id.clone(),
                    accepted_digest: self
                        .hint
                        .as_ref()
                        .expect("model recovery hint exists")
                        .accepted_digest
                        .clone(),
                    state: operation.state.clone(),
                });
                (result, operation)
            }
            Ok(envelope) => {
                // A failed envelope without an Operation is definitive. It needs no
                // recovery read, which could itself fail and turn a safe retry into an
                // apparently uncertain save.
                self.pending_model_inputs.remove(&model_save_key);
                self.hint = None;
                let erased = serde_json::from_value(
                    serde_json::to_value(envelope).map_err(|_| "RESPONSE_DATA_INVALID")?,
                )
                .map_err(|_| "RESPONSE_DATA_INVALID")?;
                return Err(DesktopFailure::backend(erased));
            }
            Err(failure) => {
                let recovered = self.find_pending().await?;
                let Some(recovered) = recovered.as_ref() else {
                    return Err(failure.into());
                };
                let operation = operation_reference(recovered);
                let result = ApplyResultV1 {
                    operation_id: operation.operation_id.clone(),
                    accepted_digest: self
                        .hint
                        .as_ref()
                        .expect("model recovery hint exists")
                        .accepted_digest
                        .clone(),
                    state: operation.state.clone(),
                };
                (result, operation)
            }
        };
        if terminal(&operation.state) {
            self.release_pending_model_inputs(&model_save_key)?;
        }
        self.resume_observing();
        Ok(ModelSaveAccepted { result, operation })
    }
    pub(super) fn release_pending_model_inputs(
        &mut self,
        idempotency_key: &str,
    ) -> Result<(), DesktopFailure> {
        let Some(candidates) = self.pending_model_inputs.get(idempotency_key).cloned() else {
            return Ok(());
        };
        for candidate in &candidates {
            self.authority.release_model_input(candidate)?;
        }
        self.pending_model_inputs.remove(idempotency_key);
        Ok(())
    }
}

fn confirmation_key_details(
    change: &ComputeManagementChangeV2,
    masks: Option<&ComputeKeyMasksViewV1>,
    english: bool,
) -> String {
    let label = |id: &str, generation: Option<u64>| {
        let mask = masks
            .and_then(|view| {
                view.keys.iter().find(|key| {
                    key.key_id == id
                        && generation.is_none_or(|generation| generation == key.generation)
                })
            })
            .map(|key| key.masked_key.as_str());
        let suffix: String = id
            .chars()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        mask.map_or_else(
            || format!("ID {suffix}"),
            |mask| format!("{mask} · {suffix}"),
        )
    };
    let mut details = String::new();
    for edit in &change.key_edits {
        let (action, labels) = match edit {
            ComputeKeyEditV2::Remove {
                key_id,
                expected_generation,
            } => (
                if english { "Remove" } else { "移除" },
                label(key_id, Some(*expected_generation)),
            ),
            ComputeKeyEditV2::Replace {
                key_id,
                expected_generation,
                ..
            } => (
                if english { "Replace" } else { "替换" },
                label(key_id, Some(*expected_generation)),
            ),
            ComputeKeyEditV2::SetEnabled {
                key_id,
                expected_generation,
                enabled,
            } => (
                if *enabled {
                    if english { "Enable" } else { "启用" }
                } else if english {
                    "Disable"
                } else {
                    "停用"
                },
                label(key_id, Some(*expected_generation)),
            ),
            ComputeKeyEditV2::SetOrder { key_ids } => (
                if english {
                    "Keep in order"
                } else {
                    "保存后保留的顺序"
                },
                key_ids
                    .iter()
                    .map(|id| label(id, None))
                    .collect::<Vec<_>>()
                    .join(" → "),
            ),
            _ => continue,
        };
        details.push_str(&format!("\n{action}: {labels}"));
    }
    details
}

fn operation_reference(operation: &ClientOperationViewV1) -> OperationReferenceV1 {
    OperationReferenceV1 {
        operation_id: operation.operation_id.clone(),
        state: operation.state.clone(),
        sequence: operation.sequence,
        cancellable: operation.cancellable,
    }
}

#[cfg(test)]
#[path = "model_save_tests.rs"]
mod tests;
