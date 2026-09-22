use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use thiserror::Error;

use crate::ports::{
    CasOutcome, CredentialLeaseRequest, ExecutionScope, HeaderSecretLeaseRequest,
    ProbeLeaseOutcome, RuntimeStateEntry, RuntimeStateKey,
};
use crate::server::core_runtime::adapters;
use crate::server::core_runtime::model_ir::{ContentPart, ModelIrError, ModelRequestIRV1};
use crate::server::core_runtime::profiles::{
    CandidateProtocolProfile, CapabilityError, ContextProjectionError, CostClassV1,
    ExclusionReasonCodeV1, PLANNER_INPUT_SCHEMA, PlannerCandidateFactsV1, PlannerInputV1,
};
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

use crate::server::publication::{GatewayPublicationInstaller, PublishedGatewayPublication};

/// Live aggregate publication source. The durable installer is the local P0
/// adapter; remote feeds can implement the same pin operation later.
pub trait RuntimePublicationFeed: Send + Sync {
    fn pin(&self) -> Option<Arc<PublishedGatewayPublication>>;
}

/// Request-owned source of immutable Planner facts. Implementations may read
/// only compiled/static configuration; runtime state, credentials and network
/// observations are deliberately absent from this interface.
pub trait PlannerInputAuthority: Send + Sync {
    fn build_input(
        &self,
        request: ModelRequestIRV1,
        authorized: &AuthorizedRequestPlan,
    ) -> Result<PlannerInputV1, PortError>;
}

#[derive(Default)]
pub struct PublicationPlannerInputAuthority;

impl PublicationPlannerInputAuthority {
    pub(crate) fn bind_request_reasoning(
        document: &serde_json::Value,
        ingress: IngressProtocol,
        authorized: &AuthorizedRequestPlan,
    ) -> Result<Option<hiroute_domain::CanonicalDigest>, PortError> {
        use crate::server::core_runtime::profiles::PlannerRouteIdentityV2;

        if !matches!(
            authorized.planner_policy().identity,
            PlannerRouteIdentityV2::Fixed { .. }
        ) {
            return Ok(None);
        }
        let native = match ingress {
            IngressProtocol::Responses => document.get("reasoning"),
            IngressProtocol::ChatCompletions => document.get("reasoning_effort"),
            IngressProtocol::Messages => (document.get("thinking").is_some()
                || document.get("output_config").is_some())
            .then_some(document),
        };
        let Some(native) = native else {
            return Ok(None);
        };
        let bindings = authorized
            .core_binding()
            .candidate_bindings()
            .map_err(|_| PortError::Rejected)?;
        let [binding] = bindings else {
            return Err(PortError::Rejected);
        };
        let attempt = authorized
            .core_binding()
            .resolve_attempt(*binding)
            .map_err(|_| PortError::Rejected)?;
        let [config_id] = attempt.plan().config_cell_ids.as_ref() else {
            return Err(PortError::Rejected);
        };
        let configs = attempt
            .acquire_attempt_configs()
            .map_err(|_| PortError::Rejected)?;
        let config = configs.value(*config_id).ok_or(PortError::Rejected)?;
        let candidate: crate::server::publication::CandidateBindingV1 =
            serde_json::from_slice(&config.bytes).map_err(|_| PortError::Rejected)?;
        let mut matching = candidate
            .protocol_profiles
            .iter()
            .filter(|profile| profile.ingress_protocol == domain_protocol(ingress));
        let canonical = matching.next().ok_or(PortError::Rejected)?;
        if matching.next().is_some() {
            return Err(PortError::Rejected);
        }
        let profile: CandidateProtocolProfile = serde_json::from_value(
            serde_json::to_value(canonical).map_err(|_| PortError::Rejected)?,
        )
        .map_err(|_| PortError::Rejected)?;
        let selected = profile
            .select_native_reasoning(native, ingress)
            .map_err(|_| PortError::InvalidReasoningControl)?;
        let selected = selected
            .selected_reasoning()
            .map_err(|_| PortError::InvalidReasoningControl)?;
        Ok(Some(
            hiroute_domain::CanonicalDigest::of(selected).map_err(|_| PortError::Rejected)?,
        ))
    }
}

