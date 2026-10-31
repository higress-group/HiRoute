use super::*;

fn fact<T>(value: T) -> ComputeManagementFactValueV2<T> {
    ComputeManagementFactValueV2 {
        value: Some(value),
        basis: ComputeManagementFactBasisV2::UserDeclared,
    }
}

fn source(authentication: GatewayAuthenticationSemanticsV1) -> ComputeManagementSourceV2 {
    let target = ComputeManagementTargetV2 {
        scheme: "https".into(),
        authority: "api.example.test".into(),
        port: 443,
        request_path: "/v1/responses".into(),
        upstream_protocol: UpstreamProtocol::Responses,
        protocol_profile_id: "profile/responses".into(),
        protocol_profile_revision: 1,
    };
    let destination = target.credential_destination().unwrap();
    let credentials = if matches!(authentication, GatewayAuthenticationSemanticsV1::None) {
        Vec::new()
    } else {
        vec![ComputeManagedCredentialV2 {
            key_id: "credential/source-a/key-a".into(),
            credential: CredentialRefV1::new(
                "credential/source-a/key-a",
                "source/source-a",
                "hirouted",
                "provider-auth",
                [destination],
                1,
            )
            .unwrap(),
            fingerprint: CanonicalDigest::of_bytes(b"key-a"),
            ordinal: 0,
            enabled: true,
        }]
    };
    ComputeManagementSourceV2 {
        schema: COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into(),
        source_id: "source-a".into(),
        revision: 1,
        lineage_digest: CanonicalDigest::of_bytes(b"lineage-a"),
        display_name: "Example".into(),
        provenance: ComputeManagementProvenanceV2::UserConfigured {
            configuration_revision: 1,
            evidence_digest: CanonicalDigest::of_bytes(b"evidence"),
        },
        target,
        authentication,
        state: MaterializationState::Ready,
        models: vec![ComputeManagedModelV2 {
            model_ref: "model/a".into(),
            binding_id: "binding/a".into(),
            revision: 1,
            upstream_model_id: "model-a".into(),
            display_name: "Model A".into(),
            catalog_configuration_id: None,
            membership: ComputeManagementMembershipV2::UserDeclared,
            execution_eligible: true,
            capabilities: ComputeManagedCapabilitiesV2 {
                tool: fact(true),
                vision: fact(false),
                streaming: fact(true),
                context_tokens: fact(16_384),
                max_output_tokens: fact(4_096),
                native_reasoning: ComputeManagementFactValueV2 {
                    value: None,
                    basis: ComputeManagementFactBasisV2::Unknown,
                },
            },
            capability_evidence_digest: CanonicalDigest::of_bytes(b"capabilities"),
        }],
        native_recheck: None,
        additional_native_endpoints: Vec::new(),
        decision_targets: Vec::new(),
        credentials,
        validation: None,
        last_candidate_ref: "candidate/a".into(),
        last_candidate_revision: 4,
    }
}

#[test]
fn ready_authentication_requires_an_enabled_key_but_none_does_not() {
    assert_eq!(
        source(GatewayAuthenticationSemanticsV1::Bearer).validate(),
        Ok(())
    );
    assert_eq!(
        source(GatewayAuthenticationSemanticsV1::None).validate(),
        Ok(())
    );
    let mut missing = source(GatewayAuthenticationSemanticsV1::Bearer);
    missing.credentials.clear();
    assert_eq!(
        missing.validate(),
        Err(ComputeManagementErrorV2::InvalidState)
    );
    missing.state = MaterializationState::NeedsCredential;
    assert_eq!(missing.validate(), Ok(()));
}

