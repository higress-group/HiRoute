use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

use hiroute_domain::{
    CanonicalDigest, ConnectorRegistryBundleV1, ReleaseFactsManifestV2, ReleaseModelDataBundleV2,
    SourceOrigin, UpstreamProtocol,
};
use hiroute_integrations::{CpaSupervisorPort, TrustedReleaseCatalog};
use parking_lot::{Condvar, Mutex};
use semver::Version;
use serde_json::json;

use super::*;
use crate::accounts::{
    AccountDiscoveryError, AccountSnapshotRecord, CpaControlPlane, ManagedAccountIdentity,
};
use crate::artifact::{PinnedCpaArtifact, PinnedCpaBinaryLocator};
use crate::config::{
    CpaManagedConfigContract, InstanceSecrets, ensure_private_dir, private_atomic_write,
    validate_private_file,
};
use crate::http::{LoopbackRequest, request};
use crate::{
    BorrowedCodexAuthSpec, CpaAccountKind, CpaArtifactError, CpaAttemptError,
    CpaDownstreamCredentialPort, CpaProcessError, CpaProfileBinding, ExactCpaAttemptRequest,
    ExactCpaCredentialRequest,
};
use hiroute_diagnostics::runtime::{DiagnosticRuntime, RuntimeConfig};

// Exercise the current publication preparation API, including both fresh boundaries.
fn prepare_target(
    runtime: &ManagedCpaRuntime,
    request: ExactCpaAttemptRequest<'_>,
) -> Result<crate::PreparedCpaTarget, CpaAttemptError> {
    let batch = runtime
        .begin_routing_batch()
        .map_err(|_| CpaAttemptError::Unavailable)?;
    let target = batch.prepare_target(request)?;
    if !batch.finish().map_err(|_| CpaAttemptError::Unavailable)? {
        return Err(CpaAttemptError::Unavailable);
    }
    Ok(target)
}

const VERSION: &str = crate::MANAGED_CPA_ARTIFACT_VERSION;

#[path = "tests/borrowed_stock.rs"]
mod borrowed_stock;
#[path = "tests/cancelled_start.rs"]
mod cancelled_start;
#[path = "tests/control_races.rs"]
mod control_races;
#[path = "tests/fixture_isolation.rs"]
mod fixture_isolation;
#[path = "tests/managed_recovery.rs"]
mod managed_recovery;
#[path = "tests/owner_rollback.rs"]
mod owner_rollback;
#[path = "tests/proxy_recovery.rs"]
mod proxy_recovery;
#[path = "tests/request_io.rs"]
mod request_io;
#[path = "tests/routing_batch.rs"]
mod routing_batch;
#[path = "tests/runtime_set_discovery.rs"]
mod runtime_set_discovery;

#[derive(Clone)]
struct FixtureLocator(VerifiedCpaBinary);

impl CpaBinaryLocator for FixtureLocator {
    fn locate(&self) -> Result<VerifiedCpaBinary, CpaArtifactError> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct FakeBackend {
    next_pid: AtomicU32,
    spawn_count: AtomicUsize,
    attach_count: AtomicUsize,
    processes: Mutex<BTreeMap<u32, Arc<FakeProcess>>>,
    proxy_environments: Mutex<Vec<crate::proxy_environment::ProxyEnvironment>>,
}

#[derive(Default)]
struct FakeProcess {
    exit: Mutex<Option<CpaExit>>,
}

struct FakeHandle {
    pid: u32,
    process: Arc<FakeProcess>,
}

impl FakeBackend {
    fn spawn_count(&self) -> usize {
        self.spawn_count.load(Ordering::SeqCst)
    }

    fn attach_count(&self) -> usize {
        self.attach_count.load(Ordering::SeqCst)
    }

    fn crash_latest(&self, code: i32) {
        let process = self
            .processes
            .lock()
            .iter()
            .rev()
            .find(|(_, process)| process.exit.lock().is_none())
            .map(|(_, process)| Arc::clone(process))
            .expect("one fake process is live");
        *process.exit.lock() = Some(CpaExit { code: Some(code) });
    }
}

impl CpaProcessBackend for FakeBackend {
    fn spawn(&self, launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
        self.proxy_environments
            .lock()
            .push(launch.proxy_environment.clone());
        let bytes = std::fs::read(&launch.config_path).map_err(CpaProcessError::Spawn)?;
        CpaManagedConfigContract::validate_rendered(&bytes)
            .map_err(|_| CpaProcessError::InvalidLaunch)?;
        let pid = 40_000 + self.next_pid.fetch_add(1, Ordering::SeqCst);
        let process = Arc::new(FakeProcess::default());
        self.processes.lock().insert(pid, Arc::clone(&process));
        self.spawn_count.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeHandle { pid, process }))
    }

    fn attach_authenticated(&self, pid: u32) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
        let process = self
            .processes
            .lock()
            .get(&pid)
            .filter(|process| process.exit.lock().is_none())
            .cloned()
            .ok_or(CpaProcessError::NotRunning)?;
        self.attach_count.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeHandle { pid, process }))
    }

    fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError> {
        if pid == std::process::id() {
            return Ok(true);
        }
        Ok(self
            .processes
            .lock()
            .get(&pid)
            .is_some_and(|process| process.exit.lock().is_none()))
    }
}

impl CpaProcessHandle for FakeHandle {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn try_exit(&mut self) -> Result<Option<CpaExit>, CpaProcessError> {
        Ok(*self.process.exit.lock())
    }

    fn shutdown(&mut self, _timeout: Duration) -> Result<CpaExit, CpaProcessError> {
        let exit = CpaExit { code: Some(0) };
        *self.process.exit.lock() = Some(exit);
        Ok(exit)
    }
}

#[derive(Default)]
struct FakeControl {
    accounts: Mutex<Vec<AccountSnapshotRecord>>,
    probes: AtomicUsize,
    discoveries: AtomicUsize,
    last_client_version: Mutex<Option<String>>,
    refresh_requested: AtomicBool,
    fail_probes: AtomicBool,
    discovery_error: Mutex<Option<AccountDiscoveryError>>,
}

impl FakeControl {
    fn set_accounts(&self, accounts: Vec<AccountSnapshotRecord>) {
        *self.accounts.lock() = accounts;
    }

