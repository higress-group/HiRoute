//! Bounded compatibility execution; no runtime selection, filesystem or blocking
//! authority lock is touched until the job is on Tokio's blocking executor.
use std::sync::Arc;

use hiroute_cpa_bridge::{
    CpaAccountKind, CpaDownstreamCredentialCapability, CpaDownstreamCredentialPort,
    CpaRequestContext, ExactCpaCredentialRequest,
};
use hiroute_gateway::ports::ExecutionScope;
use hiroute_gateway::server::composition::PortError;
use tokio::sync::Semaphore;

pub(super) struct CpaExecutor {
    codex: ProviderExecutor,
    claude: ProviderExecutor,
}

struct ProviderExecutor {
    active: Arc<Semaphore>,
    waiting: Arc<Semaphore>,
}

impl ProviderExecutor {
    fn new() -> Self {
        Self {
            active: Arc::new(Semaphore::new(2)),
            waiting: Arc::new(Semaphore::new(16)),
        }
    }
}

impl CpaExecutor {
    pub(super) fn new() -> Self {
        Self {
            codex: ProviderExecutor::new(),
            claude: ProviderExecutor::new(),
        }
    }

    pub(super) async fn lease<C>(
        &self,
        port: Arc<C>,
        request: ExactCpaCredentialRequest<'_>,
        scope: &ExecutionScope,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, PortError>
    where
        C: CpaDownstreamCredentialPort + Send + Sync + 'static,
    {
        let executor = match CpaAccountKind::from_connector(request.connector_id) {
            Some(CpaAccountKind::Claude) => &self.claude,
            Some(CpaAccountKind::Codex) => &self.codex,
            None => return Err(PortError::Rejected),
        };
        let permit = match executor.active.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let _waiting = executor
                    .waiting
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| PortError::Unavailable("CPA credential queue full"))?;
                scope
                    .run(executor.active.clone().acquire_owned())
                    .await
                    .map_err(|_| PortError::Rejected)?
                    .map_err(|_| PortError::Unavailable("CPA credential executor closed"))?
            }
        };
        scope.ensure_active().map_err(|_| PortError::Rejected)?;
        let cancellation = scope.cancellation().clone();
        let context = CpaRequestContext::new(scope.deadline())
            .with_cancellation_check(Arc::new(move || cancellation.is_cancelled()));
        // Dropping this future, including through an outer ExecutionScope, revokes
        // the running job. The permit stays inside that job until it actually exits.
        let _cancel = CancelOnDrop(context.clone());
        let owned = OwnedRequest::from(request);
        let job = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            port.lease_downstream_capability_scoped(owned.borrow(), &context)
        });
        scope
            .run(job)
            .await
            .map_err(|_| PortError::Rejected)?
            .map_err(|_| PortError::Unavailable("CPA credential executor failed"))?
            .map_err(|error| match error {
                hiroute_cpa_bridge::CpaAttemptError::Unavailable => {
                    PortError::Unavailable("CpaDownstreamCredentialPort")
                }
                _ => PortError::Rejected,
            })
    }
}

struct CancelOnDrop(CpaRequestContext);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct OwnedRequest {
    credential_id: String,
    connector_id: String,
    upstream_model_id: String,
    protocol: hiroute_domain::UpstreamProtocol,
    address: std::net::SocketAddr,
    request_path: String,
    native_transport_model: String,
    runtime_epoch: u64,
    target_epoch: u64,
    excluded_key_ids: Vec<Arc<str>>,
}

impl From<ExactCpaCredentialRequest<'_>> for OwnedRequest {
    fn from(value: ExactCpaCredentialRequest<'_>) -> Self {
        Self {
            credential_id: value.credential_id.into(),
            connector_id: value.connector_id.into(),
            upstream_model_id: value.upstream_model_id.into(),
            protocol: value.protocol,
            address: value.address,
            request_path: value.request_path.into(),
            native_transport_model: value.native_transport_model.into(),
            runtime_epoch: value.runtime_epoch,
            target_epoch: value.target_epoch,
            excluded_key_ids: value.excluded_key_ids.to_vec(),
        }
    }
}

impl OwnedRequest {
    fn borrow(&self) -> ExactCpaCredentialRequest<'_> {
        ExactCpaCredentialRequest {
            credential_id: &self.credential_id,
            connector_id: &self.connector_id,
            upstream_model_id: &self.upstream_model_id,
            protocol: self.protocol,
            address: self.address,
            request_path: &self.request_path,
            native_transport_model: &self.native_transport_model,
            runtime_epoch: self.runtime_epoch,
            target_epoch: self.target_epoch,
            excluded_key_ids: &self.excluded_key_ids,
        }
    }
}
