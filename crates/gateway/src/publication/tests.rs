use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use hiroute_gateway_core::core::execution_plan::ResolvedTargetBindingId;
use hiroute_gateway_core::core::publication::InstallError;

use super::*;
use crate::server::dispatch::GatewayRequestAuthority;
use crate::server::request_plan::IngressProtocol;

static DIRECTORY_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[test]
fn production_body_plan_admits_large_requests_without_a_proxy_byte_quota() {
    use hiroute_gateway_core::runtime::body::{BodyDirection, BodyPlanExecutor};
    let aggregate = super::compiler::compile(&snapshot(1, "renderer")).unwrap();
    let plan = &aggregate.aliases["alpha"]
        .execution
        .request_plan
        .logical_request
        .body_plan;
    let mut owner =
        BodyPlanExecutor::new(BodyDirection::LogicalRequest, plan.clone(), usize::MAX).unwrap();
    let bytes = 32 * 1024 * 1024;
    owner.preflight_content_length(bytes).unwrap();
    for _ in 0..512 {
        owner.admit_chunk(64 * 1024).unwrap();
    }
    assert_eq!(owner.finish().unwrap(), bytes);
}

pub(super) struct TestDirectory(PathBuf);

impl TestDirectory {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "hiroute-publication-test-{}-{}",
            std::process::id(),
            DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn candidate(local_id: u32) -> CandidateBindingV1 {
    exact_test_candidate(local_id, IngressProtocol::Responses, None)
}

pub(super) fn snapshot(revision: u64, renderer: &str) -> GatewayPublicationSnapshotV3 {
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "authority",
        1,
        revision,
        renderer,
        vec![
            AliasPlanV1 {
                served_model_id: "alpha".into(),
                purpose: "alpha purpose".into(),
                agent_plan_revision: 10,
                protocols: vec![IngressProtocol::Responses],
                overall_timeout_ms: 1_000,
                max_attempts: 1,
                routing: None,
                candidates: vec![candidate(1)],
            },
            AliasPlanV1 {
                served_model_id: "beta".into(),
                purpose: "beta purpose".into(),
                agent_plan_revision: 20,
                protocols: vec![IngressProtocol::Messages],
                overall_timeout_ms: 2_000,
                max_attempts: 2,
                routing: None,
                candidates: vec![exact_test_candidate(2, IngressProtocol::Messages, None)],
            },
        ],
        vec![
            GrantV1 {
                grant_id: "grant-alpha".into(),
                generation: 1,
                bearer_token_sha256: token_sha256("token-alpha"),
                protocol: IngressProtocol::Responses,
                routes: [test_plan_route("alpha", 10)].into(),
            },
            GrantV1 {
                grant_id: "grant-beta".into(),
                generation: 1,
                bearer_token_sha256: token_sha256("token-beta"),
                protocol: IngressProtocol::Messages,
                routes: [test_plan_route("beta", 20)].into(),
            },
        ],
    )
    .unwrap()
}

fn rest_classifier_snapshot() -> GatewayPublicationSnapshotV3 {
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "authority",
        1,
        1,
        "renderer",
        vec![AliasPlanV1 {
            served_model_id: "smart".into(),
            purpose: "smart".into(),
            agent_plan_revision: 10,
            protocols: vec![IngressProtocol::Responses],
            overall_timeout_ms: 5_000,
            max_attempts: 2,
            routing: Some(AliasRoutingV1 {
                agent_plan_id: "legacy/smart".into(),
                plan_display_name: Some("Smart".into()),
                request_owned: AliasRequestOwnedRouteV1::Classified {
                    classifier: AliasComplexityClassifierV1 {
                        revision: "classifier/v1".into(),
                        mode: hiroute_domain::ComplexityClassifierModeV1::Rest {
                            endpoint: "http://127.0.0.1:4317/v1/decisions".into(),
                            timeout_ms: hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
                            auth_header: None,
                        },
                        user_keywords: vec!["complex".into()],
                    },
                    simple_groups: vec![AliasGroupIdV1::Economy],
                    complex_groups: vec![AliasGroupIdV1::Primary],
                },
                groups: vec![
                    AliasModelGroupV1 {
                        group_id: AliasGroupIdV1::Economy,
                        candidate_local_ids: vec![1],
                    },
                    AliasModelGroupV1 {
                        group_id: AliasGroupIdV1::Primary,
                        candidate_local_ids: vec![2],
                    },
                ],
            }),
            candidates: vec![candidate(1), candidate(2)],
        }],
        vec![GrantV1 {
            grant_id: "grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256("token"),
            protocol: IngressProtocol::Responses,
            routes: [test_plan_route("smart", 10)].into(),
        }],
    )
    .unwrap()
}