    fn set_fail_probes(&self, fail: bool) {
        self.fail_probes.store(fail, Ordering::SeqCst);
    }
}

impl CpaControlPlane for FakeControl {
    fn probe_ready(
        &self,
        address: SocketAddr,
        secrets: &InstanceSecrets,
        expected_version: &str,
        _timeout: Duration,
    ) -> Result<(), AccountDiscoveryError> {
        assert!(address.ip().is_loopback());
        assert_ne!(secrets.downstream.expose(), secrets.management.expose());
        assert_eq!(expected_version, VERSION);
        self.probes.fetch_add(1, Ordering::SeqCst);
        if self.fail_probes.load(Ordering::SeqCst) {
            return Err(AccountDiscoveryError::NotReady);
        }
        Ok(())
    }

    fn discover_and_pin(
        &self,
        address: SocketAddr,
        auth_dir: &Path,
        managed_identities: &[ManagedAccountIdentity],
        secrets: &InstanceSecrets,
        expected_version: &str,
        timeout: Duration,
        refresh_models: bool,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        self.discoveries.fetch_add(1, Ordering::SeqCst);
        self.refresh_requested
            .store(refresh_models, Ordering::SeqCst);
        *self.last_client_version.lock() =
            std::fs::read(auth_dir.join("hiroute-managed-codex.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|value| value["hiroute_client_version"].as_str().map(str::to_owned));
        self.probe_ready(address, secrets, expected_version, timeout)?;
        if let Some(error) = self.discovery_error.lock().take() {
            return Err(error);
        }
        let mut accounts = self.accounts.lock().clone();
        for identity in managed_identities {
            let account = accounts
                .iter_mut()
                .find(|account| account.account_kind == identity.account_kind)
                .ok_or(AccountDiscoveryError::AccountDisappeared)?;
            account.stock_file_name = identity.stock_file_name.clone();
            account.account_digest = identity.account_digest.clone();
            account.prefix = identity.prefix()?;
            account.generation = identity.generation;
        }
        Ok(accounts)
    }
}

fn snapshot(kind: CpaAccountKind, marker: char, model: &str) -> AccountSnapshotRecord {
    AccountSnapshotRecord {
        account_kind: kind,
        stock_id: format!("stock-{marker}"),
        stock_auth_index: format!("private-index-{marker}"),
        stock_file_name: format!("private-file-{marker}.json"),
        prefix: match kind {
            CpaAccountKind::Codex => "hiroute-codex-current".into(),
            CpaAccountKind::Claude => format!("hiroute-{}", marker.to_string().repeat(24)),
        },
        account_digest: marker.to_string().repeat(64),
        generation: 1,
        observed_model_ids: BTreeSet::from([model.to_owned()]),
        active: true,
    }
}

fn bindings() -> Vec<CpaProfileBinding> {
    vec![
        CpaProfileBinding {
            account_kind: CpaAccountKind::Codex,
            connector_id: "connector.cpa.codex".into(),
            connection_option_id: "codex.subscription.global.v1".into(),
            endpoint_profile_id: "endpoint.cpa.codex".into(),
        },
        CpaProfileBinding {
            account_kind: CpaAccountKind::Claude,
            connector_id: "connector.cpa.claude".into(),
            connection_option_id: "claude.subscription.test.v1".into(),
            endpoint_profile_id: "endpoint.cpa.claude".into(),
        },
    ]
}

fn fixture_catalog() -> Arc<TrustedReleaseCatalog> {
    let evidence = CanonicalDigest::of_bytes(b"fixture-evidence");
    let mut registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
        "../../../../assets/release-facts/current/bundle/connector-registry.json"
    ))
    .unwrap();
    // Replace the production Claude contract with this fixture's controlled endpoint.
    registry
        .connectors
        .retain(|item| item.connector_id != "connector.cpa.claude");
    registry
        .endpoint_profiles
        .retain(|item| item.endpoint_profile_id != "endpoint.cpa.claude");
    registry
        .connection_options
        .retain(|item| item.connector_id != "connector.cpa.claude");
    registry.connectors.push(
        serde_json::from_value(connector_json(
            "connector.cpa.claude",
            "endpoint.cpa.claude",
        ))
        .unwrap(),
    );
    registry.endpoint_profiles.push(
        serde_json::from_value(profile_json(
            "connector.cpa.claude",
            "endpoint.cpa.claude",
            "messages",
            "https://messages.fixture.invalid",
            "/v1/messages",
            evidence.to_string(),
        ))
        .unwrap(),
    );
    registry.connection_options.push(
        serde_json::from_value(option_json(
            "connector.cpa.claude",
            "endpoint.cpa.claude",
            "claude.subscription.test.v1",
        ))
        .unwrap(),
    );

    let mut models: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    // The controlled profile above replaces its protocol adapter and offering.
    // Its current catalog references must be replaced at the same boundary.
    models
        .data
        .model_endpoint_capabilities
        .retain(|capability| capability.connector_id != "connector.cpa.claude");
    models
        .data
        .offers
        .retain(|offer| offer.endpoint_profile_id != "endpoint.cpa.claude");
    models.data.model_endpoint_capabilities.push(
        serde_json::from_value(capability_json(
            "claude",
            "connector.cpa.claude",
            "endpoint.cpa.claude",
            "messages",
            "claude-sonnet-5",
            "model.anthropic.claude-sonnet-5",
            evidence.to_string(),
        ))
        .unwrap(),
    );
    models.data.offers.push(
        serde_json::from_value(offer_json(
            "claude",
            "endpoint.cpa.claude",
            "model.anthropic.claude-sonnet-5",
            evidence.to_string(),
        ))
        .unwrap(),
    );

