use super::*;
use hiroute_application::compute_management::TrustedComputeCandidateRegistry;
use hiroute_application_api::{ModelConnectionAuthenticationStatusV1, NativeNetworkRouteKindV1};
use hiroute_domain::{NativeNetworkRegistryV1, NativeProxyEnvironmentV1};
use std::io::{Read, Write};
use std::net::TcpListener;

fn draft(id: &str) -> NativeModelConnectionDraftV1 {
    NativeModelConnectionDraftV1 {
        display_template_id: None,
        inference_model_id: None,
        candidate_ref: None,
        lineage_ref: "lineage/native/id-validation".into(),
        trusted_lineage_digest: None,
        display_name: "中文接入名称".into(),
        existing_source_id: None,
        edit_revision: 1,
        check_id: "check/id-validation".into(),
        base_url: "http://127.0.0.1:8123/v1".into(),
        base_kind: ModelConnectionBaseKindV1::ApiRoot,
        request_path_override: None,
        inventory_path_override: None,
        protocol: UpstreamProtocol::Responses,
        protocol_profile_id: "profile/custom/responses".into(),
        protocol_profile_revision: 1,
        protocol_header_semantics: GatewayHeaderSemanticsV1 {
            content_type: "application/json".into(),
            required_headers: Vec::new(),
            forbidden_forward_headers: Vec::new(),
        },
        authentication: GatewayAuthenticationSemanticsV1::None,
        additional_native_endpoints: Vec::new(),
        provenance: NativeConnectionProvenanceInputV1::UserConfigured {
            configuration_revision: 1,
        },
        qualification: NativeConnectionQualificationV1 {
            free_access: None,
            evidence_ref: None,
        },
        runtime_fallback_denied_model_ids: Default::default(),
        models: vec![NativeModelDeclarationV1 {
            upstream_model_id: id.into(),
            display_name: "中文模型名称".into(),
            catalog_configuration_id: None,
            membership: ComputeModelMembershipV2::UserDeclared,
            capabilities: NativeModelCapabilityDeclarationV1::default(),
        }],
    }
}

fn context(
    policy: NativeNetworkPolicyV1,
    environment: impl IntoIterator<Item = (String, String)>,
    epoch: &str,
) -> NativeModelConnectionNetworkContextV1 {
    let registry = NativeNetworkRegistryV1::new(NativeProxyEnvironmentV1::from_variables(
        epoch.into(),
        environment,
    ));
    NativeModelConnectionNetworkContextV1 {
        snapshot: registry.pin().unwrap(),
        edit: NativeNetworkEditV1 {
            expected_network_revision: 0,
            policy,
        },
    }
}

#[test]
fn network_check_uses_explicit_proxy_without_upstream_dns_and_keeps_proxy_auth_separate() {
    for reject in [false, true] {
        let direct = TcpListener::bind("127.0.0.1:0").unwrap();
        direct.set_nonblocking(true).unwrap();
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = proxy.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = proxy.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.extend(byte);
                assert!(request.len() < 8192);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(
                request.starts_with("GET http://origin.invalid:"),
                "{request}"
            );
            let proxy_auth = request
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
                .map(|(_, value)| value.trim());
            assert_eq!(proxy_auth, Some("Basic cHJveHk6c2VjcmV0"));
            assert!(request.contains("Bearer provider-key"));
            let body = br#"{"data":[{"id":"network-model"}]}"#;
            let status = if reject {
                "407 Proxy Authentication Required"
            } else {
                "200 OK"
            };
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            socket.write_all(body).unwrap();
        });
        let service = NativeModelConnectionServiceV1::new(
            TrustedComputeCandidateRegistry::new(),
            ReqwestModelDirectoryTransportV1,
        );
        let candidate = ComputeCandidateRefV2 {
            candidate_ref: "candidate/network".into(),
            candidate_revision: 1,
        };
        service.reserve_candidate_ref(&candidate).unwrap();
        let mut draft = draft("network-model");
        draft.base_url = format!(
            "http://origin.invalid:{}/v1",
            direct.local_addr().unwrap().port()
        );
        draft.inventory_path_override = Some("/v1/models".into());
        draft.candidate_ref = Some(candidate.candidate_ref.clone());
        draft.authentication = GatewayAuthenticationSemanticsV1::Bearer;
        let secret = ProtectedSecret::new(b"provider-key".to_vec()).unwrap();
        let checked = service
            .check_with_network(
                draft,
                NativeModelConnectionCredentialV1::Protected {
                    descriptor: ProtectedInputSourceDescriptorV1::ManualInput,
                    input_slot: candidate.candidate_ref.clone(),
                    secret: &secret,
                },
                &ModelConnectionProbeCancellationV1::default(),
                context(
                    NativeNetworkPolicyV1::Inherit,
                    [(
                        "HTTP_PROXY".into(),
                        format!("http://proxy:secret@{address}"),
                    )],
                    "check-epoch",
                ),
            )
            .unwrap();
        server.join().unwrap();
        assert_eq!(
            direct.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(
            checked.network.route.route,
            Some(NativeNetworkRouteKindV1::Proxy)
        );
        assert_eq!(
            checked.failure.as_ref().map(|f| f.phase),
            reject.then_some(Phase::ProxyAuth)
        );
        assert_ne!(
            checked.check.authentication,
            ModelConnectionAuthenticationStatusV1::Rejected
        );
        let serialized = serde_json::to_string(&checked).unwrap();
        for secret in ["proxy:secret", "provider-key", "cHJveHk6c2VjcmV0"] {
            assert!(!serialized.contains(secret));
        }
        assert_eq!(
            service
                .network_check_evidence(&checked.check.candidate.candidate)
                .unwrap(),
            checked.network
        );
    }
}