impl PlannerInputAuthority for PublicationPlannerInputAuthority {
    fn build_input(
        &self,
        request: ModelRequestIRV1,
        authorized: &AuthorizedRequestPlan,
    ) -> Result<PlannerInputV1, PortError> {
        build_planner_input(request, authorized)
    }
}

/// Retired startup sidecar authority. Executable planner facts must come from
/// the sealed publication candidate envelope, so opening this adapter always
/// fails closed.
pub struct FilePlannerInputAuthority;

impl FilePlannerInputAuthority {
    pub fn open(_path: &Path) -> Result<Self, PortError> {
        // Sidecar planner facts are no longer an executable authority. Startup
        // composition must install the sealed publication candidate envelope.
        Err(PortError::Rejected)
    }
}

impl PlannerInputAuthority for FilePlannerInputAuthority {
    fn build_input(
        &self,
        request: ModelRequestIRV1,
        authorized: &AuthorizedRequestPlan,
    ) -> Result<PlannerInputV1, PortError> {
        build_planner_input(request, authorized)
    }
}

fn build_planner_input(
    request: ModelRequestIRV1,
    authorized: &AuthorizedRequestPlan,
) -> Result<PlannerInputV1, PortError> {
    let authorities = authorized
        .candidates()
        .iter()
        .map(|candidate| (candidate.binding_local_id, candidate))
        .collect::<BTreeMap<_, _>>();
    let mut facts = Vec::new();
    for binding in authorized
        .core_binding()
        .candidate_bindings()
        .map_err(|_| PortError::Rejected)?
    {
        let attempt = authorized
            .core_binding()
            .resolve_attempt(*binding)
            .map_err(|_| PortError::Rejected)?;
        if attempt.plan().config_cell_ids.len() != 1 {
            return Err(PortError::Rejected);
        }
        let config_id = attempt.plan().config_cell_ids[0];
        let configs = attempt
            .acquire_attempt_configs()
            .map_err(|_| PortError::Rejected)?;
        let config = configs.value(config_id).ok_or(PortError::Rejected)?;
        if config.generation.0 != authorized.publication_revision() {
            return Err(PortError::Rejected);
        }
        let candidate: crate::server::publication::CandidateBindingV1 =
            serde_json::from_slice(&config.bytes).map_err(|_| PortError::Rejected)?;
        let authority = authorities
            .get(&binding.local_id())
            .ok_or(PortError::Rejected)?;
        if candidate.local_id != binding.local_id()
            || candidate.stable_target_key != attempt.plan().stable_target_key.as_str()
            || candidate.adapter_id != attempt.plan().adapter_id.as_str()
            || candidate.credential_refs.len() != attempt.plan().credential_refs.len()
            || candidate
                .credential_refs
                .iter()
                .zip(attempt.plan().credential_refs.iter())
                .any(|(candidate, plan)| candidate != plan.as_str())
            || candidate.credential_refs.len() != authority.credential_refs.len()
            || candidate
                .credential_refs
                .iter()
                .zip(authority.credential_refs.iter())
                .any(|(candidate, authority)| candidate != authority.as_ref())
            || candidate.operational_target.uri() != authority.endpoint.as_ref()
        {
            return Err(PortError::Rejected);
        }
        let ingress = domain_protocol(request.ingress_protocol);
        let mut matching_profiles = candidate
            .protocol_profiles
            .iter()
            .filter(|profile| profile.ingress_protocol == ingress);
        let canonical_profile = matching_profiles.next().ok_or(PortError::Rejected)?;
        if matching_profiles.next().is_some() {
            return Err(PortError::Rejected);
        }
        let profile: CandidateProtocolProfile = serde_json::from_value(
            serde_json::to_value(canonical_profile).map_err(|_| PortError::Rejected)?,
        )
        .map_err(|_| PortError::Rejected)?;
        let profile = profile
            .for_request_reasoning(&request)
            .map_err(|_| PortError::InvalidReasoningControl)?;
        let projection = project_candidate_facts(&request, &profile)?;
        let mut candidate = PlannerCandidateFactsV1::seal(
            candidate.stable_target_key.clone(),
            candidate.stable_target_key,
            profile,
            projection.target_serialized_bytes,
            None,
            CostClassV1::Free,
            None,
        )
        .map_err(|_| PortError::Rejected)?;
        candidate.statically_enabled = true;
        candidate.request_projection_exclusion = projection.exclusion;
        facts.push(candidate);
    }
    if facts.len() != authorities.len() {
        return Err(PortError::Rejected);
    }
    let policy = authorized.planner_policy().as_ref().clone();
    Ok(PlannerInputV1 {
        schema_version: PLANNER_INPUT_SCHEMA.into(),
        request,
        correlated_branch: None,
        classification_decision: None,
        classification_facts: None,
        context_hold: None,
        policy,
        candidates: facts,
    })
}