    let registry = serde_json::to_vec(&registry).unwrap();
    let models = serde_json::to_vec(&models).unwrap();
    let decoded_registry: ConnectorRegistryBundleV1 = serde_json::from_slice(&registry).unwrap();
    let decoded_models: ReleaseModelDataBundleV2 = serde_json::from_slice(&models).unwrap();
    let manifest = ReleaseFactsManifestV2 {
        schema: hiroute_domain::RELEASE_FACTS_SCHEMA_V2.into(),
        tool_version: hiroute_domain::RELEASE_FACTS_TOOL_VERSION_V2.into(),
        catalog_id: "fixture-cpa-runtime".into(),
        product_release: decoded_registry.product_release.clone(),
        sequence: 1,
        connector_registry_digest: CanonicalDigest::of_bytes(&registry),
        model_data_digest: CanonicalDigest::of_bytes(&models),
        cross_reference_digest: decoded_models
            .cross_reference_digest(&decoded_registry)
            .unwrap(),
    };
    Arc::new(
        TrustedReleaseCatalog::load_release_facts(
            &serde_json::to_vec(&manifest).unwrap(),
            &registry,
            &models,
        )
        .unwrap(),
    )
}

fn connector_json(connector: &str, profile: &str) -> serde_json::Value {
    json!({
        "connector_id": connector, "revision": 1, "runtime_kind": "cpa_bridge",
        "implementation_ref": "bridge/cpa", "implementation_revision": 1,
        "accepted_origins": ["agent_subscription"], "authentication": "connector_owned_opaque",
        "required_secret_slots": [], "endpoint_profile_refs": [profile],
        "catalog_adapter_ref": "catalog.cpa", "catalog_adapter_revision": 1,
        "error_classifier_ref": "errors.cpa", "error_classifier_revision": 1,
        "usage_decoder_ref": "usage.cpa", "usage_decoder_revision": 1,
        "cache_policy_ref": "cache.cpa", "cache_policy_revision": 1
    })
}

fn profile_json(
    connector: &str,
    profile: &str,
    protocol: &str,
    base_url: &str,
    path: &str,
    evidence: String,
) -> serde_json::Value {
    json!({
        "endpoint_profile_id": profile, "revision": 1, "connector_id": connector,
        "connector_revision": 1, "provider_platform_id": format!("provider.{protocol}"),
        "service_offering_id": "subscription", "entitlement_id": "subscription",
        "usage_scope": "account", "region_id": "global",
        "logical_endpoint_group": format!("logical.{protocol}"),
        "protocol_endpoints": [{"protocol_endpoint_id": format!("{profile}.{protocol}"),
          "protocol": protocol, "base_url": base_url,
          "request_path": path, "adapter_ref": "adapter.cpa", "adapter_revision": 1,
          "stable_preference": 0}],
        "inventory_strategy": "bundled_catalog", "verification_evidence": evidence,
        "last_verified_at": 1
    })
}

fn option_json(connector: &str, profile: &str, option: &str) -> serde_json::Value {
    json!({
        "connection_option_id": option, "display_name": format!("{connector} subscription"),
        "origin": "agent_subscription", "connector_id": connector, "connector_revision": 1,
        "endpoint_profile_id": profile, "endpoint_profile_revision": 1,
        "billing_class": "subscription"
    })
}

fn capability_json(
    marker: &str,
    connector: &str,
    profile: &str,
    protocol: &str,
    upstream: &str,
    model: &str,
    evidence: String,
) -> serde_json::Value {
    json!({"capability_id": format!("capability.{marker}"), "revision": 1,
      "model_configuration_id": model, "connector_id": connector, "connector_revision": 1,
      "endpoint_profile_id": profile, "endpoint_profile_revision": 1,
      "protocol_endpoint_id": format!("{profile}.{protocol}"), "upstream_protocol": protocol,
      "upstream_model_id": upstream, "required_adapter_ref": "adapter.cpa",
      "required_adapter_revision": 1, "evidence_digest": evidence})
}

fn offer_json(marker: &str, profile: &str, model: &str, evidence: String) -> serde_json::Value {
    json!({"offer_id": format!("offer.{marker}"), "revision": 1,
      "endpoint_profile_id": profile, "endpoint_profile_revision": 1,
      "service_offering_id": "subscription", "entitlement_id": "subscription",
      "usage_scope": "account", "region_id": "global", "model_configuration_ids": [model],
      "billing_class": "subscription", "evidence_digest": evidence})
}

fn fixture_runtime(
    root: &tempfile::TempDir,
    backend: Arc<FakeBackend>,
    control: Arc<FakeControl>,
    max_restarts: u8,
) -> ManagedCpaRuntime {
    let codex_auth_source = fixture_codex_auth_source(root);
    let spec = CpaRuntimeSpec {
        instance_id: "fixture-cpa".into(),
        state_root: root.path().join("state"),
        auth_dir: root.path().join("auth"),
        borrowed_claude_auth: None,
        managed_oauth: None,
        // Runtime fixtures use a fake control plane, not the host's Codex installation.
        // Version-probe cases opt into their own executable explicitly.
        borrowed_codex_auth: Some(
            BorrowedCodexAuthSpec::new(codex_auth_source)
                .with_executable(root.path().join("fixture-codex-not-installed")),
        ),
        bindings: bindings(),
        // Durable fixture writes and parallel process scheduling need a setup budget.
        // Tests of readiness deadlines and caller cancellation set their own budgets.
        startup_timeout: Duration::from_secs(3),
        control_timeout: Duration::from_secs(2),
        shutdown_timeout: Duration::from_secs(2),
        restart_policy: RestartPolicy {
            max_restarts,
            window: Duration::from_secs(1),
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        },
    };
    let locator = Arc::new(FixtureLocator(VerifiedCpaBinary::fixture(
        PathBuf::from("/fixture/cliproxyapi"),
        Version::parse(VERSION).unwrap(),
        "a".repeat(64),
    )));
    ManagedCpaRuntime::with_components(spec, fixture_catalog(), locator, backend, control).unwrap()
}

#[test]
fn managed_runtime_reports_lifecycle_without_cpa_output() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let logs = tempfile::tempdir().unwrap();
    ensure_private_dir(logs.path()).unwrap();
    let diagnostics_root = logs.path().join("d");
    let report = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Cpa,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    });
    let runtime = fixture_runtime(&root, backend, control, 2).with_diagnostics(report.port());
    runtime.start().unwrap();
    runtime.shutdown().unwrap();
    runtime.shutdown().unwrap_err();
    report.shutdown();
    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    for kind in ["cpa_lifecycle", "cpa_stage"] {
        assert!(
            log.contains(&format!("\"{kind}\":")),
            "{kind} missing: {log}"
        );
    }
    for phase in ["start", "ready", "exit"] {
        assert!(
            log.contains(&format!("\"phase\":\"{phase}\"")),
            "{phase}: {log}"
        );
    }
    for stage in ["artifact_validate", "ready_wait", "control_call"] {
        assert!(
            log.contains(&format!("\"stage\":\"{stage}\"")),
            "{stage}: {log}"
        );
    }
    assert!(!log.contains("cliproxyapi"), "artifact path leaked");
}