pub(super) fn publish(
    installer: &GatewayPublicationInstaller,
    snapshot: GatewayPublicationSnapshotV3,
) {
    let GatewayPrepareOutcome::Prepared(prepared) = installer.prepare(snapshot).unwrap() else {
        panic!("unexpected duplicate")
    };
    installer.publish(prepared).unwrap();
}

fn reseal(mut snapshot: GatewayPublicationSnapshotV3) -> GatewayPublicationSnapshotV3 {
    snapshot.payload_digest.clear();
    snapshot.payload_digest = snapshot.canonical_digest().unwrap();
    snapshot
}

#[test]
fn rest_classifier_accepts_trusted_http_https_and_optional_custom_header() {
    let directory = TestDirectory::new();
    let installer =
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap();
    assert!(installer.prepare(rest_classifier_snapshot()).is_ok());

    let mut remote_http = rest_classifier_snapshot();
    let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut remote_http.aliases[0]
        .routing
        .as_mut()
        .unwrap()
        .request_owned
    else {
        panic!("expected classified source")
    };
    let hiroute_domain::ComplexityClassifierModeV1::Rest { endpoint, .. } = &mut classifier.mode
    else {
        panic!("expected REST classifier")
    };
    *endpoint = "http://classifier.example/v1/decisions".into();
    assert!(installer.prepare(reseal(remote_http)).is_ok());

    let mut remote_without_auth = rest_classifier_snapshot();
    let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut remote_without_auth.aliases
        [0]
    .routing
    .as_mut()
    .unwrap()
    .request_owned
    else {
        panic!("expected classified source")
    };
    let hiroute_domain::ComplexityClassifierModeV1::Rest { endpoint, .. } = &mut classifier.mode
    else {
        panic!("expected REST classifier")
    };
    *endpoint = "https://classifier.example/v1/decisions".into();
    assert!(installer.prepare(reseal(remote_without_auth)).is_ok());

    let mut remote_bearer = rest_classifier_snapshot();
    let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut remote_bearer.aliases[0]
        .routing
        .as_mut()
        .unwrap()
        .request_owned
    else {
        panic!("expected classified source")
    };
    let hiroute_domain::ComplexityClassifierModeV1::Rest {
        endpoint,
        auth_header,
        ..
    } = &mut classifier.mode
    else {
        panic!("expected REST classifier")
    };
    *endpoint = "https://classifier.example/v1/decisions".into();
    *auth_header = Some(hiroute_domain::ClassifierAuthHeaderV1 {
        name: "X-API-Key".into(),
        value_secret_ref: "classifier/main".into(),
    });
    assert!(installer.prepare(reseal(remote_bearer)).is_ok());

    for timeout_ms in [0, hiroute_domain::MAX_REST_CLASSIFIER_TIMEOUT_MS + 1] {
        let mut invalid_timeout = rest_classifier_snapshot();
        let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut invalid_timeout.aliases
            [0]
        .routing
        .as_mut()
        .unwrap()
        .request_owned
        else {
            panic!("expected classified source")
        };
        let hiroute_domain::ComplexityClassifierModeV1::Rest {
            timeout_ms: value, ..
        } = &mut classifier.mode
        else {
            panic!("expected REST classifier")
        };
        *value = timeout_ms;
        assert!(matches!(
            compile_classifier_mode_authority(&classifier.mode, 1),
            Err(PublicationInstallError::InvalidPlannerPolicy)
        ));
        assert!(installer.prepare(reseal(invalid_timeout)).is_err());
    }
}