#[test]
fn one_native_source_shares_one_key_across_distinct_protocol_endpoints() {
    let mut value = source(GatewayAuthenticationSemanticsV1::Bearer);
    let messages = ComputeNativeEndpointV3 {
        target: ComputeManagementTargetV2 {
            scheme: "https".into(),
            authority: "messages.example.test".into(),
            port: 443,
            request_path: "/v1/messages".into(),
            upstream_protocol: UpstreamProtocol::Messages,
            protocol_profile_id: "profile/messages".into(),
            protocol_profile_revision: 1,
        },
        authentication: GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header: "x-api-key".into(),
        },
        recheck: None,
    };
    let messages_destination = messages.target.credential_destination().unwrap();
    value.additional_native_endpoints.push(messages.clone());
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::InvalidCredential)
    );
    let destinations = value.native_destinations().unwrap();
    value.credentials[0].credential = CredentialRefV1::new(
        "credential/source-a/key-a",
        "source/source-a",
        "hirouted",
        "provider-auth",
        destinations,
        1,
    )
    .unwrap();
    assert_eq!(value.validate(), Ok(()));
    assert!(
        value.credentials[0]
            .credential
            .allowed_destinations()
            .contains(&messages_destination)
    );
    value.additional_native_endpoints.push(messages);
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::DuplicateIdentity)
    );
    value.additional_native_endpoints.pop();
    value.additional_native_endpoints[0].authentication = GatewayAuthenticationSemanticsV1::None;
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::InvalidCredential)
    );
}

#[test]
fn decision_targets_are_distinct_from_models_and_preserve_legacy_source_bytes() {
    let mut source = source(GatewayAuthenticationSemanticsV1::Bearer);
    let legacy = serde_json::to_string(&source).unwrap();
    let legacy_digest = source.digest().unwrap();
    let reopened: ComputeManagementSourceV2 = serde_json::from_str(&legacy).unwrap();
    assert_eq!(serde_json::to_string(&reopened).unwrap(), legacy);
    assert_eq!(reopened.digest().unwrap(), legacy_digest);

    let mut target = ComputeDecisionTargetV4::new(
        "https://EXAMPLE.test/v1/decisions",
        crate::DecisionWireProtocolV2::OpenaiDecisions,
    )
    .unwrap();
    assert_eq!(target.endpoint, "https://example.test/v1/decisions");
    let first = target
        .consumer(
            &source.source_id,
            &source.lineage_digest,
            "decision/a",
            1,
            "model-a",
        )
        .unwrap();
    let second = target
        .consumer(
            &source.source_id,
            &source.lineage_digest,
            "decision/b",
            1,
            "model-b",
        )
        .unwrap();
    assert_ne!(first.consumer_binding_id, second.consumer_binding_id);
    target.consumers = vec![first, second];
    source.schema = COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V4.into();
    source.decision_targets.push(target.clone());
    let destinations = source.native_destinations().unwrap();
    assert!(destinations.contains(&target.destination_id));
    assert!(target.destination_id.starts_with("decision-target/"));
    assert_eq!(destinations.len(), 2);
    let key = &mut source.credentials[0];
    key.credential = CredentialRefV1::new(
        &key.key_id,
        format!("source/{}", source.source_id),
        "hirouted",
        "provider-auth",
        destinations,
        2,
    )
    .unwrap();
    assert_eq!(source.validate(), Ok(()));
    source.decision_targets[0].consumers[0].model = "substituted-model".into();
    assert_eq!(
        source.validate(),
        Err(ComputeManagementErrorV2::InvalidTarget)
    );
    source.decision_targets[0] = target;
    source.schema = COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V2.into();
    assert_eq!(
        source.validate(),
        Err(ComputeManagementErrorV2::UnsupportedSchema)
    );
}

