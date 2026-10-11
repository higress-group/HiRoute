use std::sync::Arc;

use hiroute_application::compute_management::{
    ComputeApprovedSubscriptionCheckV2, ComputeCandidateCapabilityFactsV2,
    ComputeCandidateFactBasisV2, ComputeCandidateFactValueV2, ComputeCandidateModelFactsV2,
    ComputeSubscriptionMaterializationPort, ComputeSubscriptionResourceOwnerV2,
    ComputeSubscriptionResourceReceiptV2, ComputeSubscriptionValidationFactsV2,
    ProtectedInputSourceDescriptorV1,
};
use hiroute_application_api::{
    ComputeCandidateRefV2, ComputeModelMembershipV2, ComputeSavedSourceExpectationV2,
    ComputeValidationRefV2, OperationReferenceV1,
};
use hiroute_domain::{
    CanonicalDigest, EffectiveInventoryModelV1, InventoryDisposition, NativeReasoningCapabilityV1,
    PortError, PortErrorCode, PortResult,
};
use hiroute_integrations::{CpaRegisteredSourceV1, register_cpa_account};

use crate::{BorrowedSubscriptionEvidence, CpaHealth, CpaLifecycleError, ManagedCpaRuntime};

#[derive(Clone)]
pub struct CpaSubscriptionEffectContext {
    approval_operation: OperationReferenceV1,
    candidate: ComputeCandidateRefV2,
    protected_source: ProtectedInputSourceDescriptorV1,
    evidence: BorrowedSubscriptionEvidence,
    existing_source: Option<ComputeSavedSourceExpectationV2>,
    connector_id: String,
    resource_receipt: ComputeSubscriptionResourceReceiptV2,
}

impl CpaSubscriptionEffectContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        approval_operation: OperationReferenceV1,
        candidate: ComputeCandidateRefV2,
        protected_source: ProtectedInputSourceDescriptorV1,
        evidence: BorrowedSubscriptionEvidence,
        existing_source: Option<ComputeSavedSourceExpectationV2>,
        connector_id: impl Into<String>,
        resource_receipt: ComputeSubscriptionResourceReceiptV2,
    ) -> PortResult<Self> {
        let connector_id = connector_id.into();
        candidate.validate_shape().map_err(|_| invalid_context())?;
        if approval_operation.operation_id.trim().is_empty()
            || approval_operation.sequence == 0
            || connector_id != evidence.kind().connector_id()
            || existing_source.as_ref().is_some_and(|source| {
                source.source_id.trim().is_empty() || source.expected_revision == 0
            })
        {
            return Err(invalid_context());
        }
        Ok(Self {
            approval_operation,
            candidate,
            protected_source,
            evidence,
            existing_source,
            connector_id,
            resource_receipt,
        })
    }
}

pub struct CpaSubscriptionMaterializer {
    runtime: Arc<dyn CpaSubscriptionRuntimePort>,
    context: CpaSubscriptionEffectContext,
}

impl CpaSubscriptionMaterializer {
    pub fn new(runtime: Arc<ManagedCpaRuntime>, context: CpaSubscriptionEffectContext) -> Self {
        Self { runtime, context }
    }

    #[cfg(test)]
    fn with_runtime(
        runtime: Arc<dyn CpaSubscriptionRuntimePort>,
        context: CpaSubscriptionEffectContext,
    ) -> Self {
        Self { runtime, context }
    }

    fn validate_input(&self, input: &ComputeApprovedSubscriptionCheckV2) -> PortResult<()> {
        if input.approval_operation != self.context.approval_operation
            || input.candidate != self.context.candidate
            || &input.expected_evidence_digest != self.context.evidence.evidence_digest()
            || input.existing_source != self.context.existing_source
            || input.protected_source != self.context.protected_source
        {
            return Err(PortError::new(
                PortErrorCode::PermissionDenied,
                "cpa-subscription-approved-context",
            ));
        }
        Ok(())
    }
}

