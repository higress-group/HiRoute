use std::{
    net::TcpListener,
    sync::{
        Arc, Barrier, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

#[path = "model_connections/support.rs"]
mod support;
use support::{ControlledServer, CountingTransport, TestOnlyComputeCandidatePort};

use hiroute_application::compute_management::ComputeCandidatePort;
use hiroute_application_api::{
    ComputeCandidateFactStateV2, ComputeCandidateModelFactBasisV2, ComputeCandidateRefV2,
    ModelConnectionAuthenticationStatusV1, ModelConnectionDirectoryStatusV1,
    ModelConnectionInferenceStatusV1, ModelConnectionReachabilityV1,
};
use hiroute_domain::{
    FreeAccess, GatewayAuthenticationSemanticsV1, GatewayHeaderSemanticsV1,
    NativeReasoningCapabilityV1, ProtectedSecret, UpstreamProtocol,
};
use hiroute_integrations::{
    ModelConnectionBaseKindV1, ModelConnectionProbeCancellationV1,
    ModelConnectionProbeCredentialV1, ModelConnectionProbeLimitsV1, ModelDirectoryHttpResponseV1,
    ModelDirectoryTransportErrorV1, ModelDirectoryTransportV1, NativeCandidateFactBasisV1,
    NativeCandidateFactValueV1, NativeConnectionProvenanceInputV1, NativeConnectionQualificationV1,
    NativeModelCapabilityDeclarationV1, NativeModelConnectionCredentialV1,
    NativeModelConnectionDraftV1, NativeModelConnectionServiceV1, NativeModelDeclarationV1,
    NormalizedModelConnectionTargetV1, ReqwestModelDirectoryTransportV1,
};

fn fact<T>(value: T) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: Some(value),
        basis: NativeCandidateFactBasisV1::UserDeclared,
    }
}

fn registered_fact<T>(value: T) -> NativeCandidateFactValueV1<T> {
    NativeCandidateFactValueV1 {
        value: Some(value),
        basis: NativeCandidateFactBasisV1::RegisteredCatalog,
    }
}

fn model(id: &str) -> NativeModelDeclarationV1 {
    NativeModelDeclarationV1 {
        upstream_model_id: id.into(),
        display_name: id.into(),
        catalog_configuration_id: None,
        membership: hiroute_application_api::ComputeModelMembershipV2::UserDeclared,
        capabilities: NativeModelCapabilityDeclarationV1 {
            tool: fact(true),
            vision: fact(false),
            streaming: fact(true),
            context_tokens: fact(32_768),
            max_output_tokens: fact(4_096),
            native_reasoning: fact(NativeReasoningCapabilityV1::Discrete {
                parameter: "reasoning_effort".into(),
                profiles: vec!["low".into(), "medium".into(), "high".into()],
            }),
        },
    }
}

fn registered_model(id: &str) -> NativeModelDeclarationV1 {
    NativeModelDeclarationV1 {
        upstream_model_id: id.into(),
        display_name: "Registered model".into(),
        catalog_configuration_id: Some("model.registered".into()),
        membership: hiroute_application_api::ComputeModelMembershipV2::Catalog,
        capabilities: NativeModelCapabilityDeclarationV1 {
            tool: registered_fact(true),
            vision: registered_fact(false),
            streaming: registered_fact(true),
            context_tokens: registered_fact(32_768),
            max_output_tokens: registered_fact(4_096),
            native_reasoning: registered_fact(NativeReasoningCapabilityV1::Fixed {
                profile: "provider-default".into(),
            }),
        },
    }
}

