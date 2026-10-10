mod cpa_async;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hiroute_application::publication::{PublicationTargetError, PublicationTargetPort};
use hiroute_cpa_bridge::{
    CpaAttemptError, CpaDownstreamCredentialCapability, CpaDownstreamCredentialPort,
    ExactCpaCredentialRequest,
};
use hiroute_domain::{
    CanonicalDigest, ComputeRuntimeStateStoreV1, ConnectorRuntimeKind,
    GatewayAuthenticationSemanticsV1, GatewayOperationalTargetV1, HeaderSecretLeaseRequestV1,
    NativeCredentialAuthorityV1, NativeCredentialAuthorizationCapabilityV1,
    NativeCredentialCapabilityErrorV1, NativeCredentialLeaseRequestV1, NativeCredentialLeaseV1,
    PortError as ProductPortError, PortErrorCode, PortResult, RuntimeProbeAcquireOutcomeV1,
    RuntimeProbeLeaseRequestV1, RuntimeProbeLeaseV1, RuntimeStateIdentityV1, RuntimeStateV1,
    SensitiveAuthorizationTargetV1,
};
use hiroute_gateway::ports::{
    CredentialLeaseRequest, ExecutionScope, HeaderSecretLeaseRequest, ProbeLeaseOutcome,
    RuntimeHealth, RuntimeStateEntry, RuntimeStateKey,
};
use hiroute_gateway::server::composition::{
    CredentialResolver, RuntimePublicationFeed, RuntimeStateStore,
};
use hiroute_gateway::server::core_runtime::profiles::AuthenticationSemantics;
use hiroute_gateway::server::publication::GatewayPublicationInstaller;
use hiroute_gateway::server::request_plan::IngressProtocol;
use http::{HeaderMap, header};
use tokio_util::sync::CancellationToken;

use super::{GatewayCredentialResolver, GatewayPublicationAdapter, GatewayRuntimeStateStore};

mod observation_collection;
mod publication_scope;

fn scope() -> ExecutionScope {
    ExecutionScope::new(
        Instant::now() + Duration::from_secs(5),
        CancellationToken::new(),
    )
}

#[test]
fn gateway_ports_publication_installs_one_verified_aggregate_and_pins_it() {
    let golden = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../e2e/product/fixtures/routing/current-publication.v3.json"),
    )
    .unwrap();
    let publication =
        serde_json::from_str::<hiroute_domain::GatewayPublicationV1>(&golden).unwrap();
    let record = hiroute_domain::PublicationRecordV1::from_publication(
        publication.workspace_id.clone(),
        &publication,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let installer = Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("gateway-lkg.json")).unwrap(),
    );
    let adapter = GatewayPublicationAdapter::new(Arc::clone(&installer));

    adapter.activate_verified(&record).unwrap();
    assert!(RuntimePublicationFeed::pin(&adapter).is_none());
    assert!(adapter.verify_installed(&record).unwrap());
    adapter.resume_requests().unwrap();
    let pinned = RuntimePublicationFeed::pin(&adapter).unwrap();
    assert_eq!(
        pinned.publication_revision(),
        publication.publication_revision.get()
    );
    assert_eq!(pinned.authority_id(), publication.authority_id);
    adapter.activate_verified(&record).unwrap();
    assert_eq!(
        RuntimePublicationFeed::pin(&adapter)
            .unwrap()
            .payload_digest(),
        pinned.payload_digest()
    );

    let mut tampered = record;
    std::sync::Arc::make_mut(&mut tampered.bytes).push(b' ');
    assert_eq!(
        adapter.activate_verified(&tampered),
        Err(PublicationTargetError::VerificationFailed)
    );
}