impl ComputeSubscriptionMaterializationPort for CpaSubscriptionMaterializer {
    fn materialize_selected(
        &self,
        input: ComputeApprovedSubscriptionCheckV2,
    ) -> PortResult<ComputeSubscriptionValidationFactsV2> {
        self.validate_input(&input)?;
        let observed = self.runtime.inspect().map_err(map_lifecycle_error)?;
        if observed != self.context.evidence {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "cpa-subscription-source-changed",
            ));
        }
        let mut matches = self
            .runtime
            .materialize(&self.context.evidence)
            .map_err(map_lifecycle_error)?
            .into_iter()
            .filter(|source| {
                source.source.connector_id == self.context.connector_id
                    && source.source.identity.account_subject_ref
                        == self.context.evidence.account_ref()
            });
        let registered = matches.next().ok_or_else(|| {
            PortError::new(
                PortErrorCode::NotFound,
                "cpa-subscription-account-not-materialized",
            )
        })?;
        if matches.next().is_some() {
            return Err(PortError::new(
                PortErrorCode::InvalidData,
                "cpa-subscription-account-ambiguous",
            ));
        }
        let inventory = registered
            .inventory
            .iter()
            .cloned()
            .map(|model| {
                let catalog = self
                    .runtime
                    .catalog_model_facts(&registered, &model)
                    .map_err(map_lifecycle_error)?;
                let fallback_allowed = self
                    .runtime
                    .runtime_fallback_allowed(&model.upstream_model_id);
                map_inventory_model(model, catalog, fallback_allowed)
            })
            .collect::<PortResult<Vec<_>>>()?;
        build_validation_facts(&input, &self.context, registered, inventory)
    }
}

#[derive(Clone)]
struct CpaCatalogModelFacts {
    display_name: String,
    tool: bool,
    vision: bool,
    streaming: bool,
    context_tokens: u64,
    max_output_tokens: u64,
    native_reasoning: Option<NativeReasoningCapabilityV1>,
    capability_evidence_digest: CanonicalDigest,
}

trait CpaSubscriptionRuntimePort: Send + Sync {
    fn inspect(&self) -> Result<BorrowedSubscriptionEvidence, CpaLifecycleError>;
    fn materialize(
        &self,
        expected: &BorrowedSubscriptionEvidence,
    ) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError>;
    fn catalog_model_facts(
        &self,
        source: &CpaRegisteredSourceV1,
        model: &EffectiveInventoryModelV1,
    ) -> Result<Option<CpaCatalogModelFacts>, CpaLifecycleError>;
    fn runtime_fallback_allowed(&self, upstream_model_id: &str) -> bool;
}

impl CpaSubscriptionRuntimePort for ManagedCpaRuntime {
    fn inspect(&self) -> Result<BorrowedSubscriptionEvidence, CpaLifecycleError> {
        self.inspect_subscription()
    }

    fn materialize(
        &self,
        expected: &BorrowedSubscriptionEvidence,
    ) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        self.start_expected(Some(expected))?;
        self.discover_materializations(Some(expected))?
            .iter()
            .map(|materialization| {
                register_cpa_account(&self.catalog, materialization)
                    .map_err(|_| CpaLifecycleError::InvalidMaterialization)
            })
            .collect()
    }

    fn catalog_model_facts(
        &self,
        source: &CpaRegisteredSourceV1,
        model: &EffectiveInventoryModelV1,
    ) -> Result<Option<CpaCatalogModelFacts>, CpaLifecycleError> {
        let Some(model_configuration_id) = model.model_configuration_id.as_deref() else {
            return Ok(None);
        };
        let definition = self
            .catalog
            .model_data()
            .model(model_configuration_id)
            .ok_or(CpaLifecycleError::InvalidMaterialization)?;
        let endpoint_capability = self
            .catalog
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|capability| {
                capability.model_configuration_id == model_configuration_id
                    && capability.connector_id == source.source.connector_id
                    && capability.endpoint_profile_id == source.source.identity.endpoint_profile_id
                    && capability.upstream_model_id == model.upstream_model_id
            })
            .ok_or(CpaLifecycleError::InvalidMaterialization)?;
        let native_reasoning = self
            .catalog
            .native_reasoning()
            .iter()
            .find(|candidate| candidate.model_configuration_id == model_configuration_id)
            .map(|candidate| candidate.capability.clone());
        let capability_evidence_digest = CanonicalDigest::of(&(
            "hiroute.cpa-subscription-catalog-capability/v1",
            &endpoint_capability.evidence_digest,
            definition,
            &native_reasoning,
        ))
        .map_err(|_| CpaLifecycleError::InvalidMaterialization)?;
        Ok(Some(CpaCatalogModelFacts {
            display_name: definition.display_name.clone(),
            tool: definition.capabilities.tool,
            vision: definition.capabilities.vision,
            streaming: definition.capabilities.streaming,
            context_tokens: definition.capabilities.context_tokens,
            max_output_tokens: definition.capabilities.max_output_tokens,
            native_reasoning,
            capability_evidence_digest,
        }))
    }

    fn runtime_fallback_allowed(&self, upstream_model_id: &str) -> bool {
        self.catalog
            .runtime_fallback_allows_observed_text(upstream_model_id)
    }
}