fn draft(
    base_url: String,
    authentication: GatewayAuthenticationSemanticsV1,
) -> NativeModelConnectionDraftV1 {
    let (free_access, evidence_ref) = match authentication {
        GatewayAuthenticationSemanticsV1::None => (
            Some(FreeAccess::Direct),
            Some("evidence/local-direct".into()),
        ),
        _ => (None, None),
    };
    NativeModelConnectionDraftV1 {
        display_template_id: None,
        inference_model_id: None,
        candidate_ref: None,
        lineage_ref: "lineage/native/test".into(),
        trusted_lineage_digest: None,
        display_name: "Controlled API".into(),
        existing_source_id: None,
        additional_native_endpoints: Vec::new(),
        edit_revision: 7,
        check_id: "check/native/7".into(),
        base_url,
        base_kind: ModelConnectionBaseKindV1::ApiRoot,
        request_path_override: None,
        inventory_path_override: Some("/v1/models".into()),
        protocol: UpstreamProtocol::Responses,
        protocol_profile_id: "profile/custom/responses".into(),
        protocol_profile_revision: 1,
        protocol_header_semantics: GatewayHeaderSemanticsV1 {
            content_type: "application/json".into(),
            required_headers: Vec::new(),
            forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
        },
        authentication,
        provenance: NativeConnectionProvenanceInputV1::UserConfigured {
            configuration_revision: 7,
        },
        qualification: NativeConnectionQualificationV1 {
            free_access,
            evidence_ref,
        },
        runtime_fallback_denied_model_ids: Default::default(),
        models: vec![model("manual-model")],
    }
}

#[path = "model_connections/onboarding.rs"]
mod onboarding;

#[path = "model_connections/profile_regressions.rs"]
mod profile_regressions;

#[test]
fn specified_header_reaches_only_the_confirmed_target_and_public_view_is_safe() {
    let server = ControlledServer::start(vec![(
        200,
        Vec::new(),
        r#"{"data":[{"id":"manual-model"},{"id":"listed-model"},{"bad":true}]}"#.into(),
    )]);
    let secret = ProtectedSecret::new(b"header-secret-sentinel".to_vec()).unwrap();
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let result = service
        .check(
            draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                    header: "x-api-key".into(),
                },
            ),
            NativeModelConnectionCredentialV1::Protected {
                descriptor:
                    hiroute_application::compute_management::ProtectedInputSourceDescriptorV1::ManualInput,
                input_slot: "input/native/1".into(),
                secret: &secret,
            },
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    assert_eq!(
        result.reachability,
        ModelConnectionReachabilityV1::Reachable
    );
    assert_eq!(
        result.authentication,
        ModelConnectionAuthenticationStatusV1::Verified
    );
    assert_eq!(result.directory, ModelConnectionDirectoryStatusV1::Partial);
    assert_eq!(result.checked_model_count, 2);
    assert_eq!(result.invalid_model_count, 1);
    assert_eq!(result.inference, ModelConnectionInferenceStatusV1::NotRun);
    assert_eq!(result.candidate.candidate.candidate_revision, 1);
    assert_eq!(result.candidate.correlation.edit_revision, 7);
    assert!(result.checked_at_unix_ms > 0);
    let fallback = result
        .candidate
        .models
        .iter()
        .find(|model| model.upstream_model_id == "listed-model")
        .unwrap();
    assert!(fallback.selectable);
    assert_eq!(
        fallback.fact_basis,
        ComputeCandidateModelFactBasisV2::Unknown
    );
    let public = serde_json::to_string(&result).unwrap();
    assert!(!public.contains("header-secret-sentinel"));
    assert!(!public.contains("input/native/1"));
    let requests = server.finish();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /v1/models HTTP/1.1\r\n"));
    assert!(requests[0].contains("x-api-key: header-secret-sentinel\r\n"));
    assert!(!requests[0].contains("authorization:"));
}