#[tokio::test]
async fn gateway_ports_native_credential_is_request_scoped_and_capability_only() {
    let native = Arc::new(FakeNativeAuthority::default());
    let cpa = Arc::new(FakeCpaAuthority::default());
    let resolver = GatewayCredentialResolver::new(Arc::clone(&native), cpa);
    let target = GatewayOperationalTargetV1::RegisteredHttps {
        uri: "https://api.anthropic.com/v1/messages".into(),
    };
    let target_digest = CanonicalDigest::of(&target).unwrap().to_string();
    let profile_digest = CanonicalDigest::of_bytes(b"native-profile").to_string();
    let authentication = AuthenticationSemantics::Bearer;
    let lease = resolver
        .lease_exact(
            CredentialLeaseRequest {
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
                operational_target_digest: &target_digest,
                runtime_epoch: None,
                target_epoch: None,
                protocol_profile_digest: &profile_digest,
                request_path: "/v1/messages",
                authentication: &authentication,
            },
            &scope(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(native.requests.lock().unwrap().len(), 1);
    assert_eq!(lease.credential_ref(), "credential/claude");
    assert_eq!(lease.generation(), 7);
    let mut headers = HeaderMap::new();
    lease.apply_authorization(&mut headers).unwrap();
    assert_eq!(headers[header::AUTHORIZATION], "Bearer native-sentinel");
    assert!(headers[header::AUTHORIZATION].is_sensitive());

    let header_auth = AuthenticationSemantics::ApiKeyHeader {
        header: "x-api-key".into(),
    };
    let header_lease = resolver
        .lease_exact(
            CredentialLeaseRequest {
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
                operational_target_digest: &target_digest,
                runtime_epoch: None,
                target_epoch: None,
                protocol_profile_digest: &profile_digest,
                request_path: "/v1/messages",
                authentication: &header_auth,
            },
            &scope(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(native.requests.lock().unwrap().len(), 2);
    let mut header_map = HeaderMap::new();
    header_lease.apply_authorization(&mut header_map).unwrap();
    assert_eq!(header_map["x-api-key"], "native-sentinel");
    assert!(header_map["x-api-key"].is_sensitive());
    assert!(!header_map.contains_key(header::AUTHORIZATION));

    let none_auth = AuthenticationSemantics::None;
    let no_auth_resolution = resolver
        .lease_exact(
            CredentialLeaseRequest {
                stable_binding_id: "binding/claude",
                credential_ref: "credential/none/source-local",
                credential_destination_ref: "compute-target/source-local",
                excluded_key_ids: &[],
                connector_runtime: ConnectorRuntimeKind::BuiltinNative,
                connector_id: "builtin-anthropic",
                upstream_protocol: IngressProtocol::Messages,
                upstream_model_id: "claude-sonnet",
                native_transport_model: "claude-sonnet",
                logical_endpoint: target.uri(),
                operational_target: target.uri(),
                operational_target_digest: &target_digest,
                runtime_epoch: None,
                target_epoch: None,
                protocol_profile_digest: &profile_digest,
                request_path: "/v1/messages",
                authentication: &none_auth,
            },
            &scope(),
        )
        .await;
    assert!(no_auth_resolution.is_err());
    assert_eq!(native.requests.lock().unwrap().len(), 2);

    let rewritten_model = resolver
        .lease_exact(
            CredentialLeaseRequest {
                stable_binding_id: "binding/claude",
                credential_ref: "credential/claude",
                credential_destination_ref: "connection-option/claude.custom.v1",
                excluded_key_ids: &[],
                connector_runtime: ConnectorRuntimeKind::BuiltinNative,
                connector_id: "builtin-anthropic",
                upstream_protocol: IngressProtocol::Messages,
                upstream_model_id: "claude-sonnet",
                native_transport_model: "rewritten-transport-model",
                logical_endpoint: target.uri(),
                operational_target: target.uri(),
                operational_target_digest: &target_digest,
                runtime_epoch: None,
                target_epoch: None,
                protocol_profile_digest: &profile_digest,
                request_path: "/v1/messages",
                authentication: &authentication,
            },
            &scope(),
        )
        .await;
    assert!(rewritten_model.is_err());
    assert_eq!(native.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn gateway_ports_header_secret_keeps_the_narrow_exact_contract() {
    let native = Arc::new(FakeNativeAuthority::default());
    let resolver =
        GatewayCredentialResolver::new(Arc::clone(&native), Arc::new(FakeCpaAuthority::default()));
    let lease = resolver
        .lease_header_secret(
            HeaderSecretLeaseRequest {
                secret_ref: "credential/classifier-main",
                header_name: "x-classifier-key",
            },
            &scope(),
        )
        .await
        .unwrap()
        .unwrap();

    let requests = native.header_secret_requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].secret_id, "credential/classifier-main");
    assert_eq!(requests[0].header_name, "x-classifier-key");
    drop(requests);
    let mut headers = HeaderMap::new();
    lease.apply_authorization(&mut headers).unwrap();
    assert_eq!(headers["x-classifier-key"], "native-sentinel");
    assert!(headers["x-classifier-key"].is_sensitive());
}

#[tokio::test]
async fn gateway_ports_cpa_forwards_dual_models_numeric_target_and_epochs_once() {
    let native = Arc::new(FakeNativeAuthority::default());
    let cpa = Arc::new(FakeCpaAuthority::default());
    let resolver = GatewayCredentialResolver::new(native, Arc::clone(&cpa));
    let target = GatewayOperationalTargetV1::ManagedCpaLoopback {
        uri: "http://127.0.0.1:43129/v1/responses".into(),
        runtime_epoch: 13,
        target_epoch: 21,
    };
    let target_digest = CanonicalDigest::of(&target).unwrap().to_string();
    let profile_digest = CanonicalDigest::of_bytes(b"cpa-profile").to_string();
    let authentication = AuthenticationSemantics::Bearer;
    let result = resolver
        .lease_exact(
            CredentialLeaseRequest {
                stable_binding_id: "binding/codex",
                credential_ref: "credential/cpa/account-a",
                credential_destination_ref: "connection-option/cpa.codex.v1",
                excluded_key_ids: &[Arc::from("cpa-downstream/12/20")],
                connector_runtime: ConnectorRuntimeKind::CpaBridge,
                connector_id: "connector.cpa.codex",
                upstream_protocol: IngressProtocol::Responses,
                upstream_model_id: "gpt-5.4",
                native_transport_model: "hiroute-account/gpt-5.4",
                logical_endpoint: "https://chatgpt.com/backend-api/codex/responses",
                operational_target: target.uri(),
                operational_target_digest: &target_digest,
                runtime_epoch: Some(13),
                target_epoch: Some(21),
                protocol_profile_digest: &profile_digest,
                request_path: "/v1/responses",
                authentication: &authentication,
            },
            &scope(),
        )
        .await
        .unwrap();
    assert!(result.is_none());
    {
        let calls = cpa.requests.lock().unwrap();
        assert_eq!(
            calls.as_slice(),
            &[CpaRequestSnapshot {
                credential_id: "credential/cpa/account-a".into(),
                connector_id: "connector.cpa.codex".into(),
                upstream_model_id: "gpt-5.4".into(),
                native_transport_model: "hiroute-account/gpt-5.4".into(),
                address: "127.0.0.1:43129".into(),
                request_path: "/v1/responses".into(),
                runtime_epoch: 13,
                target_epoch: 21,
                excluded_key_ids: vec!["cpa-downstream/12/20".into()],
            }]
        );
    }

    let stale_epoch = resolver
        .lease_exact(
            CredentialLeaseRequest {
                stable_binding_id: "binding/codex",
                credential_ref: "credential/cpa/account-a",
                credential_destination_ref: "connection-option/cpa.codex.v1",
                excluded_key_ids: &[],
                connector_runtime: ConnectorRuntimeKind::CpaBridge,
                connector_id: "connector.cpa.codex",
                upstream_protocol: IngressProtocol::Responses,
                upstream_model_id: "gpt-5.4",
                native_transport_model: "hiroute-account/gpt-5.4",
                logical_endpoint: "https://chatgpt.com/backend-api/codex/responses",
                operational_target: target.uri(),
                operational_target_digest: &target_digest,
                runtime_epoch: Some(12),
                target_epoch: Some(21),
                protocol_profile_digest: &profile_digest,
                request_path: "/v1/responses",
                authentication: &authentication,
            },
            &scope(),
        )
        .await;
    assert!(stale_epoch.is_err());
    assert_eq!(cpa.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn gateway_ports_runtime_state_preserves_durable_cas_and_typed_probe_outcomes() {
    let backend = SharedRuntimeStore::default();
    let restarted_backend = backend.clone();
    let adapter = GatewayRuntimeStateStore::new(backend).unwrap();
    let key =
        RuntimeStateKey::credential("binding/runtime", "credential/runtime", "key/runtime", 4);
    let now = Instant::now();
    assert_eq!(
        adapter.read_exact(&key, &scope()).await.unwrap(),
        RuntimeStateEntry::default()
    );
    let cooling = RuntimeStateEntry {
        generation: 1,
        health: RuntimeHealth::CoolingDown {
            until: now - Duration::from_millis(1),
        },
        probe_lease_until: None,
        transient_backoff_step: 0,
    };
    assert_eq!(
        adapter
            .compare_and_swap_exact(&key, 0, cooling, &scope())
            .await
            .unwrap(),
        hiroute_gateway::ports::CasOutcome::Applied { generation: 1 }
    );
    assert_eq!(
        adapter
            .acquire_probe_lease_exact(&key, 0, Instant::now(), Duration::from_secs(1), &scope(),)
            .await
            .unwrap(),
        ProbeLeaseOutcome::Conflict
    );
    assert_eq!(
        adapter
            .acquire_probe_lease_exact(&key, 1, Instant::now(), Duration::from_secs(1), &scope(),)
            .await
            .unwrap(),
        ProbeLeaseOutcome::Acquired { generation: 2 }
    );
    assert_eq!(
        adapter
            .acquire_probe_lease_exact(&key, 2, Instant::now(), Duration::from_secs(1), &scope(),)
            .await
            .unwrap(),
        ProbeLeaseOutcome::Busy
    );
    let active = RuntimeStateEntry {
        generation: 3,
        health: RuntimeHealth::Active,
        probe_lease_until: None,
        transient_backoff_step: 0,
    };
    assert_eq!(
        adapter
            .compare_and_swap_exact(&key, 2, active.clone(), &scope())
            .await
            .unwrap(),
        hiroute_gateway::ports::CasOutcome::Applied { generation: 3 }
    );
    let restarted = GatewayRuntimeStateStore::new(restarted_backend).unwrap();
    let persisted = restarted.read_exact(&key, &scope()).await.unwrap();
    assert_eq!(persisted.generation, active.generation);
    assert_eq!(persisted.health, active.health);
    assert_eq!(persisted.probe_lease_until, None);
}

#[derive(Default)]
struct FakeNativeAuthority {
    requests: Mutex<Vec<NativeCredentialLeaseRequestV1>>,
    header_secret_requests: Mutex<Vec<HeaderSecretLeaseRequestV1>>,
}

impl NativeCredentialAuthorityV1 for FakeNativeAuthority {
    fn lease_native_credential(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        request.validate().map_err(|_| {
            ProductPortError::new(PortErrorCode::InvalidData, "test.native.request")
        })?;
        self.requests.lock().unwrap().push(request.clone());
        NativeCredentialLeaseV1::issue(
            request.credential_id.clone(),
            "native-key",
            7,
            Arc::new(FakeNativeCapability {
                authentication: request.authentication.clone(),
            }),
        )
        .map(Some)
        .map_err(|_| ProductPortError::new(PortErrorCode::Corrupt, "test.native.lease"))
    }

    fn lease_header_secret(
        &self,
        request: &HeaderSecretLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        request.validate().map_err(|_| {
            ProductPortError::new(PortErrorCode::InvalidData, "test.header_secret.request")
        })?;
        self.header_secret_requests
            .lock()
            .unwrap()
            .push(request.clone());
        NativeCredentialLeaseV1::issue(
            request.secret_id.clone(),
            "header-secret-key",
            7,
            Arc::new(FakeNativeCapability {
                authentication: GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                    header: request.header_name.clone(),
                },
            }),
        )
        .map(Some)
        .map_err(|_| ProductPortError::new(PortErrorCode::Corrupt, "test.header_secret.lease"))
    }
}

struct FakeNativeCapability {
    authentication: GatewayAuthenticationSemanticsV1,
}

impl NativeCredentialAuthorizationCapabilityV1 for FakeNativeCapability {
    fn apply_authorization(
        &self,
        target: &mut dyn SensitiveAuthorizationTargetV1,
    ) -> Result<(), NativeCredentialCapabilityErrorV1> {
        match &self.authentication {
            GatewayAuthenticationSemanticsV1::Bearer => {
                target.set_sensitive_authorization(b"Bearer native-sentinel")
            }
            GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } => {
                target.set_sensitive_header(header, b"native-sentinel")
            }
            GatewayAuthenticationSemanticsV1::None => {
                Err(NativeCredentialCapabilityErrorV1::Rejected)
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CpaRequestSnapshot {
    credential_id: String,
    connector_id: String,
    upstream_model_id: String,
    native_transport_model: String,
    address: String,
    request_path: String,
    runtime_epoch: u64,
    target_epoch: u64,
    excluded_key_ids: Vec<String>,
}

#[derive(Default)]
struct FakeCpaAuthority {
    requests: Mutex<Vec<CpaRequestSnapshot>>,
}

impl CpaDownstreamCredentialPort for FakeCpaAuthority {
    fn lease_downstream_capability(
        &self,
        request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<CpaDownstreamCredentialCapability>, CpaAttemptError> {
        self.requests.lock().unwrap().push(CpaRequestSnapshot {
            credential_id: request.credential_id.into(),
            connector_id: request.connector_id.into(),
            upstream_model_id: request.upstream_model_id.into(),
            native_transport_model: request.native_transport_model.into(),
            address: request.address.to_string(),
            request_path: request.request_path.into(),
            runtime_epoch: request.runtime_epoch,
            target_epoch: request.target_epoch,
            excluded_key_ids: request
                .excluded_key_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
        });
        Ok(None)
    }
}

#[derive(Clone, Default)]
struct SharedRuntimeStore {
    state: Arc<Mutex<Option<RuntimeStateV1>>>,
}

impl ComputeRuntimeStateStoreV1 for SharedRuntimeStore {
    fn runtime_state(
        &self,
        identity: &RuntimeStateIdentityV1,
    ) -> PortResult<Option<RuntimeStateV1>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .as_ref()
            .filter(|state| state.identity() == identity)
            .cloned())
    }

    fn compare_and_set_runtime_state(
        &self,
        expected_generation: u64,
        state: &RuntimeStateV1,
    ) -> PortResult<()> {
        let mut current = self.state.lock().unwrap();
        match current.as_ref() {
            Some(value) => value
                .validate_direct_successor(state)
                .map_err(|_| conflict())?,
            None if expected_generation == 0 && state.generation() == 1 => {}
            None => return Err(conflict()),
        }
        *current = Some(state.clone());
        Ok(())
    }

    fn acquire_runtime_probe(
        &self,
        identity: &RuntimeStateIdentityV1,
        expected_generation: u64,
        request: &RuntimeProbeLeaseRequestV1,
    ) -> PortResult<RuntimeProbeAcquireOutcomeV1> {
        let mut current = self.state.lock().unwrap();
        let Some(value) = current.as_ref() else {
            return Ok(RuntimeProbeAcquireOutcomeV1::Conflict);
        };
        if value.identity() != identity || value.generation() != expected_generation {
            return Ok(RuntimeProbeAcquireOutcomeV1::Conflict);
        }
        let Ok(next) = value.acquire_probe(expected_generation, request) else {
            return Ok(RuntimeProbeAcquireOutcomeV1::Busy);
        };
        *current = Some(next.clone());
        Ok(RuntimeProbeAcquireOutcomeV1::Acquired(next))
    }

    fn complete_runtime_probe(
        &self,
        lease: &RuntimeProbeLeaseV1,
        state: &RuntimeStateV1,
    ) -> PortResult<()> {
        let mut current = self.state.lock().unwrap();
        current
            .as_ref()
            .ok_or_else(conflict)?
            .validate_probe_successor(lease, state)
            .map_err(|_| conflict())?;
        *current = Some(state.clone());
        Ok(())
    }
}

fn conflict() -> ProductPortError {
    ProductPortError::new(PortErrorCode::Conflict, "test.runtime.conflict")
}

#[test]
fn gateway_ports_module_exposes_six_independent_adapter_types() {
    fn marker<T>() {}
    marker::<GatewayPublicationAdapter>();
    marker::<GatewayCredentialResolver<FakeNativeAuthority, FakeCpaAuthority>>();
    marker::<GatewayRuntimeStateStore<SharedRuntimeStore>>();
    marker::<super::GatewayLifecycleTelemetrySink<FakeLifecycleReceiver>>();
    marker::<super::GatewayExecutionFactSink<FakeObservationWriter>>();
    marker::<super::GatewayConversationContentSink<FakeObservationWriter>>();
}

struct FakeLifecycleReceiver;

impl hiroute_domain::LifecycleTelemetryReceiverV2 for FakeLifecycleReceiver {
    fn receive_lifecycle(
        &self,
        _envelope: &hiroute_domain::LifecycleFactEnvelopeV2,
    ) -> PortResult<hiroute_domain::ObservationFeedback> {
        Err(ProductPortError::new(
            PortErrorCode::Unavailable,
            "test.lifecycle",
        ))
    }

    fn receive_lifecycle_gap(
        &self,
        _heartbeat: &hiroute_domain::ObservationGapHeartbeatV1,
    ) -> PortResult<hiroute_domain::ObservationFeedback> {
        Err(ProductPortError::new(
            PortErrorCode::Unavailable,
            "test.lifecycle",
        ))
    }
}

struct FakeObservationWriter;

impl hiroute_observation::writer::ObservationCommitPort for FakeObservationWriter {
    fn ingest_fact(
        &self,
        _envelope: &hiroute_domain::ExecutionFactEnvelopeV1,
        _channel_losses: &[hiroute_domain::LossNoticeV1],
    ) -> Result<
        hiroute_observation::writer::IngestOutcome,
        hiroute_observation::writer::ObservationStoreError,
    > {
        Err(hiroute_observation::writer::ObservationStoreError::ActivityUnavailable)
    }

    fn ingest_content(
        &self,
        _envelope: &hiroute_domain::ConversationContentEnvelopeV1,
        _channel_losses: &[hiroute_domain::LossNoticeV1],
    ) -> Result<
        hiroute_observation::writer::IngestOutcome,
        hiroute_observation::writer::ObservationStoreError,
    > {
        Err(hiroute_observation::writer::ObservationStoreError::ContentUnavailable)
    }

    fn ingest_gap_heartbeat(
        &self,
        _heartbeat: &hiroute_domain::ObservationGapHeartbeatV1,
    ) -> Result<
        hiroute_observation::writer::IngestOutcome,
        hiroute_observation::writer::ObservationStoreError,
    > {
        Err(hiroute_observation::writer::ObservationStoreError::ActivityUnavailable)
    }

    fn record_losses(
        &self,
        _channel: hiroute_domain::ObservationChannel,
        _stream: &hiroute_domain::ObservationStreamV1,
        _losses: &[hiroute_domain::LossNoticeV1],
    ) -> Result<(), hiroute_observation::writer::ObservationStoreError> {
        Ok(())
    }
}