#[test]
fn publication_prepare_failpoint_preserves_live_and_durable_last_good() {
    let directory = TestDirectory::new();
    let lkg = directory.path().join("publication.json");
    let installer = GatewayPublicationInstaller::open(&lkg).unwrap();
    publish(&installer, snapshot(1, "renderer-1"));

    let GatewayPrepareOutcome::Prepared(prepared) =
        installer.prepare(snapshot(2, "renderer-2")).unwrap()
    else {
        panic!("unexpected duplicate")
    };
    assert!(matches!(
        installer.publish_with_failpoint(prepared, PublicationFailpoint::BeforeDurableLkg),
        Err(PublicationInstallError::InjectedBeforeDurableLkg)
    ));
    assert_eq!(installer.active().unwrap().publication_revision(), 1);
    assert_eq!(
        GatewayPublicationInstaller::open(&lkg)
            .unwrap()
            .active()
            .unwrap()
            .publication_revision(),
        1
    );

    // Dropping an unpublished aggregate ticket must also relinquish its core
    // ticket so the same process can retry without a restart.
    publish(&installer, snapshot(2, "renderer-2"));
    assert_eq!(installer.active().unwrap().publication_revision(), 2);
    drop(installer);
    assert_eq!(
        GatewayPublicationInstaller::open(&lkg)
            .unwrap()
            .active()
            .unwrap()
            .publication_revision(),
        2
    );
}

#[test]
fn post_rename_uncertainty_fails_closed_until_restart() {
    let directory = TestDirectory::new();
    let lkg = directory.path().join("publication.json");
    let installer = GatewayPublicationInstaller::open(&lkg).unwrap();
    publish(&installer, snapshot(1, "renderer-1"));
    let GatewayPrepareOutcome::Prepared(prepared) =
        installer.prepare(snapshot(2, "renderer-2")).unwrap()
    else {
        panic!("unexpected duplicate")
    };

    let interrupted = installer
        .publish_with_failpoint(
            prepared,
            PublicationFailpoint::AfterRenameBeforeDirectoryDurability,
        )
        .unwrap_err();
    assert!(matches!(
        &interrupted,
        PublicationInstallError::InjectedAfterRenameBeforeDirectoryDurability
    ));
    assert!(interrupted.is_crash_boundary());
    assert!(
        installer.active().is_none(),
        "an uncertain durable/live split cannot continue serving the old root"
    );
    assert!(matches!(
        installer.prepare(snapshot(3, "renderer-3")),
        Err(PublicationInstallError::InstallerRequiresRestart)
    ));
    drop(installer);
    assert_eq!(
        GatewayPublicationInstaller::open(&lkg)
            .unwrap()
            .active()
            .unwrap()
            .publication_revision(),
        2
    );
}

#[test]
fn incompatible_publication_is_rejected_before_lkg_or_core_swap() {
    let directory = TestDirectory::new();
    let lkg = directory.path().join("publication.json");
    let installer = GatewayPublicationInstaller::open(&lkg).unwrap();
    publish(&installer, snapshot(1, "renderer-1"));
    let mut incompatible = snapshot(2, "renderer-2");
    incompatible.aliases[0].candidates[0].endpoint = "http://bad.example/?secret=x".into();
    incompatible.payload_digest = incompatible.canonical_digest().unwrap();

    assert!(matches!(
        installer.prepare(incompatible),
        Err(PublicationInstallError::Schema(
            PublicationSchemaError::InvalidCandidate(1)
        ))
    ));
    assert_eq!(installer.active().unwrap().publication_revision(), 1);
    drop(installer);
    assert_eq!(
        GatewayPublicationInstaller::open(&lkg)
            .unwrap()
            .active()
            .unwrap()
            .publication_revision(),
        1
    );
}