fn fixture_codex_auth_source(root: &tempfile::TempDir) -> PathBuf {
    ensure_private_dir(root.path()).unwrap();
    let path = root.path().join("codex-auth.json");
    if !path.exists() {
        write_fixture_codex_source(
            &path,
            "fixture-account-one",
            "fixture-codex-access-lease",
            "fixture.codex.id-token",
            "fixture-refresh-time",
        );
    }
    path
}

fn write_fixture_codex_source(
    path: &Path,
    account_id: &str,
    access_token: &str,
    id_token: &str,
    last_refresh: &str,
) {
    let bytes = serde_json::to_vec(&json!({
        "OPENAI_API_KEY": null,
        "auth_mode": "chatgpt",
        "last_refresh": last_refresh,
        "tokens": {
            "access_token": access_token,
            "id_token": id_token,
            "refresh_token": "fixture-refresh-never-imported",
            "account_id": account_id
        }
    }))
    .unwrap();
    private_atomic_write(path, &bytes).unwrap();
}

#[test]
fn lifecycle_materializes_codex_and_claude_without_transport_identity() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![
        snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5"),
        snapshot(CpaAccountKind::Claude, 'b', "claude-sonnet-5"),
    ]);
    let runtime = fixture_runtime(&root, Arc::clone(&backend), Arc::clone(&control), 3);
    let health = runtime.start().unwrap();
    let CpaHealth::Ready { address, .. } = health else {
        panic!("runtime was not ready")
    };
    assert!(address.ip().is_loopback());

    let sources = runtime.discover_registered_sources().unwrap();
    assert_eq!(sources.len(), 2);
    assert!(
        sources
            .iter()
            .all(|source| source.source.origin == SourceOrigin::Cpa)
    );
    assert!(
        sources
            .iter()
            .all(|source| !source.source.source_id.contains("127.0.0.1"))
    );
    let codex = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    runtime
        .apply_account_management(&codex.account_subject, 1, CpaSourceManagementState::Enabled)
        .unwrap();
    assert!(!format!("{codex:?}").contains("private-index"));

    let prepared = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &codex.credential_ref,
            upstream_model_id: "gpt-5.5",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    assert_eq!(prepared.address(), address);
    assert_eq!(prepared.request_path(), "/v1/responses");
    assert_eq!(prepared.upstream_model_id(), "gpt-5.5");
    assert_eq!(
        prepared.native_transport_model(),
        "hiroute-codex-current/gpt-5.5"
    );
    assert_ne!(
        prepared.upstream_model_id(),
        prepared.native_transport_model()
    );
    assert!(matches!(
        runtime.lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: "credential/cpa/unknown",
            connector_id: prepared.connector_id(),
            upstream_model_id: prepared.upstream_model_id(),
            protocol: prepared.protocol(),
            address: prepared.address(),
            request_path: prepared.request_path(),
            native_transport_model: prepared.native_transport_model(),
            runtime_epoch: prepared.runtime_epoch(),
            target_epoch: prepared.target_epoch(),
            excluded_key_ids: &[],
        }),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert!(matches!(
        runtime.lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: prepared.credential_ref().credential_id(),
            connector_id: prepared.connector_id(),
            upstream_model_id: prepared.upstream_model_id(),
            protocol: prepared.protocol(),
            address: prepared.address(),
            request_path: prepared.request_path(),
            native_transport_model: prepared.upstream_model_id(),
            runtime_epoch: prepared.runtime_epoch(),
            target_epoch: prepared.target_epoch(),
            excluded_key_ids: &[],
        }),
        Err(CpaAttemptError::UnregisteredTarget)
    ));
    let capability = runtime
        .lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: prepared.credential_ref().credential_id(),
            connector_id: prepared.connector_id(),
            upstream_model_id: prepared.upstream_model_id(),
            protocol: prepared.protocol(),
            // Durable publications can outlive the daemon process that owned
            // this address and epoch pair. Exact identity is authoritative;
            // the capability must return the current managed target.
            address: SocketAddr::from(([127, 0, 0, 1], 9)),
            request_path: prepared.request_path(),
            native_transport_model: prepared.native_transport_model(),
            runtime_epoch: prepared.runtime_epoch().saturating_add(100),
            target_epoch: prepared.target_epoch().saturating_add(100),
            excluded_key_ids: &[],
        })
        .unwrap()
        .unwrap();
    assert_eq!(capability.credential_ref(), prepared.credential_ref());
    assert_eq!(capability.address(), prepared.address());
    assert_eq!(capability.request_path(), prepared.request_path());
    assert_eq!(
        capability.generation(),
        prepared.credential_ref().generation()
    );
    let excluded_key_ids = [Arc::<str>::from(capability.key_id())];
    assert!(
        runtime
            .lease_downstream_capability(ExactCpaCredentialRequest {
                credential_id: prepared.credential_ref().credential_id(),
                connector_id: prepared.connector_id(),
                upstream_model_id: "gpt-5.5",
                protocol: prepared.protocol(),
                address: prepared.address(),
                request_path: prepared.request_path(),
                native_transport_model: prepared.native_transport_model(),
                runtime_epoch: prepared.runtime_epoch(),
                target_epoch: prepared.target_epoch(),
                excluded_key_ids: &excluded_key_ids,
            })
            .unwrap()
            .is_none()
    );
    let mut headers = http::HeaderMap::new();
    capability.apply_authorization(&mut headers).unwrap();
    assert!(headers[http::header::AUTHORIZATION].is_sensitive());
    for (protocol, request_path) in [
        (UpstreamProtocol::Messages, "/v1/messages"),
        (UpstreamProtocol::ChatCompletions, "/v1/chat/completions"),
    ] {
        let face = prepare_target(
            &runtime,
            ExactCpaAttemptRequest {
                credential_ref: &codex.credential_ref,
                upstream_model_id: "gpt-5.5",
                protocol,
            },
        )
        .unwrap();
        assert_eq!(face.address(), prepared.address());
        assert_eq!(face.request_path(), request_path);
        assert_eq!(
            face.native_transport_model(),
            prepared.native_transport_model()
        );
        let leased = runtime
            .lease_downstream_capability(ExactCpaCredentialRequest {
                credential_id: face.credential_ref().credential_id(),
                connector_id: face.connector_id(),
                upstream_model_id: face.upstream_model_id(),
                protocol: face.protocol(),
                address: face.address(),
                request_path: face.request_path(),
                native_transport_model: face.native_transport_model(),
                runtime_epoch: face.runtime_epoch(),
                target_epoch: face.target_epoch(),
                excluded_key_ids: &[],
            })
            .unwrap()
            .unwrap();
        assert_eq!(leased.request_path(), request_path);
    }

    let config = std::fs::read(root.path().join("state/fixture-cpa/config.yaml")).unwrap();
    CpaManagedConfigContract::validate_rendered(&config).unwrap();
    let exit = runtime.shutdown().unwrap();
    assert_eq!(exit.code, Some(0));
    assert!(matches!(
        runtime.lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: prepared.credential_ref().credential_id(),
            connector_id: prepared.connector_id(),
            upstream_model_id: prepared.upstream_model_id(),
            protocol: prepared.protocol(),
            address: prepared.address(),
            request_path: prepared.request_path(),
            native_transport_model: prepared.native_transport_model(),
            runtime_epoch: prepared.runtime_epoch(),
            target_epoch: prepared.target_epoch(),
            excluded_key_ids: &[],
        }),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert_eq!(
        capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    assert!(!root.path().join("state/fixture-cpa/owner.lock").exists());
}