fn build_validation_facts(
    input: &ComputeApprovedSubscriptionCheckV2,
    context: &CpaSubscriptionEffectContext,
    registered: CpaRegisteredSourceV1,
    inventory: Vec<ComputeCandidateModelFactsV2>,
) -> PortResult<ComputeSubscriptionValidationFactsV2> {
    let account_ref = registered.source.identity.account_subject_ref.clone();
    let inventory_revision = digest_revision(
        &CanonicalDigest::of(&(
            "hiroute.cpa-subscription-inventory/v1",
            &registered.inventory,
        ))
        .map_err(|_| invalid_materialization())?,
    )?;
    let validation_digest = CanonicalDigest::of(&(
        "hiroute.cpa-subscription-validation/v1",
        &input.approval_operation,
        &input.candidate,
        context.evidence.binding_evidence_digest(),
        inventory_revision,
    ))
    .map_err(|_| invalid_materialization())?;
    let validation = ComputeValidationRefV2 {
        approval_operation: input.approval_operation.clone(),
        validation_ref: format!(
            "validation/cpa/{}",
            validation_digest
                .as_str()
                .strip_prefix("sha256:")
                .unwrap_or(validation_digest.as_str())
        ),
        validation_revision: inventory_revision,
    };
    validation
        .validate_shape()
        .map_err(|_| invalid_materialization())?;
    let resource_owner = match &input.existing_source {
        Some(source) => ComputeSubscriptionResourceOwnerV2::SavedSource(source.clone()),
        None => ComputeSubscriptionResourceOwnerV2::ApprovalOperation {
            operation: input.approval_operation.clone(),
        },
    };
    Ok(ComputeSubscriptionValidationFactsV2 {
        validation,
        original_candidate: input.candidate.clone(),
        verified_evidence_digest: context.evidence.binding_evidence_digest().clone(),
        account_ref,
        inventory_revision,
        inventory,
        resource_owner,
        resource_receipt: context.resource_receipt.clone(),
    })
}

