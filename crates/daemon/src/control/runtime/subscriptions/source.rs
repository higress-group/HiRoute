//! Credential ownership projections shared by discovery, checking and maintenance.
//!
//! Discovery never starts CPA or reads native credentials. A managed session is only a
//! candidate locator; the exact runtime validates its immutable account before execution.

use super::*;
use hiroute_application::control::ComputeManagementControlError;
use hiroute_cpa_bridge::{CpaLoginSession, CpaLoginState};
use hiroute_integrations::ProtectedAgentSubscriptionSourceV1;

#[derive(Clone)]
pub(in crate::control::runtime) enum SubscriptionSource {
    Native(ProtectedAgentSubscriptionSourceV1),
    Managed {
        session: CpaLoginSession,
        descriptor: ProtectedInputSourceDescriptorV1,
        evidence: CanonicalDigest,
    },
}

impl SubscriptionSource {
    fn managed(session: CpaLoginSession) -> Result<Self, ComputeManagementControlError> {
        let account = session
            .account_ref
            .as_deref()
            .ok_or(ComputeManagementControlError::Corrupt)?;
        if session.state != CpaLoginState::Authorized {
            return Err(ComputeManagementControlError::Conflict);
        }
        let source_ref = format!(
            "cpa/managed/{}/{}",
            session.kind.stock_provider(),
            session.login_ref
        );
        let evidence = CanonicalDigest::of(&(
            "hiroute.cpa-managed-subscription-discovery/v1",
            &source_ref,
            account,
        ))
        .map_err(|_| ComputeManagementControlError::Corrupt)?;
        Ok(Self::Managed {
            session,
            descriptor: ProtectedInputSourceDescriptorV1::DiscoveredConfig {
                scanner_id: "builtin.cpa-managed-login".into(),
                scanner_version: "1".into(),
                source_ref,
                field_selector: "managed-subscription".into(),
                observed_revision: 1,
            },
            evidence,
        })
    }

    pub(super) fn kind(&self) -> CpaAccountKind {
        match self {
            Self::Native(source) => source.kind(),
            Self::Managed { session, .. } => session.kind,
        }
    }

    pub(super) fn descriptor(&self) -> &ProtectedInputSourceDescriptorV1 {
        match self {
            Self::Native(source) => source.descriptor(),
            Self::Managed { descriptor, .. } => descriptor,
        }
    }

    pub(super) fn evidence_digest(&self) -> &CanonicalDigest {
        match self {
            Self::Native(source) => source.evidence_digest(),
            Self::Managed { evidence, .. } => evidence,
        }
    }

    pub(super) fn expected_account_ref(&self) -> Option<&str> {
        match self {
            Self::Native(_) => None,
            Self::Managed { session, .. } => session.account_ref.as_deref(),
        }
    }

    pub(super) fn is_managed(&self) -> bool {
        matches!(self, Self::Managed { .. })
    }
}

impl LocalControlAdapter {
    pub(super) fn subscription_source_for_candidate(
        &self,
        candidate: &str,
    ) -> Result<Option<SubscriptionSource>, ComputeManagementControlError> {
        source_for_candidate(&self.scanner, self.cpa_runtime.as_deref(), candidate)
    }

    pub(super) fn subscription_discovery_sources(
        &self,
        kind: CpaAccountKind,
    ) -> Result<Vec<SubscriptionSource>, ComputeManagementControlError> {
        let Some(runtimes) = self.cpa_runtime.as_ref() else {
            return Ok(Vec::new());
        };
        let mut sources = Vec::new();
        if runtimes.for_kind(kind).is_some()
            && let Ok(Some(source)) = self.scanner.subscription_source(kind)
        {
            sources.push(SubscriptionSource::Native(source));
        }
        for session in runtimes.login_sessions(kind) {
            if session.state == CpaLoginState::Authorized {
                sources.push(SubscriptionSource::managed(session)?);
            }
        }
        Ok(sources)
    }
}

pub(super) fn source_for_candidate(
    scanner: &hiroute_integrations::FilesystemAgentScannerV1,
    runtimes: Option<&hiroute_cpa_bridge::ManagedCpaRuntimeSet>,
    candidate: &str,
) -> Result<Option<SubscriptionSource>, ComputeManagementControlError> {
    let kind =
        CpaAccountKind::from_candidate(candidate).ok_or(ComputeManagementControlError::Invalid)?;
    let managed_prefix = format!("candidate/cpa/{}/managed/", kind.stock_provider());
    if candidate.starts_with(&managed_prefix) {
        return runtimes
            .and_then(|runtimes| {
                runtimes.login_sessions(kind).into_iter().find(|session| {
                    session.candidate_ref() == candidate
                        && session.state == CpaLoginState::Authorized
                })
            })
            .map(SubscriptionSource::managed)
            .transpose();
    }
    let source = scanner
        .subscription_source(kind)
        .map_err(|_| ComputeManagementControlError::Unavailable)?
        .map(SubscriptionSource::Native);
    source
        .map(|source| {
            if subscription_candidate_ref(&source)? != candidate {
                return Err(ComputeManagementControlError::Conflict);
            }
            Ok(source)
        })
        .transpose()
}