#[test]
fn crash_restart_is_bounded_and_reports_child_exit_separately() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5")]);
    let runtime = fixture_runtime(&root, Arc::clone(&backend), control, 1);
    runtime.start().unwrap();
    let materialized = runtime
        .materialize_account("connector.cpa.codex", "endpoint.cpa.codex")
        .unwrap();
    runtime
        .apply_account_management(
            &materialized.account_subject,
            1,
            CpaSourceManagementState::Enabled,
        )
        .unwrap();
    let target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &materialized.credential_ref,
            upstream_model_id: "gpt-5.5",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    let capability = runtime
        .lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: target.credential_ref().credential_id(),
            connector_id: target.connector_id(),
            upstream_model_id: "gpt-5.5",
            protocol: target.protocol(),
            address: target.address(),
            request_path: target.request_path(),
            native_transport_model: target.native_transport_model(),
            runtime_epoch: target.runtime_epoch(),
            target_epoch: target.target_epoch(),
            excluded_key_ids: &[],
        })
        .unwrap()
        .unwrap();
    backend.crash_latest(29);
    assert_eq!(
        runtime.health().unwrap(),
        CpaHealth::Crashed {
            exit: CpaExit { code: Some(29) },
            restart_count: 0
        }
    );
    assert_eq!(runtime.discover_registered_sources().unwrap().len(), 1);
    assert_eq!(backend.spawn_count(), 2);
    capability
        .apply_authorization(&mut http::HeaderMap::new())
        .expect("supervised restart preserves the target epoch");
    let restarted = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &materialized.credential_ref,
            upstream_model_id: "gpt-5.5",
            protocol: UpstreamProtocol::Responses,
        },
    )
    .unwrap();
    assert_eq!(restarted.address(), target.address());
    assert_eq!(restarted.runtime_epoch(), target.runtime_epoch());
    assert_eq!(restarted.target_epoch(), target.target_epoch());
    backend.crash_latest(31);
    assert!(matches!(
        runtime.discover_registered_sources(),
        Err(CpaLifecycleError::CrashLoop(CpaExit { code: Some(31) }))
    ));
    assert_eq!(runtime.last_exit(), Some(CpaExit { code: Some(31) }));
}

