use super::*;
use hiroute_domain::NativeCredentialAccessDeniedV1;
use hiroute_gateway::server::composition::PortError;

#[tokio::test]
async fn exact_native_lease_keeps_source_and_key_denials_distinct_from_authority_errors() {
    for reason in [
        NativeCredentialAccessDeniedV1::SourceDisabled,
        NativeCredentialAccessDeniedV1::CredentialDisabled,
    ] {
        let native = Arc::new(FakeNativeAuthority {
            denial: Some(reason),
            ..FakeNativeAuthority::default()
        });
        let resolver = GatewayCredentialResolver::new(
            Arc::clone(&native),
            Arc::new(FakeCpaAuthority::default()),
        );
        let target = GatewayOperationalTargetV1::RegisteredHttps {
            uri: "https://api.anthropic.com/v1/messages".into(),
        };
        let digest = CanonicalDigest::of(&target).unwrap();
        let profile = CanonicalDigest::of_bytes(b"native-profile");
        let result = resolver
            .lease_exact(
                CredentialLeaseRequest {
                    execution_binding_id: "binding/claude",
                    stable_binding_id: "binding/claude",
                    credential_ref: "credential/claude",
                    credential_destination_ref: "connection-option/claude.custom.v1",
                    excluded_key_ids: &[],
                    connector_runtime: ConnectorRuntimeKind::BuiltinNative,
                    connector_id: "builtin-anthropic",
                    upstream_protocol: IngressProtocol::Messages,
                    upstream_model_id: "claude-sonnet",
                    native_transport_model: "claude-sonnet",
                    logical_endpoint: target.uri(),
                    operational_target: target.uri(),
                    operational_target_digest: digest.as_str(),
                    runtime_epoch: None,
                    target_epoch: None,
                    protocol_profile_digest: profile.as_str(),
                    request_path: "/v1/messages",
                    authentication: &AuthenticationSemantics::Bearer,
                },
                &scope(),
            )
            .await;
        assert_eq!(result.err(), Some(PortError::CredentialDenied(reason)));
        assert_eq!(native.requests.lock().unwrap().len(), 1);
    }
}
