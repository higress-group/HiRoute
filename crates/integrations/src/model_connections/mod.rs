//! Native API candidate normalization and bounded model-directory checks.
//!
//! The adapter receives protected credential bytes only through its trusted Rust call. Public
//! results use the C0 safe candidate view and never expose those bytes or their source locator.

mod inference;
mod probe;
mod target;
#[cfg(test)]
mod tests;
mod validation;

pub use probe::*;
pub use target::*;

use validation::validate_draft;

use std::{
    collections::BTreeMap,
    fmt::{self, Write as _},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use hiroute_application::compute_management::{
    ComputeCandidateCapabilityFactsV2, ComputeCandidateFactBasisV2, ComputeCandidateFactValueV2,
    ComputeCandidateFactsV2, ComputeCandidateModelFactsV2, ComputeCandidatePort,
    ComputeCandidateProvenanceV2, ComputeCredentialBindingV2, ComputeDiscoveryEvidenceGuardV1,
    ProtectedInputSourceDescriptorV1,
};
use hiroute_application_api::{
    ComputeCandidateProducerV2, ComputeCandidateRefV2, ComputeCandidateViewV2,
    ComputeCheckCorrelationV2, ComputeModelMembershipV2, ModelConnectionCheckViewV1,
};
use hiroute_domain::{
    CanonicalDigest, FreeAccess, GatewayAuthenticationSemanticsV1, GatewayHeaderSemanticsV1,
    NativeReasoningCapabilityV1, PortError, ProtectedSecret, UpstreamProtocol,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeCandidateFactBasisV1 {
    RegisteredCatalog,
    RuntimeFallback,
    Observed,
    UserDeclared,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCandidateFactValueV1<T> {
    pub value: Option<T>,
    pub basis: NativeCandidateFactBasisV1,
}

impl<T> NativeCandidateFactValueV1<T> {
    pub const fn unknown() -> Self {
        Self {
            value: None,
            basis: NativeCandidateFactBasisV1::Unknown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeModelCapabilityDeclarationV1 {
    pub tool: NativeCandidateFactValueV1<bool>,
    pub vision: NativeCandidateFactValueV1<bool>,
    pub streaming: NativeCandidateFactValueV1<bool>,
    pub context_tokens: NativeCandidateFactValueV1<u64>,
    pub max_output_tokens: NativeCandidateFactValueV1<u64>,
    pub native_reasoning: NativeCandidateFactValueV1<NativeReasoningCapabilityV1>,
}

impl Default for NativeModelCapabilityDeclarationV1 {
    fn default() -> Self {
        Self {
            tool: NativeCandidateFactValueV1::unknown(),
            vision: NativeCandidateFactValueV1::unknown(),
            streaming: NativeCandidateFactValueV1::unknown(),
            context_tokens: NativeCandidateFactValueV1::unknown(),
            max_output_tokens: NativeCandidateFactValueV1::unknown(),
            native_reasoning: NativeCandidateFactValueV1::unknown(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeModelDeclarationV1 {
    pub upstream_model_id: String,
    pub display_name: String,
    pub catalog_configuration_id: Option<String>,
    pub membership: ComputeModelMembershipV2,
    pub capabilities: NativeModelCapabilityDeclarationV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeConnectionProvenanceInputV1 {
    Registered {
        connection_option_id: String,
        registry_version: String,
        catalog_digest: CanonicalDigest,
    },
    UserConfigured {
        configuration_revision: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeConnectionQualificationV1 {
    pub free_access: Option<FreeAccess>,
    pub evidence_ref: Option<String>,
}

/// Trusted, server-resolved input. Ordinary client payloads must not deserialize directly into
/// this type because registered provenance and fact bases require their owning authorization.
#[derive(Clone, Eq, PartialEq)]
pub struct NativeModelConnectionDraftV1 {
    pub display_template_id: Option<String>,
    pub inference_model_id: Option<String>,
    /// Omitted for the first check; the trusted service issues a random opaque reference.
    pub candidate_ref: Option<String>,
    pub lineage_ref: String,
    /// Non-wire exact identity of an existing saved source. Ordinary native and registered
    /// request DTOs cannot populate this field.
    pub trusted_lineage_digest: Option<CanonicalDigest>,
    pub display_name: String,
    pub existing_source_id: Option<String>,
    pub edit_revision: u64,
    pub check_id: String,
    pub base_url: String,
    pub base_kind: ModelConnectionBaseKindV1,
    pub request_path_override: Option<String>,
    pub inventory_path_override: Option<String>,
    pub protocol: UpstreamProtocol,
    pub protocol_profile_id: String,
    pub protocol_profile_revision: u64,
    /// Exact non-secret header semantics resolved from the trusted protocol profile.
    pub protocol_header_semantics: GatewayHeaderSemanticsV1,
    pub authentication: GatewayAuthenticationSemanticsV1,
    /// Additional exact endpoints from the trusted draft; only the first endpoint is probed.
    pub additional_native_endpoints: Vec<hiroute_domain::ComputeNativeEndpointV3>,
    pub provenance: NativeConnectionProvenanceInputV1,
    pub qualification: NativeConnectionQualificationV1,
    /// Provider-returned IDs that the bundled metadata identifies as non-text or internal-only.
    /// All other previously unknown IDs may use the explicitly marked text fallback only after
    /// this exact connection has returned them from its authenticated inventory.
    pub runtime_fallback_denied_model_ids: std::collections::BTreeSet<String>,
    pub models: Vec<NativeModelDeclarationV1>,
}

/// Credential bytes are borrowed for one explicit check and have no serializable representation.
pub enum NativeModelConnectionCredentialV1<'a> {
    NotRequired,
    PendingInput,
    Protected {
        descriptor: ProtectedInputSourceDescriptorV1,
        input_slot: String,
        secret: &'a ProtectedSecret,
    },
    Saved {
        credential_id: String,
        expected_generation: u64,
        secret: &'a ProtectedSecret,
    },
}

impl fmt::Debug for NativeModelConnectionCredentialV1<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRequired => formatter.write_str("NotRequired"),
            Self::PendingInput => formatter.write_str("PendingInput"),
            Self::Protected { .. } => formatter.write_str("Protected(<redacted>)"),
            Self::Saved {
                expected_generation,
                ..
            } => formatter
                .debug_struct("Saved")
                .field("credential_id", &"<redacted>")
                .field("expected_generation", expected_generation)
                .finish(),
        }
    }
}

#[derive(Debug, Error)]
pub enum NativeModelConnectionErrorV1 {
    #[error("native model connection draft is invalid")]
    InvalidDraft,
    #[error("native model connection target is invalid: {0}")]
    Target(#[from] ModelConnectionTargetError),
    #[error("candidate directory rejected the trusted facts: {0}")]
    Candidate(#[from] PortError),
    #[error("candidate reference could not be generated")]
    RandomUnavailable,
}

pub struct NativeModelConnectionServiceV1<P, T> {
    candidates: P,
    transport: T,
    revisions: Mutex<BTreeMap<String, u64>>,
    limits: ModelConnectionProbeLimitsV1,
}

impl<P, T> NativeModelConnectionServiceV1<P, T>
where
    P: ComputeCandidatePort,
    T: ModelDirectoryTransportV1,
{
    pub fn new(candidates: P, transport: T) -> Self {
        Self {
            candidates,
            transport,
            revisions: Mutex::new(BTreeMap::new()),
            limits: ModelConnectionProbeLimitsV1::default(),
        }
    }

    /// Allows tests to choose stricter bounds; values wider than the product defaults fail closed.
    pub fn with_limits(mut self, limits: ModelConnectionProbeLimitsV1) -> Self {
        self.limits = limits;
        self
    }

    pub fn candidate_port(&self) -> &P {
        &self.candidates
    }

    /// Binds a native-host protected input to the first revision that a subsequent check will
    /// issue. The opaque reference is generated by the native host and reaches this method only
    /// over the inherited protected channel; ordinary Local Control clients cannot reserve it.
    pub fn reserve_candidate_ref(
        &self,
        candidate: &ComputeCandidateRefV2,
    ) -> Result<(), NativeModelConnectionErrorV1> {
        if candidate.candidate_ref.trim().is_empty() || candidate.candidate_revision != 1 {
            return Err(NativeModelConnectionErrorV1::InvalidDraft);
        }
        let mut revisions = self
            .revisions
            .lock()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        match revisions.get(&candidate.candidate_ref) {
            Some(0) => Ok(()),
            Some(_) => Err(NativeModelConnectionErrorV1::InvalidDraft),
            None if revisions.len() < 256 => {
                revisions.insert(candidate.candidate_ref.clone(), 0);
                Ok(())
            }
            None => Err(NativeModelConnectionErrorV1::InvalidDraft),
        }
    }

    /// Restores the durable revision floor for a server-owned saved-source candidate. This is
    /// never called from an ordinary wire payload; the daemon first matches the reference and
    /// revision against the exact persisted source.
    pub fn restore_saved_candidate_ref(
        &self,
        candidate: &ComputeCandidateRefV2,
    ) -> Result<(), NativeModelConnectionErrorV1> {
        candidate
            .validate_shape()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        let mut revisions = self
            .revisions
            .lock()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        let can_insert = revisions.len() < 256;
        match revisions.get_mut(&candidate.candidate_ref) {
            Some(current) if *current < candidate.candidate_revision => {
                *current = candidate.candidate_revision;
            }
            Some(_) => {}
            None if can_insert => {
                revisions.insert(
                    candidate.candidate_ref.clone(),
                    candidate.candidate_revision,
                );
            }
            None => return Err(NativeModelConnectionErrorV1::InvalidDraft),
        }
        Ok(())
    }

    /// Registers a catalog-complete candidate from one exact protected local discovery without
    /// performing any network request. The caller must have rescanned and authorized the draft
    /// against its current client-bundled catalog immediately before this call.
    pub fn prepare_registered_discovery(
        &self,
        draft: NativeModelConnectionDraftV1,
        credential: NativeModelConnectionCredentialV1<'_>,
        discovery_evidence: CanonicalDigest,
    ) -> Result<ComputeCandidateViewV2, NativeModelConnectionErrorV1> {
        if discovery_evidence == CanonicalDigest::of_bytes(&[])
            || draft.candidate_ref.is_none()
            || draft.existing_source_id.is_some()
            || draft.trusted_lineage_digest.is_some()
            || !matches!(
                &draft.provenance,
                NativeConnectionProvenanceInputV1::Registered { .. }
            )
            || !matches!(
                &credential,
                NativeModelConnectionCredentialV1::Protected {
                    descriptor: ProtectedInputSourceDescriptorV1::DiscoveredConfig { .. },
                    ..
                }
            )
            || !draft.models.iter().all(registered_catalog_model)
        {
            return Err(NativeModelConnectionErrorV1::InvalidDraft);
        }
        validate_draft(&draft, &credential)?;
        let (candidate_ref, revision) = self.issue_candidate(draft.candidate_ref.as_deref())?;
        let target = normalize_model_connection_target(ModelConnectionTargetInputV1 {
            base_url: &draft.base_url,
            base_kind: draft.base_kind,
            protocol: draft.protocol,
            request_path_override: draft.request_path_override.as_deref(),
            inventory_path_override: draft.inventory_path_override.as_deref(),
            protocol_profile_id: &draft.protocol_profile_id,
            protocol_profile_revision: draft.protocol_profile_revision,
            protocol_header_semantics: &draft.protocol_header_semantics,
            authentication: &draft.authentication,
        })?;
        let base_input_digest = input_digest(&draft, &credential)?;
        let input_digest = CanonicalDigest::of(&(
            "hiroute.prepared-discovery-input/v1",
            &base_input_digest,
            &discovery_evidence,
        ))
        .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        let observed = draft
            .models
            .iter()
            .map(|model| model.upstream_model_id.clone())
            .collect::<Vec<_>>();
        let models = candidate_models(
            &candidate_ref,
            &draft.models,
            &observed,
            true,
            true,
            &draft.runtime_fallback_denied_model_ids,
        )?;
        let NativeConnectionProvenanceInputV1::Registered {
            connection_option_id,
            registry_version,
            catalog_digest,
        } = draft.provenance
        else {
            return Err(NativeModelConnectionErrorV1::InvalidDraft);
        };
        let facts = ComputeCandidateFactsV2 {
            candidate: ComputeCandidateRefV2 {
                candidate_ref: candidate_ref.clone(),
                candidate_revision: revision,
            },
            correlation: ComputeCheckCorrelationV2 {
                candidate_ref,
                edit_revision: draft.edit_revision,
                check_id: draft.check_id,
                input_digest,
            },
            producer: ComputeCandidateProducerV2::Native,
            lineage_ref: draft.lineage_ref,
            trusted_lineage_digest: None,
            display_name: draft.display_name,
            existing_source_id: None,
            evidence_digest: discovery_evidence.clone(),
            provenance: ComputeCandidateProvenanceV2::Registered {
                connection_option_id,
                registry_version,
                catalog_digest,
            },
            target: Some(target.candidate_target),
            authentication: Some(draft.authentication),
            models,
            native_recheck: Some(hiroute_domain::ComputeNativeRecheckDescriptorV2 {
                display_template_id: draft.display_template_id.clone(),
                inventory_path: target.inventory_path,
                protocol_header_semantics: draft.protocol_header_semantics,
            }),
            additional_native_endpoints: draft.additional_native_endpoints,
            discovery_guard: Some(ComputeDiscoveryEvidenceGuardV1 {
                evidence_digest: discovery_evidence,
            }),
            credential_binding: credential_binding(&credential),
            validation: None,
        };
        self.candidates
            .register_compute_candidate(facts)
            .map_err(Into::into)
    }

    pub fn check(
        &self,
        draft: NativeModelConnectionDraftV1,
        credential: NativeModelConnectionCredentialV1<'_>,
        cancellation: &ModelConnectionProbeCancellationV1,
    ) -> Result<ModelConnectionCheckViewV1, NativeModelConnectionErrorV1> {
        if !self.limits.valid() {
            return Err(NativeModelConnectionErrorV1::InvalidDraft);
        }
        let check_started = Instant::now();
        validate_draft(&draft, &credential)?;
        // Allocate before DNS or request I/O. If an older request completes after a newer one for
        // the same candidate lineage, the candidate directory rejects its lower backend revision.
        let (candidate_ref, revision) = self.issue_candidate(draft.candidate_ref.as_deref())?;
        let target = normalize_model_connection_target_with_timeout(
            ModelConnectionTargetInputV1 {
                base_url: &draft.base_url,
                base_kind: draft.base_kind,
                protocol: draft.protocol,
                request_path_override: draft.request_path_override.as_deref(),
                inventory_path_override: draft.inventory_path_override.as_deref(),
                protocol_profile_id: &draft.protocol_profile_id,
                protocol_profile_revision: draft.protocol_profile_revision,
                protocol_header_semantics: &draft.protocol_header_semantics,
                authentication: &draft.authentication,
            },
            self.limits.total_timeout.min(Duration::from_secs(2)),
        )?;
        let input_digest = input_digest(&draft, &credential)?;
        let remaining = self
            .limits
            .total_timeout
            .saturating_sub(check_started.elapsed());
        let probe_limits = ModelConnectionProbeLimitsV1 {
            total_timeout: remaining,
            ..self.limits
        };
        let mut observation = match &credential {
            NativeModelConnectionCredentialV1::PendingInput => {
                ModelDirectoryProbeObservationV1::missing_credential()
            }
            _ if remaining.is_zero() => ModelDirectoryProbeObservationV1::timed_out(),
            _ if draft.inference_model_id.is_some() => {
                let mut observation = ModelDirectoryProbeObservationV1::missing_credential();
                observation.issues.clear();
                observation
            }
            NativeModelConnectionCredentialV1::NotRequired => {
                probe_model_directory(&self.transport, &target, None, cancellation, probe_limits)
            }
            NativeModelConnectionCredentialV1::Protected { secret, .. }
            | NativeModelConnectionCredentialV1::Saved { secret, .. } => probe_model_directory(
                &self.transport,
                &target,
                Some(ModelConnectionProbeCredentialV1 {
                    authentication: &draft.authentication,
                    secret,
                }),
                cancellation,
                probe_limits,
            ),
        };
        if let Some(model) = draft.inference_model_id.as_deref()
            && !matches!(credential, NativeModelConnectionCredentialV1::PendingInput)
            && !cancellation.is_cancelled()
            && !remaining.is_zero()
        {
            let probe_credential = match &credential {
                NativeModelConnectionCredentialV1::Protected { secret, .. }
                | NativeModelConnectionCredentialV1::Saved { secret, .. } => {
                    Some(ModelConnectionProbeCredentialV1 {
                        authentication: &draft.authentication,
                        secret,
                    })
                }
                _ => None,
            };
            observation = inference::observe(
                &self.transport,
                &target,
                model,
                probe_credential,
                cancellation,
                probe_limits,
            );
        }
        let evidence_digest = CanonicalDigest::of(&(
            "native-model-connection-evidence/v1",
            &input_digest,
            &observation,
        ))
        .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        if draft.inference_model_id.is_some() && remaining.is_zero() {
            observation.inference =
                hiroute_application_api::ModelConnectionInferenceStatusV1::Failed;
        }
        let registered = matches!(
            &draft.provenance,
            NativeConnectionProvenanceInputV1::Registered { .. }
        );
        let connection_allows_ready =
            !matches!(&credential, NativeModelConnectionCredentialV1::PendingInput)
                && !cancellation.is_cancelled()
                && observation.authentication
                    != hiroute_application_api::ModelConnectionAuthenticationStatusV1::Rejected
                && observation.inference
                    != hiroute_application_api::ModelConnectionInferenceStatusV1::Failed;
        let models = candidate_models(
            &candidate_ref,
            &draft.models,
            &observation.model_ids,
            connection_allows_ready,
            registered,
            &draft.runtime_fallback_denied_model_ids,
        )?;
        if connection_allows_ready
            && registered
            && models
                .iter()
                .any(|model| model.reason.as_deref() == Some("compute.registered_model_unmatched"))
        {
            observation
                .issues
                .push(hiroute_application_api::ModelConnectionCheckIssueV1 {
                    code: "registered_model_unmatched".into(),
                    message_key: "compute.registered_model_unmatched".into(),
                    retryable: false,
                });
        }
        let provenance = match &draft.provenance {
            NativeConnectionProvenanceInputV1::Registered {
                connection_option_id,
                registry_version,
                catalog_digest,
            } => ComputeCandidateProvenanceV2::Registered {
                connection_option_id: connection_option_id.clone(),
                registry_version: registry_version.clone(),
                catalog_digest: catalog_digest.clone(),
            },
            NativeConnectionProvenanceInputV1::UserConfigured {
                configuration_revision,
            } => ComputeCandidateProvenanceV2::UserConfigured {
                configuration_revision: *configuration_revision,
                evidence_digest: input_digest.clone(),
            },
        };
        let native_recheck = Some(hiroute_domain::ComputeNativeRecheckDescriptorV2 {
            display_template_id: draft.display_template_id.clone(),
            inventory_path: target.inventory_path.clone(),
            protocol_header_semantics: draft.protocol_header_semantics.clone(),
        });
        let binding = credential_binding(&credential);
        let facts = ComputeCandidateFactsV2 {
            candidate: ComputeCandidateRefV2 {
                candidate_ref: candidate_ref.clone(),
                candidate_revision: revision,
            },
            correlation: ComputeCheckCorrelationV2 {
                candidate_ref,
                edit_revision: draft.edit_revision,
                check_id: draft.check_id,
                input_digest: input_digest.clone(),
            },
            producer: ComputeCandidateProducerV2::Native,
            lineage_ref: draft.lineage_ref,
            trusted_lineage_digest: draft.trusted_lineage_digest,
            display_name: draft.display_name,
            existing_source_id: draft.existing_source_id,
            evidence_digest,
            provenance,
            target: Some(target.candidate_target.clone()),
            authentication: Some(draft.authentication),
            models,
            native_recheck,
            additional_native_endpoints: draft.additional_native_endpoints.clone(),
            discovery_guard: None,
            credential_binding: binding,
            validation: None,
        };
        let candidate = self.candidates.register_compute_candidate(facts)?;
        Ok(ModelConnectionCheckViewV1 {
            inference_model_id: draft.inference_model_id.clone(),
            candidate,
            target: target.candidate_target,
            inventory_path: target.inventory_path,
            reachability: observation.reachability,
            authentication: observation.authentication,
            directory: observation.directory,
            protocol: observation.protocol,
            inference: observation.inference,
            checked_model_count: observation.model_ids.len() as u32,
            invalid_model_count: observation.invalid_model_count,
            pages_read: observation.pages_read,
            checked_at_unix_ms: checked_at_unix_ms()?,
            input_digest,
            issues: observation.issues,
        })
    }

    fn issue_candidate(
        &self,
        requested: Option<&str>,
    ) -> Result<(String, u64), NativeModelConnectionErrorV1> {
        let mut revisions = self
            .revisions
            .lock()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        if let Some(candidate_ref) = requested {
            if candidate_ref.trim().is_empty() {
                return Err(NativeModelConnectionErrorV1::InvalidDraft);
            }
            let revision = revisions
                .get_mut(candidate_ref)
                .ok_or(NativeModelConnectionErrorV1::InvalidDraft)?;
            *revision = revision
                .checked_add(1)
                .ok_or(NativeModelConnectionErrorV1::InvalidDraft)?;
            return Ok((candidate_ref.to_owned(), *revision));
        }
        loop {
            let candidate_ref = random_candidate_ref()?;
            if !revisions.contains_key(&candidate_ref) {
                revisions.insert(candidate_ref.clone(), 1);
                return Ok((candidate_ref, 1));
            }
        }
    }
}

fn checked_at_unix_ms() -> Result<u64, NativeModelConnectionErrorV1> {
    let value = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?
        .as_millis();
    u64::try_from(value).map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)
}

#[derive(Serialize)]
struct InputDigest<'a> {
    display_template_id: &'a Option<String>,
    inference_model_id: &'a Option<String>,
    lineage_ref: &'a str,
    trusted_lineage_digest: &'a Option<CanonicalDigest>,
    display_name: &'a str,
    existing_source_id: &'a Option<String>,
    base_url: &'a str,
    base_kind: ModelConnectionBaseKindV1,
    request_path_override: &'a Option<String>,
    inventory_path_override: &'a Option<String>,
    protocol: UpstreamProtocol,
    protocol_profile_id: &'a str,
    protocol_profile_revision: u64,
    protocol_header_semantics: &'a GatewayHeaderSemanticsV1,
    authentication: &'a GatewayAuthenticationSemanticsV1,
    additional_native_endpoints: &'a [hiroute_domain::ComputeNativeEndpointV3],
    provenance: &'a NativeConnectionProvenanceInputV1,
    qualification: &'a NativeConnectionQualificationV1,
    models: &'a [NativeModelDeclarationV1],
    credential_state: &'static str,
}

fn input_digest(
    draft: &NativeModelConnectionDraftV1,
    credential: &NativeModelConnectionCredentialV1<'_>,
) -> Result<CanonicalDigest, NativeModelConnectionErrorV1> {
    let credential_state = match credential {
        NativeModelConnectionCredentialV1::NotRequired => "not_required",
        NativeModelConnectionCredentialV1::PendingInput => "missing",
        NativeModelConnectionCredentialV1::Protected { .. } => "provided",
        NativeModelConnectionCredentialV1::Saved { .. } => "saved",
    };
    CanonicalDigest::of(&InputDigest {
        display_template_id: &draft.display_template_id,
        inference_model_id: &draft.inference_model_id,
        lineage_ref: &draft.lineage_ref,
        trusted_lineage_digest: &draft.trusted_lineage_digest,
        display_name: &draft.display_name,
        existing_source_id: &draft.existing_source_id,
        base_url: &draft.base_url,
        base_kind: draft.base_kind,
        request_path_override: &draft.request_path_override,
        inventory_path_override: &draft.inventory_path_override,
        protocol: draft.protocol,
        protocol_profile_id: &draft.protocol_profile_id,
        protocol_profile_revision: draft.protocol_profile_revision,
        protocol_header_semantics: &draft.protocol_header_semantics,
        authentication: &draft.authentication,
        additional_native_endpoints: &draft.additional_native_endpoints,
        provenance: &draft.provenance,
        qualification: &draft.qualification,
        models: &draft.models,
        credential_state,
    })
    .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)
}

fn credential_binding(
    credential: &NativeModelConnectionCredentialV1<'_>,
) -> ComputeCredentialBindingV2 {
    match credential {
        NativeModelConnectionCredentialV1::NotRequired => ComputeCredentialBindingV2::None,
        NativeModelConnectionCredentialV1::PendingInput => {
            ComputeCredentialBindingV2::NativePendingInput
        }
        NativeModelConnectionCredentialV1::Protected {
            descriptor,
            input_slot,
            ..
        } => ComputeCredentialBindingV2::NativeProtected {
            descriptor: descriptor.clone(),
            input_slot: input_slot.clone(),
        },
        NativeModelConnectionCredentialV1::Saved {
            credential_id,
            expected_generation,
            ..
        } => ComputeCredentialBindingV2::NativeSaved {
            credential_id: credential_id.clone(),
            expected_generation: *expected_generation,
        },
    }
}

fn candidate_models(
    candidate_ref: &str,
    declarations: &[NativeModelDeclarationV1],
    observed: &[String],
    connection_allows_ready: bool,
    registered: bool,
    runtime_fallback_denied_model_ids: &std::collections::BTreeSet<String>,
) -> Result<Vec<ComputeCandidateModelFactsV2>, NativeModelConnectionErrorV1> {
    let observed = observed
        .iter()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let mut models = Vec::new();
    for declaration in declarations {
        let capabilities = &declaration.capabilities;
        let capability_complete = model_is_selectable(capabilities);
        let selectable = capability_complete
            && connection_allows_ready
            && !runtime_fallback_denied_model_ids.contains(declaration.upstream_model_id.as_str());
        models.push(ComputeCandidateModelFactsV2 {
            model_ref: model_ref(candidate_ref, &declaration.upstream_model_id)?,
            upstream_model_id: declaration.upstream_model_id.clone(),
            display_name: declaration.display_name.clone(),
            catalog_configuration_id: declaration.catalog_configuration_id.clone(),
            membership: declaration.membership,
            capabilities: convert_capabilities(capabilities),
            capability_evidence_digest: CanonicalDigest::of(&(
                "declared-model-capabilities/v1",
                &declaration.upstream_model_id,
                capabilities,
            ))
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?,
            selectable,
            reason: (!selectable).then(|| {
                if runtime_fallback_denied_model_ids
                    .contains(declaration.upstream_model_id.as_str())
                {
                    "model_connections.runtime_fallback_ineligible".into()
                } else if capability_complete {
                    "model_connections.connection_check_required".into()
                } else {
                    "model_connections.capability_required".into()
                }
            }),
        });
    }
    let declared = declarations
        .iter()
        .map(|model| model.upstream_model_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for upstream_model_id in observed.difference(&declared) {
        let fallback_allowed = connection_allows_ready
            && !runtime_fallback_denied_model_ids.contains(*upstream_model_id);
        let capabilities = runtime_fallback_capabilities();
        models.push(ComputeCandidateModelFactsV2 {
            model_ref: model_ref(candidate_ref, upstream_model_id)?,
            upstream_model_id: (*upstream_model_id).to_owned(),
            display_name: (*upstream_model_id).to_owned(),
            catalog_configuration_id: None,
            membership: ComputeModelMembershipV2::Observed,
            capabilities: if fallback_allowed {
                convert_capabilities(&capabilities)
            } else {
                convert_capabilities(&NativeModelCapabilityDeclarationV1::default())
            },
            capability_evidence_digest: CanonicalDigest::of(&(
                if fallback_allowed {
                    "runtime-fallback/unclassified-text-model/v1"
                } else {
                    "observed-model-with-unknown-capabilities/v1"
                },
                upstream_model_id,
                fallback_allowed.then_some(&capabilities),
            ))
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?,
            selectable: fallback_allowed,
            reason: (!fallback_allowed).then(|| {
                if runtime_fallback_denied_model_ids.contains(*upstream_model_id) {
                    "model_connections.runtime_fallback_ineligible".into()
                } else if registered {
                    "compute.registered_model_unmatched".into()
                } else {
                    "model_connections.connection_check_required".into()
                }
            }),
        });
    }
    models.sort_by(|left, right| left.upstream_model_id.cmp(&right.upstream_model_id));
    Ok(models)
}

fn runtime_fallback_capabilities() -> NativeModelCapabilityDeclarationV1 {
    // A directory entry proves only the ID. Unknown facts remain unknown in storage.
    NativeModelCapabilityDeclarationV1::default()
}

fn model_is_selectable(capabilities: &NativeModelCapabilityDeclarationV1) -> bool {
    capabilities.context_tokens.value != Some(0)
        && capabilities.max_output_tokens.value != Some(0)
        && !matches!((capabilities.context_tokens.value, capabilities.max_output_tokens.value),
            (Some(context), Some(output)) if output > context)
        && capabilities
            .native_reasoning
            .value
            .as_ref()
            .is_none_or(|value| value.validate().is_ok())
}

fn registered_catalog_model(model: &NativeModelDeclarationV1) -> bool {
    model.catalog_configuration_id.is_some()
        && model.membership == ComputeModelMembershipV2::Catalog
        && [
            model.capabilities.tool.basis,
            model.capabilities.vision.basis,
            model.capabilities.streaming.basis,
            model.capabilities.context_tokens.basis,
            model.capabilities.max_output_tokens.basis,
            model.capabilities.native_reasoning.basis,
        ]
        .into_iter()
        .all(|basis| basis == NativeCandidateFactBasisV1::RegisteredCatalog)
}

fn convert_capabilities(
    capabilities: &NativeModelCapabilityDeclarationV1,
) -> ComputeCandidateCapabilityFactsV2 {
    ComputeCandidateCapabilityFactsV2 {
        tool: convert_fact(&capabilities.tool),
        vision: convert_fact(&capabilities.vision),
        streaming: convert_fact(&capabilities.streaming),
        context_tokens: convert_fact(&capabilities.context_tokens),
        max_output_tokens: convert_fact(&capabilities.max_output_tokens),
        native_reasoning: convert_fact(&capabilities.native_reasoning),
    }
}

fn convert_fact<T: Clone>(fact: &NativeCandidateFactValueV1<T>) -> ComputeCandidateFactValueV2<T> {
    ComputeCandidateFactValueV2 {
        value: fact.value.clone(),
        basis: match fact.basis {
            NativeCandidateFactBasisV1::RegisteredCatalog => {
                ComputeCandidateFactBasisV2::RegisteredCatalog
            }
            NativeCandidateFactBasisV1::RuntimeFallback => {
                ComputeCandidateFactBasisV2::RuntimeFallback
            }
            NativeCandidateFactBasisV1::Observed => ComputeCandidateFactBasisV2::Observed,
            NativeCandidateFactBasisV1::UserDeclared => ComputeCandidateFactBasisV2::UserDeclared,
            NativeCandidateFactBasisV1::Unknown => ComputeCandidateFactBasisV2::Unknown,
        },
    }
}

fn model_ref(
    candidate_ref: &str,
    upstream_model_id: &str,
) -> Result<String, NativeModelConnectionErrorV1> {
    let digest = CanonicalDigest::of(&(
        "native-model-reference/v1",
        candidate_ref,
        upstream_model_id,
    ))
    .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
    Ok(format!("model/{}", digest.as_str()))
}

fn random_candidate_ref() -> Result<String, NativeModelConnectionErrorV1> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| NativeModelConnectionErrorV1::RandomUnavailable)?;
    let mut encoded = String::with_capacity(32);
    for byte in bytes {
        let _ = write!(encoded, "{byte:02x}");
    }
    Ok(format!("candidate/native/{encoded}"))
}