// Closing one descriptor cannot release copies inherited by concurrently forked
// tests before exec. Exercise immediate owner handoffs in their own process while
// retaining explicit contention assertions and the duplicated-descriptor regression.
fn isolated_owner_recovery_case(case: &str) -> bool {
    const CHILD: &str = "HIROUTE_ISOLATED_CPA_OWNER_TEST";
    if std::env::var(CHILD).as_deref() == Ok(case) {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", case, "--test-threads=1"])
        .env(CHILD, case)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success()
            && stdout
                .lines()
                .any(|line| line == format!("test {case} ... ok")),
        "isolated owner recovery must execute its exact case: {stdout} {}",
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

#[test]
fn stale_live_child_is_authenticated_and_adopted_without_duplicate_spawn() {
    if isolated_owner_recovery_case(
        "runtime::tests::stale_live_child_is_authenticated_and_adopted_without_duplicate_spawn",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let first = fixture_runtime(&root, Arc::clone(&backend), Arc::clone(&control), 2);
    let CpaHealth::Ready { pid, address, .. } = first.start().unwrap() else {
        panic!("first runtime was not ready")
    };
    let owner_path = root.path().join("state/fixture-cpa/owner.lock/owner.json");
    let mut owner: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
    owner["owner_pid"] = json!(u32::MAX - 1);
    private_atomic_write(&owner_path, &serde_json::to_vec(&owner).unwrap()).unwrap();
    drop(first.inner.lock().live.as_mut().unwrap().auth_lease.take());
    std::mem::forget(first);

    let second = fixture_runtime(&root, Arc::clone(&backend), control, 2);
    // Deterministically reproduce the observed OwnerState under an extra lock owner.
    // The rejected attempt must neither steal ownership nor spawn/attach a second CPA.
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.path().join("state/fixture-cpa/.owner-reclaim.lock"))
        .unwrap();
    fs2::FileExt::try_lock_exclusive(&held).unwrap();
    let old_owner = std::fs::read(&owner_path).unwrap();
    assert!(matches!(second.start(), Err(CpaLifecycleError::OwnerState)));
    assert_eq!(std::fs::read(&owner_path).unwrap(), old_owner);
    assert_eq!(backend.spawn_count(), 1);
    assert_eq!(backend.attach_count(), 0);
    drop(held);
    assert_eq!(
        second.start().unwrap(),
        CpaHealth::Ready {
            pid,
            address,
            restart_count: 0,
            adopted: true
        }
    );
    assert_eq!(backend.spawn_count(), 1);
    assert_eq!(backend.attach_count(), 1);
    assert_eq!(second.shutdown().unwrap().code, Some(0));
}

#[test]
fn failed_orphan_authentication_restores_stale_owner_for_bounded_retry() {
    if isolated_owner_recovery_case(
        "runtime::tests::failed_orphan_authentication_restores_stale_owner_for_bounded_retry",
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let first = fixture_runtime(&root, Arc::clone(&backend), Arc::clone(&control), 2);
    let CpaHealth::Ready { pid, .. } = first.start().unwrap() else {
        panic!("first runtime was not ready")
    };
    let owner_path = root.path().join("state/fixture-cpa/owner.lock/owner.json");
    let mut owner: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
    owner["owner_pid"] = json!(u32::MAX - 1);
    private_atomic_write(&owner_path, &serde_json::to_vec(&owner).unwrap()).unwrap();
    drop(first.inner.lock().live.as_mut().unwrap().auth_lease.take());
    std::mem::forget(first);

    control.set_fail_probes(true);
    let failed = fixture_runtime(&root, Arc::clone(&backend), Arc::clone(&control), 2);
    let outcome = failed.start();
    if !matches!(outcome, Err(CpaLifecycleError::ControlUnavailable)) {
        let evidence = root.keep();
        panic!(
            "orphan authentication returned {outcome:?}; retained fixture: {}",
            evidence.display()
        );
    }
    let restored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
    assert_eq!(restored["owner_pid"], json!(u32::MAX - 1));

    control.set_fail_probes(false);
    let retry = fixture_runtime(&root, Arc::clone(&backend), control, 2);
    assert!(matches!(
        retry.start().unwrap(),
        CpaHealth::Ready {
            pid: adopted,
            adopted: true,
            ..
        } if adopted == pid
    ));
    assert_eq!(backend.spawn_count(), 1);
    assert_eq!(backend.attach_count(), 1);
    retry.shutdown().unwrap();
}

#[test]
fn removed_account_revokes_old_reference_and_readdition_rotates_generation() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    control.set_accounts(vec![
        snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5"),
        snapshot(CpaAccountKind::Claude, 'b', "claude-sonnet-5"),
    ]);
    let runtime = fixture_runtime(&root, backend, Arc::clone(&control), 2);
    runtime.start().unwrap();
    let old = runtime
        .materialize_account("connector.cpa.claude", "endpoint.cpa.claude")
        .unwrap();
    runtime
        .apply_account_management(&old.account_subject, 1, CpaSourceManagementState::Enabled)
        .unwrap();
    let old_target = prepare_target(
        &runtime,
        ExactCpaAttemptRequest {
            credential_ref: &old.credential_ref,
            upstream_model_id: "claude-sonnet-5",
            protocol: UpstreamProtocol::Messages,
        },
    )
    .unwrap();
    let old_capability = runtime
        .lease_downstream_capability(ExactCpaCredentialRequest {
            credential_id: old_target.credential_ref().credential_id(),
            connector_id: old_target.connector_id(),
            upstream_model_id: "claude-sonnet-5",
            protocol: old_target.protocol(),
            address: old_target.address(),
            request_path: old_target.request_path(),
            native_transport_model: old_target.native_transport_model(),
            runtime_epoch: old_target.runtime_epoch(),
            target_epoch: old_target.target_epoch(),
            excluded_key_ids: &[],
        })
        .unwrap()
        .unwrap();
    control.set_accounts(vec![snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5")]);
    assert_eq!(runtime.discover_registered_sources().unwrap().len(), 1);
    assert!(matches!(
        prepare_target(
            &runtime,
            ExactCpaAttemptRequest {
                credential_ref: &old.credential_ref,
                upstream_model_id: "claude-sonnet-5",
                protocol: UpstreamProtocol::Messages,
            }
        ),
        Err(CpaAttemptError::RevokedCredential)
    ));
    assert_eq!(
        old_capability.apply_authorization(&mut http::HeaderMap::new()),
        Err(CpaAttemptError::RevokedCredential)
    );
    control.set_accounts(vec![
        snapshot(CpaAccountKind::Codex, 'a', "gpt-5.5"),
        snapshot(CpaAccountKind::Claude, 'b', "claude-sonnet-5"),
    ]);
    let new = runtime
        .materialize_account("connector.cpa.claude", "endpoint.cpa.claude")
        .unwrap();
    assert_eq!(new.credential_ref.generation(), 2);
    assert_ne!(old.credential_ref, new.credential_ref);
}

#[test]
fn unsupported_stock_version_fails_before_state_or_process_creation() {
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let mut runtime = fixture_runtime(&root, Arc::clone(&backend), control, 2);
    runtime.locator = Arc::new(FixtureLocator(VerifiedCpaBinary::fixture(
        PathBuf::from("/fixture/cliproxyapi"),
        Version::parse("7.2.141").unwrap(),
        "b".repeat(64),
    )));
    assert!(matches!(
        runtime.start(),
        Err(CpaLifecycleError::UnsupportedArtifactVersion)
    ));
    assert_eq!(backend.spawn_count(), 0);
    assert!(!root.path().join("state").exists());
}

/// S3: a blocking start step is recorded as entered before it returns, so a paused start
/// still names the step it is stuck in, and the same step reports its real end afterwards.
#[test]
fn process_spawn_entry_is_visible_while_the_spawn_is_still_blocked() {
    #[derive(Default)]
    struct BlockingBackend {
        inner: FakeBackend,
        entered: Mutex<bool>,
        released: Mutex<bool>,
        release: Condvar,
    }

    impl BlockingBackend {
        fn entered(&self) -> bool {
            *self.entered.lock()
        }
        fn release(&self) {
            *self.released.lock() = true;
            self.release.notify_all();
        }
    }

    impl CpaProcessBackend for BlockingBackend {
        fn spawn(&self, launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
            *self.entered.lock() = true;
            let mut released = self.released.lock();
            while !*released {
                self.release.wait(&mut released);
            }
            drop(released);
            self.inner.spawn(launch)
        }

        fn attach_authenticated(
            &self,
            pid: u32,
        ) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
            self.inner.attach_authenticated(pid)
        }

        fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError> {
            self.inner.pid_is_running(pid)
        }
    }

    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(BlockingBackend::default());
    let logs = tempfile::tempdir().unwrap();
    ensure_private_dir(logs.path()).unwrap();
    let diagnostics_root = logs.path().join("d");
    let report = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Cpa,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    });
    let mut runtime = fixture_runtime(
        &root,
        Arc::new(FakeBackend::default()),
        Arc::new(FakeControl::default()),
        2,
    )
    .with_diagnostics(report.port());
    runtime.backend = backend.clone();

    let watcher = {
        let backend = Arc::clone(&backend);
        let path = diagnostics_root.join("daemon").join("current.jsonl");
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut found = false;
            while std::time::Instant::now() < deadline {
                let log = std::fs::read_to_string(&path).unwrap_or_default();
                if log.contains("\"stage\":\"process_spawn\"")
                    && log.contains("\"outcome\":\"entered\"")
                {
                    // The spawn is still blocked inside the backend: the entered record
                    // exists before the step ends.
                    found = !backend.released.lock().to_owned();
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            backend.release();
            found
        })
    };
    assert!(runtime.start().is_ok());
    assert!(
        watcher.join().unwrap(),
        "the entered record must be visible while the spawn is blocked"
    );
    assert!(backend.entered());
    // The diagnostic writer is asynchronous: wait for the terminal records
    // instead of racing its flush after runtime.start() returns.
    let path = diagnostics_root.join("daemon").join("current.jsonl");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let log = loop {
        let log = std::fs::read_to_string(&path).unwrap_or_default();
        if log.contains("\"stage\":\"process_spawn\"")
            && log.contains("\"outcome\":\"completed\"")
            && log.contains("\"stage\":\"ready_wait\"")
        {
            break log;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "missing CPA stage records: {log}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        log.contains("\"stage\":\"process_spawn\"") && log.contains("\"outcome\":\"completed\""),
        "the step reports its real end: {log}"
    );
    assert!(log.contains("\"stage\":\"ready_wait\""), "{log}");
    assert!(!log.contains("cliproxyapi"), "artifact path leaked: {log}");
    report.shutdown();
}

/// One CPA stage record read back from the JSONL file.
#[derive(Debug, PartialEq)]
struct StageRecord {
    stage: String,
    outcome: String,
    generation: u64,
}

fn stage_records(log: &str) -> Vec<StageRecord> {
    log.lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("record is JSON"))
        .filter_map(|record| {
            let stage = record.get("event")?.get("cpa_stage")?;
            Some(StageRecord {
                stage: stage.get("stage")?.as_str()?.to_owned(),
                outcome: stage.get("outcome")?.to_string(),
                generation: stage.get("restart_generation")?.as_u64()?,
            })
        })
        .collect()
}

