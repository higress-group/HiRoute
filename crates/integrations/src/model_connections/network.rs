//! Explicit check routes and safe evidence; no transport reads ambient proxy state.
use super::*;
use hiroute_application_api::{
    ModelConnectionCheckViewV2, NativeNetworkCheckEvidenceV1, NativeNetworkEditV1,
    NativeNetworkFailurePhaseV1 as Phase, NativeNetworkFailureV1, NativeNetworkRouteKindV1,
    NativeNetworkRouteViewV1,
};
use hiroute_domain::{
    NativeNetworkDestinationV1, NativeNetworkPolicyV1, NativeNetworkSnapshotV1,
    ResolvedNativeNetworkRouteV1,
};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct NativeModelConnectionNetworkContextV1 {
    pub snapshot: Arc<NativeNetworkSnapshotV1>,
    pub edit: NativeNetworkEditV1,
}

/// Pure preview: origin and opaque service epoch only; no environment values, DNS or request.
pub fn native_network_route_view(
    snapshot: &NativeNetworkSnapshotV1,
    policy: &NativeNetworkPolicyV1,
    url: &str,
) -> Result<NativeNetworkRouteViewV1, NativeModelConnectionErrorV1> {
    let destination = NativeNetworkDestinationV1::from_url(url)
        .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
    let host = if destination.host.contains(':') {
        format!("[{}]", destination.host)
    } else {
        destination.host.clone()
    };
    let origin = format!("{}://{}:{}", destination.scheme, host, destination.port);
    let epoch = matches!(policy, NativeNetworkPolicyV1::Inherit)
        .then(|| snapshot.environment_epoch().unwrap_or("unavailable"));
    let state_revision =
        CanonicalDigest::of(&("native-network-route-view/v1", policy, epoch, &origin))
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
    let mut view = NativeNetworkRouteViewV1 {
        mode: policy.mode(),
        route: None,
        reason: None,
        destination: origin,
        state_revision,
        failure: None,
    };
    match snapshot.resolve_policy(policy, &destination) {
        Ok(route) => {
            view.route = Some(if route.proxy.is_some() {
                NativeNetworkRouteKindV1::Proxy
            } else {
                NativeNetworkRouteKindV1::Direct
            });
            view.reason = Some(route.reason);
        }
        Err(error) => {
            view.failure = Some(NativeNetworkFailureV1 {
                phase: Phase::RouteUnavailable,
                code: error.to_string(),
                retryable: true,
            })
        }
    }
    Ok(view)
}

/// A saved consumer also observes policy edits that return to an earlier value.
pub fn native_network_route_view_at_revision(
    snapshot: &NativeNetworkSnapshotV1,
    policy: &NativeNetworkPolicyV1,
    url: &str,
    revision: u64,
) -> Result<NativeNetworkRouteViewV1, NativeModelConnectionErrorV1> {
    let mut view = native_network_route_view(snapshot, policy, url)?;
    view.state_revision = CanonicalDigest::of(&(
        "native-source-network-route/v1",
        &view.state_revision,
        revision,
    ))
    .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
    Ok(view)
}