fn domain_protocol(protocol: IngressProtocol) -> hiroute_domain::UpstreamProtocol {
    match protocol {
        IngressProtocol::Responses => hiroute_domain::UpstreamProtocol::Responses,
        IngressProtocol::ChatCompletions => hiroute_domain::UpstreamProtocol::ChatCompletions,
        IngressProtocol::Messages => hiroute_domain::UpstreamProtocol::Messages,
    }
}

struct CandidateProjectionFacts {
    target_serialized_bytes: u64,
    exclusion: Option<ExclusionReasonCodeV1>,
}

fn project_candidate_facts(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
) -> Result<CandidateProjectionFacts, PortError> {
    let tool_interface_unrepresentable = request.ingress_protocol == IngressProtocol::Responses
        && match profile.capability.upstream_protocol {
            IngressProtocol::Responses => false,
            IngressProtocol::ChatCompletions => {
                adapters::ChatToolProjection::for_request(request).is_err()
            }
            IngressProtocol::Messages => {
                !request.tool_namespaces.is_empty()
                    || request
                        .tools
                        .iter()
                        .chain(
                            request
                                .tool_namespaces
                                .iter()
                                .flat_map(|namespace| namespace.tools.iter()),
                        )
                        .any(|tool| {
                            tool.kind == crate::server::core_runtime::model_ir::ToolKindV1::Custom
                        })
            }
        };
    if tool_interface_unrepresentable {
        return Ok(CandidateProjectionFacts {
            // Planner consumes the typed exclusion before context sizing.
            target_serialized_bytes: 1,
            exclusion: Some(ExclusionReasonCodeV1::ToolInterfaceUnsupported),
        });
    }

    match project_candidate_facts_template(request, profile) {
        Ok(projected) => Ok(CandidateProjectionFacts {
            target_serialized_bytes: u64::try_from(projected.wire_len)
                .map_err(|_| PortError::Rejected)?,
            exclusion: None,
        }),
        Err(
            adapters::ProtocolAdapterError::ClientUnrepresentable(_)
            | adapters::ProtocolAdapterError::Capability(
                CapabilityError::InitialInstructionsUnsupported
                | CapabilityError::MidConversationInstructionsUnsupported,
            ),
        ) => {
            Ok(CandidateProjectionFacts {
                // The request is valid, but this candidate's protocol path
                // cannot carry it. Preserve the candidate so Planner records
                // the typed exclusion and can select a later plan member.
                target_serialized_bytes: 1,
                exclusion: Some(ExclusionReasonCodeV1::ProtocolPathUnavailable),
            })
        }
        Err(adapters::ProtocolAdapterError::Context(
            ContextProjectionError::InputTooLarge { .. }
            | ContextProjectionError::TotalTooLarge { .. },
        )) => Ok(CandidateProjectionFacts {
            // A valid request may exceed one candidate's limit. Let Planner
            // exclude that candidate rather than failing the whole input.
            target_serialized_bytes: 1,
            exclusion: Some(ExclusionReasonCodeV1::ContextTooLarge),
        }),
        Err(_) => Err(PortError::Rejected),
    }
}

