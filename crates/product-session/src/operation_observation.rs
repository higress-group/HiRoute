//! Ephemeral request identities for observing the current Desktop interaction.
use hiroute_application_api::{CanonicalDigest, PrincipalKind};
use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct SubmittedOperation {
    pub plan_id: String,
    pub principal_kind: PrincipalKind,
    pub operation_kind: String,
    pub idempotency_key: String,
    pub accepted_digest: CanonicalDigest,
    pub operation_id: Option<String>,
    pub after_sequence: u64,
    pub intent: Option<IntentEvidence>,
    pub latest_edit_not_applied: bool,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct IntentEvidence {
    pub schema: String,
    pub digest: CanonicalDigest,
}
impl IntentEvidence {
    pub fn rename(plan_id: &str, display_name: &str) -> Self {
        Self {
            schema: "hiroute.desktop-rename-intent/v1".into(),
            digest: CanonicalDigest::of_bytes(
                &serde_json::to_vec(&(plan_id, display_name)).expect("string tuple serializes"),
            ),
        }
    }

    pub fn model_save(request: &hiroute_application_api::ComputeConnectionApplyRequestV1) -> Self {
        Self {
            schema: "hiroute.desktop-model-save-intent/v1".into(),
            digest: CanonicalDigest::of_bytes(
                &serde_json::to_vec(&(&request.spec, &request.expected_revisions))
                    .expect("typed model save intent serializes"),
            ),
        }
    }

    pub fn subscription_check(
        request: &hiroute_application_api::ComputeConnectionApplyRequestV1,
    ) -> Self {
        Self {
            schema: "hiroute.desktop-subscription-check-intent/v1".into(),
            digest: CanonicalDigest::of_bytes(
                &serde_json::to_vec(&(&request.spec, &request.expected_revisions))
                    .expect("typed subscription check intent serializes"),
            ),
        }
    }

    pub fn worker_dependency_selection(
        request: &hiroute_application_api::WorkerDependenciesSelectRequestV1,
    ) -> Self {
        Self {
            schema: "hiroute.desktop-worker-dependency-selection-intent/v1".into(),
            digest: CanonicalDigest::of(request)
                .expect("typed Worker dependency selection intent serializes"),
        }
    }
}

pub async fn lookup(
    client: &hiroute_client_core::Client,
    pending: &mut Option<SubmittedOperation>,
) -> Result<Option<hiroute_application_api::ClientOperationViewV1>, crate::failure::DesktopFailure>
{
    use crate::session::query;
    use hiroute_application_api::{OperationIdempotencyLookupV1, OperationIdempotencyResultV1};
    let Some(hint) = &*pending else {
        return Ok(None);
    };
    let result: OperationIdempotencyResultV1 = query(
        client,
        "FindOperationByIdempotency",
        &OperationIdempotencyLookupV1 {
            include_model_budget_conflict: true,
            principal_kind: hint.principal_kind,
            operation_kind: hint.operation_kind.clone(),
            idempotency_key: hint.idempotency_key.clone(),
            accepted_digest: hint.accepted_digest.clone(),
        },
    )
    .await?;
    if !result.digest_matches && result.operation.is_none() {
        return Err("IDEMPOTENCY_KEY_REUSED".into());
    }
    if let Some(operation) = &result.operation {
        let hint = pending.as_mut().expect("existing hint");
        if hint
            .operation_id
            .as_ref()
            .is_some_and(|id| id != &operation.operation_id)
        {
            return Err("OPERATION_IDENTITY_MISMATCH".into());
        }
        if operation.sequence < hint.after_sequence {
            return Err("OPERATION_SNAPSHOT_STALE".into());
        }
        hint.operation_id = Some(operation.operation_id.clone());
        if !result.digest_matches {
            hint.accepted_digest = operation.accepted_digest.clone();
            hint.latest_edit_not_applied = true;
        }
        hint.after_sequence = operation.sequence;
    }
    Ok(result.operation)
}
