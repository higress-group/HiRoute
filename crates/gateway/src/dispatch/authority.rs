use std::sync::Arc;
use std::time::{Duration, Instant};

use hiroute_gateway_core::core::execution_plan::PlanError;
use http::StatusCode;
use thiserror::Error;

use super::{
    ModelSelector, RunRequestAuthorityError, RunRequestAuthorityPort, SelectorError,
    VerifiedRunRequestAuthority,
};
use crate::server::composition::RuntimePublicationFeed;
use crate::server::publication::{CompiledGrant, CompiledGrantRoute, PublishedGatewayPublication};
use crate::server::request_plan::{
    AuthorizedRequestPlan, IngressProtocol, RequestAuthorityReceipt,
};

const DEFAULT_SELECTOR_LIMIT: usize = 16 * 1024;
/// Time allowed to identify the top-level model alias. This ingress bound is
/// deliberately independent of every AgentPlan deadline: adding or removing
/// another alias cannot change how long an incomplete selector pins the
/// aggregate publication.
const DEFAULT_SELECTOR_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct GatewayRequestAuthority {
    publications: Arc<dyn RuntimePublicationFeed>,
    run_authority: Option<Arc<dyn RunRequestAuthorityPort>>,
    selector_limit: usize,
}

impl GatewayRequestAuthority {
    pub fn new<F>(publications: Arc<F>) -> Self
    where
        F: RuntimePublicationFeed + 'static,
    {
        Self::from_feed(publications)
    }

    pub fn from_feed(publications: Arc<dyn RuntimePublicationFeed>) -> Self {
        Self {
            publications,
            run_authority: None,
            selector_limit: DEFAULT_SELECTOR_LIMIT,
        }
    }

    pub fn with_selector_limit(mut self, selector_limit: usize) -> Self {
        self.selector_limit = selector_limit;
        self
    }

    pub fn with_run_request_authority(
        mut self,
        authority: Arc<dyn RunRequestAuthorityPort>,
    ) -> Self {
        self.run_authority = Some(authority);
        self
    }

    /// Pins and authenticates one aggregate publication before any body read.
    pub fn begin(
        &self,
        protocol: IngressProtocol,
        authorization: Option<&str>,
    ) -> Result<AuthenticatedRequest, DispatchError> {
        self.begin_at(protocol, authorization, Instant::now())
    }

    pub fn begin_at(
        &self,
        protocol: IngressProtocol,
        authorization: Option<&str>,
        request_started_at: Instant,
    ) -> Result<AuthenticatedRequest, DispatchError> {
        let authorization = authorization.ok_or(DispatchError::Unauthorized)?;
        let (publication, grant, run) = if is_run_bearer(authorization) {
            let authority = self
                .run_authority
                .as_ref()
                .ok_or(DispatchError::Unauthorized)?;
            let run = authority
                .authenticate_run(authorization, protocol)
                .map_err(map_run_error)?;
            let (publication, grant) = run.begin(authorization, protocol).map_err(map_run_error)?;
            (publication, grant, Some(run))
        } else {
            let publication = self
                .publications
                .pin()
                .ok_or(DispatchError::PublicationUnavailable)?;
            let grant = publication
                .authenticate_bearer(authorization)
                .ok_or(DispatchError::Unauthorized)?;
            (publication, grant, None)
        };
        if grant.protocol != protocol {
            return Err(DispatchError::AgentPlanProtocolUnsupported);
        }
        let has_protocol_alias = grant.routes.values().any(|route| match route {
            CompiledGrantRoute::Plan { alias } => publication
                .aliases
                .get(alias)
                .is_some_and(|alias| alias.execution.protocols.contains(&protocol)),
            CompiledGrantRoute::Fixed { execution, .. } => execution.protocols.contains(&protocol),
        });
        if !has_protocol_alias {
            return Err(DispatchError::AgentPlanProtocolUnsupported);
        }
        let selector_deadline = request_started_at
            .checked_add(DEFAULT_SELECTOR_TIMEOUT)
            .ok_or(DispatchError::InvalidDeadline)?;
        Ok(AuthenticatedRequest {
            publication,
            grant,
            protocol,
            request_started_at,
            selector_deadline,
            run,
        })
    }

    pub fn authorize_bytes(
        &self,
        protocol: IngressProtocol,
        authorization: Option<&str>,
        body: &[u8],
        now: Instant,
    ) -> Result<AuthorizedRequestPlan, DispatchError> {
        let authenticated = self.begin_at(protocol, authorization, now)?;
        let mut selector = ModelSelector::new(self.selector_limit);
        let alias = match selector.feed(body)? {
            Some(alias) => alias,
            None => selector.finish()?,
        };
        authenticated.authorize_alias(&alias, Instant::now())
    }

    pub fn selector_limit(&self) -> usize {
        self.selector_limit
    }
}

pub struct AuthenticatedRequest {
    publication: Arc<PublishedGatewayPublication>,
    grant: CompiledGrant,
    protocol: IngressProtocol,
    request_started_at: Instant,
    selector_deadline: Instant,
    run: Option<VerifiedRunRequestAuthority>,
}