#[test]
fn managed_cpa_target_requires_exact_numeric_loopback_path_and_digest() {
    let invalid = [
        "http://localhost:4317/v1/responses",
        "http://192.0.2.1:4317/v1/responses",
        "http://user@127.0.0.1:4317/v1/responses",
        "http://127.0.0.1:4317/v1/responses?shadow=1",
        "http://127.0.0.1:4317/v1/messages",
        "http://127.0.0.1:0/v1/responses",
    ];
    for uri in invalid {
        let mut value = snapshot(1, "renderer");
        let candidate = &mut value.aliases[0].candidates[0];
        candidate.connector_runtime = hiroute_domain::ConnectorRuntimeKind::CpaBridge;
        candidate.operational_target =
            hiroute_domain::GatewayOperationalTargetV1::ManagedCpaLoopback {
                uri: uri.into(),
                runtime_epoch: 7,
                target_epoch: 9,
            };
        candidate.operational_target_digest =
            hiroute_domain::CanonicalDigest::of(&candidate.operational_target).unwrap();
        value = reseal(value);
        assert!(matches!(
            value.validate(),
            Err(PublicationSchemaError::InvalidCandidate(1))
        ));
    }

    let mut digest_drift = snapshot(1, "renderer");
    let candidate = &mut digest_drift.aliases[0].candidates[0];
    candidate.connector_runtime = hiroute_domain::ConnectorRuntimeKind::CpaBridge;
    candidate.operational_target = hiroute_domain::GatewayOperationalTargetV1::ManagedCpaLoopback {
        uri: "http://127.0.0.1:4317/v1/responses".into(),
        runtime_epoch: 7,
        target_epoch: 9,
    };
    digest_drift = reseal(digest_drift);
    assert!(matches!(
        digest_drift.validate(),
        Err(PublicationSchemaError::InvalidCandidate(1))
    ));
}

#[test]
fn managed_cpa_target_and_profile_compile_without_dns_or_identity_rewrite() {
    let directory = TestDirectory::new();
    let installer = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    let mut value = snapshot(1, "renderer");
    value.aliases[0].candidates[0] = exact_cpa_test_candidate(
        1,
        IngressProtocol::Responses,
        "https://chatgpt.com/backend-api/codex/responses",
        "127.0.0.1:4317".parse().unwrap(),
    );
    publish(&installer, reseal(value));
    let authorized = GatewayRequestAuthority::new(Arc::clone(&installer))
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer token-alpha"),
            br#"{"model":"alpha"}"#,
            std::time::Instant::now(),
        )
        .unwrap();
    let candidate = &authorized.candidates()[0];
    assert_eq!(
        candidate.endpoint.as_ref(),
        "http://127.0.0.1:4317/v1/responses"
    );
    let binding = ResolvedTargetBindingId::new(
        authorized.core_binding().plan_revision(),
        candidate.binding_local_id,
    );
    let attempt = authorized.core_binding().resolve_attempt(binding).unwrap();
    assert!(!attempt.plan().transport_target.requires_resolution());
    assert_eq!(
        attempt.plan().transport_target.addresses.as_ref(),
        &["127.0.0.1:4317".parse().unwrap()]
    );
    assert_eq!(attempt.plan().config_cell_ids.len(), 1);
    let configs = attempt.acquire_attempt_configs().unwrap();
    let sealed: CandidateBindingV1 = serde_json::from_slice(
        &configs
            .value(attempt.plan().config_cell_ids[0])
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert_eq!(
        sealed.endpoint,
        "https://chatgpt.com/backend-api/codex/responses"
    );
    assert_eq!(
        sealed.operational_target.uri(),
        "http://127.0.0.1:4317/v1/responses"
    );
    assert_eq!(sealed.upstream_model_id, "native-model-1");
    assert_eq!(
        sealed.native_transport_model,
        "hiroute-fixture/native-model-1"
    );
    assert_eq!(
        sealed.protocol_profiles[0].capability.native_model,
        sealed.native_transport_model
    );
    assert_ne!(sealed.upstream_model_id, sealed.native_transport_model);
}

