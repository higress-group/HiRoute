//! One production diagnostic and its admitted network/credential projections.
use hiroute_application::control::ControlReadError;
use hiroute_application_api::{
    DecisionFailureCodeV2 as Failure, DecisionServiceTestResultSchemaV2,
    DecisionServiceTestResultSchemaV3, DecisionServiceTestResultV2, DecisionServiceTestResultV3,
    DecisionTestOutcomeV2, NativeNetworkCheckEvidenceV1, NativeNetworkFailurePhaseV1 as Phase,
    NativeNetworkFailureV1,
};
use hiroute_domain::{CanonicalDigest, ComplexityClassifierModeV1, DecisionServiceRecord};
use hiroute_gateway::server::core_runtime::{
    ClassifierDiagnosticError as Error, ClassifierNetworkDiagnostic, ProductionGatewayRuntime,
};

pub(super) fn test(
    gateway: &ProductionGatewayRuntime,
    service: &DecisionServiceRecord,
    configuration_digest: &CanonicalDigest,
) -> Result<DecisionServiceTestResultV3, ControlReadError> {
    let mode = match service {
        DecisionServiceRecord::V1(service) => ComplexityClassifierModeV1::DecisionService {
            service: Box::new(service.clone()),
        },
        DecisionServiceRecord::V2(service) => ComplexityClassifierModeV1::DecisionServiceV2 {
            service: Box::new(service.clone()),
        },
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| ControlReadError::Unavailable)?;
    let started = std::time::Instant::now();
    let (result, admitted, network) =
        runtime.block_on(gateway.test_classifier_decision_with_network(&mode));
    let failure_code = match result {
        Ok(_) => None,
        Err(Error::SourceDisabled) => Some(Failure::DecisionSourceDisabled),
        Err(Error::NoUsableCredentials) => Some(Failure::DecisionNoUsableCredentials),
        Err(Error::InvalidConfig | Error::Integrity) => Some(Failure::DecisionInvalidConfiguration),
        Err(Error::Timeout | Error::Cancelled) => Some(Failure::DecisionTimeout),
        Err(Error::AuthenticationRejected) => Some(Failure::DecisionAuthenticationRejected),
        Err(Error::RateLimited) => Some(Failure::DecisionRateLimited),
        Err(Error::RejectedInput | Error::EndpointRejected) => Some(Failure::DecisionInputRejected),
        Err(Error::InvalidOutput) => Some(Failure::DecisionMalformedOutput),
        Err(Error::Unavailable | Error::Resource) => Some(Failure::DecisionTransportUnavailable),
    };
    let (network, failure) = evidence(network);
    Ok(DecisionServiceTestResultV3 {
        schema: DecisionServiceTestResultSchemaV3::V3,
        check: DecisionServiceTestResultV2 {
            schema: DecisionServiceTestResultSchemaV2::V2,
            outcome: if failure_code.is_some() {
                DecisionTestOutcomeV2::Failed
            } else {
                DecisionTestOutcomeV2::Passed
            },
            failure_code,
            duration_millis: Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)),
            tested_configuration_digest: configuration_digest.clone(),
            tested_credential_state_revision: admitted,
        },
        network,
        failure,
    })
}

fn evidence(
    diagnostic: Option<ClassifierNetworkDiagnostic>,
) -> (
    Option<NativeNetworkCheckEvidenceV1>,
    Option<NativeNetworkFailureV1>,
) {
    let Some(diagnostic) = diagnostic else {
        return (None, None);
    };
    let evidence = diagnostic
        .snapshot
        .source(&diagnostic.source_id)
        .ok()
        .and_then(|record| {
            hiroute_integrations::native_network_route_view_at_revision(
                &diagnostic.snapshot,
                &record.policy,
                &diagnostic.endpoint,
                record.network_revision,
            )
            .ok()
            .map(|route| NativeNetworkCheckEvidenceV1 {
                policy: record.policy.clone(),
                expected_network_revision: record.network_revision,
                route,
            })
        });
    let phase = diagnostic
        .failure
        .or_else(|| {
            evidence
                .as_ref()
                .and_then(|evidence| evidence.route.failure.as_ref().map(|failure| failure.phase))
        })
        .or_else(|| evidence.is_none().then_some(Phase::RouteUnavailable));
    let failure = phase.map(|phase| NativeNetworkFailureV1 {
        phase,
        code: "NATIVE_DECISION_NETWORK_FAILED".into(),
        retryable: !matches!(phase, Phase::ProxyAuth | Phase::UpstreamAuth),
    });
    (evidence, failure)
}