impl AuthenticatedRequest {
    pub fn authorize_alias(
        &self,
        served_model_id: &str,
        selection_completed_at: Instant,
    ) -> Result<AuthorizedRequestPlan, DispatchError> {
        if selection_completed_at >= self.selector_deadline {
            return Err(DispatchError::RequestDeadlineExceeded);
        }
        if let Some(run) = &self.run {
            run.authorize_alias(served_model_id, self.protocol)
                .map_err(map_run_error)?;
        }
        let route = self
            .grant
            .routes
            .get(served_model_id)
            .ok_or(DispatchError::AgentModelNotGranted)?;
        let (execution, provenance, plan_display_name) = match route {
            CompiledGrantRoute::Plan { alias } => {
                let alias = self
                    .publication
                    .aliases
                    .get(alias)
                    .ok_or(DispatchError::AgentPlanNotAvailable)?;
                let digest = hiroute_domain::CanonicalDigest::parse(
                    alias.agent_plan_semantic_digest.as_ref(),
                )
                .map_err(|_| DispatchError::AgentPlanNotAvailable)?;
                (
                    &alias.execution,
                    hiroute_domain::ModelRequestRouteV2::Plan {
                        revision: alias.agent_plan_revision,
                        semantic_digest: digest,
                    },
                    alias.plan_display_name.clone(),
                )
            }
            CompiledGrantRoute::Fixed {
                binding_digest,
                execution,
            } => (
                execution,
                hiroute_domain::ModelRequestRouteV2::Fixed {
                    binding_digest: binding_digest.clone(),
                },
                None,
            ),
        };
        if !execution.protocols.contains(&self.protocol) {
            return Err(DispatchError::AgentPlanProtocolUnsupported);
        }
        let deadline = self
            .request_started_at
            .checked_add(execution.overall_timeout)
            .ok_or(DispatchError::InvalidDeadline)?;
        if selection_completed_at >= deadline {
            return Err(DispatchError::RequestDeadlineExceeded);
        }
        let mut core = self.publication.core.bind_request()?;
        core.bind_route(&execution.route)?;
        let receipt = RequestAuthorityReceipt {
            schema_version: "hiroute.gateway.request-authority/v2",
            workspace_id: Arc::from(self.publication.workspace_id()),
            authority_id: Arc::from(self.publication.authority_id()),
            authority_epoch: self.publication.authority_epoch(),
            publication_revision: self.publication.publication_revision(),
            publication_digest: Arc::from(self.publication.payload_digest()),
            route: provenance,
            plan_display_name,
            served_model_id: Arc::from(served_model_id),
            grant_id: Arc::clone(&self.grant.grant_id),
            grant_generation: self.grant.generation,
            ingress_protocol: self.protocol,
            max_attempts: execution.max_attempts,
            overall_timeout_ms: execution.overall_timeout.as_millis() as u64,
        };
        let verified_run_observation = self
            .run
            .as_ref()
            .map(VerifiedRunRequestAuthority::observation_context);
        Ok(AuthorizedRequestPlan::new(
            receipt,
            verified_run_observation,
            deadline,
            Arc::clone(&execution.request_plan),
            Arc::clone(&execution.planner_policy),
            execution.classifier.as_ref().map(Arc::clone),
            Arc::clone(&execution.candidates),
            Arc::clone(&execution.pricing_bindings),
            core,
        )
        .with_web_search_allowed(self.run.as_ref().is_none_or(|run| run.network_allowed())))
    }

    pub fn request_started_at(&self) -> Instant {
        self.request_started_at
    }

    pub fn selector_deadline(&self) -> Instant {
        self.selector_deadline
    }
}

fn is_run_bearer(authorization: &str) -> bool {
    authorization
        .strip_prefix("Bearer ")
        .is_some_and(|token| token.starts_with("hr_run_model_"))
}

fn map_run_error(error: RunRequestAuthorityError) -> DispatchError {
    match error {
        RunRequestAuthorityError::Denied => DispatchError::Unauthorized,
        RunRequestAuthorityError::Unavailable => DispatchError::PublicationUnavailable,
    }
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("gateway publication is unavailable")]
    PublicationUnavailable,
    #[error("gateway request grant is missing or invalid")]
    Unauthorized,
    #[error("AgentPlan is unavailable for this grant")]
    AgentPlanNotAvailable,
    #[error("model is not granted to this agent connection")]
    AgentModelNotGranted,
    #[error("AgentPlan does not support this ingress protocol")]
    AgentPlanProtocolUnsupported,
    #[error(transparent)]
    Selector(#[from] SelectorError),
    #[error(transparent)]
    CorePlan(#[from] PlanError),
    #[error("request deadline overflowed")]
    InvalidDeadline,
    #[error("request exceeded its frozen logical deadline")]
    RequestDeadlineExceeded,
}

impl DispatchError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::PublicationUnavailable => "GATEWAY_PUBLICATION_UNAVAILABLE",
            Self::Unauthorized => "GATEWAY_GRANT_UNAUTHORIZED",
            Self::AgentPlanNotAvailable => "AGENT_PLAN_NOT_AVAILABLE",
            Self::AgentModelNotGranted => "AGENT_MODEL_NOT_GRANTED",
            Self::AgentPlanProtocolUnsupported => "AGENT_PROTOCOL_UNSUPPORTED",
            Self::Selector(SelectorError::LimitExceeded(_)) => "MODEL_SELECTOR_LIMIT_EXCEEDED",
            Self::Selector(_) => "MODEL_SELECTOR_INVALID",
            Self::RequestDeadlineExceeded => "REQUEST_DEADLINE_EXCEEDED",
            Self::CorePlan(_) | Self::InvalidDeadline => "REQUEST_PLAN_BIND_FAILED",
        }
    }

    pub const fn status(&self) -> StatusCode {
        match self {
            Self::PublicationUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::AgentPlanNotAvailable | Self::AgentModelNotGranted => StatusCode::NOT_FOUND,
            Self::AgentPlanProtocolUnsupported => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Selector(_) => StatusCode::BAD_REQUEST,
            Self::RequestDeadlineExceeded => StatusCode::REQUEST_TIMEOUT,
            Self::CorePlan(_) | Self::InvalidDeadline => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