#[test]
fn authorized_request_freezes_exact_price_identity_and_protocol_usage_semantics() {
    use hiroute_domain::{
        GatewayCandidatePricingIdentityV1, InputUsageMeaningV1, OutputUsageMeaningV1,
        UsageFrameKindV1,
    };

    let directory = TestDirectory::new();
    let installer = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    let mut value = snapshot(1, "renderer");
    for alias in &mut value.aliases {
        let candidate = &mut alias.candidates[0];
        candidate.pricing_identity = Some(GatewayCandidatePricingIdentityV1 {
            source_id: format!("source-{}", candidate.local_id),
            source_identity_digest: hiroute_domain::CanonicalDigest::of_bytes(
                format!("source-{}", candidate.local_id).as_bytes(),
            ),
            model_configuration_id: candidate.protocol_profiles[0]
                .capability
                .model_configuration_id
                .clone(),
            actual_offer_ref: format!("offer-{}", candidate.local_id),
        });
    }
    publish(&installer, reseal(value));

    for (protocol, token, alias, expected_input) in [
        (
            IngressProtocol::Responses,
            "token-alpha",
            "alpha",
            InputUsageMeaningV1::IncludesExclusiveCache,
        ),
        (
            IngressProtocol::Messages,
            "token-beta",
            "beta",
            InputUsageMeaningV1::UncachedOnly,
        ),
    ] {
        let authorized = GatewayRequestAuthority::new(Arc::clone(&installer))
            .authorize_bytes(
                protocol,
                Some(&format!("Bearer {token}")),
                format!(r#"{{"model":"{alias}"}}"#).as_bytes(),
                std::time::Instant::now(),
            )
            .unwrap();
        let bindings = authorized.pricing_bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            bindings[0].source_id.as_ref(),
            if alias == "alpha" {
                "source-1"
            } else {
                "source-2"
            }
        );
        assert_eq!(
            bindings[0].actual_offer_ref.as_ref(),
            if alias == "alpha" {
                "offer-1"
            } else {
                "offer-2"
            }
        );
        assert_eq!(
            bindings[0].usage_semantics.frame_kind,
            UsageFrameKindV1::Cumulative
        );
        assert_eq!(bindings[0].usage_semantics.input, expected_input);
        assert_eq!(
            bindings[0].usage_semantics.output,
            OutputUsageMeaningV1::IncludesReasoning
        );
        assert!(bindings[0].usage_semantics.cache_buckets_exclusive);
    }
}

#[test]
fn managed_cpa_ipv6_target_compiles_as_an_exact_socket_without_dns() {
    let mut value = snapshot(1, "renderer");
    value.aliases[0].candidates[0] = exact_cpa_test_candidate(
        1,
        IngressProtocol::Responses,
        "https://chatgpt.com/backend-api/codex/responses",
        "[::1]:4317".parse().unwrap(),
    );
    let compiled = super::compiler::compile(&reseal(value)).unwrap();
    let plan = compiled
        .envelope
        .attempt_plan_index_handle
        .plans()
        .find(|(_, plan)| plan.stable_target_key.as_str() == "target-1")
        .unwrap()
        .1;
    assert!(!plan.transport_target.requires_resolution());
    assert_eq!(
        plan.transport_target.addresses.as_ref(),
        &["[::1]:4317".parse().unwrap()]
    );
}

#[test]
fn ordered_credential_refs_are_identical_in_candidate_authority_and_attempt_plan() {
    let directory = TestDirectory::new();
    let installer = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    let mut value = snapshot(1, "renderer");
    value.aliases[0].candidates[0].credential_refs =
        vec!["credential-a".into(), "credential-b".into()];
    publish(&installer, reseal(value));
    let authorized = GatewayRequestAuthority::new(Arc::clone(&installer))
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer token-alpha"),
            br#"{"model":"alpha"}"#,
            std::time::Instant::now(),
        )
        .unwrap();
    let authority = &authorized.candidates()[0];
    assert_eq!(
        authority
            .credential_refs
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>(),
        ["credential-a", "credential-b"]
    );
    let binding = ResolvedTargetBindingId::new(
        authorized.core_binding().plan_revision(),
        authority.binding_local_id,
    );
    let attempt = authorized.core_binding().resolve_attempt(binding).unwrap();
    assert_eq!(
        attempt
            .plan()
            .credential_refs
            .iter()
            .map(|credential| credential.as_str())
            .collect::<Vec<_>>(),
        ["credential-a", "credential-b"]
    );

    let mut duplicate = snapshot(1, "renderer");
    duplicate.aliases[0].candidates[0].credential_refs =
        vec!["credential-a".into(), "credential-a".into()];
    assert!(matches!(
        reseal(duplicate).validate(),
        Err(PublicationSchemaError::InvalidCandidate(1))
    ));
}

