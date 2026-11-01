//! Two independent random run capabilities. No upstream credential, administrator audience,
//! or child-delegation audience exists here. A prefix is routing syntax, never authorization.
use hiroute_application::delegation::safety::{RunSafetyBinding, RunSafetyProjection};
use hiroute_domain::delegation::{DelegationErrorV1, DelegationRunV1};
use hiroute_domain::{
    AgentAccessGrantMaterial, AgentIngressProtocolV1, CanonicalDigest, ProtectedSecret,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use zeroize::Zeroizing;

pub struct RunCredentialPair {
    pub model: ProtectedSecret,
    pub self_query: ProtectedSecret,
}

#[derive(Clone, Debug)]
pub struct RunCredentialFingerprints {
    pub model: CanonicalDigest,
    pub self_query: CanonicalDigest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunCredentialAudience {
    Model,
    SelfQuery,
}

impl RunCredentialPair {
    pub fn generate() -> Result<Self, DelegationErrorV1> {
        Ok(Self {
            model: generate("hr_run_model_")?,
            self_query: generate("hr_run_query_")?,
        })
    }
    pub fn fingerprints(&self) -> RunCredentialFingerprints {
        RunCredentialFingerprints {
            model: CanonicalDigest::of_bytes(self.model.expose()),
            self_query: CanonicalDigest::of_bytes(self.self_query.expose()),
        }
    }
}

fn generate(prefix: &str) -> Result<ProtectedSecret, DelegationErrorV1> {
    let mut entropy = Zeroizing::new([0u8; 32]);
    getrandom::fill(entropy.as_mut()).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let material = AgentAccessGrantMaterial::from_csprng_entropy(*entropy);
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.extend_from_slice(material.expose());
    ProtectedSecret::new(bytes).map_err(|_| DelegationErrorV1::InvalidArguments)
}

/// Internal metadata resolved through a committed run; never deserialized from the Worker
/// or constructed from its claimed task/run headers. Exact Plan pinning remains a separate
/// mandatory admission/Gateway concern, not authority conferred by this record.
pub struct RunCredentialRecord {
    pub task_id: String,
    pub run_id: String,
    pub lease_id: String,
    pub safety: RunSafetyBinding,
    pub model_alias: String,
    pub protocol: AgentIngressProtocolV1,
    pub fingerprints: RunCredentialFingerprints,
}

pub struct RunCredentialVerifier {
    record: RunCredentialRecord,
    safety: Arc<RunSafetyProjection>,
    revoked: AtomicBool,
    last_request_failure: Mutex<Option<hiroute_gateway::server::dispatch::RunRequestFailure>>,
}

impl hiroute_application::delegation::control::DelegationRunDenial for RunCredentialVerifier {
    fn deny(
        &self,
        run: &hiroute_domain::delegation::DelegationRunV1,
    ) -> Result<(), DelegationErrorV1> {
        if self.record.run_id != run.run_id
            || self.record.task_id != run.task_id
            || self.record.lease_id != run.lease_id
            || self.record.safety.workspace != run.workspace_id
            || self.record.safety.daemon_epoch != run.daemon_epoch
        {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        self.revoke();
        Ok(())
    }
}

impl RunCredentialVerifier {
    /// Caller activates only after accepted runtime commit, before launching the Worker.
    pub fn new(
        record: RunCredentialRecord,
        safety: Arc<RunSafetyProjection>,
    ) -> Result<Self, DelegationErrorV1> {
        if [
            &record.task_id,
            &record.run_id,
            &record.lease_id,
            &record.model_alias,
        ]
        .iter()
        .any(|value| value.is_empty() || value.len() > 256 || value.chars().any(char::is_control))
            || record.fingerprints.model == record.fingerprints.self_query
        {
            return Err(DelegationErrorV1::InvalidArguments);
        }
        Ok(Self {
            record,
            safety,
            revoked: AtomicBool::new(false),
            last_request_failure: Mutex::new(None),
        })
    }

    pub fn authenticate(
        self: &Arc<Self>,
        secret: &[u8],
        audience: RunCredentialAudience,
        now_ms: u64,
    ) -> Result<VerifiedRunCredential, DelegationErrorV1> {
        if secret.is_empty() || secret.len() > 256 {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        let expected = match audience {
            RunCredentialAudience::Model => &self.record.fingerprints.model,
            RunCredentialAudience::SelfQuery => &self.record.fingerprints.self_query,
        };
        if &CanonicalDigest::of_bytes(secret) != expected {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        self.check(now_ms)?;
        Ok(VerifiedRunCredential {
            verifier: self.clone(),
            audience,
        })
    }

    /// Irreversible for this lease, including already authenticated requests. Persist the
    /// terminal/cancel intent before ACP/process side effects; recovery denies the old epoch.
    pub fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }

    /// Internal composition check for the persistent journal.  It does not authenticate a
    /// secret or expose the record; it prevents a lifecycle owner from accidentally revoking a
    /// verifier belonging to another accepted lease.
    pub(crate) fn protects_run(&self, run: &DelegationRunV1) -> bool {
        self.record.task_id == run.task_id
            && self.record.run_id == run.run_id
            && self.record.lease_id == run.lease_id
            && self.record.safety.workspace == run.workspace_id
            && self.record.safety.daemon_epoch == run.daemon_epoch
            && self.record.safety.permit_id == run.permit_id
            && self.record.safety.permit_generation == run.permit_generation
            && self.record.safety.expires_at_ms == run.deadline_ms
    }

    pub(crate) fn model_fingerprint(&self) -> CanonicalDigest {
        self.record.fingerprints.model.clone()
    }

    pub(crate) fn record_request_failure(
        &self,
        failure: hiroute_gateway::server::dispatch::RunRequestFailure,
    ) {
        if let Ok(mut latest) = self.last_request_failure.lock() {
            *latest = Some(failure);
        }
    }

    pub(crate) fn last_request_failure(
        &self,
    ) -> Option<hiroute_gateway::server::dispatch::RunRequestFailure> {
        self.last_request_failure
            .lock()
            .ok()
            .and_then(|latest| *latest)
    }

    fn check(&self, now_ms: u64) -> Result<(), DelegationErrorV1> {
        if self.revoked.load(Ordering::Acquire) {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        self.safety.check(&self.record.safety, now_ms)
    }
}

/// Carries verified task/run identity, but intentionally no Serialize/Deserialize or public
/// constructor. Every use rechecks the shared current projection and this run's terminal bit.
pub struct VerifiedRunCredential {
    verifier: Arc<RunCredentialVerifier>,
    audience: RunCredentialAudience,
}

impl VerifiedRunCredential {
    pub fn check_model(
        &self,
        model: &str,
        protocol: AgentIngressProtocolV1,
        now_ms: u64,
    ) -> Result<(), DelegationErrorV1> {
        if self.audience != RunCredentialAudience::Model
            || model != self.verifier.record.model_alias
            || protocol != self.verifier.record.protocol
        {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        self.verifier.check(now_ms)
    }

    pub fn check_self_query(
        &self,
        task: &str,
        run: &str,
        now_ms: u64,
    ) -> Result<(), DelegationErrorV1> {
        if self.audience != RunCredentialAudience::SelfQuery
            || task != self.task_id()
            || run != self.run_id()
        {
            return Err(DelegationErrorV1::PermissionDenied);
        }
        self.verifier.check(now_ms)
    }
    pub fn task_id(&self) -> &str {
        &self.verifier.record.task_id
    }
    pub fn run_id(&self) -> &str {
        &self.verifier.record.run_id
    }
    pub fn lease_id(&self) -> &str {
        &self.verifier.record.lease_id
    }
}

#[cfg(test)]
mod tests;
