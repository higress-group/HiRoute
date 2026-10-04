//! Daemon implementation of Gateway's narrow exact-run authority.
//!
//! This is an in-memory index over already accepted runs.  It holds no raw run secret and never
//! falls back to the aggregate publication: each entry compiles the exact retained Plan version
//! into Gateway's isolated run handle, while the verifier rechecks revocation and lease safety on
//! every request.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_domain::delegation::{DelegationErrorV1, DelegationRunV1, DelegationTaskV1};
use hiroute_domain::{
    AgentIngressProtocolV1, AgentModelGrantV2, AgentModelRouteV2, AliasRegistryV1, CanonicalDigest,
    GatewayAccessGrantV1, GatewayPublicationRevision, GatewayPublicationV1, PlanVersionV1,
};
use hiroute_gateway::server::dispatch::{
    RunObservationMetadata, RunRequestAuthorityError, RunRequestAuthorityPort, RunRequestLocator,
    RunRequestSafetyPort, VerifiedRunRequestAuthority,
};
use hiroute_gateway::server::publication::RunPublicationHandle;
use hiroute_gateway::server::request_plan::IngressProtocol;

use super::credentials::{RunCredentialAudience, RunCredentialVerifier};

const RUN_AUTHORITY_RENDERER: &str = "hiroute.delegation-run/v1";

#[derive(Default)]
pub struct DelegationRunAuthority {
    entries: Mutex<BTreeMap<CanonicalDigest, Arc<RunAuthorityEntry>>>,
}

struct RunAuthorityEntry {
    network_allowed: bool,
    locator: RunRequestLocator,
    model_alias: String,
    protocol: IngressProtocol,
    publication: Arc<RunPublicationHandle>,
    safety: Arc<RunSafety>,
}

struct RunSafety {
    locator: RunRequestLocator,
    verifier: Arc<RunCredentialVerifier>,
}

impl DelegationRunAuthority {
    /// Registers one accepted run after its exact version reservation and in-memory verifier are
    /// both ready.  No current publication is consulted: old accepted work remains pinned to the
    /// version that admission retained.
    pub fn register(
        &self,
        task: &DelegationTaskV1,
        run: &DelegationRunV1,
        version: Arc<PlanVersionV1>,
        verifier: Arc<RunCredentialVerifier>,
    ) -> Result<(), DelegationErrorV1> {
        let entry = Arc::new(build_entry(task, run, version.as_ref(), verifier)?);
        let fingerprint = entry.safety.verifier.model_fingerprint();
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        if let Some(existing) = entries.get(&fingerprint) {
            return if existing.locator == entry.locator
                && existing.model_alias == entry.model_alias
                && existing.protocol == entry.protocol
            {
                Ok(())
            } else {
                Err(DelegationErrorV1::Conflict)
            };
        }
        entries.insert(fingerprint, entry);
        Ok(())
    }

    /// Removing the index cannot resurrect an in-flight request: the caller first revokes the
    /// matching verifier, so a cloned Gateway authority still fails its request-time safety check.
    pub fn unregister(&self, run: &DelegationRunV1, verifier: &RunCredentialVerifier) {
        let fingerprint = verifier.model_fingerprint();
        if let Ok(mut entries) = self.entries.lock()
            && entries.get(&fingerprint).is_some_and(|entry| {
                entry.locator.workspace_id() == &run.workspace_id
                    && entry.locator.task_id() == run.task_id
                    && entry.locator.run_id() == run.run_id
                    && entry.locator.lease_id() == run.lease_id
            })
        {
            entries.remove(&fingerprint);
        }
    }

    /// Irreversibly denies every in-memory request credential for this exact durable lease.
    /// A run that has not registered yet is valid input: its persisted cancel bit will be seen
    /// before attachment, so absence from this acceleration index is not an authorization gap.
    pub(crate) fn deny_run(&self, run: &DelegationRunV1) -> Result<(), DelegationErrorV1> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| DelegationErrorV1::StorageUnavailable)?;
        for entry in entries.values() {
            if entry.locator.workspace_id() == &run.workspace_id
                && entry.locator.task_id() == run.task_id
                && entry.locator.run_id() == run.run_id
                && entry.locator.lease_id() == run.lease_id
            {
                entry.safety.verifier.revoke();
            }
        }
        Ok(())
    }
}