#[test]
fn profile_digest_and_unknown_critical_fact_fail_before_install() {
    let mut drift = snapshot(1, "renderer");
    drift.aliases[0].candidates[0].protocol_profiles[0]
        .capability
        .native_model = "rewritten-model".into();
    drift.aliases[0].candidates[0].protocol_profile_digest =
        hiroute_domain::CanonicalDigest::of(&drift.aliases[0].candidates[0].protocol_profiles)
            .unwrap();
    drift = reseal(drift);
    assert!(matches!(
        drift.validate(),
        Err(PublicationSchemaError::InvalidCandidate(1))
    ));

    let mut unknown = snapshot(1, "renderer");
    let candidate = &mut unknown.aliases[0].candidates[0];
    candidate.protocol_profiles[0].connector.authentication =
        hiroute_domain::GatewayCriticalFactV1::Unknown;
    candidate.protocol_profile_digest =
        hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    unknown = reseal(unknown);
    assert!(matches!(
        unknown.validate(),
        Err(PublicationSchemaError::InvalidCandidate(1))
    ));

    let mut named_header = snapshot(1, "renderer");
    let candidate = &mut named_header.aliases[0].candidates[0];
    candidate.protocol_profiles[0].connector.authentication =
        hiroute_domain::GatewayCriticalFactV1::Exact(
            hiroute_domain::GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: "x-api-key".into(),
            },
        );
    candidate.protocol_profile_digest =
        hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    named_header = reseal(named_header);
    named_header
        .validate()
        .expect("an exact named API key header is a valid native profile");

    let mut invalid_destination = snapshot(1, "renderer");
    invalid_destination.aliases[0].candidates[0].credential_destination_ref =
        "connection-option/../untrusted".into();
    invalid_destination = reseal(invalid_destination);
    assert!(matches!(
        invalid_destination.validate(),
        Err(PublicationSchemaError::InvalidCandidate(1))
    ));

    let mut unknown_field = serde_json::to_value(snapshot(1, "renderer")).unwrap();
    unknown_field["aliases"][0]["candidates"][0]["protocol_profiles"][0]["unsealed_overlay"] =
        serde_json::json!(true);
    assert!(serde_json::from_value::<GatewayPublicationSnapshotV3>(unknown_field).is_err());
}

#[path = "tests/grant_projection.rs"]
mod grant_projection;

#[test]
fn request_budget_product_bounds_reject_overflow_and_unbounded_attempts() {
    let mut maximum = snapshot(1, "renderer");
    maximum.aliases[0].overall_timeout_ms = 3_600_000;
    maximum.aliases[0].max_attempts = 6;
    assert!(reseal(maximum).validate().is_ok());

    let mut above_timeout_bound = snapshot(1, "renderer");
    above_timeout_bound.aliases[0].overall_timeout_ms = 3_600_001;
    assert!(matches!(
        above_timeout_bound.validate(),
        Err(PublicationSchemaError::InvalidAlias(alias)) if alias == "alpha"
    ));

    let mut above_attempt_bound = snapshot(1, "renderer");
    above_attempt_bound.aliases[0].max_attempts = 7;
    assert!(matches!(
        above_attempt_bound.validate(),
        Err(PublicationSchemaError::InvalidAlias(alias)) if alias == "alpha"
    ));

    let mut excessive_timeout = snapshot(1, "renderer");
    excessive_timeout.aliases[0].overall_timeout_ms = u64::MAX;
    assert!(matches!(
        excessive_timeout.validate(),
        Err(PublicationSchemaError::InvalidAlias(alias)) if alias == "alpha"
    ));

    let mut excessive_attempts = snapshot(1, "renderer");
    excessive_attempts.aliases[0].max_attempts = u32::MAX;
    assert!(matches!(
        excessive_attempts.validate(),
        Err(PublicationSchemaError::InvalidAlias(alias)) if alias == "alpha"
    ));
}

#[test]
fn unresolved_dns_authority_has_no_executable_socket_address() {
    let directory = TestDirectory::new();
    let installer = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    publish(&installer, snapshot(1, "renderer"));
    let authority = GatewayRequestAuthority::new(Arc::clone(&installer));
    let authorized = authority
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer token-alpha"),
            br#"{"model":"alpha"}"#,
            std::time::Instant::now(),
        )
        .unwrap();
    let local_id = authorized.candidates()[0].binding_local_id;
    let binding = ResolvedTargetBindingId::new(authorized.core_binding().plan_revision(), local_id);
    let attempt = authorized.core_binding().resolve_attempt(binding).unwrap();

    assert!(attempt.plan().transport_target.addresses.is_empty());
    assert!(attempt.plan().transport_target.requires_resolution());
    assert_eq!(
        attempt.plan().transport_target.unresolved_authority(),
        Some("provider-1.invalid")
    );
}