/// A step that ran reports one `Entered` and exactly one terminal, in that order.
fn single_terminal(records: &[StageRecord], stage: &str) -> String {
    let relevant: Vec<&StageRecord> = records.iter().filter(|item| item.stage == stage).collect();
    let entered: Vec<usize> = relevant
        .iter()
        .enumerate()
        .filter(|(_, item)| item.outcome == "\"entered\"")
        .map(|(position, _)| position)
        .collect();
    let terminals: Vec<&StageRecord> = relevant
        .iter()
        .filter(|item| item.outcome != "\"entered\"")
        .copied()
        .collect();
    assert_eq!(entered.len(), 1, "{stage}: one entered in {records:?}");
    assert_eq!(
        terminals.len(),
        1,
        "{stage}: exactly one terminal in {records:?}"
    );
    let terminal_position = relevant
        .iter()
        .position(|item| std::ptr::eq(*item, terminals[0]))
        .unwrap();
    assert!(
        entered[0] < terminal_position,
        "{stage}: the entered record precedes its terminal"
    );
    assert_eq!(
        relevant[entered[0]].generation, terminals[0].generation,
        "{stage}: both records belong to one generation"
    );
    terminals[0].outcome.clone()
}

/// R5: a CPA start reports each step once at its own real boundary. A spawn that completed
/// is never followed by a fabricated spawn failure when a later step fails, and adoption —
/// which never spawns — reports no spawn step at all.
#[test]
fn cpa_stage_terminals_are_unique_ordered_and_never_fabricated() {
    if isolated_owner_recovery_case(
        "runtime::tests::cpa_stage_terminals_are_unique_ordered_and_never_fabricated",
    ) {
        return;
    }
    // A spawn that completes, then a bounded ready wait that fails.
    let root = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    ensure_private_dir(logs.path()).unwrap();
    let diagnostics_root = logs.path().join("d");
    let report = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Cpa,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    });
    let control = Arc::new(FakeControl::default());
    control.set_fail_probes(true);
    let mut runtime = fixture_runtime(&root, Arc::new(FakeBackend::default()), control, 2)
        .with_diagnostics(report.port());
    runtime.spec.startup_timeout = Duration::from_millis(200);
    let started = std::time::Instant::now();
    let actual = runtime.start();
    assert!(
        matches!(&actual, Err(CpaLifecycleError::StartupTimeout)),
        "startup result: {actual:?}; elapsed: {:?}",
        started.elapsed()
    );
    report.shutdown();
    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    let records = stage_records(&log);
    assert_eq!(
        single_terminal(&records, "process_spawn"),
        "\"completed\"",
        "the spawn really completed: {records:?}"
    );
    assert!(
        single_terminal(&records, "ready_wait").contains("ready_timeout"),
        "{records:?}"
    );
    assert_eq!(
        records
            .iter()
            .filter(|item| item.stage == "process_spawn")
            .count(),
        2,
        "no second spawn terminal appears after the ready wait failed: {records:?}"
    );
    assert_eq!(
        records
            .iter()
            .filter(|item| item.stage == "ready_wait")
            .count(),
        2,
        "{records:?}"
    );
    assert!(
        records
            .iter()
            .position(|item| item.stage == "process_spawn" && item.outcome != "\"entered\"")
            .unwrap()
            < records
                .iter()
                .position(|item| item.stage == "ready_wait" && item.outcome == "\"entered\"")
                .unwrap(),
        "the spawn terminal precedes the ready wait: {records:?}"
    );

    // Adoption: the process already exists, so no spawn step is reported and the failure
    // belongs to the ready wait that really ran.
    let root = tempfile::tempdir().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let control = Arc::new(FakeControl::default());
    let first = fixture_runtime(&root, Arc::clone(&backend), Arc::clone(&control), 2);
    first.start().unwrap();
    let owner_path = root.path().join("state/fixture-cpa/owner.lock/owner.json");
    let mut owner: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&owner_path).unwrap()).unwrap();
    owner["owner_pid"] = json!(u32::MAX - 1);
    private_atomic_write(&owner_path, &serde_json::to_vec(&owner).unwrap()).unwrap();
    drop(first.inner.lock().live.as_mut().unwrap().auth_lease.take());
    std::mem::forget(first);

    let logs = tempfile::tempdir().unwrap();
    ensure_private_dir(logs.path()).unwrap();
    let diagnostics_root = logs.path().join("d");
    let report = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Cpa,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    });
    control.set_fail_probes(true);
    let adopting =
        fixture_runtime(&root, Arc::clone(&backend), control, 2).with_diagnostics(report.port());
    assert!(matches!(
        adopting.start(),
        Err(CpaLifecycleError::ControlUnavailable)
    ));
    report.shutdown();
    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    let records = stage_records(&log);
    assert!(
        records.iter().all(|item| item.stage != "process_spawn"),
        "adoption never reports a spawn step: {records:?}"
    );
    assert!(
        single_terminal(&records, "ready_wait").contains("ready_rejected"),
        "{records:?}"
    );
}

