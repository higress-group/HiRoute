//! Provider-isolated CPA instances sharing the same lifecycle implementation.
use crate::{
    CpaAccountKind, CpaAttemptError, CpaDownstreamCredentialCapability,
    CpaDownstreamCredentialPort, CpaLifecycleError, CpaRegisteredSourcePort, CpaRoutingBatch,
    ExactCpaCredentialRequest, ManagedCpaRuntime,
};
use hiroute_integrations::CpaRegisteredSourceV1;
use std::sync::Arc;

pub struct ManagedCpaRuntimeSet {
    runtimes: Vec<Arc<ManagedCpaRuntime>>,
}
impl ManagedCpaRuntimeSet {
    pub fn new(runtimes: Vec<Arc<ManagedCpaRuntime>>) -> Result<Self, CpaLifecycleError> {
        let mut kinds = std::collections::BTreeSet::new();
        for runtime in &runtimes {
            let kind = runtime
                .managed_kind()
                .ok_or(CpaLifecycleError::InvalidSpec)?;
            if !kinds.insert(kind) {
                return Err(CpaLifecycleError::InvalidSpec);
            }
        }
        Ok(Self { runtimes })
    }
    pub fn for_kind(&self, kind: CpaAccountKind) -> Option<Arc<ManagedCpaRuntime>> {
        self.runtimes
            .iter()
            .find(|runtime| runtime.managed_kind() == Some(kind))
            .cloned()
    }
    pub fn for_connector(&self, connector: &str) -> Option<Arc<ManagedCpaRuntime>> {
        self.for_kind(CpaAccountKind::from_connector(connector)?)
    }
    pub fn shutdown(&self) -> Result<(), CpaLifecycleError> {
        let mut error = None;
        for runtime in &self.runtimes {
            if let Err(e) = runtime.shutdown() {
                error = Some(e);
            }
        }
        error.map_or(Ok(()), Err)
    }
}
impl From<Arc<ManagedCpaRuntime>> for ManagedCpaRuntimeSet {
    fn from(runtime: Arc<ManagedCpaRuntime>) -> Self {
        Self {
            runtimes: vec![runtime],
        }
    }
}
impl CpaRegisteredSourcePort for ManagedCpaRuntimeSet {
    fn discover_registered_sources(&self) -> Result<Vec<CpaRegisteredSourceV1>, CpaLifecycleError> {
        let mut sources = Vec::new();
        for runtime in &self.runtimes {
            // A missing/failed optional subscription must not remove healthy sibling facts.
            if let Ok(found) = runtime.discover_registered_sources() {
                sources.extend(found);
            }
        }
        Ok(sources)
    }
    fn begin_routing_batch(&self) -> Result<CpaRoutingBatch<'_>, CpaLifecycleError> {
        let mut batch: Option<CpaRoutingBatch<'_>> = None;
        for runtime in &self.runtimes {
            if let Ok(next) = runtime.begin_routing_batch() {
                if let Some(batch) = &mut batch {
                    batch.extend(next);
                } else {
                    batch = Some(next);
                }
            }
        }
        batch.ok_or(CpaLifecycleError::NotStarted)
    }
}
impl CpaDownstreamCredentialPort for ManagedCpaRuntimeSet {
    fn lease_downstream_capability(
        &self,
        request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        self.for_connector(request.connector_id)
            .ok_or(CpaAttemptError::UnregisteredTarget)?
            .lease_downstream_capability(request)
    }
}