#[test]
fn prepared_ticket_cannot_cross_installer_or_mutate_foreign_lkg() {
    let directory = TestDirectory::new();
    let first_lkg = directory.path().join("first.json");
    let second_lkg = directory.path().join("second.json");
    let first = GatewayPublicationInstaller::open(&first_lkg).unwrap();
    let second = GatewayPublicationInstaller::open(&second_lkg).unwrap();
    let GatewayPrepareOutcome::Prepared(prepared) = first.prepare(snapshot(1, "renderer")).unwrap()
    else {
        panic!("unexpected duplicate")
    };

    assert!(matches!(
        second.publish(prepared),
        Err(PublicationInstallError::ForeignPreparedPublication)
    ));
    assert!(second.active().is_none());
    assert!(!second_lkg.exists());

    // The rejected ticket is relinquished at its owning installer as well.
    publish(&first, snapshot(1, "renderer"));
    assert_eq!(first.active().unwrap().publication_revision(), 1);
}

#[test]
fn grant_catalog_is_exactly_scoped_and_etag_supports_exact_304() {
    let directory = TestDirectory::new();
    let installer = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    publish(&installer, snapshot(1, "renderer-1"));
    let catalog = GatewayCatalog::new(Arc::clone(&installer));

    let alpha = catalog.get(Some("Bearer token-alpha"), None).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&alpha.body).unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 1);
    assert_eq!(body["data"][0]["id"], "alpha");
    assert!(!String::from_utf8_lossy(&alpha.body).contains("beta"));

    let unchanged = catalog
        .get(Some("Bearer token-alpha"), Some(&alpha.etag))
        .unwrap();
    assert_eq!(unchanged.status, http::StatusCode::NOT_MODIFIED);
    assert!(unchanged.body.is_empty());
    assert_eq!(unchanged.etag, alpha.etag);

    let beta = catalog.get(Some("Bearer token-beta"), None).unwrap();
    assert_ne!(beta.etag, alpha.etag);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&beta.body).unwrap()["data"][0]["id"],
        "beta"
    );
}

#[test]
fn selected_request_drops_unrelated_alias_closure_at_authority_boundary() {
    let directory = TestDirectory::new();
    let installer = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap(),
    );
    publish(&installer, snapshot(1, "renderer-1"));
    let old = installer.active().unwrap();
    let unrelated = Arc::downgrade(&old.aliases["beta"].execution.request_plan);
    let authority = GatewayRequestAuthority::new(Arc::clone(&installer));
    let selected = authority
        .authorize_bytes(
            IngressProtocol::Responses,
            Some("Bearer token-alpha"),
            br#"{"model":"alpha"}"#,
            std::time::Instant::now(),
        )
        .unwrap();

    drop(old);
    publish(&installer, snapshot(2, "renderer-2"));
    assert!(
        unrelated.upgrade().is_none(),
        "an alpha request must not retain beta's retired request plan"
    );
    assert_eq!(selected.publication_revision(), 1);
}

