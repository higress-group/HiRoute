use hiroute_domain::{
    CanonicalDigest, ChangeSpecV1, GatewayAuthenticationSemanticsV1, MaterializationState,
    RevisionSetV1, UpstreamProtocol,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::OperationReferenceV1;

pub const COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2: &str = "hiroute.compute-management-change/v2";
pub const COMPUTE_SUBSCRIPTION_CHECK_SCHEMA_V2: &str = "hiroute.compute-subscription-check/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCandidateRefV2 {
    pub candidate_ref: String,
    /// Server-issued revision of the complete candidate facts. This is never a UI edit counter.
    pub candidate_revision: u64,
}

impl ComputeCandidateRefV2 {
    pub fn validate_shape(&self) -> Result<(), ComputeManagementContractErrorV2> {
        if self.candidate_ref.trim().is_empty() || self.candidate_revision == 0 {
            Err(ComputeManagementContractErrorV2::InvalidCandidate)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCheckCorrelationV2 {
    pub candidate_ref: String,
    /// Client draft counter used only to discard a stale check result.
    pub edit_revision: u64,
    pub check_id: String,
    pub input_digest: CanonicalDigest,
}

impl ComputeCheckCorrelationV2 {
    pub fn validate_for(
        &self,
        candidate: &ComputeCandidateRefV2,
    ) -> Result<(), ComputeManagementContractErrorV2> {
        candidate.validate_shape()?;
        if self.candidate_ref != candidate.candidate_ref
            || self.check_id.trim().is_empty()
            || self.edit_revision == 0
            || self.input_digest == CanonicalDigest::of_bytes(&[])
        {
            Err(ComputeManagementContractErrorV2::InvalidCorrelation)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeCandidateProducerV2 {
    Native,
    Cpa,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeCandidateProvenanceKindV2 {
    Registered,
    UserConfigured,
    ConnectorOwned,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeModelMembershipV2 {
    Catalog,
    Observed,
    UserDeclared,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeCandidateInputStateV2 {
    NotRequired,
    Provided,
    Missing,
    Unavailable,
}

/// Completeness of the trusted facts behind a public candidate.
///
/// This reports fact presence only. `Complete` allows the candidate to enter save preview, while
/// save intent validation, persisted management state, and runtime readiness remain separate
/// decisions.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeCandidateFactStateV2 {
    PendingCredential,
    PendingApproval,
    Complete,
}

impl ComputeCandidateFactStateV2 {
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// Safe, normalized candidate target. It contains no URL userinfo, query, credential, or locator.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCandidateTargetV2 {
    pub scheme: String,
    pub authority: String,
    pub port: u16,
    pub request_path: String,
    pub upstream_protocol: UpstreamProtocol,
    pub protocol_profile_id: String,
    pub protocol_profile_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCandidateModelViewV2 {
    pub model_ref: String,
    pub upstream_model_id: String,
    pub display_name: String,
    pub membership: ComputeModelMembershipV2,
    pub fact_basis: ComputeCandidateModelFactBasisV2,
    pub selectable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeCandidateModelFactBasisV2 {
    RegisteredCatalog,
    RuntimeFallback,
    Observed,
    UserDeclared,
    ConnectorVerified,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCandidateIssueV2 {
    pub code: String,
    pub message_key: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeValidationRefV2 {
    pub approval_operation: OperationReferenceV1,
    pub validation_ref: String,
    pub validation_revision: u64,
}

impl ComputeValidationRefV2 {
    pub fn validate_shape(&self) -> Result<(), ComputeManagementContractErrorV2> {
        if self.approval_operation.operation_id.trim().is_empty()
            || self.validation_ref.trim().is_empty()
            || self.validation_revision == 0
        {
            Err(ComputeManagementContractErrorV2::InvalidValidation)
        } else {
            Ok(())
        }
    }
}

/// Public candidate view shared by connection flows and the model-management UI.
///
/// Protected input slots, filesystem selectors, Secret references, OAuth material, account refs,
/// and runtime capabilities have no representation in this type.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeCandidateViewV2 {
    pub candidate: ComputeCandidateRefV2,
    pub correlation: ComputeCheckCorrelationV2,
    pub producer: ComputeCandidateProducerV2,
    pub provenance: ComputeCandidateProvenanceKindV2,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_source_id: Option<String>,
    pub models: Vec<ComputeCandidateModelViewV2>,
    pub input_state: ComputeCandidateInputStateV2,
    pub fact_state: ComputeCandidateFactStateV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ComputeValidationRefV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ComputeCandidateIssueV2>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeManagementSubjectV2 {
    Candidate { candidate: ComputeCandidateRefV2 },
    SavedSource { source_id: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeManagementIntentV2 {
    SaveReady,
    SaveDisabled,
}

impl ComputeManagementIntentV2 {
    /// The one mapping used by the connection UI's `enable` input.
    pub const fn from_enable(enable: bool) -> Self {
        if enable {
            Self::SaveReady
        } else {
            Self::SaveDisabled
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeKeyEditV2 {
    Add {
        input_candidate: ComputeCandidateRefV2,
    },
    Replace {
        key_id: String,
        expected_generation: u64,
        input_candidate: ComputeCandidateRefV2,
    },
    Remove {
        key_id: String,
        expected_generation: u64,
    },
    SetEnabled {
        key_id: String,
        expected_generation: u64,
        enabled: bool,
    },
    SetOrder {
        key_ids: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementChangeV2 {
    pub schema: String,
    pub subject: ComputeManagementSubjectV2,
    pub expected_revisions: RevisionSetV1,
    pub selected_model_refs: Vec<String>,
    pub intent: ComputeManagementIntentV2,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub key_edits: Vec<ComputeKeyEditV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ComputeValidationRefV2>,
    /// Explicit lifecycle intent. Omitted legacy saves retain replacement semantics and bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<ComputeManagementEditV1>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeManagementEditV1 {
    Rename { display_name: String },
    AppendModels,
    RemoveModels,
    Delete,
}

impl ComputeManagementChangeV2 {
    pub fn validate_shape(&self) -> Result<(), ComputeManagementContractErrorV2> {
        if self.schema != COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2 {
            return Err(ComputeManagementContractErrorV2::UnsupportedSchema);
        }
        match &self.subject {
            ComputeManagementSubjectV2::Candidate { candidate } => candidate.validate_shape()?,
            ComputeManagementSubjectV2::SavedSource { source_id }
                if source_id.trim().is_empty() =>
            {
                return Err(ComputeManagementContractErrorV2::InvalidCandidate);
            }
            ComputeManagementSubjectV2::SavedSource { .. } => {}
        }
        if self
            .selected_model_refs
            .iter()
            .any(|value| value.trim().is_empty())
        {
            return Err(ComputeManagementContractErrorV2::InvalidCandidate);
        }
        if let Some(validation) = &self.validation {
            validation.validate_shape()?;
        }
        if let Some(edit) = &self.edit {
            let saved = matches!(self.subject, ComputeManagementSubjectV2::SavedSource { .. });
            let valid = match edit {
                ComputeManagementEditV1::Rename { display_name } => {
                    !display_name.trim().is_empty()
                        && display_name.chars().count() <= 60
                        && !display_name.chars().any(char::is_control)
                        && (!saved || self.selected_model_refs.is_empty())
                }
                ComputeManagementEditV1::AppendModels => {
                    !saved && !self.selected_model_refs.is_empty()
                }
                ComputeManagementEditV1::RemoveModels => {
                    saved && !self.selected_model_refs.is_empty()
                }
                ComputeManagementEditV1::Delete => saved && self.selected_model_refs.is_empty(),
            };
            if !valid
                || (!self.key_edits.is_empty()
                    && (saved || !matches!(edit, ComputeManagementEditV1::Rename { .. })))
            {
                return Err(ComputeManagementContractErrorV2::InvalidCandidate);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSaveChangeViewV2 {
    pub resource_kind: String,
    pub resource_id: String,
    pub action: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSavePreviewV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<ComputeCandidateRefV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ComputeValidationRefV2>,
    pub spec: ChangeSpecV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
    pub changes: Vec<ComputeSaveChangeViewV2>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_plan_refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeSaveDispositionV2 {
    Saved,
    Pending,
    NeedsInput,
    Conflict,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSavedBindingV2 {
    pub model_ref: String,
    pub binding_id: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSaveResultV2 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<ComputeCandidateRefV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ComputeValidationRefV2>,
    pub disposition: ComputeSaveDispositionV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<ComputeSavedBindingV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub management_state: Option<MaterializationState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<OperationReferenceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSavedSourceExpectationV2 {
    pub source_id: String,
    pub expected_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionCheckChangeV2 {
    pub schema: String,
    pub candidate: ComputeCandidateRefV2,
    pub expected_evidence_digest: CanonicalDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_source: Option<ComputeSavedSourceExpectationV2>,
}

impl ComputeSubscriptionCheckChangeV2 {
    pub fn validate_shape(&self) -> Result<(), ComputeManagementContractErrorV2> {
        if self.schema != COMPUTE_SUBSCRIPTION_CHECK_SCHEMA_V2 {
            return Err(ComputeManagementContractErrorV2::UnsupportedSchema);
        }
        self.candidate.validate_shape()?;
        if self.expected_evidence_digest == CanonicalDigest::of_bytes(&[])
            || self.existing_source.as_ref().is_some_and(|value| {
                value.source_id.trim().is_empty() || value.expected_revision == 0
            })
        {
            return Err(ComputeManagementContractErrorV2::InvalidCandidate);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionCheckPreviewV2 {
    pub candidate: ComputeCandidateRefV2,
    pub display_scope: String,
    pub spec: ChangeSpecV1,
    pub accept_digest: CanonicalDigest,
    pub expected_revisions: RevisionSetV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeSubscriptionCheckStatusV2 {
    Checking,
    Verified,
    SourceChanged,
    NeedsAuth,
    Unavailable,
    Failed,
    Released,
    Retained,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionCheckResultV2 {
    pub candidate: ComputeCandidateRefV2,
    pub approval_operation: OperationReferenceV1,
    pub status: ComputeSubscriptionCheckStatusV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_operation: Option<OperationReferenceV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<ComputeValidationRefV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_candidate: Option<ComputeCandidateViewV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ComputeSubscriptionCheckResultV2 {
    pub fn validate_shape(&self) -> Result<(), ComputeManagementContractErrorV2> {
        self.candidate.validate_shape()?;
        if self.approval_operation.operation_id.trim().is_empty() {
            return Err(ComputeManagementContractErrorV2::InvalidOperationLink);
        }
        if let Some(validation) = &self.validation {
            validation.validate_shape()?;
            if validation.approval_operation.operation_id != self.approval_operation.operation_id {
                return Err(ComputeManagementContractErrorV2::InvalidOperationLink);
            }
        }
        if self.save_operation.as_ref().is_some_and(|save| {
            save.operation_id.trim().is_empty()
                || save.operation_id == self.approval_operation.operation_id
        }) {
            return Err(ComputeManagementContractErrorV2::InvalidOperationLink);
        }
        match self.status {
            ComputeSubscriptionCheckStatusV2::Checking
                if self.validation.is_some()
                    || self.checked_candidate.is_some()
                    || self.save_operation.is_some() =>
            {
                Err(ComputeManagementContractErrorV2::InvalidValidation)
            }
            ComputeSubscriptionCheckStatusV2::Verified => {
                let checked = self
                    .checked_candidate
                    .as_ref()
                    .ok_or(ComputeManagementContractErrorV2::InvalidValidation)?;
                if self.validation.is_none()
                    || checked.candidate.validate_shape().is_err()
                    || checked
                        .correlation
                        .validate_for(&checked.candidate)
                        .is_err()
                    || checked.candidate.candidate_ref != self.candidate.candidate_ref
                    || checked.candidate.candidate_revision <= self.candidate.candidate_revision
                {
                    Err(ComputeManagementContractErrorV2::InvalidValidation)
                } else {
                    Ok(())
                }
            }
            ComputeSubscriptionCheckStatusV2::Retained if self.save_operation.is_none() => {
                Err(ComputeManagementContractErrorV2::InvalidOperationLink)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ComputeManagementContractErrorV2 {
    #[error("unsupported compute-management schema")]
    UnsupportedSchema,
    #[error("invalid compute candidate reference")]
    InvalidCandidate,
    #[error("invalid compute check correlation")]
    InvalidCorrelation,
    #[error("invalid compute validation reference or result")]
    InvalidValidation,
    #[error("approval and save Operation links are inconsistent")]
    InvalidOperationLink,
}

/// Authentication metadata is safe to share; credential material and selectors are not.
pub type ComputeAuthenticationV2 = GatewayAuthenticationSemanticsV1;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;

    fn operation(id: &str) -> OperationReferenceV1 {
        OperationReferenceV1 {
            operation_id: id.to_owned(),
            state: "accepted".to_owned(),
            sequence: 1,
            cancellable: true,
        }
    }

    fn candidate(revision: u64) -> ComputeCandidateRefV2 {
        ComputeCandidateRefV2 {
            candidate_ref: "candidate/native".to_owned(),
            candidate_revision: revision,
        }
    }

    #[test]
    fn edit_revision_cannot_replace_the_server_candidate_revision() {
        let correlation = ComputeCheckCorrelationV2 {
            candidate_ref: "candidate/native".to_owned(),
            edit_revision: 7,
            check_id: "check/7".to_owned(),
            input_digest: CanonicalDigest::of_bytes(b"draft-7"),
        };
        let candidate = candidate(42);

        assert_eq!(correlation.edit_revision, 7);
        assert_eq!(candidate.candidate_revision, 42);
        assert_eq!(correlation.validate_for(&candidate), Ok(()));
    }

    #[test]
    fn enable_has_one_save_intent_mapping() {
        assert_eq!(
            ComputeManagementIntentV2::from_enable(true),
            ComputeManagementIntentV2::SaveReady
        );
        assert_eq!(
            ComputeManagementIntentV2::from_enable(false),
            ComputeManagementIntentV2::SaveDisabled
        );
    }

    #[test]
    fn candidate_fact_state_has_stable_wire_values() {
        let cases = [
            (
                ComputeCandidateFactStateV2::PendingCredential,
                "pending_credential",
                false,
            ),
            (
                ComputeCandidateFactStateV2::PendingApproval,
                "pending_approval",
                false,
            ),
            (ComputeCandidateFactStateV2::Complete, "complete", true),
        ];

        for (state, wire, complete) in cases {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(wire));
            assert_eq!(state.is_complete(), complete);
        }
    }

    #[test]
    fn public_candidate_rejects_protected_input_fields() {
        let value = json!({
            "candidate": {"candidate_ref":"candidate/native","candidate_revision":42},
            "correlation": {
                "candidate_ref":"candidate/native",
                "edit_revision":7,
                "check_id":"check/7",
                "input_digest": CanonicalDigest::of_bytes(b"draft-7")
            },
            "producer":"native",
            "provenance":"user_configured",
            "display_name":"Native API",
            "models":[],
            "input_state":"provided",
            "fact_state":"complete",
            "issues":[],
            "input_slot":"protected-canary"
        });

        assert!(serde_json::from_value::<ComputeCandidateViewV2>(value).is_err());
    }

    #[test]
    fn subscription_validation_keeps_approval_and_save_operations_distinct() {
        let approval = operation("operation/check-a");
        let save = operation("operation/save-b");
        let result = ComputeSubscriptionCheckResultV2 {
            candidate: candidate(11),
            approval_operation: approval.clone(),
            status: ComputeSubscriptionCheckStatusV2::Retained,
            save_operation: Some(save.clone()),
            validation: Some(ComputeValidationRefV2 {
                approval_operation: approval.clone(),
                validation_ref: "validation/cpa-1".to_owned(),
                validation_revision: 1,
            }),
            checked_candidate: None,
            reason: None,
        };

        let encoded = serde_json::to_value(&result).unwrap();
        let decoded: ComputeSubscriptionCheckResultV2 = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.validate_shape(), Ok(()));
        assert_eq!(decoded.approval_operation, approval);
        assert_eq!(decoded.save_operation, Some(save));
        assert_ne!(
            decoded.approval_operation.operation_id,
            decoded.save_operation.unwrap().operation_id
        );

        let mut invalid = result;
        invalid.save_operation = Some(invalid.approval_operation.clone());
        assert_eq!(
            invalid.validate_shape(),
            Err(ComputeManagementContractErrorV2::InvalidOperationLink)
        );
    }

    #[test]
    fn management_change_remains_a_closed_versioned_shape() {
        let change = ComputeManagementChangeV2 {
            edit: None,
            schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.to_owned(),
            subject: ComputeManagementSubjectV2::Candidate {
                candidate: candidate(42),
            },
            expected_revisions: RevisionSetV1 {
                target: 1,
                dependencies: BTreeMap::new(),
            },
            selected_model_refs: vec!["model/one".to_owned()],
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(),
            validation: None,
        };
        assert_eq!(change.validate_shape(), Ok(()));
        let mut encoded = serde_json::to_value(change).unwrap();
        encoded["protected_input_slot"] = json!("must-not-be-accepted");

        assert!(serde_json::from_value::<ComputeManagementChangeV2>(encoded).is_err());
    }

    #[test]
    fn subscription_check_is_separate_from_model_selection() {
        let check = json!({
            "schema": COMPUTE_SUBSCRIPTION_CHECK_SCHEMA_V2,
            "candidate": {"candidate_ref":"candidate/cpa","candidate_revision":10},
            "expected_evidence_digest": CanonicalDigest::of_bytes(b"cpa-evidence"),
            "selected_model_refs": ["model/must-not-be-accepted"]
        });

        assert!(serde_json::from_value::<ComputeSubscriptionCheckChangeV2>(check).is_err());
    }
}