fn map_inventory_model(
    model: EffectiveInventoryModelV1,
    catalog: Option<CpaCatalogModelFacts>,
    runtime_fallback_allowed: bool,
) -> PortResult<ComputeCandidateModelFactsV2> {
    let matched = model.disposition == InventoryDisposition::CatalogMatched;
    let (display_name, capabilities, capability_evidence_digest) = match (matched, catalog) {
        (true, Some(catalog)) => (
            catalog.display_name,
            ComputeCandidateCapabilityFactsV2 {
                tool: registered_fact(catalog.tool),
                vision: registered_fact(catalog.vision),
                streaming: registered_fact(catalog.streaming),
                context_tokens: registered_fact(catalog.context_tokens),
                max_output_tokens: registered_fact(catalog.max_output_tokens),
                native_reasoning: catalog
                    .native_reasoning
                    .map(registered_fact)
                    .unwrap_or_else(unknown_fact),
            },
            catalog.capability_evidence_digest,
        ),
        (true, None) | (false, Some(_)) => return Err(invalid_materialization()),
        (false, None) => {
            let capabilities = if runtime_fallback_allowed {
                runtime_fallback_capabilities()
            } else {
                unknown_capabilities()
            };
            let evidence = CanonicalDigest::of(&(
                if runtime_fallback_allowed {
                    "hiroute.cpa-subscription-runtime-fallback/v1"
                } else {
                    "hiroute.cpa-subscription-observed-model/v1"
                },
                &model.upstream_model_id,
                model.disposition,
                &model.metadata,
                runtime_fallback_allowed,
            ))
            .map_err(|_| invalid_materialization())?;
            (model.upstream_model_id.clone(), capabilities, evidence)
        }
    };
    Ok(ComputeCandidateModelFactsV2 {
        model_ref: format!("cpa-model/{}", model.upstream_model_id),
        upstream_model_id: model.upstream_model_id.clone(),
        display_name,
        catalog_configuration_id: model.model_configuration_id,
        membership: ComputeModelMembershipV2::Observed,
        capabilities,
        capability_evidence_digest,
        selectable: matched || runtime_fallback_allowed,
        reason: (!(matched || runtime_fallback_allowed)).then(|| "inventory_only".to_owned()),
    })
}

fn runtime_fallback_capabilities() -> ComputeCandidateCapabilityFactsV2 {
    ComputeCandidateCapabilityFactsV2 {
        tool: runtime_fallback_fact(true),
        vision: runtime_fallback_fact(false),
        streaming: runtime_fallback_fact(true),
        context_tokens: runtime_fallback_fact(131_072),
        max_output_tokens: runtime_fallback_fact(8_192),
        native_reasoning: runtime_fallback_fact(NativeReasoningCapabilityV1::Fixed {
            profile: "non-thinking".into(),
        }),
    }
}

fn runtime_fallback_fact<T>(value: T) -> ComputeCandidateFactValueV2<T> {
    ComputeCandidateFactValueV2 {
        value: Some(value),
        basis: ComputeCandidateFactBasisV2::RuntimeFallback,
    }
}

fn registered_fact<T>(value: T) -> ComputeCandidateFactValueV2<T> {
    ComputeCandidateFactValueV2 {
        value: Some(value),
        basis: ComputeCandidateFactBasisV2::RegisteredCatalog,
    }
}

fn unknown_capabilities() -> ComputeCandidateCapabilityFactsV2 {
    ComputeCandidateCapabilityFactsV2 {
        tool: unknown_fact(),
        vision: unknown_fact(),
        streaming: unknown_fact(),
        context_tokens: unknown_fact(),
        max_output_tokens: unknown_fact(),
        native_reasoning: unknown_fact(),
    }
}

fn unknown_fact<T>() -> ComputeCandidateFactValueV2<T> {
    ComputeCandidateFactValueV2 {
        value: None,
        basis: ComputeCandidateFactBasisV2::Unknown,
    }
}

fn digest_revision(digest: &CanonicalDigest) -> PortResult<u64> {
    let hex = digest
        .as_str()
        .strip_prefix("sha256:")
        .ok_or_else(invalid_materialization)?;
    let revision = u64::from_str_radix(&hex[..16], 16).map_err(|_| invalid_materialization())?;
    Ok(revision.max(1))
}

fn invalid_context() -> PortError {
    PortError::new(
        PortErrorCode::InvalidData,
        "cpa-subscription-effect-context",
    )
}

fn invalid_materialization() -> PortError {
    PortError::new(
        PortErrorCode::InvalidData,
        "cpa-subscription-materialization",
    )
}