#[test]
fn shared_decision_registration_rebinds_only_when_destination_union_changes() {
    let source = source(GatewayAuthenticationSemanticsV1::Bearer);
    let service = |id: &str, revision, model: &str| crate::DecisionServiceV2 {
        schema: crate::DecisionServiceSchemaV2::V2,
        id: id.into(),
        revision,
        name: "Decision".into(),
        connection: crate::DecisionConnectionV2::Native {
            provider: "example".into(),
            wire_protocol: crate::DecisionWireProtocolV2::OpenaiDecisions,
            model: model.into(),
            endpoint: "https://example.test/v1/decisions".into(),
            timeout_ms: 1000,
            authentication: crate::DecisionAuthenticationV2::SharedSource {
                source_id: source.source_id.clone(),
                source_lineage_digest: source.lineage_digest.clone(),
            },
        },
    };
    let first = register_decision_consumer(&source, &service("decision/a", 1, "model-a")).unwrap();
    assert_eq!(first.secrets.len(), 1);
    assert_eq!(first.secrets[0].kind(), crate::SecretMutationKind::Rebind);
    assert!(first.secrets[0].input_slot().is_none());
    assert_eq!(
        first.after.credentials[0].key_id,
        source.credentials[0].key_id
    );
    assert_eq!(
        first.after.credentials[0].fingerprint,
        source.credentials[0].fingerprint
    );
    assert_eq!(first.after.credentials[0].credential.generation(), 2);

    let second =
        register_decision_consumer(&first.after, &service("decision/b", 1, "model-b")).unwrap();
    assert!(second.secrets.is_empty());
    assert_eq!(second.after.credentials[0].credential.generation(), 2);
    assert_eq!(second.after.decision_targets[0].consumers.len(), 2);
    let removed_one = remove_decision_consumers(&second.after, "decision/a")
        .unwrap()
        .unwrap();
    assert!(removed_one.secrets.is_empty());
    assert_eq!(removed_one.after.decision_targets[0].consumers.len(), 1);
    let removed_last = remove_decision_consumers(&removed_one.after, "decision/b")
        .unwrap()
        .unwrap();
    assert_eq!(removed_last.secrets.len(), 1);
    assert_eq!(removed_last.after.credentials[0].credential.generation(), 3);
    assert!(removed_last.after.decision_targets.is_empty());
    assert_eq!(
        removed_last.after.native_destinations().unwrap(),
        source.native_destinations().unwrap()
    );
}

#[test]
fn compound_decision_plan_rejects_missing_rebind_and_membership_tampering() {
    let source = source(GatewayAuthenticationSemanticsV1::Bearer);
    let service = crate::DecisionServiceV2 {
        schema: crate::DecisionServiceSchemaV2::V2,
        id: "decision/a".into(),
        revision: 1,
        name: "Decision".into(),
        connection: crate::DecisionConnectionV2::Native {
            provider: "example".into(),
            wire_protocol: crate::DecisionWireProtocolV2::SystemOne,
            model: "model-a".into(),
            endpoint: "https://example.test/decision".into(),
            timeout_ms: 1000,
            authentication: crate::DecisionAuthenticationV2::SharedSource {
                source_id: source.source_id.clone(),
                source_lineage_digest: source.lineage_digest.clone(),
            },
        },
    };
    let transition = register_decision_consumer(&source, &service).unwrap();
    let input = crate::DecisionServiceOperationInputV2 {
        change: crate::DecisionServiceChangeV2 {
            id: service.id.clone(),
            expected_revision: 0,
            service: Some(service),
            expected_source_revisions: vec![crate::DecisionExpectedSourceRevisionV1 {
                source_id: source.source_id.clone(),
                revision: source.revision,
            }],
        },
        input_slot: None,
    };
    let spec = crate::ChangeSpecV1 {
        schema_version: crate::CHANGE_SPEC_SCHEMA_V1,
        command_id: "decision.connections.apply".into(),
        resource_id: Some(input.change.id.clone()),
        desired_state: serde_json::to_value(&input).unwrap(),
    };
    let operation = crate::DecisionServiceOperationV2 {
        schema: crate::operation::DECISION_SERVICE_OPERATION_SCHEMA_V2.into(),
        input,
        sources: vec![crate::DecisionSourceMutationV1 {
            before: transition.before,
            after: transition.after,
        }],
    };
    let control = serde_json::json!({"decision_service_operation_v2":operation});
    let make = |control, secrets| {
        crate::TransactionPlanV1::from_registered_typed_planner(
            spec.clone(),
            control,
            None,
            secrets,
            vec![],
            vec![],
        )
    };
    assert!(make(control.clone(), transition.secrets.clone()).is_ok());
    assert!(make(control.clone(), vec![]).is_err());
    let mut tampered = control;
    tampered["decision_service_operation_v2"]["sources"][0]["after"]["decision_targets"][0]["consumers"]
        [0]["model"] = serde_json::json!("other");
    assert!(make(tampered, transition.secrets).is_err());
}

#[test]
fn disabled_source_may_retain_ordered_keys_without_becoming_ready() {
    let mut disabled = source(GatewayAuthenticationSemanticsV1::Bearer);
    disabled.state = MaterializationState::Disabled;
    assert_eq!(disabled.validate(), Ok(()));
    disabled.credentials[0].ordinal = 2;
    assert_eq!(
        disabled.validate(),
        Err(ComputeManagementErrorV2::DuplicateIdentity)
    );
}