/// S3: a CPA start that fails before it can serve names the step that failed with a stable
/// code, keeps the business error and never claims a ready phase.
#[test]
fn cpa_start_failures_report_their_stage_code_and_business_error() {
    struct RefusingLocator;
    impl CpaBinaryLocator for RefusingLocator {
        fn locate(&self) -> Result<VerifiedCpaBinary, CpaArtifactError> {
            Err(CpaArtifactError::Binary(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "absent",
            )))
        }
    }

    struct FailingSpawn(FakeBackend);
    impl CpaProcessBackend for FailingSpawn {
        fn spawn(&self, _launch: &CpaLaunch) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
            Err(CpaProcessError::Spawn(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "refused",
            )))
        }
        fn attach_authenticated(
            &self,
            pid: u32,
        ) -> Result<Box<dyn CpaProcessHandle>, CpaProcessError> {
            self.0.attach_authenticated(pid)
        }
        fn pid_is_running(&self, pid: u32) -> Result<bool, CpaProcessError> {
            self.0.pid_is_running(pid)
        }
    }

    // The trusted artifact cannot be located.
    let root = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    ensure_private_dir(logs.path()).unwrap();
    let diagnostics_root = logs.path().join("d");
    let report = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Cpa,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    });
    let mut runtime = fixture_runtime(
        &root,
        Arc::new(FakeBackend::default()),
        Arc::new(FakeControl::default()),
        2,
    )
    .with_diagnostics(report.port());
    runtime.locator = Arc::new(RefusingLocator);
    assert!(matches!(
        runtime.start(),
        Err(CpaLifecycleError::Artifact(_))
    ));
    report.shutdown();
    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    assert!(log.contains("\"stage\":\"artifact_locate\""), "{log}");
    assert!(
        log.contains("\"outcome\":{\"failed\":{\"code\":\"artifact_unavailable\"}}"),
        "{log}"
    );
    assert!(!log.contains("\"phase\":\"ready\""), "{log}");

    // The process cannot be started.
    let root = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    ensure_private_dir(logs.path()).unwrap();
    let diagnostics_root = logs.path().join("d");
    let report = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Cpa,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    });
    let mut runtime = fixture_runtime(
        &root,
        Arc::new(FakeBackend::default()),
        Arc::new(FakeControl::default()),
        2,
    )
    .with_diagnostics(report.port());
    runtime.backend = Arc::new(FailingSpawn(FakeBackend::default()));
    assert!(matches!(
        runtime.start(),
        Err(CpaLifecycleError::Process(_))
    ));
    report.shutdown();
    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    assert!(log.contains("\"stage\":\"process_spawn\""), "{log}");
    assert!(
        log.contains("\"outcome\":{\"failed\":{\"code\":\"spawn_failed\"}}"),
        "{log}"
    );
    assert!(!log.contains("\"phase\":\"ready\""), "{log}");

    // The process starts but never becomes ready inside its bounded wait.
    let root = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    ensure_private_dir(logs.path()).unwrap();
    let diagnostics_root = logs.path().join("d");
    let report = DiagnosticRuntime::start(RuntimeConfig {
        root: diagnostics_root.clone(),
        role: hiroute_diagnostics::event::ProcessRole::Daemon,
        component: hiroute_diagnostics::record::Component::Cpa,
        parent_session_id: None,
        level_override: Some(hiroute_diagnostics::level::DiagnosticLevel::Debug),
    });
    let control = Arc::new(FakeControl::default());
    control.set_fail_probes(true);
    let mut runtime = fixture_runtime(&root, Arc::new(FakeBackend::default()), control, 2)
        .with_diagnostics(report.port());
    runtime.spec.startup_timeout = Duration::from_millis(200);
    let started = std::time::Instant::now();
    let actual = runtime.start();
    assert!(
        matches!(&actual, Err(CpaLifecycleError::StartupTimeout)),
        "startup result: {actual:?}; elapsed: {:?}",
        started.elapsed()
    );
    assert!(
        started.elapsed() >= Duration::from_millis(200),
        "the existing bounded ready wait is unchanged"
    );
    report.shutdown();
    let log =
        std::fs::read_to_string(diagnostics_root.join("daemon").join("current.jsonl")).unwrap();
    assert!(
        log.contains("\"stage\":\"ready_wait\"") && log.contains("\"outcome\":\"entered\""),
        "the ready wait is visible while it blocks: {log}"
    );
    assert!(
        log.contains("\"outcome\":{\"failed\":{\"code\":\"ready_timeout\"}}"),
        "{log}"
    );
    assert!(!log.contains("\"phase\":\"ready\""), "{log}");
    assert!(!log.contains("cliproxyapi"), "artifact path leaked: {log}");
}
