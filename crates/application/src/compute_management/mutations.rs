//! Application-owned planning for one atomic source/model/key management save.

use std::collections::BTreeSet;

use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateRefV2, ComputeConnectionApplyRequestV1,
    ComputeKeyEditV2, ComputeManagementChangeV2, ComputeManagementSubjectV2, ComputeSavePreviewV2,
};
use hiroute_domain::{
    CHANGE_SPEC_SCHEMA_V1, CanonicalDigest, ChangeSpecV1, ComputeManagedCredentialV2,
    ComputeManagementRepositoryPort, ComputeManagementSourceV2, ComputeManagementTargetV2,
    CredentialRefV1, GatewayAuthenticationSemanticsV1, MaterializationState, PortError,
    PortErrorCode, SecretMutationV1, SecretStorePort, TransactionPlanV1, WorkspaceId,
};
use thiserror::Error;

use crate::{PreparedTransactionV1, ProtectedInputPort, TransactionError};

use super::mutation_support::*;
use super::{ComputeCandidateFactsV2, ComputeCandidatePort, ComputeCredentialBindingV2};

/// A public preview paired with its sealed, non-wire transaction plan.
pub struct ComputeManagementPreparedPreviewV2 {
    pub result: ComputeSavePreviewV2,
    plan: TransactionPlanV1,
    discovery_guards: Vec<ComputePreparedDiscoveryGuardV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeSubscriptionMaintenanceScopeV1 {
    pub source_id: String,
    pub expected_source_revision: u64,
}

#[derive(Clone, Copy)]
enum ModelSelectionPolicy {
    Strict,
    RetainSubscriptionMembers,
}

impl ComputeManagementPreparedPreviewV2 {
    pub fn plan(&self) -> &TransactionPlanV1 {
        &self.plan
    }
}

/// Admission-only discovery evidence. This value is never serialized into the ChangeSpec or
/// durable Operation plan.
#[derive(Clone)]
pub(crate) struct ComputePreparedDiscoveryGuardV1 {
    pub(crate) input_slot: String,
    pub(crate) evidence_digest: CanonicalDigest,
}

pub struct ComputeManagementPlanner<'a, C, R, S, I> {
    candidates: &'a C,
    repository: &'a R,
    secrets: &'a S,
    protected_inputs: &'a I,
    workspace: WorkspaceId,
}