#[test]
fn fixed_grants_isolate_same_name_and_revocation_without_hidden_plans() {
    let directory = TestDirectory::new();
    let lkg = directory.path().join("publication.json");
    let installer = Arc::new(GatewayPublicationInstaller::open(&lkg).unwrap());
    let grants: Vec<_> = (1..=2)
        .map(|id| {
            let binding = candidate(id);
            GrantV1 {
                grant_id: format!("fixed-{id}"),
                generation: 1,
                bearer_token_sha256: token_sha256(&format!("fixed-token-{id}")),
                protocol: IngressProtocol::Responses,
                routes: [(
                    "Native.Model/v1".into(),
                    ModelRouteV2::Fixed {
                        binding_digest: hiroute_domain::CanonicalDigest::of(&binding).unwrap(),
                        binding: Box::new(binding),
                        overall_timeout_ms: 1000,
                        max_attempts: 2,
                    },
                )]
                .into(),
            }
        })
        .collect();
    let snapshot = GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "fixed-authority",
        1,
        1,
        "renderer",
        vec![],
        grants.clone(),
    )
    .unwrap();
    for change_binding in [false, true] {
        let mut changed = snapshot.clone();
        let ModelRouteV2::Fixed {
            binding,
            binding_digest,
            ..
        } = changed.grants[0].routes.values_mut().next().unwrap()
        else {
            panic!("expected fixed route");
        };
        if change_binding {
            binding.credential_refs[0] = "another-account-credential".into();
        } else {
            *binding_digest = hiroute_domain::CanonicalDigest::of_bytes(b"unrelated-binding");
        }
        changed.payload_digest = changed.canonical_digest().unwrap();
        assert!(matches!(
            changed.validate(),
            Err(PublicationSchemaError::InvalidGrant(_))
        ));
        assert!(installer.prepare(changed).is_err());
        assert!(installer.active().is_none());
    }
    publish(&installer, snapshot);
    assert!(installer.active().unwrap().aliases.is_empty());
    let authority = GatewayRequestAuthority::new(Arc::clone(&installer));
    let now = std::time::Instant::now();
    for id in 1..=2 {
        let token = format!("Bearer fixed-token-{id}");
        let authorized = authority
            .authorize_bytes(
                IngressProtocol::Responses,
                Some(&token),
                br#"{"model":"Native.Model/v1"}"#,
                now,
            )
            .unwrap();
        assert_eq!(authorized.served_model_id(), "Native.Model/v1");
        assert_eq!(authorized.agent_plan_revision(), None);
        assert!(authorized.receipt().plan_display_name.is_none());
        assert!(matches!(
            authorized.receipt().route,
            hiroute_domain::ModelRequestRouteV2::Fixed { .. }
        ));
        assert_eq!(authorized.candidates().len(), 1);
        assert_eq!(authorized.candidates()[0].binding_local_id, id);
        assert_eq!(authorized.max_attempts(), 2);
        let binding = ResolvedTargetBindingId::new(authorized.core_binding().plan_revision(), id);
        let attempt = authorized.core_binding().resolve_attempt(binding).unwrap();
        assert_eq!(
            attempt.plan().credential_refs[0].as_str(),
            format!("credential-{id}")
        );
        let other = ResolvedTargetBindingId::new(authorized.core_binding().plan_revision(), 3 - id);
        assert!(authorized.core_binding().resolve_attempt(other).is_err());
        let catalog = GatewayCatalog::new(Arc::clone(&installer));
        let response = catalog.get(Some(&token), None).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 1);
        assert!(body["data"][0].get("agent_plan_revision").is_none());
        assert!(body["data"][0].get("purpose").is_none());
    }
    let pinned = authority
        .begin_at(
            IngressProtocol::Responses,
            Some("Bearer fixed-token-1"),
            now,
        )
        .unwrap();
    publish(
        &installer,
        GatewayPublicationSnapshotV3::seal(
            "personal/default",
            "fixed-authority",
            1,
            2,
            "renderer",
            vec![],
            vec![grants[1].clone()],
        )
        .unwrap(),
    );
    assert!(
        authority
            .begin_at(
                IngressProtocol::Responses,
                Some("Bearer fixed-token-1"),
                now
            )
            .is_err()
    );
    assert_eq!(
        pinned
            .authorize_alias("Native.Model/v1", now)
            .unwrap()
            .publication_revision(),
        1
    );
    drop(authority);
    drop(installer);
    let reopened = Arc::new(GatewayPublicationInstaller::open(&lkg).unwrap());
    assert!(reopened.active().unwrap().aliases.is_empty());
    let authority = GatewayRequestAuthority::new(reopened);
    assert!(
        authority
            .begin_at(
                IngressProtocol::Responses,
                Some("Bearer fixed-token-1"),
                now
            )
            .is_err()
    );
    assert!(
        authority
            .authorize_bytes(
                IngressProtocol::Responses,
                Some("Bearer fixed-token-2"),
                br#"{"model":"Native.Model/v1"}"#,
                now
            )
            .is_ok()
    );
}

#[test]
fn production_composition_opens_without_an_oracle_fixture() {
    let directory = TestDirectory::new();
    let launcher = crate::server::GatewayLauncher::production(
        "127.0.0.1:0".parse().unwrap(),
        directory.path().join("missing-lkg.json"),
    );
    assert!(launcher.is_ok());
    assert!(!directory.path().join("missing-lkg.json").exists());
}