pub(super) fn pending_candidate(
    source: &SubscriptionSource,
    revision: u64,
    existing_source: Option<(String, CanonicalDigest)>,
) -> Result<ComputeCandidateFactsV2, hiroute_application::control::ComputeManagementControlError> {
    let candidate_ref = subscription_candidate_ref(source)?;
    let lineage_ref = match source.descriptor() {
        ProtectedInputSourceDescriptorV1::DiscoveredConfig { source_ref, .. } => source_ref.clone(),
        ProtectedInputSourceDescriptorV1::ManualInput => {
            return Err(hiroute_application::control::ComputeManagementControlError::Corrupt);
        }
    };
    let candidate = ComputeCandidateRefV2 {
        candidate_ref: candidate_ref.clone(),
        candidate_revision: revision,
    };
    let (existing_source_id, trusted_lineage_digest) = existing_source
        .map(|(source_id, lineage)| (Some(source_id), Some(lineage)))
        .unwrap_or((None, None));
    Ok(ComputeCandidateFactsV2 {
        candidate: candidate.clone(),
        correlation: ComputeCheckCorrelationV2 {
            candidate_ref: candidate_ref.clone(),
            edit_revision: revision,
            check_id: format!("check/cpa/{}/{revision}", source.kind().stock_provider()),
            input_digest: source.evidence_digest().clone(),
        },
        producer: ComputeCandidateProducerV2::Cpa,
        lineage_ref,
        trusted_lineage_digest,
        display_name: match source.kind() {
            CpaAccountKind::Codex => "Codex subscription",
            CpaAccountKind::Claude => "Claude Code subscription",
        }
        .into(),
        existing_source_id,
        evidence_digest: source.evidence_digest().clone(),
        provenance: ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval {
            connector_id: source.kind().connector_id().into(),
        },
        target: None,
        authentication: None,
        models: Vec::new(),
        native_recheck: None,
        additional_native_endpoints: Vec::new(),
        discovery_guard: None,
        credential_binding: ComputeCredentialBindingV2::CpaPendingApproval {
            protected_source: source.descriptor().clone(),
        },
        validation: None,
    })
}

pub(super) fn subscription_candidate_ref(
    source: &SubscriptionSource,
) -> Result<String, hiroute_application::control::ComputeManagementControlError> {
    if let SubscriptionSource::Managed { session, .. } = source {
        return Ok(session.candidate_ref());
    }
    let ProtectedInputSourceDescriptorV1::DiscoveredConfig { source_ref, .. } = source.descriptor()
    else {
        return Err(hiroute_application::control::ComputeManagementControlError::Corrupt);
    };
    let domain = match source.kind() {
        CpaAccountKind::Codex => "hiroute.codex-subscription-candidate/v1",
        CpaAccountKind::Claude => "hiroute.claude-subscription-candidate/v1",
    };
    let digest = CanonicalDigest::of(&(domain, source_ref))
        .map_err(|_| hiroute_application::control::ComputeManagementControlError::Corrupt)?;
    Ok(format!(
        "candidate/cpa/{}/{}",
        source.kind().stock_provider(),
        digest.as_str().trim_start_matches("sha256:")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorized_login_is_only_a_pending_candidate_until_explicit_check_and_save() {
        let session = CpaLoginSession {
            login_ref: "login-opaque".into(),
            kind: CpaAccountKind::Claude,
            state: CpaLoginState::Authorized,
            account_ref: Some("account/cpa/opaque".into()),
        };
        let candidate_ref = session.candidate_ref();
        let source = SubscriptionSource::managed(session).unwrap();
        let facts = pending_candidate(&source, 1, None).unwrap();
        facts.validate_shape().unwrap();
        assert_eq!(facts.candidate.candidate_ref, candidate_ref);
        assert!(matches!(
            facts.provenance,
            ComputeCandidateProvenanceV2::ConnectorOwnedPendingApproval { .. }
        ));
        assert!(facts.target.is_none());
        assert!(facts.authentication.is_none());
        assert!(facts.models.is_empty());
        assert!(facts.validation.is_none());
    }

    #[test]
    fn pending_or_cancelled_login_never_becomes_a_checkable_source() {
        for state in [
            CpaLoginState::Pending,
            CpaLoginState::Cancelled,
            CpaLoginState::Forgotten,
        ] {
            let session = CpaLoginSession {
                login_ref: "login-opaque".into(),
                kind: CpaAccountKind::Codex,
                state,
                account_ref: Some("account/cpa/opaque".into()),
            };
            assert!(SubscriptionSource::managed(session).is_err());
        }
    }
}