#[test]
fn network_check_fails_closed_on_invalid_environment_and_binds_opaque_epoch() {
    let service = NativeModelConnectionServiceV1::new(
        TrustedComputeCandidateRegistry::new(),
        ReqwestModelDirectoryTransportV1,
    );
    let mut draft = draft("network-model");
    draft.base_url = "https://origin.invalid/v1".into();
    draft.inventory_path_override = Some("/v1/models".into());
    let checked = service
        .check_with_network(
            draft,
            NativeModelConnectionCredentialV1::NotRequired,
            &ModelConnectionProbeCancellationV1::default(),
            context(
                NativeNetworkPolicyV1::Inherit,
                [(
                    "HTTPS_PROXY".into(),
                    "socks5://invalid-secret@proxy.invalid".into(),
                )],
                "first",
            ),
        )
        .unwrap();
    assert_eq!(
        checked.failure.as_ref().unwrap().phase,
        Phase::RouteUnavailable
    );
    assert_eq!(checked.network.route.route, None);
    assert!(
        !serde_json::to_string(&checked)
            .unwrap()
            .contains("invalid-secret")
    );
    let first = context(NativeNetworkPolicyV1::Inherit, [], "first");
    let second = context(NativeNetworkPolicyV1::Inherit, [], "second");
    let a = native_network_route_view(
        &first.snapshot,
        &first.edit.policy,
        "https://origin.invalid/api?private=value",
    )
    .unwrap();
    let b = native_network_route_view(
        &second.snapshot,
        &second.edit.policy,
        "https://origin.invalid/api",
    )
    .unwrap();
    assert_ne!(a.state_revision, b.state_revision);
    assert!(!a.destination.contains("private"));
    assert_eq!(a.route, Some(NativeNetworkRouteKindV1::Direct));
    assert_eq!(
        a.reason,
        Some(hiroute_domain::NativeNetworkRouteReasonV1::NoProxyConfigured)
    );
}

#[test]
fn network_saved_decision_preview_tracks_its_actual_port_and_policy_revision_without_io() {
    let policy = NativeNetworkPolicyV1::Manual {
        proxy_url: "http://proxy.invalid:8080".into(),
        no_proxy: "decision.invalid:8443".into(),
    };
    let context = context(policy.clone(), [], "decision-epoch");
    let decision = native_network_route_view_at_revision(
        &context.snapshot,
        &policy,
        "https://decision.invalid:8443/v1/decisions?private=value",
        2,
    )
    .unwrap();
    let generation = native_network_route_view_at_revision(
        &context.snapshot,
        &policy,
        "https://decision.invalid:443/v1/responses",
        2,
    )
    .unwrap();
    assert_eq!(decision.route, Some(NativeNetworkRouteKindV1::Direct));
    assert_eq!(generation.route, Some(NativeNetworkRouteKindV1::Proxy));
    assert_eq!(decision.destination, "https://decision.invalid:8443");
    let returned_policy = native_network_route_view_at_revision(
        &context.snapshot,
        &policy,
        "https://decision.invalid:8443/v1/decisions",
        4,
    )
    .unwrap();
    assert_ne!(decision.state_revision, returned_policy.state_revision);
}