fn project_candidate_facts_template(
    request: &ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
) -> Result<adapters::PreparedNativeTemplate, adapters::ProtocolAdapterError> {
    match adapters::project_candidate_request_template(request, profile) {
        Ok(projected) => Ok(projected),
        Err(
            error @ adapters::ProtocolAdapterError::ModelIr(
                ModelIrError::ProviderStateNotPortable | ModelIrError::ToolIdBindingRequired(_),
            ),
        ) => {
            let exact_owner = profile.exact_provider_path()?;
            // Size a non-executable candidate without granting it continuation
            // authority. The original Tool IDs and opaque-state owners stay in
            // Planner input; its state gate excludes mismatching candidates.
            let mut sizing_request = request.clone();
            let mut mismatched = false;
            for owner in sizing_request
                .tool_id_map
                .iter_mut()
                .map(|entry| &mut entry.owner)
                .chain(
                    sizing_request
                        .provider_state
                        .iter_mut()
                        .map(|state| &mut state.owner),
                )
                .chain(
                    sizing_request
                        .instructions
                        .iter_mut()
                        .flat_map(|instruction| instruction.content.iter_mut())
                        .chain(
                            sizing_request
                                .messages
                                .iter_mut()
                                .flat_map(|message| message.content.iter_mut()),
                        )
                        .filter_map(|part| match part {
                            ContentPart::ProviderState { state } => Some(&mut state.owner),
                            _ => None,
                        }),
                )
            {
                if *owner != exact_owner {
                    mismatched = true;
                    *owner = exact_owner.clone();
                }
            }
            if !mismatched {
                return Err(error);
            }
            adapters::project_candidate_request_template(&sizing_request, profile)
        }
        Err(error) => Err(error),
    }
}

impl RuntimePublicationFeed for GatewayPublicationInstaller {
    fn pin(&self) -> Option<Arc<PublishedGatewayPublication>> {
        self.active()
    }
}

/// Provider credential authority. PROCESS-22003 composes this port but never
/// invokes it; the authorized request plan is the required input for 22004.
#[async_trait]
pub trait CredentialResolver: Send + Sync {
    fn acquire(&self, credential_ref: &str) -> Result<CredentialLease, PortError>;

    async fn lease_exact(
        &self,
        _request: CredentialLeaseRequest<'_>,
        _scope: &ExecutionScope,
    ) -> Result<Option<crate::ports::CredentialLease>, PortError> {
        Err(PortError::Unavailable("CredentialResolver"))
    }