impl RunRequestAuthorityPort for DelegationRunAuthority {
    fn authenticate_run(
        &self,
        authorization: &str,
        protocol: IngressProtocol,
    ) -> Result<VerifiedRunRequestAuthority, RunRequestAuthorityError> {
        let token = model_bearer(authorization)?;
        let fingerprint = CanonicalDigest::of_bytes(token.as_bytes());
        let entry = self
            .entries
            .lock()
            .map_err(|_| RunRequestAuthorityError::Unavailable)?
            .get(&fingerprint)
            .cloned()
            .ok_or(RunRequestAuthorityError::Denied)?;
        if entry.protocol != protocol {
            return Err(RunRequestAuthorityError::Denied);
        }
        // Authenticate before returning a Gateway handle so an old/revoked credential never
        // obtains a reusable branch object. `begin` repeats the same request-time check.
        entry.safety.authorize_request(
            authorization,
            &entry.locator,
            &entry.model_alias,
            protocol,
        )?;
        let safety: Arc<dyn RunRequestSafetyPort> = entry.safety.clone();
        VerifiedRunRequestAuthority::new(
            entry.locator.clone(),
            Arc::clone(&entry.publication),
            safety,
            entry.network_allowed,
        )
    }
}

impl RunRequestSafetyPort for RunSafety {
    fn authorize_request(
        &self,
        authorization: &str,
        locator: &RunRequestLocator,
        model_alias: &str,
        protocol: IngressProtocol,
    ) -> Result<(), RunRequestAuthorityError> {
        if locator != &self.locator {
            return Err(RunRequestAuthorityError::Denied);
        }
        let token = model_bearer(authorization)?;
        let now = now_ms().map_err(|_| RunRequestAuthorityError::Unavailable)?;
        let credential = self
            .verifier
            .authenticate(token.as_bytes(), RunCredentialAudience::Model, now)
            .map_err(|_| RunRequestAuthorityError::Denied)?;
        credential
            .check_model(model_alias, product_protocol(protocol)?, now)
            .map_err(|_| RunRequestAuthorityError::Denied)
    }
}