#[test]
fn observed_non_text_model_cannot_enter_the_conservative_runtime_fallback() {
    let server = ControlledServer::start(vec![(
        200,
        Vec::new(),
        r#"{"data":[{"id":"gpt-image-2"}]}"#.into(),
    )]);
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let mut input = draft(
        server.base_url.clone(),
        GatewayAuthenticationSemanticsV1::None,
    );
    input.models.clear();
    input
        .runtime_fallback_denied_model_ids
        .insert("gpt-image-2".into());

    let result = service
        .check(
            input,
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    let image = result
        .candidate
        .models
        .iter()
        .find(|model| model.upstream_model_id == "gpt-image-2")
        .unwrap();
    assert!(!image.selectable);
    assert_eq!(
        image.reason.as_deref(),
        Some("model_connections.runtime_fallback_ineligible")
    );
    assert_eq!(image.fact_basis, ComputeCandidateModelFactBasisV2::Unknown);
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn none_authentication_sends_no_credential_and_missing_bearer_does_not_touch_network() {
    let server = ControlledServer::start(vec![(
        200,
        Vec::new(),
        r#"{"data":[{"id":"manual-model"}]}"#.into(),
    )]);
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let direct = service
        .check(
            draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::None,
            ),
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    assert_eq!(
        direct.authentication,
        ModelConnectionAuthenticationStatusV1::NotRequired
    );
    assert_eq!(
        direct.candidate.fact_state,
        ComputeCandidateFactStateV2::Complete
    );
    let requests = server.finish();
    assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
    assert!(!requests[0].to_ascii_lowercase().contains("x-api-key:"));

    let pending_service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let pending = pending_service
        .check(
            draft(
                "http://127.0.0.1:9/v1".into(),
                GatewayAuthenticationSemanticsV1::Bearer,
            ),
            NativeModelConnectionCredentialV1::PendingInput,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    assert_eq!(pending.reachability, ModelConnectionReachabilityV1::NotRun);
    assert_eq!(
        pending.candidate.fact_state,
        ComputeCandidateFactStateV2::PendingCredential
    );
}

#[test]
fn declared_model_remains_selectable_when_the_directory_endpoint_is_absent() {
    let server = ControlledServer::start(vec![(404, Vec::new(), "{}".into())]);
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let result = service
        .check(
            draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::None,
            ),
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();

    assert_eq!(
        result.directory,
        ModelConnectionDirectoryStatusV1::Unavailable
    );
    assert_eq!(result.issues[0].code, "DIRECTORY_UNAVAILABLE");
    assert_eq!(
        result.candidate.fact_state,
        ComputeCandidateFactStateV2::Complete
    );
    let declared = result
        .candidate
        .models
        .iter()
        .find(|model| model.upstream_model_id == "manual-model")
        .unwrap();
    assert!(declared.selectable);
    assert_eq!(
        declared.membership,
        hiroute_application_api::ComputeModelMembershipV2::UserDeclared
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn registered_inventory_keeps_catalog_matches_and_admits_unknown_text_fallback() {
    let server = ControlledServer::start(vec![(
        200,
        Vec::new(),
        r#"{"data":[{"id":"registered-model"},{"id":"unexpected-model"},{"bad":true}]}"#.into(),
    )]);
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let mut registered = draft(
        server.base_url.clone(),
        GatewayAuthenticationSemanticsV1::None,
    );
    registered.models = vec![registered_model("registered-model")];
    registered.provenance = NativeConnectionProvenanceInputV1::Registered {
        connection_option_id: "registered.payg.v1".into(),
        registry_version: "registry-v1".into(),
        catalog_digest: hiroute_domain::CanonicalDigest::of_bytes(b"catalog"),
    };
    let result = service
        .check(
            registered,
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();

    assert_eq!(result.directory, ModelConnectionDirectoryStatusV1::Partial);
    let matched = result
        .candidate
        .models
        .iter()
        .find(|model| model.upstream_model_id == "registered-model")
        .unwrap();
    assert!(matched.selectable);
    let unmatched = result
        .candidate
        .models
        .iter()
        .find(|model| model.upstream_model_id == "unexpected-model")
        .unwrap();
    assert!(unmatched.selectable);
    assert_eq!(unmatched.reason, None);
    assert_eq!(
        unmatched.fact_basis,
        ComputeCandidateModelFactBasisV2::Unknown
    );
    assert!(
        !result
            .issues
            .iter()
            .any(|issue| issue.code == "registered_model_unmatched")
    );
    assert_eq!(result.inference, ModelConnectionInferenceStatusV1::NotRun);
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn saved_native_credential_is_applied_but_its_reference_stays_trusted_only() {
    let server = ControlledServer::start(vec![(
        200,
        Vec::new(),
        r#"{"data":[{"id":"manual-model"}]}"#.into(),
    )]);
    let secret = ProtectedSecret::new(b"saved-secret-sentinel".to_vec()).unwrap();
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let result = service
        .check(
            draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::Bearer,
            ),
            NativeModelConnectionCredentialV1::Saved {
                credential_id: "credential/private/sentinel".into(),
                expected_generation: 9,
                secret: &secret,
            },
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();

    let public = serde_json::to_string(&result).unwrap();
    assert!(!public.contains("saved-secret-sentinel"));
    assert!(!public.contains("credential/private/sentinel"));
    let requests = server.finish();
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer saved-secret-sentinel")
    );
}

#[test]
fn malformed_protected_bindings_fail_before_any_target_request() {
    let secret = ProtectedSecret::new(b"never-sent".to_vec()).unwrap();
    let transport = CountingTransport::default();
    let calls = transport.calls();
    let service =
        NativeModelConnectionServiceV1::new(TestOnlyComputeCandidatePort::new(), transport);
    let target = || {
        draft(
            "http://127.0.0.1:9/v1".into(),
            GatewayAuthenticationSemanticsV1::Bearer,
        )
    };

    assert!(
        service
            .check(
                target(),
                NativeModelConnectionCredentialV1::Protected {
                    descriptor:
                        hiroute_application::compute_management::ProtectedInputSourceDescriptorV1::ManualInput,
                    input_slot: String::new(),
                    secret: &secret,
                },
                &ModelConnectionProbeCancellationV1::default(),
            )
            .is_err()
    );
    assert!(
        service
            .check(
                target(),
                NativeModelConnectionCredentialV1::Saved {
                    credential_id: "credential/invalid".into(),
                    expected_generation: 0,
                    secret: &secret,
                },
                &ModelConnectionProbeCancellationV1::default(),
            )
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn auth_failure_and_redirect_are_distinct_and_never_retried() {
    for (status, expected_code, expected_authentication) in [
        (
            401,
            "AUTH_REJECTED",
            ModelConnectionAuthenticationStatusV1::Rejected,
        ),
        (
            403,
            "DIRECTORY_FORBIDDEN",
            ModelConnectionAuthenticationStatusV1::Unknown,
        ),
        (
            404,
            "DIRECTORY_UNAVAILABLE",
            ModelConnectionAuthenticationStatusV1::Unknown,
        ),
        (
            429,
            "RATE_LIMITED",
            ModelConnectionAuthenticationStatusV1::Unknown,
        ),
        (
            301,
            "REDIRECT_REJECTED",
            ModelConnectionAuthenticationStatusV1::Unknown,
        ),
    ] {
        let redirect_target = TcpListener::bind("127.0.0.1:0").unwrap();
        redirect_target.set_nonblocking(true).unwrap();
        let location = format!("http://{}/stolen", redirect_target.local_addr().unwrap());
        let server = ControlledServer::start(vec![(
            status,
            vec![("Location".into(), location)],
            "{}".into(),
        )]);
        let secret = ProtectedSecret::new(b"bearer-secret-sentinel".to_vec()).unwrap();
        let service = NativeModelConnectionServiceV1::new(
            TestOnlyComputeCandidatePort::new(),
            ReqwestModelDirectoryTransportV1,
        );
        let result = service
            .check(
                draft(
                    server.base_url.clone(),
                    GatewayAuthenticationSemanticsV1::Bearer,
                ),
                NativeModelConnectionCredentialV1::Protected {
                    descriptor:
                        hiroute_application::compute_management::ProtectedInputSourceDescriptorV1::ManualInput,
                    input_slot: "input/native/1".into(),
                    secret: &secret,
                },
                &ModelConnectionProbeCancellationV1::default(),
            )
            .unwrap();
        assert_eq!(result.issues[0].code, expected_code);
        assert_eq!(result.authentication, expected_authentication);
        let requests = server.finish();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("authorization: bearer bearer-secret-sentinel")
        );
        assert!(matches!(
            redirect_target.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
    }
}

#[test]
fn aggregate_response_limit_stops_the_check_without_exposing_the_body() {
    let server = ControlledServer::start(vec![(200, Vec::new(), "x".repeat(65))]);
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    )
    .with_limits(ModelConnectionProbeLimitsV1 {
        response_bytes: 64,
        ..ModelConnectionProbeLimitsV1::default()
    });
    let result = service
        .check(
            draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::None,
            ),
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    assert_eq!(result.issues[0].code, "RESPONSE_TOO_LARGE");
    assert_eq!(
        result.reachability,
        ModelConnectionReachabilityV1::Reachable
    );
    assert_eq!(result.directory, ModelConnectionDirectoryStatusV1::Partial);
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains(&"x".repeat(65))
    );
    assert_eq!(server.finish().len(), 1);
}

#[test]
fn pagination_is_bounded_and_uses_only_the_same_inventory_target() {
    let server = ControlledServer::start(vec![
        (
            200,
            Vec::new(),
            r#"{"data":[{"id":"model-1"}],"has_more":true,"last_id":"model-1"}"#.into(),
        ),
        (
            200,
            Vec::new(),
            r#"{"data":[{"id":"model-2"}],"has_more":true,"last_id":"model-2"}"#.into(),
        ),
    ]);
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    )
    .with_limits(ModelConnectionProbeLimitsV1 {
        pages: 2,
        ..ModelConnectionProbeLimitsV1::default()
    });
    let result = service
        .check(
            draft(
                server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::None,
            ),
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    assert_eq!(result.pages_read, 2);
    assert_eq!(result.directory, ModelConnectionDirectoryStatusV1::Partial);
    let requests = server.finish();
    assert!(requests[0].starts_with("GET /v1/models HTTP/1.1\r\n"));
    assert!(requests[1].starts_with("GET /v1/models?after=model-1 HTTP/1.1\r\n"));
}

#[test]
fn changing_target_and_authentication_issues_new_backend_facts() {
    let first_server = ControlledServer::start(vec![(
        200,
        Vec::new(),
        r#"{"data":[{"id":"manual-model"}]}"#.into(),
    )]);
    let second_server = ControlledServer::start(vec![(
        200,
        Vec::new(),
        r#"{"data":[{"id":"manual-model"}]}"#.into(),
    )]);
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let first = service
        .check(
            draft(
                first_server.base_url.clone(),
                GatewayAuthenticationSemanticsV1::None,
            ),
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    let secret = ProtectedSecret::new(b"replacement-secret".to_vec()).unwrap();
    let mut replacement_draft = draft(
        second_server.base_url.clone(),
        GatewayAuthenticationSemanticsV1::Bearer,
    );
    replacement_draft.candidate_ref = Some(first.candidate.candidate.candidate_ref.clone());
    replacement_draft.edit_revision = 8;
    replacement_draft.check_id = "check/native/8".into();
    let replacement = service
        .check(
            replacement_draft,
            NativeModelConnectionCredentialV1::Protected {
                descriptor:
                    hiroute_application::compute_management::ProtectedInputSourceDescriptorV1::ManualInput,
                input_slot: "input/native/2".into(),
                secret: &secret,
            },
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();

    assert_eq!(first.candidate.candidate.candidate_revision, 1);
    assert_eq!(replacement.candidate.candidate.candidate_revision, 2);
    assert_ne!(first.input_digest, replacement.input_digest);
    assert_eq!(replacement.candidate.correlation.edit_revision, 8);
    assert_eq!(first_server.finish().len(), 1);
    assert_eq!(second_server.finish().len(), 1);
}

#[test]
fn a_pre_cancelled_check_sends_zero_requests() {
    let cancellation = ModelConnectionProbeCancellationV1::default();
    cancellation.cancel();
    let service = NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let result = service
        .check(
            draft(
                "http://127.0.0.1:9/v1".into(),
                GatewayAuthenticationSemanticsV1::None,
            ),
            NativeModelConnectionCredentialV1::NotRequired,
            &cancellation,
        )
        .unwrap();
    assert_eq!(result.reachability, ModelConnectionReachabilityV1::NotRun);
    assert_eq!(result.issues[0].code, "CHECK_CANCELLED");
    assert!(
        result
            .candidate
            .models
            .iter()
            .all(|model| !model.selectable)
    );
}

struct ReorderedTransportState {
    calls: AtomicUsize,
    first_started: Barrier,
    release_first: (Mutex<bool>, Condvar),
}

#[derive(Clone)]
struct ReorderedTransport {
    state: Arc<ReorderedTransportState>,
}

impl ReorderedTransport {
    fn new() -> Self {
        Self {
            state: Arc::new(ReorderedTransportState {
                calls: AtomicUsize::new(0),
                first_started: Barrier::new(2),
                release_first: (Mutex::new(false), Condvar::new()),
            }),
        }
    }
}

impl ModelDirectoryTransportV1 for ReorderedTransport {
    fn get(
        &self,
        _target: &NormalizedModelConnectionTargetV1,
        _query: Option<&str>,
        _credential: Option<ModelConnectionProbeCredentialV1<'_>>,
        _timeout: Duration,
        _response_limit: usize,
    ) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
        let call = self.state.calls.fetch_add(1, Ordering::SeqCst);
        if call == 1 {
            self.state.first_started.wait();
            let (lock, ready) = &self.state.release_first;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = ready.wait(released).unwrap();
            }
        }
        Ok(ModelDirectoryHttpResponseV1 {
            status: 200,
            body: format!(r#"{{"data":[{{"id":"observed-{call}"}}]}}"#).into_bytes(),
            truncated: false,
        })
    }
}

#[test]
fn producer_surfaces_test_port_rejection_when_older_check_finishes_late() {
    let transport = ReorderedTransport::new();
    let transport_state = Arc::clone(&transport.state);
    let service = Arc::new(NativeModelConnectionServiceV1::new(
        TestOnlyComputeCandidatePort::new(),
        transport,
    ));
    let seed = service
        .check(
            draft(
                "http://127.0.0.1:9/v1".into(),
                GatewayAuthenticationSemanticsV1::None,
            ),
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    let candidate_ref = seed.candidate.candidate.candidate_ref;
    let old_service = Arc::clone(&service);
    let mut old_draft = draft(
        "http://127.0.0.1:9/v1".into(),
        GatewayAuthenticationSemanticsV1::None,
    );
    old_draft.edit_revision = 3;
    old_draft.check_id = "check/native/3".into();
    old_draft.candidate_ref = Some(candidate_ref.clone());
    let old = thread::spawn(move || {
        old_service.check(
            old_draft,
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
    });

    transport_state.first_started.wait();
    let mut current_draft = draft(
        "http://127.0.0.1:9/v1".into(),
        GatewayAuthenticationSemanticsV1::None,
    );
    current_draft.edit_revision = 4;
    current_draft.check_id = "check/native/4".into();
    current_draft.candidate_ref = Some(candidate_ref.clone());
    let current = service
        .check(
            current_draft,
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
        )
        .unwrap();
    let (lock, ready) = &transport_state.release_first;
    *lock.lock().unwrap() = true;
    ready.notify_all();
    assert_eq!(current.candidate.candidate.candidate_revision, 3);
    assert_eq!(current.candidate.correlation.edit_revision, 4);
    assert!(old.join().unwrap().is_err());
    let stored = service
        .candidate_port()
        .get_compute_candidate(&ComputeCandidateRefV2 {
            candidate_ref,
            candidate_revision: 3,
        })
        .unwrap();
    assert_eq!(stored.correlation.edit_revision, 4);
}