fn map_lifecycle_error(error: CpaLifecycleError) -> PortError {
    if let Some(failure) = error.subscription_failure() {
        return failure;
    }
    let (code, context) = match error {
        CpaLifecycleError::BorrowedCodexAuthSourceChanged
        | CpaLifecycleError::BorrowedClaudeAuthSourceChanged
        | CpaLifecycleError::ManagedOAuthAccountChanged => {
            (PortErrorCode::Conflict, "cpa-subscription-source-changed")
        }
        CpaLifecycleError::StaleSourceManagement => {
            (PortErrorCode::Conflict, "cpa-subscription-management-stale")
        }
        CpaLifecycleError::InvalidMaterialization | CpaLifecycleError::InvalidSpec => (
            PortErrorCode::InvalidData,
            "cpa-subscription-materialization",
        ),
        _ => (PortErrorCode::Unavailable, "cpa-subscription-runtime"),
    };
    PortError::new(code, context)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpaSubscriptionAvailability {
    Ready,
    Stopped,
    RuntimeUnavailable,
    ArtifactUnavailable,
    NeedsAuthentication,
}

pub fn cpa_subscription_availability(
    result: Result<CpaHealth, &CpaLifecycleError>,
) -> CpaSubscriptionAvailability {
    match result {
        Ok(CpaHealth::Ready { .. }) => CpaSubscriptionAvailability::Ready,
        Ok(CpaHealth::Stopped { .. }) => CpaSubscriptionAvailability::Stopped,
        Ok(
            CpaHealth::Crashed { .. } | CpaHealth::Unhealthy { .. } | CpaHealth::CrashLoop { .. },
        ) => CpaSubscriptionAvailability::RuntimeUnavailable,
        Err(
            CpaLifecycleError::Artifact(_)
            | CpaLifecycleError::ArtifactChanged
            | CpaLifecycleError::UnsupportedArtifactVersion,
        ) => CpaSubscriptionAvailability::ArtifactUnavailable,
        Err(
            CpaLifecycleError::BorrowedClaudeAuthMissing
            | CpaLifecycleError::ManagedOAuthCredentialsMissing
            | CpaLifecycleError::InvalidManagedOAuthCredentials
            | CpaLifecycleError::ManagedOAuthAuthenticationRequired
            | CpaLifecycleError::ManagedOAuthAccountChanged
            | CpaLifecycleError::BorrowedClaudeAuthUnavailable
            | CpaLifecycleError::InvalidBorrowedClaudeAuth
            | CpaLifecycleError::BorrowedClaudeAuthSourceChanged
            | CpaLifecycleError::BorrowedCodexAuthMissing
            | CpaLifecycleError::BorrowedCodexStoreUnsupported
            | CpaLifecycleError::BorrowedCodexLoginUnsupported
            | CpaLifecycleError::BorrowedCodexAccountMissing
            | CpaLifecycleError::BorrowedCodexAuthUnavailable
            | CpaLifecycleError::InvalidBorrowedCodexAuth
            | CpaLifecycleError::BorrowedCodexAuthSourceChanged,
        ) => CpaSubscriptionAvailability::NeedsAuthentication,
        Err(_) => CpaSubscriptionAvailability::RuntimeUnavailable,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CpaSubscriptionSaveHandoff {
    NotSubmitted,
    Pending(OperationReferenceV1),
    Saved(OperationReferenceV1),
    FailedAndCompensated(OperationReferenceV1),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CpaSubscriptionReleaseDecision {
    ReleaseApprovalResource,
    ObserveSave(OperationReferenceV1),
    RetainSaved(OperationReferenceV1),
    PreserveExistingSource,
}

pub fn decide_subscription_release(
    owner: &ComputeSubscriptionResourceOwnerV2,
    save: &CpaSubscriptionSaveHandoff,
) -> CpaSubscriptionReleaseDecision {
    if matches!(owner, ComputeSubscriptionResourceOwnerV2::SavedSource(_)) {
        return CpaSubscriptionReleaseDecision::PreserveExistingSource;
    }
    match save {
        CpaSubscriptionSaveHandoff::NotSubmitted
        | CpaSubscriptionSaveHandoff::FailedAndCompensated(_) => {
            CpaSubscriptionReleaseDecision::ReleaseApprovalResource
        }
        CpaSubscriptionSaveHandoff::Pending(operation) => {
            CpaSubscriptionReleaseDecision::ObserveSave(operation.clone())
        }
        CpaSubscriptionSaveHandoff::Saved(operation) => {
            CpaSubscriptionReleaseDecision::RetainSaved(operation.clone())
        }
    }
}

#[cfg(test)]
mod tests;