impl<P: ComputeCandidatePort, T: ModelDirectoryTransportV1> NativeModelConnectionServiceV1<P, T> {
    pub fn check_with_network(
        &self,
        draft: NativeModelConnectionDraftV1,
        credential: NativeModelConnectionCredentialV1<'_>,
        cancellation: &ModelConnectionProbeCancellationV1,
        context: NativeModelConnectionNetworkContextV1,
    ) -> Result<ModelConnectionCheckViewV2, NativeModelConnectionErrorV1> {
        let policy = context
            .edit
            .policy
            .normalized()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
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
        let url = if draft.inference_model_id.is_some() || target.inventory_path.is_none() {
            target.request_url()
        } else {
            target.inventory_url(None)
        };
        let route = native_network_route_view(&context.snapshot, &policy, &url)?;
        let identity = CanonicalDigest::of(&(
            "native-network-check/v1",
            &policy,
            context.edit.expected_network_revision,
            &route.state_revision,
        ))
        .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        let transport = RoutedTransport {
            inner: &self.transport,
            snapshot: &context.snapshot,
            policy: &policy,
            failure: Mutex::new(route.failure.clone()),
        };
        let check =
            self.check_using(draft, credential, cancellation, &transport, Some(&identity))?;
        let mut failure = transport
            .failure
            .into_inner()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        if failure.is_none() {
            let phase = if check.inference
                == hiroute_application_api::ModelConnectionInferenceStatusV1::Failed
            {
                Some(Phase::Inference)
            } else if matches!(
                check.directory,
                hiroute_application_api::ModelConnectionDirectoryStatusV1::Invalid
                    | hiroute_application_api::ModelConnectionDirectoryStatusV1::Unavailable
            ) {
                Some(Phase::Directory)
            } else {
                None
            };
            failure = phase.map(|phase| NativeNetworkFailureV1 {
                phase,
                code: phase_code(phase).into(),
                retryable: true,
            });
        }
        let network = NativeNetworkCheckEvidenceV1 {
            policy,
            expected_network_revision: context.edit.expected_network_revision,
            route,
        };
        let candidate = &check.candidate.candidate;
        let mut checks = self
            .network_checks
            .lock()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?;
        if checks
            .get(&candidate.candidate_ref)
            .is_none_or(|(revision, _)| *revision <= candidate.candidate_revision)
        {
            checks.insert(
                candidate.candidate_ref.clone(),
                (candidate.candidate_revision, network.clone()),
            );
        }
        Ok(ModelConnectionCheckViewV2 {
            check,
            network,
            failure,
        })
    }

    /// Save authority uses the exact current backend candidate revision, never UI supplied evidence.
    pub fn network_check_evidence(
        &self,
        candidate: &ComputeCandidateRefV2,
    ) -> Result<NativeNetworkCheckEvidenceV1, NativeModelConnectionErrorV1> {
        self.network_checks
            .lock()
            .map_err(|_| NativeModelConnectionErrorV1::InvalidDraft)?
            .get(&candidate.candidate_ref)
            .filter(|(r, _)| *r == candidate.candidate_revision)
            .map(|(_, e)| e.clone())
            .ok_or(NativeModelConnectionErrorV1::InvalidDraft)
    }
}