impl<'a, C, R, S, I> ComputeManagementPlanner<'a, C, R, S, I>
where
    C: ComputeCandidatePort,
    R: ComputeManagementRepositoryPort,
    S: SecretStorePort,
    I: ProtectedInputPort,
{
    pub fn new(candidates: &'a C, repository: &'a R, secrets: &'a S, inputs: &'a I) -> Self {
        Self {
            candidates,
            repository,
            secrets,
            protected_inputs: inputs,
            workspace: WorkspaceId::default(),
        }
    }

    pub fn preview(
        &self,
        change: ComputeManagementChangeV2,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        self.preview_with_policy(change, ModelSelectionPolicy::Strict, None)
    }

    pub fn preview_subscription_maintenance(
        &self,
        change: ComputeManagementChangeV2,
        scope: &ComputeSubscriptionMaintenanceScopeV1,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        self.preview_with_policy(
            change,
            ModelSelectionPolicy::RetainSubscriptionMembers,
            Some(scope),
        )
    }

    fn preview_with_policy(
        &self,
        change: ComputeManagementChangeV2,
        policy: ModelSelectionPolicy,
        maintenance_scope: Option<&ComputeSubscriptionMaintenanceScopeV1>,
    ) -> Result<ComputeManagementPreparedPreviewV2, ComputeManagementPlanningErrorV2> {
        change
            .validate_shape()
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let snapshot = self
            .repository
            .compute_management_snapshot(&self.workspace)?;
        if change.expected_revisions != snapshot.revisions {
            return Err(ComputeManagementPlanningErrorV2::RevisionConflict);
        }

        let resolved = self.resolve_subject(&change, &snapshot.sources)?;
        let current = resolved.current;
        if let Some(scope) = maintenance_scope {
            let source = current
                .as_ref()
                .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
            if source.source_id != scope.source_id
                || source.revision != scope.expected_source_revision
                || source.state != MaterializationState::Ready
                || !source.provenance.is_connector_owned()
                || resolved
                    .candidate
                    .as_ref()
                    .and_then(|candidate| candidate.existing_source_id.as_ref())
                    != Some(&scope.source_id)
            {
                return Err(ComputeManagementPlanningErrorV2::RevisionConflict);
            }
        }
        let selection_policy = match (policy, current.as_ref(), resolved.candidate.as_ref()) {
            (ModelSelectionPolicy::Strict, Some(source), Some(candidate))
                if source.provenance.is_connector_owned()
                    && candidate.producer
                        == hiroute_application_api::ComputeCandidateProducerV2::Cpa
                    && candidate.existing_source_id.as_deref()
                        == Some(source.source_id.as_str()) =>
            {
                ModelSelectionPolicy::RetainSubscriptionMembers
            }
            _ => policy,
        };
        let desired_revision = current
            .as_ref()
            .map_or(0, |source| source.revision)
            .checked_add(1)
            .ok_or(ComputeManagementPlanningErrorV2::InvalidChange)?;
        let mut desired = if let Some(candidate) = resolved.candidate.as_ref() {
            self.source_from_candidate(
                candidate,
                current.as_ref(),
                &change,
                &resolved.source_id,
                &resolved.lineage_digest,
                desired_revision,
                selection_policy,
            )?
        } else {
            let mut source = current
                .clone()
                .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
            source.revision = desired_revision;
            source.models = select_saved_models(&source, &change.selected_model_refs)?;
            validate_saved_validation(&source, change.validation.as_ref())?;
            source
        };

        let mut secret_mutations = Vec::new();
        let mut discovery_guards = Vec::new();
        if let Some(candidate) = resolved.candidate.as_ref() {
            self.materialize_primary_candidate_key(
                candidate,
                current.as_ref(),
                &change,
                &mut desired,
                &mut secret_mutations,
                &mut discovery_guards,
            )?;
        }
        self.apply_key_edits(
            &change.key_edits,
            &resolved.lineage_digest,
            &mut desired,
            &mut secret_mutations,
            &mut discovery_guards,
        )?;
        self.rebind_unchanged_keys(current.as_ref(), &mut desired, &mut secret_mutations)?;
        normalize_key_order(&mut desired.credentials);
        desired.state = management_state(
            change.intent,
            &desired.provenance,
            &desired.authentication,
            &desired.additional_native_endpoints,
            &desired.credentials,
        )?;
        desired
            .validate()
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidCandidate)?;
        secret_mutations.sort_by(|left, right| {
            left.credential()
                .credential_id()
                .cmp(right.credential().credential_id())
        });
        discovery_guards.sort_by(|left, right| left.input_slot.cmp(&right.input_slot));
        discovery_guards.dedup_by(|left, right| {
            left.input_slot == right.input_slot && left.evidence_digest == right.evidence_digest
        });

        let spec = ChangeSpecV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            command_id: "compute.connection.apply".into(),
            resource_id: Some(desired.source_id.clone()),
            desired_state: serde_json::to_value(&change)
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?,
        };
        let plan = TransactionPlanV1::from_compute_management_planner(
            spec.clone(),
            current.as_ref(),
            desired.clone(),
            secret_mutations,
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let accept_digest = CanonicalDigest::of(&(
            "hiroute.compute-management-preview/v2",
            &spec,
            &snapshot.revisions,
            plan.control(),
            plan.secrets(),
        ))
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let changes = preview_changes(current.as_ref(), &desired);
        Ok(ComputeManagementPreparedPreviewV2 {
            result: ComputeSavePreviewV2 {
                candidate: resolved.candidate.map(|facts| facts.candidate),
                validation: change.validation,
                spec,
                accept_digest,
                expected_revisions: snapshot.revisions,
                changes,
                // Plan references are joined by the central routing composition. This local
                // planner never guesses that an absent adapter means an absent reference.
                affected_plan_refs: Vec::new(),
            },
            plan,
            discovery_guards,
        })
    }

    pub fn prepare_apply(
        &self,
        request: ComputeConnectionApplyRequestV1,
    ) -> Result<PreparedTransactionV1, ComputeManagementPlanningErrorV2> {
        if request.idempotency_key.is_empty() {
            return Err(ComputeManagementPlanningErrorV2::InvalidChange);
        }
        let change: ComputeManagementChangeV2 =
            serde_json::from_value(request.spec.desired_state.clone())
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let reproduced = self.preview(change)?;
        if reproduced.result.spec != request.spec
            || reproduced.result.accept_digest != request.accept_digest
            || reproduced.result.expected_revisions != request.expected_revisions
        {
            return Err(ComputeManagementPlanningErrorV2::PreviewStale);
        }
        let apply = hiroute_application_api::ApplyRequestV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            spec: request.spec,
            accept_digest: request.accept_digest.clone(),
            expected_revisions: request.expected_revisions,
            idempotency_key: request.idempotency_key,
            apply_capability: None,
        };
        PreparedTransactionV1::for_compute_management(
            apply,
            request.accept_digest,
            reproduced.plan,
            reproduced.discovery_guards,
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)
    }

    pub fn prepare_subscription_maintenance_apply(
        &self,
        request: ComputeConnectionApplyRequestV1,
        apply_capability: String,
        scope: &ComputeSubscriptionMaintenanceScopeV1,
        revalidate: impl FnOnce() -> Result<(), TransactionError> + Send + 'static,
    ) -> Result<PreparedTransactionV1, ComputeManagementPlanningErrorV2> {
        if request.idempotency_key.is_empty() || apply_capability.is_empty() {
            return Err(ComputeManagementPlanningErrorV2::InvalidChange);
        }
        let change: ComputeManagementChangeV2 =
            serde_json::from_value(request.spec.desired_state.clone())
                .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)?;
        let reproduced = self.preview_subscription_maintenance(change, scope)?;
        if reproduced.result.spec != request.spec
            || reproduced.result.accept_digest != request.accept_digest
            || reproduced.result.expected_revisions != request.expected_revisions
        {
            return Err(ComputeManagementPlanningErrorV2::PreviewStale);
        }
        let apply = hiroute_application_api::ApplyRequestV1 {
            schema_version: CHANGE_SPEC_SCHEMA_V1,
            spec: request.spec,
            accept_digest: request.accept_digest.clone(),
            expected_revisions: request.expected_revisions,
            idempotency_key: request.idempotency_key,
            apply_capability: Some(apply_capability),
        };
        PreparedTransactionV1::for_compute_management(
            apply,
            request.accept_digest,
            reproduced.plan,
            reproduced.discovery_guards,
        )
        .map(|prepared| prepared.with_revalidation(revalidate))
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidChange)
    }

    fn resolve_subject(
        &self,
        change: &ComputeManagementChangeV2,
        sources: &[ComputeManagementSourceV2],
    ) -> Result<ResolvedSubject, ComputeManagementPlanningErrorV2> {
        match &change.subject {
            ComputeManagementSubjectV2::Candidate { candidate } => {
                let facts = self.candidates.resolve_compute_candidate(candidate)?;
                facts.validate_shape()?;
                let lineage_digest = candidate_lineage_digest(&facts)?;
                let current = if let Some(source_id) = &facts.existing_source_id {
                    let source = sources
                        .iter()
                        .find(|source| &source.source_id == source_id)
                        .cloned()
                        .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
                    if source.lineage_digest != lineage_digest {
                        return Err(ComputeManagementPlanningErrorV2::LineageConflict);
                    }
                    Some(source)
                } else {
                    sources
                        .iter()
                        .find(|source| source.lineage_digest == lineage_digest)
                        .cloned()
                };
                let source_id = current.as_ref().map_or_else(
                    || format!("source/managed-{}", digest_suffix(&lineage_digest, 24)),
                    |source| source.source_id.clone(),
                );
                Ok(ResolvedSubject {
                    candidate: Some(facts),
                    current,
                    source_id,
                    lineage_digest,
                })
            }
            ComputeManagementSubjectV2::SavedSource { source_id } => {
                let current = sources
                    .iter()
                    .find(|source| &source.source_id == source_id)
                    .cloned()
                    .ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?;
                Ok(ResolvedSubject {
                    candidate: None,
                    lineage_digest: current.lineage_digest.clone(),
                    current: Some(current),
                    source_id: source_id.clone(),
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn source_from_candidate(
        &self,
        candidate: &ComputeCandidateFactsV2,
        current: Option<&ComputeManagementSourceV2>,
        change: &ComputeManagementChangeV2,
        source_id: &str,
        lineage_digest: &CanonicalDigest,
        revision: u64,
        selection_policy: ModelSelectionPolicy,
    ) -> Result<ComputeManagementSourceV2, ComputeManagementPlanningErrorV2> {
        if matches!(
            candidate.credential_binding,
            ComputeCredentialBindingV2::CpaPendingApproval { .. }
        ) {
            return Err(ComputeManagementPlanningErrorV2::ApprovalRequired);
        }
        let target = candidate
            .target
            .as_ref()
            .ok_or(ComputeManagementPlanningErrorV2::InvalidCandidate)?;
        let authentication = candidate
            .authentication
            .clone()
            .ok_or(ComputeManagementPlanningErrorV2::InvalidCandidate)?;
        let provenance = map_provenance(&candidate.provenance)?;
        let validation = map_validation(candidate.validation.as_ref());
        if candidate.validation.as_ref() != change.validation.as_ref() {
            return Err(ComputeManagementPlanningErrorV2::ValidationConflict);
        }
        let models = match selection_policy {
            ModelSelectionPolicy::Strict => select_candidate_models(
                source_id,
                current,
                candidate,
                &change.selected_model_refs,
                change.intent,
            )?,
            ModelSelectionPolicy::RetainSubscriptionMembers => retain_subscription_models(
                source_id,
                current.ok_or(ComputeManagementPlanningErrorV2::SourceNotFound)?,
                candidate,
                &change.selected_model_refs,
            )?,
        };
        Ok(ComputeManagementSourceV2 {
            schema: hiroute_domain::COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
            source_id: source_id.into(),
            revision,
            lineage_digest: lineage_digest.clone(),
            display_name: candidate.display_name.clone(),
            provenance,
            target: ComputeManagementTargetV2 {
                scheme: target.scheme.clone(),
                authority: target.authority.clone(),
                port: target.port,
                request_path: target.request_path.clone(),
                upstream_protocol: target.upstream_protocol,
                protocol_profile_id: target.protocol_profile_id.clone(),
                protocol_profile_revision: target.protocol_profile_revision,
            },
            authentication,
            state: MaterializationState::Disabled,
            models,
            native_recheck: candidate.native_recheck.clone(),
            additional_native_endpoints: candidate.additional_native_endpoints.clone(),
            credentials: current
                .map(|source| source.credentials.clone())
                .unwrap_or_default(),
            validation,
            last_candidate_ref: candidate.candidate.candidate_ref.clone(),
            last_candidate_revision: candidate.candidate.candidate_revision,
        })
    }

    fn materialize_primary_candidate_key(
        &self,
        candidate: &ComputeCandidateFactsV2,
        current: Option<&ComputeManagementSourceV2>,
        change: &ComputeManagementChangeV2,
        desired: &mut ComputeManagementSourceV2,
        secret_mutations: &mut Vec<SecretMutationV1>,
        discovery_guards: &mut Vec<ComputePreparedDiscoveryGuardV1>,
    ) -> Result<(), ComputeManagementPlanningErrorV2> {
        match &candidate.credential_binding {
            ComputeCredentialBindingV2::NativeProtected { input_slot, .. }
                if current.is_none()
                    && !change.key_edits.iter().any(|edit| {
                        matches!(edit, ComputeKeyEditV2::Add { input_candidate } if input_candidate == &candidate.candidate)
                    }) =>
            {
                let key_id = managed_key_id(&desired.source_id, &candidate.candidate)?;
                let (credential, mutation) = self.materialize_input(
                    desired,
                    candidate,
                    &key_id,
                    0,
                    input_slot,
                    discovery_guards,
                )?;
                desired.credentials.push(credential);
                secret_mutations.push(mutation);
            }
            ComputeCredentialBindingV2::NativeSaved {
                credential_id,
                expected_generation,
            } => {
                let valid = current.is_some_and(|source| {
                    source.credentials.iter().any(|credential| {
                        credential.key_id == *credential_id
                            && credential.credential.generation() == *expected_generation
                    })
                });
                if !valid {
                    return Err(ComputeManagementPlanningErrorV2::CredentialConflict);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn apply_key_edits(
        &self,
        edits: &[ComputeKeyEditV2],
        lineage_digest: &CanonicalDigest,
        desired: &mut ComputeManagementSourceV2,
        secret_mutations: &mut Vec<SecretMutationV1>,
        discovery_guards: &mut Vec<ComputePreparedDiscoveryGuardV1>,
    ) -> Result<(), ComputeManagementPlanningErrorV2> {
        if !edits.is_empty()
            && (!desired.provenance.is_native()
                || (matches!(
                    desired.authentication,
                    GatewayAuthenticationSemanticsV1::None
                ) && desired.additional_native_endpoints.iter().all(|endpoint| {
                    endpoint.authentication == GatewayAuthenticationSemanticsV1::None
                })))
        {
            return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
        }
        let mut secret_touched = BTreeSet::new();
        let mut replaced = BTreeSet::new();
        let mut enabled_touched = BTreeSet::new();
        let removed = edits
            .iter()
            .filter_map(|edit| match edit {
                ComputeKeyEditV2::Remove { key_id, .. } => Some(key_id.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let mut order_seen = false;
        for edit in edits {
            if order_seen {
                return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
            }
            match edit {
                ComputeKeyEditV2::Add { input_candidate } => {
                    let input = self.resolve_key_input(input_candidate, lineage_digest, desired)?;
                    let key_id = managed_key_id(&desired.source_id, input_candidate)?;
                    if desired
                        .credentials
                        .iter()
                        .any(|credential| credential.key_id == key_id)
                        || !secret_touched.insert(key_id.clone())
                    {
                        return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
                    }
                    let slot = protected_slot(&input)?;
                    let (credential, mutation) = self.materialize_input(
                        desired,
                        &input,
                        &key_id,
                        0,
                        slot,
                        discovery_guards,
                    )?;
                    desired.credentials.push(credential);
                    secret_mutations.push(mutation);
                }
                ComputeKeyEditV2::Replace {
                    key_id,
                    expected_generation,
                    input_candidate,
                } => {
                    if !secret_touched.insert(key_id.clone()) {
                        return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
                    }
                    let index = key_index(&desired.credentials, key_id, *expected_generation)?;
                    let input = self.resolve_key_input(input_candidate, lineage_digest, desired)?;
                    let slot = protected_slot(&input)?;
                    let enabled = desired.credentials[index].enabled;
                    let ordinal = desired.credentials[index].ordinal;
                    let (mut credential, mutation) = self.materialize_input(
                        desired,
                        &input,
                        key_id,
                        *expected_generation,
                        slot,
                        discovery_guards,
                    )?;
                    credential.enabled = enabled;
                    credential.ordinal = ordinal;
                    desired.credentials[index] = credential;
                    secret_mutations.push(mutation);
                    replaced.insert(key_id.clone());
                }
                ComputeKeyEditV2::Remove {
                    key_id,
                    expected_generation,
                } => {
                    if !secret_touched.insert(key_id.clone()) {
                        return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
                    }
                    let index = key_index(&desired.credentials, key_id, *expected_generation)?;
                    let removed = desired.credentials.remove(index);
                    let observed = self.secrets.generation(&removed.credential)?;
                    if observed != *expected_generation {
                        return Err(ComputeManagementPlanningErrorV2::CredentialConflict);
                    }
                    secret_mutations.push(
                        SecretMutationV1::delete(removed.credential, *expected_generation)
                            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?,
                    );
                }
                ComputeKeyEditV2::SetEnabled {
                    key_id,
                    expected_generation,
                    enabled,
                } => {
                    if removed.contains(key_id.as_str()) || !enabled_touched.insert(key_id.clone())
                    {
                        return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
                    }
                    let index = key_index_after_replacement(
                        &desired.credentials,
                        key_id,
                        *expected_generation,
                        replaced.contains(key_id),
                    )?;
                    desired.credentials[index].enabled = *enabled;
                }
                ComputeKeyEditV2::SetOrder { key_ids } => {
                    if order_seen {
                        return Err(ComputeManagementPlanningErrorV2::InvalidKeyEdit);
                    }
                    order_seen = true;
                    reorder_keys(&mut desired.credentials, key_ids)?;
                }
            }
        }
        Ok(())
    }

    fn resolve_key_input(
        &self,
        candidate: &ComputeCandidateRefV2,
        lineage_digest: &CanonicalDigest,
        desired: &ComputeManagementSourceV2,
    ) -> Result<ComputeCandidateFactsV2, ComputeManagementPlanningErrorV2> {
        let facts = self.candidates.resolve_compute_candidate(candidate)?;
        facts.validate_shape()?;
        if facts.producer != ComputeCandidateProducerV2::Native
            || candidate_lineage_digest(&facts)? != *lineage_digest
            || facts.authentication.as_ref() != Some(&desired.authentication)
            || facts.additional_native_endpoints != desired.additional_native_endpoints
            || facts.target.as_ref().is_none_or(|target| {
                target.scheme != desired.target.scheme
                    || target.authority != desired.target.authority
                    || target.port != desired.target.port
                    || target.request_path != desired.target.request_path
                    || target.upstream_protocol != desired.target.upstream_protocol
                    || target.protocol_profile_id != desired.target.protocol_profile_id
                    || target.protocol_profile_revision != desired.target.protocol_profile_revision
            })
            || !matches!(
                facts.credential_binding,
                ComputeCredentialBindingV2::NativeProtected { .. }
            )
        {
            return Err(ComputeManagementPlanningErrorV2::CredentialConflict);
        }
        Ok(facts)
    }

    fn materialize_input(
        &self,
        desired: &ComputeManagementSourceV2,
        input: &ComputeCandidateFactsV2,
        key_id: &str,
        expected_generation: u64,
        input_slot: &str,
        discovery_guards: &mut Vec<ComputePreparedDiscoveryGuardV1>,
    ) -> Result<(ComputeManagedCredentialV2, SecretMutationV1), ComputeManagementPlanningErrorV2>
    {
        if let Some(guard) = &input.discovery_guard {
            self.protected_inputs
                .validate_discovery_evidence(input_slot, &guard.evidence_digest)
                .map_err(|_| ComputeManagementPlanningErrorV2::PreviewStale)?;
            discovery_guards.push(ComputePreparedDiscoveryGuardV1 {
                input_slot: input_slot.to_owned(),
                evidence_digest: guard.evidence_digest.clone(),
            });
        }
        let destinations = desired
            .native_destinations()
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidCandidate)?;
        let reference = CredentialRefV1::new(
            key_id,
            format!("source/{}", desired.source_id),
            "hirouted",
            "provider-auth",
            destinations.iter().cloned(),
            expected_generation,
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?;
        if self.secrets.generation(&reference)? != expected_generation {
            return Err(ComputeManagementPlanningErrorV2::CredentialConflict);
        }
        let secret = self.protected_inputs.read_secret(input_slot)?;
        let fingerprint = self.secrets.fingerprint(&secret)?;
        let mutation = SecretMutationV1::upsert(
            reference,
            expected_generation,
            input_slot,
            Some(fingerprint.clone()),
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?;
        let desired_reference = CredentialRefV1::new(
            key_id,
            format!("source/{}", desired.source_id),
            "hirouted",
            "provider-auth",
            destinations,
            expected_generation
                .checked_add(1)
                .ok_or(ComputeManagementPlanningErrorV2::InvalidKeyEdit)?,
        )
        .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?;
        debug_assert_eq!(
            input.candidate.candidate_ref,
            input.correlation.candidate_ref
        );
        Ok((
            ComputeManagedCredentialV2 {
                key_id: key_id.into(),
                credential: desired_reference,
                fingerprint,
                ordinal: desired.credentials.len() as u32,
                enabled: true,
            },
            mutation,
        ))
    }

    fn rebind_unchanged_keys(
        &self,
        current: Option<&ComputeManagementSourceV2>,
        desired: &mut ComputeManagementSourceV2,
        secret_mutations: &mut Vec<SecretMutationV1>,
    ) -> Result<(), ComputeManagementPlanningErrorV2> {
        let Some(current) = current else {
            return Ok(());
        };
        let destinations = desired
            .native_destinations()
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidCandidate)?;
        if current
            .native_destinations()
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidCandidate)?
            == destinations
        {
            return Ok(());
        }
        let touched = secret_mutations
            .iter()
            .map(|mutation| mutation.credential().credential_id().to_owned())
            .collect::<BTreeSet<_>>();
        for credential in &mut desired.credentials {
            if touched.contains(credential.key_id.as_str()) {
                continue;
            }
            let Some(before) = current
                .credentials
                .iter()
                .find(|entry| entry.key_id == credential.key_id)
            else {
                return Err(ComputeManagementPlanningErrorV2::CredentialConflict);
            };
            if self.secrets.generation(&before.credential)? != before.credential.generation() {
                return Err(ComputeManagementPlanningErrorV2::CredentialConflict);
            }
            let next_generation = before
                .credential
                .generation()
                .checked_add(1)
                .ok_or(ComputeManagementPlanningErrorV2::InvalidKeyEdit)?;
            let mutation = SecretMutationV1::rebind(
                before.credential.clone(),
                destinations.clone(),
                before.fingerprint.clone(),
            )
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?;
            credential.credential = CredentialRefV1::new(
                &credential.key_id,
                format!("source/{}", desired.source_id),
                "hirouted",
                "provider-auth",
                destinations.iter().cloned(),
                next_generation,
            )
            .map_err(|_| ComputeManagementPlanningErrorV2::InvalidKeyEdit)?;
            secret_mutations.push(mutation);
        }
        Ok(())
    }
}

struct ResolvedSubject {
    candidate: Option<ComputeCandidateFactsV2>,
    current: Option<ComputeManagementSourceV2>,
    source_id: String,
    lineage_digest: CanonicalDigest,
}

impl From<PortError> for ComputeManagementPlanningErrorV2 {
    fn from(value: PortError) -> Self {
        match value.code {
            PortErrorCode::NotFound => Self::SourceNotFound,
            PortErrorCode::Conflict => Self::RevisionConflict,
            _ => Self::Port(value),
        }
    }
}

#[derive(Debug, Error)]
pub enum ComputeManagementPlanningErrorV2 {
    #[error("compute management change is invalid")]
    InvalidChange,
    #[error("compute candidate is invalid")]
    InvalidCandidate,
    #[error("compute source was not found")]
    SourceNotFound,
    #[error("candidate lineage conflicts with the saved source")]
    LineageConflict,
    #[error("candidate approval must complete before save")]
    ApprovalRequired,
    #[error("subscription validation does not match")]
    ValidationConflict,
    #[error("selected model is not available")]
    ModelNotSelectable,
    #[error("a credential is required for ready save")]
    CredentialRequired,
    #[error("credential generation or identity changed")]
    CredentialConflict,
    #[error("key edit is invalid")]
    InvalidKeyEdit,
    #[error("control revision changed")]
    RevisionConflict,
    #[error("saved preview is stale")]
    PreviewStale,
    #[error("compute management port failed: {0}")]
    Port(PortError),
}