#[test]
fn custom_header_rejects_reserved_or_injected_names() {
    for header in ["Authorization", "X-Key\r\nInjected: yes", "Host"] {
        let invalid = source(GatewayAuthenticationSemanticsV1::ApiKeyHeader {
            header: header.into(),
        });
        assert_eq!(
            invalid.validate(),
            Err(ComputeManagementErrorV2::InvalidAuthentication)
        );
    }
}

#[test]
fn explicit_http_and_https_targets_survive_source_serialization() {
    for scheme in ["http", "https"] {
        for authority in [
            "192.0.2.10",
            "198.51.100.10",
            "gateway.invalid",
            "localhost",
            "127.0.0.1",
            "::1",
            "2001:db8::10",
        ] {
            let mut value = source(GatewayAuthenticationSemanticsV1::None);
            value.target.scheme = scheme.into();
            value.target.authority = authority.into();
            value.target.port = 8123;
            assert_eq!(value.validate(), Ok(()));
            let reopened: ComputeManagementSourceV2 =
                serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
            assert_eq!(reopened, value);
            assert_eq!(reopened.validate(), Ok(()));
        }
    }
}

#[test]
fn explicit_http_retains_target_and_credential_scope_guards() {
    let mut value = source(GatewayAuthenticationSemanticsV1::None);
    for scheme in ["ftp", "file", "ws", ""] {
        value.target.scheme = scheme.into();
        assert_eq!(
            value.validate(),
            Err(ComputeManagementErrorV2::InvalidTarget)
        );
    }
    value.target.scheme = "http".into();
    for authority in [
        "",
        "user:secret@gateway.invalid",
        "gateway.invalid/path",
        "gateway.invalid?key=secret",
    ] {
        value.target.authority = authority.into();
        assert_eq!(
            value.validate(),
            Err(ComputeManagementErrorV2::InvalidTarget)
        );
    }
    value.target.authority = "gateway.invalid".into();
    value.target.port = 0;
    assert_eq!(
        value.validate(),
        Err(ComputeManagementErrorV2::InvalidTarget)
    );

    // Expanding supported transport must not give a saved HTTPS key authority
    // over a different HTTP destination. The save planner must bind it afresh.
    let mut authenticated = source(GatewayAuthenticationSemanticsV1::Bearer);
    authenticated.target.scheme = "http".into();
    assert!(authenticated.target.validate().is_ok());
    assert!(authenticated.validate().is_err());
}

#[test]
fn native_recheck_context_is_optional_safe_and_backward_compatible() {
    let legacy = source(GatewayAuthenticationSemanticsV1::None);
    let legacy_json = serde_json::to_value(&legacy).unwrap();
    assert!(legacy_json.get("native_recheck").is_none());
    let decoded: ComputeManagementSourceV2 = serde_json::from_value(legacy_json).unwrap();
    assert!(decoded.native_recheck.is_none());
    assert_eq!(decoded.validate(), Ok(()));

    let mut current = legacy;
    current.native_recheck = Some(ComputeNativeRecheckDescriptorV2 {
        display_template_id: None,
        inventory_path: Some("/v1/models".into()),
        protocol_header_semantics: GatewayHeaderSemanticsV1 {
            content_type: "application/json".into(),
            required_headers: vec![("anthropic-version".into(), "2023-06-01".into())],
            forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
        },
    });
    assert_eq!(current.validate(), Ok(()));
    assert!(
        serde_json::to_value(&current).unwrap()["native_recheck"]["inventory_path"]
            == serde_json::json!("/v1/models")
    );

    current.native_recheck.as_mut().unwrap().inventory_path =
        Some("https://forged.test/models".into());
    assert_eq!(
        current.validate(),
        Err(ComputeManagementErrorV2::InvalidTarget)
    );
    current.native_recheck.as_mut().unwrap().inventory_path = Some("/v1/models".into());
    current
        .native_recheck
        .as_mut()
        .unwrap()
        .protocol_header_semantics
        .required_headers[0]
        .1 = "value\r\ninjected: true".into();
    assert_eq!(
        current.validate(),
        Err(ComputeManagementErrorV2::InvalidTarget)
    );
}
