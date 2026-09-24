//! Durable protected subscription-validation record codec.
//!
//! This representation lives only in control.db. It is never embedded in Operation JSON or
//! returned directly to Local Control clients.

use hiroute_application::compute_management::{
    ComputeCandidateCapabilityFactsV2, ComputeCandidateFactBasisV2, ComputeCandidateFactValueV2,
    ComputeCandidateFactsV2, ComputeCandidateModelFactsV2, ComputeCandidateProvenanceV2,
    ComputeCredentialBindingV2, ComputeSubscriptionValidationFactsV2,
};
use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateRefV2, ComputeCandidateTargetV2,
    ComputeCheckCorrelationV2, ComputeModelMembershipV2, ComputeValidationRefV2,
    OperationReferenceV1,
};
use hiroute_domain::{
    CanonicalDigest, GatewayAuthenticationSemanticsV1, NativeReasoningCapabilityV1, PortResult,
};
use serde::{Deserialize, Serialize};

use super::{CONNECTOR_ID, invalid};

const RECORD_SCHEMA: &str = "hiroute.compute-subscription-validation-record/v1";

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum StoredFactBasisV1 {
    RegisteredCatalog,
    RuntimeFallback,
    Observed,
    UserDeclared,
    ConnectorVerified,
    Unknown,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredFactV1<T> {
    value: Option<T>,
    basis: StoredFactBasisV1,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredCapabilitiesV1 {
    tool: StoredFactV1<bool>,
    vision: StoredFactV1<bool>,
    streaming: StoredFactV1<bool>,
    context_tokens: StoredFactV1<u64>,
    max_output_tokens: StoredFactV1<u64>,
    native_reasoning: StoredFactV1<NativeReasoningCapabilityV1>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredModelV1 {
    model_ref: String,
    upstream_model_id: String,
    display_name: String,
    catalog_configuration_id: Option<String>,
    membership: ComputeModelMembershipV2,
    capabilities: StoredCapabilitiesV1,
    capability_evidence_digest: CanonicalDigest,
    selectable: bool,
    reason: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoredSubscriptionValidationV1 {
    schema: String,
    approval_operation: OperationReferenceV1,
    pub(super) original_candidate: ComputeCandidateRefV2,
    pub(super) checked_candidate: ComputeCandidateRefV2,
    correlation: ComputeCheckCorrelationV2,
    lineage_ref: String,
    trusted_lineage_digest: Option<CanonicalDigest>,
    display_name: String,
    pub(super) existing_source_id: Option<String>,
    pub(super) evidence_digest: CanonicalDigest,
    connector_id: String,
    pub(super) account_ref: String,
    target: ComputeCandidateTargetV2,
    authentication: GatewayAuthenticationSemanticsV1,
    models: Vec<StoredModelV1>,
    pub(super) validation: ComputeValidationRefV2,
    inventory_revision: u64,
    receipt_ref: String,
    receipt_revision: u64,
}

impl StoredSubscriptionValidationV1 {
    pub(super) fn new(
        approval_operation: OperationReferenceV1,
        pending: &ComputeCandidateFactsV2,
        checked: &ComputeCandidateFactsV2,
        validation: &ComputeSubscriptionValidationFactsV2,
    ) -> PortResult<Self> {
        let (connector_id, account_ref) = match &checked.provenance {
            ComputeCandidateProvenanceV2::ConnectorOwned {
                connector_id,
                account_ref,
            } => (connector_id.clone(), account_ref.clone()),
            _ => return Err(invalid("subscription.record.provenance")),
        };
        let target = checked
            .target
            .clone()
            .ok_or_else(|| invalid("subscription.record.target"))?;
        let authentication = checked
            .authentication
            .clone()
            .ok_or_else(|| invalid("subscription.record.authentication"))?;
        Ok(Self {
            schema: RECORD_SCHEMA.into(),
            approval_operation,
            original_candidate: pending.candidate.clone(),
            checked_candidate: checked.candidate.clone(),
            correlation: checked.correlation.clone(),
            lineage_ref: checked.lineage_ref.clone(),
            trusted_lineage_digest: checked.trusted_lineage_digest.clone(),
            display_name: checked.display_name.clone(),
            existing_source_id: checked.existing_source_id.clone(),
            evidence_digest: checked.evidence_digest.clone(),
            connector_id,
            account_ref,
            target,
            authentication,
            models: checked.models.iter().map(StoredModelV1::from).collect(),
            validation: validation.validation.clone(),
            inventory_revision: validation.inventory_revision,
            receipt_ref: validation.resource_receipt.receipt_ref().to_owned(),
            receipt_revision: validation.resource_receipt.revision(),
        })
    }

    pub(super) fn checked_facts(&self) -> PortResult<ComputeCandidateFactsV2> {
        if self.schema != RECORD_SCHEMA
            || self.connector_id != CONNECTOR_ID
            || self.inventory_revision == 0
            || self.receipt_ref.is_empty()
            || self.receipt_revision == 0
        {
            return Err(invalid("subscription.record.shape"));
        }
        let facts = ComputeCandidateFactsV2 {
            candidate: self.checked_candidate.clone(),
            correlation: self.correlation.clone(),
            producer: ComputeCandidateProducerV2::Cpa,
            lineage_ref: self.lineage_ref.clone(),
            trusted_lineage_digest: self.trusted_lineage_digest.clone(),
            display_name: self.display_name.clone(),
            existing_source_id: self.existing_source_id.clone(),
            evidence_digest: self.evidence_digest.clone(),
            provenance: ComputeCandidateProvenanceV2::ConnectorOwned {
                connector_id: self.connector_id.clone(),
                account_ref: self.account_ref.clone(),
            },
            target: Some(self.target.clone()),
            authentication: Some(self.authentication.clone()),
            models: self
                .models
                .iter()
                .map(ComputeCandidateModelFactsV2::from)
                .collect(),
            native_recheck: None,
            additional_native_endpoints: Vec::new(),
            discovery_guard: None,
            credential_binding: ComputeCredentialBindingV2::CpaOwned {
                account_ref: self.account_ref.clone(),
                validation: self.validation.clone(),
            },
            validation: Some(self.validation.clone()),
        };
        facts.validate_shape()?;
        Ok(facts)
    }
}

impl From<&ComputeCandidateModelFactsV2> for StoredModelV1 {
    fn from(value: &ComputeCandidateModelFactsV2) -> Self {
        Self {
            model_ref: value.model_ref.clone(),
            upstream_model_id: value.upstream_model_id.clone(),
            display_name: value.display_name.clone(),
            catalog_configuration_id: value.catalog_configuration_id.clone(),
            membership: value.membership,
            capabilities: StoredCapabilitiesV1::from(&value.capabilities),
            capability_evidence_digest: value.capability_evidence_digest.clone(),
            selectable: value.selectable,
            reason: value.reason.clone(),
        }
    }
}

impl From<&StoredModelV1> for ComputeCandidateModelFactsV2 {
    fn from(value: &StoredModelV1) -> Self {
        Self {
            model_ref: value.model_ref.clone(),
            upstream_model_id: value.upstream_model_id.clone(),
            display_name: value.display_name.clone(),
            catalog_configuration_id: value.catalog_configuration_id.clone(),
            membership: value.membership,
            capabilities: ComputeCandidateCapabilityFactsV2::from(&value.capabilities),
            capability_evidence_digest: value.capability_evidence_digest.clone(),
            selectable: value.selectable,
            reason: value.reason.clone(),
        }
    }
}

impl From<&ComputeCandidateCapabilityFactsV2> for StoredCapabilitiesV1 {
    fn from(value: &ComputeCandidateCapabilityFactsV2) -> Self {
        Self {
            tool: StoredFactV1::from(&value.tool),
            vision: StoredFactV1::from(&value.vision),
            streaming: StoredFactV1::from(&value.streaming),
            context_tokens: StoredFactV1::from(&value.context_tokens),
            max_output_tokens: StoredFactV1::from(&value.max_output_tokens),
            native_reasoning: StoredFactV1::from(&value.native_reasoning),
        }
    }
}

impl From<&StoredCapabilitiesV1> for ComputeCandidateCapabilityFactsV2 {
    fn from(value: &StoredCapabilitiesV1) -> Self {
        Self {
            tool: ComputeCandidateFactValueV2::from(&value.tool),
            vision: ComputeCandidateFactValueV2::from(&value.vision),
            streaming: ComputeCandidateFactValueV2::from(&value.streaming),
            context_tokens: ComputeCandidateFactValueV2::from(&value.context_tokens),
            max_output_tokens: ComputeCandidateFactValueV2::from(&value.max_output_tokens),
            native_reasoning: ComputeCandidateFactValueV2::from(&value.native_reasoning),
        }
    }
}

impl<T: Clone> From<&ComputeCandidateFactValueV2<T>> for StoredFactV1<T> {
    fn from(value: &ComputeCandidateFactValueV2<T>) -> Self {
        Self {
            value: value.value.clone(),
            basis: match value.basis {
                ComputeCandidateFactBasisV2::RegisteredCatalog => {
                    StoredFactBasisV1::RegisteredCatalog
                }
                ComputeCandidateFactBasisV2::RuntimeFallback => StoredFactBasisV1::RuntimeFallback,
                ComputeCandidateFactBasisV2::Observed => StoredFactBasisV1::Observed,
                ComputeCandidateFactBasisV2::UserDeclared => StoredFactBasisV1::UserDeclared,
                ComputeCandidateFactBasisV2::ConnectorVerified => {
                    StoredFactBasisV1::ConnectorVerified
                }
                ComputeCandidateFactBasisV2::Unknown => StoredFactBasisV1::Unknown,
            },
        }
    }
}

impl<T: Clone> From<&StoredFactV1<T>> for ComputeCandidateFactValueV2<T> {
    fn from(value: &StoredFactV1<T>) -> Self {
        Self {
            value: value.value.clone(),
            basis: match value.basis {
                StoredFactBasisV1::RegisteredCatalog => {
                    ComputeCandidateFactBasisV2::RegisteredCatalog
                }
                StoredFactBasisV1::RuntimeFallback => ComputeCandidateFactBasisV2::RuntimeFallback,
                StoredFactBasisV1::Observed => ComputeCandidateFactBasisV2::Observed,
                StoredFactBasisV1::UserDeclared => ComputeCandidateFactBasisV2::UserDeclared,
                StoredFactBasisV1::ConnectorVerified => {
                    ComputeCandidateFactBasisV2::ConnectorVerified
                }
                StoredFactBasisV1::Unknown => ComputeCandidateFactBasisV2::Unknown,
            },
        }
    }
}

pub(super) fn decode_stored(value: &str) -> PortResult<StoredSubscriptionValidationV1> {
    let stored: StoredSubscriptionValidationV1 =
        serde_json::from_str(value).map_err(|_| invalid("subscription.record.decode"))?;
    stored.checked_facts()?;
    Ok(stored)
}