fn build_entry(
    task: &DelegationTaskV1,
    run: &DelegationRunV1,
    version: &PlanVersionV1,
    verifier: Arc<RunCredentialVerifier>,
) -> Result<RunAuthorityEntry, DelegationErrorV1> {
    let work = version
        .configuration
        .work
        .as_ref()
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    if task.workspace_id != run.workspace_id
        || task.task_id != run.task_id
        || task.plan.plan_id != version.reference.plan_id
        || task.plan.plan_revision != version.reference.content_revision
        || task.plan.plan_digest != version.reference.content_digest
        || task.plan.exact_reference != exact_reference(&version.reference)
        || task.plan.model_alias != version.compiled.model_alias().as_str()
        || task.plan.harness != work.harness
        || !verifier.protects_run(run)
    {
        return Err(DelegationErrorV1::Conflict);
    }
    if CanonicalDigest::of(work).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?
        != task.plan.harness_configuration_digest
    {
        return Err(DelegationErrorV1::Conflict);
    }
    let protocol = gateway_protocol(work.protocol)?;
    let plan_id = version.compiled.agent_plan_id().clone();
    let alias = version.compiled.model_alias().clone();
    let plan_grant = AgentModelGrantV2::seal(
        work.protocol,
        BTreeMap::from([(
            alias.as_str().to_owned(),
            AgentModelRouteV2::Plan {
                plan_id: plan_id.clone(),
                alias: alias.clone(),
                revision: version.compiled.body.agent_plan_revision,
                semantic_digest: version.compiled.body.materialized_route_digest.clone(),
            },
        )]),
    )
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    // The bearer verifier and RunRequestLocator below remain exact-run authority.  The receipt
    // identity is deliberately task-stable so an authenticated Continue can replay Tool
    // call/result history whose opaque IDs were accepted during an earlier run of this task.
    // Plan revision/digest and the Gateway's accepted-ID authority remain additional boundaries.
    let task_scope = CanonicalDigest::of(&(
        "hiroute.delegation-tool-continuation-scope/v1",
        &run.workspace_id,
        &task.task_id,
    ))
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let task_scope = task_scope
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(DelegationErrorV1::CapabilityUnavailable)?;
    let authority_id = format!("delegation-task/{task_scope}");
    let access_grant = GatewayAccessGrantV1::new(
        authority_id.clone(),
        1,
        verifier.model_fingerprint(),
        work.protocol,
        plan_grant,
    )
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let mut aliases = AliasRegistryV1::default();
    aliases.active.insert(plan_id, alias.clone());
    let publication = GatewayPublicationV1::seal(
        run.workspace_id.clone(),
        authority_id,
        1,
        GatewayPublicationRevision::new(1).map_err(|_| DelegationErrorV1::CapabilityUnavailable)?,
        RUN_AUTHORITY_RENDERER,
        aliases,
        vec![version.compiled.clone()],
        vec![access_grant],
    )
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let projection = publication
        .gateway_snapshot()
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let snapshot = hiroute_integrations::gateway::project_publication(&projection)
        .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let handle = RunPublicationHandle::compile_exact(
        snapshot,
        alias.as_str(),
        protocol,
        &verifier.model_fingerprint(),
    )
    .map_err(|_| DelegationErrorV1::CapabilityUnavailable)?;
    let locator = RunRequestLocator::new(
        run.workspace_id.clone(),
        run.task_id.clone(),
        run.run_id.clone(),
        run.lease_id.clone(),
        task.plan.exact_reference.clone(),
        RunObservationMetadata::new(
            task.plan.plan_id.as_str(),
            task.plan.plan_revision,
            format!(
                "publication/{}/digest/{}",
                task.plan.publication_revision, task.plan.publication_digest
            ),
            match task.plan.harness {
                hiroute_domain::delegation::WorkerHarnessV1::CodexCli => "codex",
                hiroute_domain::delegation::WorkerHarnessV1::ClaudeCode => "claude",
                hiroute_domain::delegation::WorkerHarnessV1::QoderCli => "qoder",
            },
            task.session
                .as_ref()
                .and_then(|session| session.native_session_id.clone()),
            task.parent_task_ref.clone(),
            run.continued_from.clone(),
        )
        .map_err(|_| DelegationErrorV1::Conflict)?,
    )
    .map_err(|_| DelegationErrorV1::Conflict)?;
    let safety = Arc::new(RunSafety {
        locator: locator.clone(),
        verifier,
    });
    Ok(RunAuthorityEntry {
        network_allowed: run.execution.network
            == hiroute_domain::delegation::WorkerNetworkV1::Allowed,
        locator,
        model_alias: alias.as_str().to_owned(),
        protocol,
        publication: handle,
        safety,
    })
}

fn model_bearer(authorization: &str) -> Result<&str, RunRequestAuthorityError> {
    let token = authorization
        .strip_prefix("Bearer ")
        .filter(|token| token.starts_with("hr_run_model_"))
        .filter(|token| !token.is_empty() && token.len() <= 256)
        .ok_or(RunRequestAuthorityError::Denied)?;
    if token.bytes().any(|byte| !byte.is_ascii_graphic()) {
        return Err(RunRequestAuthorityError::Denied);
    }
    Ok(token)
}

fn product_protocol(
    protocol: IngressProtocol,
) -> Result<AgentIngressProtocolV1, RunRequestAuthorityError> {
    match protocol {
        IngressProtocol::Responses => Ok(AgentIngressProtocolV1::Responses),
        IngressProtocol::Messages => Ok(AgentIngressProtocolV1::Messages),
        IngressProtocol::ChatCompletions => Err(RunRequestAuthorityError::Denied),
    }
}

fn gateway_protocol(
    protocol: AgentIngressProtocolV1,
) -> Result<IngressProtocol, DelegationErrorV1> {
    match protocol {
        AgentIngressProtocolV1::Responses => Ok(IngressProtocol::Responses),
        AgentIngressProtocolV1::Messages => Ok(IngressProtocol::Messages),
    }
}

fn exact_reference(reference: &hiroute_domain::PlanExecutionRef) -> String {
    format!(
        "plan/{}/revision/{}/digest/{}",
        reference.plan_id.as_str(),
        reference.content_revision,
        reference.content_digest
    )
}

fn now_ms() -> Result<u64, DelegationErrorV1> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DelegationErrorV1::DeadlineExceeded)?
        .as_millis();
    u64::try_from(millis).map_err(|_| DelegationErrorV1::DeadlineExceeded)
}

#[cfg(test)]
mod tests;