    async fn lease_header_secret(
        &self,
        _request: HeaderSecretLeaseRequest<'_>,
        _scope: &ExecutionScope,
    ) -> Result<Option<crate::ports::CredentialLease>, PortError> {
        Err(PortError::Unavailable("HeaderSecretResolver"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialLease {
    pub credential_ref: Arc<str>,
    pub generation: u64,
}

#[async_trait]
pub trait RuntimeStateStore: Send + Sync {
    async fn read_exact(
        &self,
        _key: &RuntimeStateKey,
        _scope: &ExecutionScope,
    ) -> Result<RuntimeStateEntry, PortError> {
        Err(PortError::Unavailable("RuntimeStateStore"))
    }

    /// Atomically applies `next` only when the stored generation equals
    /// `expected_generation`. `next.generation` must be exactly
    /// `expected_generation + 1`; an adapter must preserve and return that
    /// generation, or leave storage unchanged and report `Conflict`.
    async fn compare_and_swap_exact(
        &self,
        _key: &RuntimeStateKey,
        _expected_generation: u64,
        _next: RuntimeStateEntry,
        _scope: &ExecutionScope,
    ) -> Result<CasOutcome, PortError> {
        Err(PortError::Unavailable("RuntimeStateStore"))
    }

    async fn acquire_probe_lease_exact(
        &self,
        _key: &RuntimeStateKey,
        _expected_generation: u64,
        _now: std::time::Instant,
        _lease_duration: std::time::Duration,
        _scope: &ExecutionScope,
    ) -> Result<ProbeLeaseOutcome, PortError> {
        Err(PortError::Unavailable("RuntimeStateStore"))
    }
}
pub trait LifecycleTelemetrySink: Send + Sync {}
pub trait ExecutionFactSink: Send + Sync {}
pub trait ConversationContentSink: Send + Sync {}

pub struct ProductionPorts {
    pub publications: Arc<dyn RuntimePublicationFeed>,
    pub credentials: Arc<dyn CredentialResolver>,
    pub runtime_state: Arc<dyn RuntimeStateStore>,
    pub lifecycle: Arc<dyn LifecycleTelemetrySink>,
    pub execution_facts: Arc<dyn ExecutionFactSink>,
    pub conversation_content: Arc<dyn ConversationContentSink>,
}

impl ProductionPorts {
    pub fn fail_closed(publications: Arc<GatewayPublicationInstaller>) -> Self {
        let unavailable = Arc::new(UnavailableAdapter);
        Self {
            publications,
            credentials: unavailable.clone(),
            runtime_state: unavailable.clone(),
            lifecycle: unavailable.clone(),
            execution_facts: unavailable.clone(),
            conversation_content: unavailable,
        }
    }
}

#[derive(Debug)]
struct UnavailableAdapter;

impl CredentialResolver for UnavailableAdapter {
    fn acquire(&self, _credential_ref: &str) -> Result<CredentialLease, PortError> {
        Err(PortError::Unavailable("CredentialResolver"))
    }
}

impl RuntimeStateStore for UnavailableAdapter {}
impl LifecycleTelemetrySink for UnavailableAdapter {}
impl ExecutionFactSink for UnavailableAdapter {}
impl ConversationContentSink for UnavailableAdapter {}

#[async_trait]
impl RuntimeStateStore for crate::ports::InMemoryRuntimeStateStore {
    async fn read_exact(
        &self,
        key: &RuntimeStateKey,
        scope: &ExecutionScope,
    ) -> Result<RuntimeStateEntry, PortError> {
        crate::ports::RuntimeStateStore::read(self, key, scope)
            .await
            .map_err(|_| PortError::Rejected)
    }

    async fn compare_and_swap_exact(
        &self,
        key: &RuntimeStateKey,
        expected_generation: u64,
        next: RuntimeStateEntry,
        scope: &ExecutionScope,
    ) -> Result<CasOutcome, PortError> {
        crate::ports::RuntimeStateStore::compare_and_swap(
            self,
            key,
            expected_generation,
            next,
            scope,
        )
        .await
        .map_err(|_| PortError::Rejected)
    }

    async fn acquire_probe_lease_exact(
        &self,
        key: &RuntimeStateKey,
        expected_generation: u64,
        now: std::time::Instant,
        lease_duration: std::time::Duration,
        scope: &ExecutionScope,
    ) -> Result<ProbeLeaseOutcome, PortError> {
        crate::ports::RuntimeStateStore::acquire_probe_lease(
            self,
            key,
            expected_generation,
            now,
            lease_duration,
            scope,
        )
        .await
        .map_err(|_| PortError::Rejected)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialFile {
    schema_version: String,
    /// Non-secret manifest only. Each exact credential reference points to a
    /// separate lease-set file that is not opened until that candidate wins.
    credentials: std::collections::BTreeMap<String, PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialLeaseFile {
    schema_version: String,
    credential_ref: String,
    keys: Vec<CredentialFileEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialFileEntry {
    key_id: String,
    generation: u64,
    authorization: String,
}

/// Startup credential authority used by the standalone binary. Startup reads
/// only a non-secret reference manifest. A lease opens exactly one selected
/// credential file, so neither the runtime nor this adapter eagerly parses an
/// unrelated secret pool.
pub struct FileCredentialResolver {
    credentials: std::collections::BTreeMap<Arc<str>, PathBuf>,
}

impl FileCredentialResolver {
    pub fn open(path: &Path) -> Result<Self, PortError> {
        let bytes = std::fs::read(path).map_err(|_| PortError::Rejected)?;
        let file: CredentialFile =
            serde_json::from_slice(&bytes).map_err(|_| PortError::Rejected)?;
        if file.schema_version != "hiroute.gateway.credentials/v1" || file.credentials.is_empty() {
            return Err(PortError::Rejected);
        }
        let parent = path.parent().ok_or(PortError::Rejected)?;
        let credentials = file
            .credentials
            .into_iter()
            .map(|(credential_ref, lease_path)| {
                if credential_ref.trim().is_empty()
                    || lease_path.as_os_str().is_empty()
                    || lease_path
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return Err(PortError::Rejected);
                }
                let lease_path = if lease_path.is_absolute() {
                    lease_path
                } else {
                    parent.join(lease_path)
                };
                Ok((Arc::from(credential_ref), lease_path))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { credentials })
    }

    fn exact_leases(
        &self,
        credential_ref: &str,
    ) -> Result<Vec<crate::ports::CredentialLease>, PortError> {
        let path = self
            .credentials
            .get(credential_ref)
            .ok_or(PortError::Rejected)?;
        let bytes = std::fs::read(path).map_err(|_| PortError::Rejected)?;
        Self::decode_exact_leases(credential_ref, &bytes)
    }

    fn decode_exact_leases(
        credential_ref: &str,
        bytes: &[u8],
    ) -> Result<Vec<crate::ports::CredentialLease>, PortError> {
        let file: CredentialLeaseFile =
            serde_json::from_slice(bytes).map_err(|_| PortError::Rejected)?;
        if file.schema_version != "hiroute.gateway.credential-leases/v1"
            || file.credential_ref != credential_ref
            || file.keys.is_empty()
        {
            return Err(PortError::Rejected);
        }
        file.keys
            .into_iter()
            .map(|entry| {
                crate::ports::CredentialLease::new(
                    credential_ref,
                    entry.key_id,
                    entry.generation,
                    entry.authorization,
                )
                .map_err(|_| PortError::Rejected)
            })
            .collect()
    }
}

#[async_trait]
impl CredentialResolver for FileCredentialResolver {
    fn acquire(&self, credential_ref: &str) -> Result<CredentialLease, PortError> {
        let lease = self
            .exact_leases(credential_ref)?
            .into_iter()
            .next()
            .ok_or(PortError::Rejected)?;
        Ok(CredentialLease {
            credential_ref: Arc::from(credential_ref),
            generation: lease.generation(),
        })
    }

    async fn lease_exact(
        &self,
        request: CredentialLeaseRequest<'_>,
        _scope: &ExecutionScope,
    ) -> Result<Option<crate::ports::CredentialLease>, PortError> {
        let target = hiroute_domain::GatewayOperationalTargetV1::RegisteredHttps {
            uri: request.operational_target.to_owned(),
        };
        if request.connector_runtime != hiroute_domain::ConnectorRuntimeKind::BuiltinNative
            || request.runtime_epoch.is_some()
            || request.target_epoch.is_some()
            || request.stable_binding_id.trim().is_empty()
            || request.credential_ref.trim().is_empty()
            || request
                .credential_destination_ref
                .strip_prefix("connection-option/")
                .is_none_or(str::is_empty)
            || request.connector_id.trim().is_empty()
            || request.upstream_model_id.trim().is_empty()
            || request.native_transport_model.trim().is_empty()
            || request.upstream_model_id != request.native_transport_model
            || request.logical_endpoint != request.operational_target
            || !target.validate_for(request.connector_runtime, request.operational_target)
            || target.request_path() != Some(request.request_path)
            || !matches!(
                hiroute_domain::CanonicalDigest::of(&target),
                Ok(digest) if digest.as_str() == request.operational_target_digest
            )
            || hiroute_domain::CanonicalDigest::parse(request.protocol_profile_digest).is_err()
            || request.authentication
                != &crate::server::core_runtime::profiles::AuthenticationSemantics::Bearer
        {
            return Err(PortError::Rejected);
        }
        let path = self
            .credentials
            .get(request.credential_ref)
            .ok_or(PortError::Rejected)?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| PortError::Rejected)?;
        Ok(Self::decode_exact_leases(request.credential_ref, &bytes)?
            .into_iter()
            .find(|lease| {
                request
                    .excluded_key_ids
                    .iter()
                    .all(|excluded| excluded.as_ref() != lease.key_id())
            }))
    }

    async fn lease_header_secret(
        &self,
        request: HeaderSecretLeaseRequest<'_>,
        scope: &ExecutionScope,
    ) -> Result<Option<crate::ports::CredentialLease>, PortError> {
        scope.ensure_active().map_err(|_| PortError::Rejected)?;
        let header_name = http::HeaderName::from_bytes(request.header_name.as_bytes())
            .map_err(|_| PortError::Rejected)?;
        let path = self
            .credentials
            .get(request.secret_ref)
            .ok_or(PortError::Rejected)?;
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_| PortError::Rejected)?;
        let file: CredentialLeaseFile =
            serde_json::from_slice(&bytes).map_err(|_| PortError::Rejected)?;
        if file.schema_version != "hiroute.gateway.credential-leases/v1"
            || file.credential_ref != request.secret_ref
            || file.keys.len() != 1
        {
            return Err(PortError::Rejected);
        }
        let entry = file.keys.into_iter().next().ok_or(PortError::Rejected)?;
        let lease = crate::ports::CredentialLease::new_header(
            request.secret_ref,
            entry.key_id,
            entry.generation,
            header_name,
            entry.authorization,
        )
        .map(Some)
        .map_err(|_| PortError::Rejected)?;
        scope.ensure_active().map_err(|_| PortError::Rejected)?;
        Ok(lease)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PortError {
    #[error("production port is unavailable: {0}")]
    Unavailable(&'static str),
    #[error("production port rejected the operation")]
    Rejected,
    #[error("requested fixed-model reasoning cannot be represented")]
    InvalidReasoningControl,
}

#[cfg(test)]
mod sizing_tests {
    use super::*;
    use crate::server::core_runtime::profiles::{
        Fidelity, NativeProviderStateEmission, StateAffinity, fixed_reasoning,
    };
    use serde_json::json;

    #[test]
    fn non_owner_sizing_preserves_request_and_rejects_unrelated_errors() {
        let mut owner = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "luna",
            fixed_reasoning("fixed"),
        );
        owner.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
        owner.capability.request.provider_state = Fidelity::Exact;
        owner.capability.request.state_affinity = StateAffinity::ExactOwner;
        owner.capability.response.provider_state = Fidelity::Exact;
        owner.capability.response.state_affinity = StateAffinity::ExactOwner;
        let mut other = owner.clone();
        other.capability.native_model = "terra".into();
        let mut request = adapters::decode_ingress_request_with_bindings(
            IngressProtocol::Responses,
            &json!({"model":"route","input":[{"type":"reasoning","summary":[],"encrypted_content":"state"}]}),
            &adapters::IngressRequestBindings {
                provider_state_owner: Some(owner.exact_provider_path().unwrap()),
                tool_id_map: Vec::new(),
            },
        ).unwrap();
        let before = serde_json::to_value(&request).unwrap();
        assert!(project_candidate_facts_template(&request, &other).is_ok());
        assert_eq!(serde_json::to_value(&request).unwrap(), before);
        assert!(adapters::project_candidate_request_template(&request, &other).is_err());
        request
            .responses_reasoning_history
            .get_mut(&0)
            .unwrap()
            .encrypted_content =
            crate::server::core_runtime::model_ir::ResponsesReasoningEncryptedContentV1::Absent;
        assert!(project_candidate_facts_template(&request, &other).is_err());
    }

    #[test]
    fn unsupported_responses_instruction_roles_exclude_messages_without_failing_planner_input() {
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Messages,
            "glm-5.3",
            fixed_reasoning("fixed"),
        );
        for input in [
            json!([
                {"type":"message","role":"developer","content":[{"type":"input_text","text":"bounds"}]},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"task"}]}
            ]),
            json!([
                {"type":"message","role":"user","content":[{"type":"input_text","text":"task"}]},
                {"type":"message","role":"developer","content":[{"type":"input_text","text":"late bounds"}]}
            ]),
        ] {
            let request = adapters::decode_ingress_request(
                IngressProtocol::Responses,
                &json!({"model":"route","input":input}),
            )
            .unwrap();
            let facts = project_candidate_facts(&request, &profile).unwrap();
            assert_eq!(
                facts.exclusion,
                Some(ExclusionReasonCodeV1::ProtocolPathUnavailable)
            );
        }
    }

    #[test]
    fn oversized_codex_instructions_and_tools_exclude_candidate_not_planner_input() {
        let mut profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "small-context-model",
            fixed_reasoning("fixed"),
        );
        profile.capability.context.max_input_tokens =
            crate::server::core_runtime::profiles::CriticalFact::Exact(4096);

        let cases = [
            json!({
                "model": "route",
                "instructions": "i".repeat(5000),
                "input": "hello",
            }),
            json!({
                "model": "route",
                "input": "hello",
                "tools": [{
                    "type": "function",
                    "name": "shell",
                    "description": "d".repeat(5000),
                    "parameters": {"type": "object", "properties": {}},
                }],
            }),
        ];
        for body in cases {
            let request = adapters::decode_ingress_request(IngressProtocol::Responses, &body)
                .expect("valid Codex-shaped request");
            let facts = project_candidate_facts(&request, &profile)
                .expect("candidate-specific context mismatch must not fail Planner input");
            assert_eq!(
                facts.exclusion,
                Some(ExclusionReasonCodeV1::ContextTooLarge)
            );
        }
    }
}