struct RoutedTransport<'a, T> {
    inner: &'a T,
    snapshot: &'a NativeNetworkSnapshotV1,
    policy: &'a NativeNetworkPolicyV1,
    failure: Mutex<Option<NativeNetworkFailureV1>>,
}
impl<T> RoutedTransport<'_, T> {
    fn resolve(
        &self,
        url: &str,
    ) -> Result<ResolvedNativeNetworkRouteV1, ModelDirectoryTransportErrorV1> {
        let route = NativeNetworkDestinationV1::from_url(url)
            .and_then(|d| self.snapshot.resolve_policy(self.policy, &d));
        route.map_err(|_| ModelDirectoryTransportErrorV1::Network(Phase::RouteUnavailable))
    }
    fn observed(
        &self,
        result: Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1>,
        stage: Phase,
        proxied: bool,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        let phase = match &result {
            Err(ModelDirectoryTransportErrorV1::Network(phase)) => Some(*phase),
            Err(_) => Some(Phase::TransportUnknown),
            Ok(response) if proxied && response.status == 407 => Some(Phase::ProxyAuth),
            Ok(response) if matches!(response.status, 401 | 403) => Some(Phase::UpstreamAuth),
            Ok(response) if !(200..300).contains(&response.status) || response.truncated => {
                Some(stage)
            }
            _ => None,
        };
        if let Some(phase) = phase
            && let Ok(mut failure) = self.failure.lock()
        {
            *failure = Some(NativeNetworkFailureV1 {
                phase,
                code: phase_code(phase).into(),
                retryable: true,
            });
        }
        if matches!(phase, Some(Phase::ProxyAuth)) {
            return Err(ModelDirectoryTransportErrorV1::Network(Phase::ProxyAuth));
        }
        result
    }
}
impl<T: ModelDirectoryTransportV1> ModelDirectoryTransportV1 for RoutedTransport<'_, T> {
    fn get(
        &self,
        target: &NormalizedModelConnectionTargetV1,
        query: Option<&str>,
        credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        timeout: Duration,
        response_limit: usize,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        let resolved = self.resolve(&target.inventory_url(query));
        let proxied = resolved.as_ref().is_ok_and(|route| route.proxy.is_some());
        let result = resolved.and_then(|route| {
            self.inner
                .get_with_route(target, query, credential, timeout, response_limit, &route)
        });
        self.observed(result, Phase::Directory, proxied)
    }
    fn infer(
        &self,
        target: &NormalizedModelConnectionTargetV1,
        model: &str,
        credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        limits: ModelConnectionProbeLimitsV1,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        let resolved = self.resolve(&target.request_url());
        let proxied = resolved.as_ref().is_ok_and(|route| route.proxy.is_some());
        let result = resolved.and_then(|route| {
            self.inner
                .infer_with_route(target, model, credential, limits, &route)
        });
        self.observed(result, Phase::Inference, proxied)
    }
}
fn phase_code(phase: Phase) -> &'static str {
    match phase {
        Phase::RouteUnavailable => "native.network.route_unavailable",
        Phase::ProxyConnect => "native.network.proxy_connect",
        Phase::ProxyTls => "native.network.proxy_tls",
        Phase::ProxyAuth => "native.network.proxy_auth",
        Phase::ProxyNegotiation => "native.network.proxy_negotiation",
        Phase::UpstreamConnect => "native.network.upstream_connect",
        Phase::UpstreamTls => "native.network.upstream_tls",
        Phase::UpstreamAuth => "native.network.upstream_auth",
        Phase::Directory => "native.network.directory_failed",
        Phase::Inference => "native.network.inference_failed",
        Phase::TransportUnknown => "native.network.transport_unknown",
    }
}

/// Only classify phases observable through reqwest's public error API. In particular,
/// CONNECT/TLS errors that do not identify a leg remain TransportUnknown.
pub(super) fn request_error(
    error: reqwest::Error,
    route: Option<&ResolvedNativeNetworkRouteV1>,
    destination_scheme: &str,
) -> ModelDirectoryTransportErrorV1 {
    use std::error::Error;
    let proxy = route.and_then(|route| route.proxy.as_ref());
    let mut source = error.source();
    let mut refused = false;
    while let Some(cause) = source {
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            refused |= matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::AddrNotAvailable
            );
        }
        source = cause.source();
    }
    let plain_forward =
        proxy.is_some_and(|proxy| proxy.scheme == "http") && destination_scheme == "http";
    if error.is_connect()
        && (refused || plain_forward || (proxy.is_none() && destination_scheme == "http"))
    {
        return ModelDirectoryTransportErrorV1::Network(if proxy.is_some() {
            Phase::ProxyConnect
        } else {
            Phase::UpstreamConnect
        });
    }
    if error.is_timeout() {
        ModelDirectoryTransportErrorV1::Timeout
    } else {
        ModelDirectoryTransportErrorV1::Unavailable
    }
}

pub(super) fn client(
    timeout: Duration,
    route: Option<&ResolvedNativeNetworkRouteV1>,
) -> Result<reqwest::blocking::Client, ModelDirectoryTransportErrorV1> {
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout)
        .timeout(timeout);
    if let Some(proxy) = route.and_then(|r| r.proxy.as_ref()) {
        let url = std::str::from_utf8(proxy.protected_url().expose())
            .map_err(|_| ModelDirectoryTransportErrorV1::Unavailable)?;
        builder = builder.proxy(
            reqwest::Proxy::all(url).map_err(|_| ModelDirectoryTransportErrorV1::Unavailable)?,
        );
    }
    builder
        .build()
        .map_err(|_| ModelDirectoryTransportErrorV1::Unavailable)
}

#[cfg(test)]
mod tests;
