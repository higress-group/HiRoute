//! Production `hirouted` listener bootstrap and the Oracle-first product seam.

#[path = "composition/mod.rs"]
pub mod composition;
#[path = "core_runtime.rs"]
pub mod core_runtime;
#[path = "dispatch/mod.rs"]
pub mod dispatch;
pub mod managed;
#[path = "publication/mod.rs"]
pub mod publication;
#[path = "request_plan/mod.rs"]
pub mod request_plan;
#[cfg(feature = "e2e-test-control")]
#[path = "test_control/mod.rs"]
pub mod test_control;

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use self::composition::{
    FileCredentialResolver, FilePlannerInputAuthority, PlannerInputAuthority, PortError,
    ProductionPorts, PublicationPlannerInputAuthority,
};
use self::core_runtime::ProductionGatewayRuntime;
use self::publication::{
    GatewayPrepareOutcome, GatewayPublicationInstaller, GatewayPublicationSnapshotV3,
    PublicationInstallError,
};
#[cfg(feature = "e2e-test-control")]
use self::test_control::{
    E2eControlEndpoint, E2eControlError, E2eControlHandle, E2eControlOptions,
};
use async_trait::async_trait;
use bytes::Bytes;
use hiroute_gateway_core::transport::pingora::GatewayHttpApp;
use hiroute_gateway_core::transport::{
    GatewayLifecycle, GatewayResponseHead, GatewaySession, SessionReuse, TransportError,
};
use http::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use self::managed::{ManagedGatewayError, ManagedGatewayHandle, ManagedGatewayPhase};

pub const LAUNCH_FIXTURE_SCHEMA: &str = "hiroute.gateway.e2e-fixture/v2";
pub const PUBLICATION_SCHEMA: &str = "hiroute.gateway.publication-snapshot/v1";
pub const READY_SCHEMA: &str = "hiroute.gateway.ready/v1";
pub const NOT_IMPLEMENTED_CODE: &str = "GATEWAY_EXECUTION_NOT_IMPLEMENTED";
pub const LAUNCHER_EXECUTABLE_SHA256_ENV: &str = "HIROUTE_LAUNCHER_EXECUTABLE_SHA256";

const NOT_IMPLEMENTED_BODY: &[u8] = br#"{"schema_version":"hiroute.gateway.error/v1","code":"GATEWAY_EXECUTION_NOT_IMPLEMENTED","phase":"oracle_frozen_before_product"}"#;
const LISTENER_WORKER_THREADS: usize = 2;

pub struct GatewayLauncher {
    listen: SocketAddr,
    mode: GatewayMode,
    launch_identity: GatewayLaunchIdentity,
}

#[derive(Clone, Debug, Default)]
pub struct GatewayLaunchIdentity {
    executable_sha256: Option<Arc<str>>,
}

impl GatewayLaunchIdentity {
    pub fn from_environment() -> Result<Self, GatewayLauncherError> {
        let Some(value) = std::env::var_os(LAUNCHER_EXECUTABLE_SHA256_ENV) else {
            return Ok(Self::default());
        };
        let value = value
            .into_string()
            .map_err(|_| GatewayLauncherError::InvalidExecutableDigest)?;
        Self::from_executable_sha256(value)
    }

    pub fn from_executable_sha256(
        executable_sha256: impl Into<Arc<str>>,
    ) -> Result<Self, GatewayLauncherError> {
        let executable_sha256 = executable_sha256.into();
        if !is_digest(&executable_sha256) {
            return Err(GatewayLauncherError::InvalidExecutableDigest);
        }
        Ok(Self {
            executable_sha256: Some(executable_sha256),
        })
    }

    fn executable_sha256(&self) -> Option<&Arc<str>> {
        self.executable_sha256.as_ref()
    }
}

enum GatewayMode {
    Oracle(Box<LaunchFixtureEnvelope>),
    Production {
        runtime: Arc<ProductionGatewayRuntime>,
        #[cfg(feature = "e2e-test-control")]
        test_control: Option<E2eControlEndpoint>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchFixtureEnvelope {
    schema_version: String,
    oracle_version: String,
    contract_digest: String,
    publication: PublicationSnapshot,
    provider_bindings: Vec<ProviderBinding>,
    secret_store: SecretStore,
    observation: ObservationFixture,
    readiness: ReadinessFixture,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationSnapshot {
    schema_version: String,
    authority: PublicationAuthority,
    epoch: u64,
    revision: String,
    payload_digest: String,
    protocol_routes: Vec<Value>,
    alias_index: Vec<Value>,
    grant_index: Vec<Value>,
    agent_plans: Vec<Value>,
    adapter_catalog: Vec<Value>,
    model_catalog: Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PublicationAuthority {
    issuer: String,
    subject: String,
    policy_revision: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderBinding {
    binding_ref: String,
    credential_ref: String,
    endpoint: String,
    protocol: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretStore {
    client_grants: BTreeMap<String, String>,
    credentials: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservationFixture {
    directory: PathBuf,
    run_nonce: String,
    terminal_receipt: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadinessFixture {
    nonce: String,
    path: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
struct ReadyDocument<'a> {
    schema_version: &'static str,
    run_nonce: &'a str,
    process_id: u32,
    listen_address: String,
    executable_sha256: Option<&'a str>,
}

impl GatewayLauncher {
    pub fn from_fixture(listen: SocketAddr, path: &Path) -> Result<Self, GatewayLauncherError> {
        Self::from_fixture_with_identity(listen, path, GatewayLaunchIdentity::default())
    }

    pub fn from_fixture_with_identity(
        listen: SocketAddr,
        path: &Path,
        launch_identity: GatewayLaunchIdentity,
    ) -> Result<Self, GatewayLauncherError> {
        if !listen.ip().is_loopback() || listen.port() == 0 {
            return Err(GatewayLauncherError::ListenerMustUseReservedAddress(listen));
        }
        let bytes = std::fs::read(path).map_err(GatewayLauncherError::FixtureIo)?;
        let fixture: LaunchFixtureEnvelope =
            serde_json::from_slice(&bytes).map_err(GatewayLauncherError::FixtureJson)?;
        fixture.validate()?;
        Ok(Self {
            listen,
            mode: GatewayMode::Oracle(Box::new(fixture)),
            launch_identity,
        })
    }

    /// Builds the real six-port production composition without any fixture.
    /// A missing LKG starts in a typed unavailable state. Embeddings that keep
    /// a live publication feed use `from_runtime` with their composed runtime.
    pub fn production(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
    ) -> Result<Self, GatewayLauncherError> {
        Self::production_with_publication(listen, lkg_path, None)
    }

    /// Starts the real production composition and optionally applies one
    /// directly sealed startup publication through the same prepare/durable/
    /// live installer used by future remote feeds. This is configuration, not
    /// an Oracle fixture, and contains no credential material.
    pub fn production_with_publication(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
        publication_path: Option<&Path>,
    ) -> Result<Self, GatewayLauncherError> {
        Self::production_with_runtime_files(listen, lkg_path, publication_path, None)
    }

    pub fn production_with_runtime_files(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
        publication_path: Option<&Path>,
        credential_path: Option<&Path>,
    ) -> Result<Self, GatewayLauncherError> {
        Self::production_with_runtime_files_and_planner(
            listen,
            lkg_path,
            publication_path,
            credential_path,
            None,
        )
    }

    pub fn production_with_runtime_files_and_planner(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
        publication_path: Option<&Path>,
        credential_path: Option<&Path>,
        planner_path: Option<&Path>,
    ) -> Result<Self, GatewayLauncherError> {
        Self::production_with_runtime_files_inner(
            listen,
            lkg_path,
            publication_path,
            credential_path,
            planner_path,
            GatewayLaunchIdentity::default(),
            #[cfg(feature = "e2e-test-control")]
            None,
        )
    }

    pub fn production_with_runtime_files_and_planner_with_identity(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
        publication_path: Option<&Path>,
        credential_path: Option<&Path>,
        planner_path: Option<&Path>,
        launch_identity: GatewayLaunchIdentity,
    ) -> Result<Self, GatewayLauncherError> {
        Self::production_with_runtime_files_inner(
            listen,
            lkg_path,
            publication_path,
            credential_path,
            planner_path,
            launch_identity,
            #[cfg(feature = "e2e-test-control")]
            None,
        )
    }

    /// The explicit test-control option is kept at the outermost production
    /// composition boundary. Normal startup passes `None`, so it constructs
    /// neither a control handle nor a listener.
    #[cfg(feature = "e2e-test-control")]
    pub fn production_with_runtime_files_planner_and_test_control(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
        publication_path: Option<&Path>,
        credential_path: Option<&Path>,
        planner_path: Option<&Path>,
        test_control_options: Option<E2eControlOptions>,
    ) -> Result<Self, GatewayLauncherError> {
        Self::production_with_runtime_files_inner(
            listen,
            lkg_path,
            publication_path,
            credential_path,
            planner_path,
            GatewayLaunchIdentity::default(),
            test_control_options,
        )
    }

    #[cfg(feature = "e2e-test-control")]
    pub fn production_with_runtime_files_planner_test_control_and_identity(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
        publication_path: Option<&Path>,
        credential_path: Option<&Path>,
        planner_path: Option<&Path>,
        test_control_options: Option<E2eControlOptions>,
        launch_identity: GatewayLaunchIdentity,
    ) -> Result<Self, GatewayLauncherError> {
        Self::production_with_runtime_files_inner(
            listen,
            lkg_path,
            publication_path,
            credential_path,
            planner_path,
            launch_identity,
            test_control_options,
        )
    }

    fn production_with_runtime_files_inner(
        listen: SocketAddr,
        lkg_path: impl AsRef<Path>,
        publication_path: Option<&Path>,
        credential_path: Option<&Path>,
        planner_path: Option<&Path>,
        launch_identity: GatewayLaunchIdentity,
        #[cfg(feature = "e2e-test-control")] test_control_options: Option<E2eControlOptions>,
    ) -> Result<Self, GatewayLauncherError> {
        let publications = Arc::new(GatewayPublicationInstaller::open(lkg_path)?);
        if let Some(publication_path) = publication_path {
            let bytes = std::fs::read(publication_path)
                .map_err(GatewayLauncherError::ProductionPublicationIo)?;
            let snapshot: GatewayPublicationSnapshotV3 = serde_json::from_slice(&bytes)
                .map_err(GatewayLauncherError::ProductionPublicationJson)?;
            if let GatewayPrepareOutcome::Prepared(prepared) = publications.prepare(snapshot)? {
                publications.publish(prepared)?;
            }
        }
        #[cfg(feature = "e2e-test-control")]
        let test_control = test_control_options
            .map(|options| {
                let handle = E2eControlHandle::new(Arc::clone(&publications));
                E2eControlEndpoint::new(options, handle)
            })
            .transpose()?;
        #[cfg(feature = "e2e-test-control")]
        if test_control
            .as_ref()
            .is_some_and(|control| control.listen() == listen)
        {
            return Err(E2eControlError::ListenerCollision.into());
        }
        let mut ports = ProductionPorts::fail_closed(publications);
        if let Some(credential_path) = credential_path {
            ports.credentials = Arc::new(FileCredentialResolver::open(credential_path)?);
            ports.runtime_state = Arc::new(crate::ports::InMemoryRuntimeStateStore::default());
        }
        #[cfg(feature = "e2e-test-control")]
        if let Some(control) = &test_control {
            ports.runtime_state = control
                .command_handle()
                .wrap_runtime_state(Arc::clone(&ports.runtime_state));
        }
        let planner: Arc<dyn PlannerInputAuthority> = match planner_path {
            Some(path) => Arc::new(FilePlannerInputAuthority::open(path)?),
            None => Arc::new(PublicationPlannerInputAuthority),
        };
        let runtime = ProductionGatewayRuntime::compose_with_planner(ports, planner);
        let runtime = match launch_identity.executable_sha256() {
            Some(digest) => runtime.with_executable_sha256(Arc::clone(digest)),
            None => runtime,
        };
        let runtime = Arc::new(runtime);
        Ok(Self {
            listen,
            mode: GatewayMode::Production {
                runtime,
                #[cfg(feature = "e2e-test-control")]
                test_control,
            },
            launch_identity,
        })
    }

    pub fn from_runtime(
        listen: SocketAddr,
        runtime: Arc<ProductionGatewayRuntime>,
    ) -> Result<Self, GatewayLauncherError> {
        Ok(Self {
            listen,
            mode: GatewayMode::Production {
                runtime,
                #[cfg(feature = "e2e-test-control")]
                test_control: None,
            },
            launch_identity: GatewayLaunchIdentity::default(),
        })
    }

    /// Starts the Pingora listener. The readiness probe and every product
    /// request traverse this same `GatewayHttpApp`.
    pub fn serve(self) -> Result<(), GatewayLauncherError> {
        self.into_server(ListenerRunStyle::Standalone)?
            .run_forever()
    }

    /// Starts the production listener on a managed thread and returns only
    /// after the real production readiness endpoint has answered.
    pub fn start_managed(self) -> Result<ManagedGatewayHandle, ManagedGatewayError> {
        self::managed::start(self)
    }

    pub(super) fn into_server(
        self,
        run_style: ListenerRunStyle,
    ) -> Result<pingora_core::server::Server, GatewayLauncherError> {
        let Self {
            listen,
            mode,
            launch_identity,
        } = self;
        #[cfg(feature = "e2e-test-control")]
        let (lifecycle, test_control) = match mode {
            GatewayMode::Oracle(fixture) => (
                ListenerLifecycle::Oracle(OracleFixtureAdapter {
                    listen,
                    nonce: fixture.readiness.nonce,
                    readiness_path: fixture.readiness.path,
                    executable_sha256: launch_identity.executable_sha256,
                }),
                None,
            ),
            GatewayMode::Production {
                runtime,
                test_control,
            } => (ListenerLifecycle::Production(runtime), test_control),
        };
        #[cfg(not(feature = "e2e-test-control"))]
        let lifecycle = match mode {
            GatewayMode::Oracle(fixture) => ListenerLifecycle::Oracle(OracleFixtureAdapter {
                listen,
                nonce: fixture.readiness.nonce,
                readiness_path: fixture.readiness.path,
                executable_sha256: launch_identity.executable_sha256,
            }),
            GatewayMode::Production { runtime } => ListenerLifecycle::Production(runtime),
        };
        let mut server = match run_style {
            ListenerRunStyle::Standalone => pingora_core::server::Server::new(None)
                .map_err(|error| GatewayLauncherError::Listener(error.to_string()))?,
            #[cfg(unix)]
            ListenerRunStyle::Managed => {
                let mut configuration = pingora_core::server::configuration::ServerConf::new()
                    .ok_or_else(|| {
                        GatewayLauncherError::Listener(
                            "cannot construct managed listener configuration".to_owned(),
                        )
                    })?;
                configuration.grace_period_seconds = Some(0);
                configuration.graceful_shutdown_timeout_seconds = Some(1);
                pingora_core::server::Server::new_with_opt_and_conf(None, configuration)
            }
        };
        server.bootstrap();
        let app = GatewayHttpApp::new(Arc::new(lifecycle));
        let mut service =
            pingora_core::services::listening::Service::new("hirouted".to_owned(), app);
        service.threads = Some(LISTENER_WORKER_THREADS);
        service.add_tcp(&listen.to_string());
        server.add_service(service);
        #[cfg(feature = "e2e-test-control")]
        if let Some(test_control) = test_control {
            let app = GatewayHttpApp::new(test_control.listener());
            let mut service = pingora_core::services::listening::Service::new(
                "hirouted-e2e-control".to_owned(),
                app,
            );
            service.threads = Some(1);
            service.add_tcp(&test_control.listen().to_string());
            server.add_service(service);
        }
        Ok(server)
    }
}

#[derive(Clone, Copy)]
pub(super) enum ListenerRunStyle {
    Standalone,
    #[cfg(unix)]
    Managed,
}

enum ListenerLifecycle {
    Oracle(OracleFixtureAdapter),
    Production(Arc<ProductionGatewayRuntime>),
}

#[async_trait]
impl GatewayLifecycle for ListenerLifecycle {
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError> {
        match self {
            Self::Oracle(lifecycle) => lifecycle.process(session).await,
            Self::Production(lifecycle) => lifecycle.process(session).await,
        }
    }
}

impl LaunchFixtureEnvelope {
    fn validate(&self) -> Result<(), GatewayLauncherError> {
        if self.schema_version != LAUNCH_FIXTURE_SCHEMA
            || self.oracle_version != "p0-gateway-oracle-v1"
            || !is_digest(&self.contract_digest)
            || self.publication.schema_version != PUBLICATION_SCHEMA
            || self.publication.authority.issuer.is_empty()
            || self.publication.authority.subject.is_empty()
            || self.publication.authority.policy_revision.is_empty()
            || self.publication.epoch == 0
            || self.publication.revision.is_empty()
            || self.publication.protocol_routes.is_empty()
            || self.publication.alias_index.is_empty()
            || self.publication.grant_index.is_empty()
            || self.publication.agent_plans.is_empty()
            || self.publication.adapter_catalog.is_empty()
            || !self.publication.model_catalog.is_object()
            || self.provider_bindings.is_empty()
            || self.secret_store.client_grants.is_empty()
            || self.secret_store.credentials.is_empty()
            || self.observation.run_nonce.len() < 32
            || self.readiness.nonce.len() < 32
            || !self.observation.directory.is_absolute()
            || !self.observation.terminal_receipt.is_absolute()
            || !self.readiness.path.is_absolute()
        {
            return Err(GatewayLauncherError::FixtureEnvelope);
        }
        for binding in &self.provider_bindings {
            if binding.binding_ref.is_empty()
                || !self
                    .secret_store
                    .credentials
                    .contains_key(&binding.credential_ref)
                || !binding.endpoint.starts_with("http://127.0.0.1:")
                || !matches!(
                    binding.protocol.as_str(),
                    "responses" | "chat_completions" | "messages"
                )
            {
                return Err(GatewayLauncherError::FixtureEnvelope);
            }
        }
        let mut value =
            serde_json::to_value(&self.publication).map_err(GatewayLauncherError::FixtureJson)?;
        let claimed = value
            .as_object_mut()
            .and_then(|object| object.remove("payload_digest"))
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .ok_or(GatewayLauncherError::FixtureEnvelope)?;
        if canonical_digest(&value) != claimed || contains_forbidden_case_key(&value) {
            return Err(GatewayLauncherError::PublicationDigest);
        }
        Ok(())
    }
}

/// Explicit compatibility adapter for the frozen PROCESS-22002 fixture. It is
/// selected only by `--fixture` and still traverses the shared Pingora app.
struct OracleFixtureAdapter {
    listen: SocketAddr,
    nonce: String,
    readiness_path: PathBuf,
    executable_sha256: Option<Arc<str>>,
}

#[async_trait]
impl GatewayLifecycle for OracleFixtureAdapter {
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError> {
        let request = session.request_head()?;
        let ready_path = format!("/_hiroute/oracle-ready/{}", self.nonce);
        let nonce_matches = request
            .headers
            .get("x-hiroute-oracle-nonce")
            .and_then(|value| value.to_str().ok())
            == Some(self.nonce.as_str());
        let (status, body) = if request.method == Method::GET
            && request.path_and_query.as_ref() == ready_path
            && nonce_matches
        {
            let ready = ReadyDocument {
                schema_version: READY_SCHEMA,
                run_nonce: &self.nonce,
                process_id: std::process::id(),
                listen_address: self.listen.to_string(),
                executable_sha256: self.executable_sha256.as_deref(),
            };
            write_ready(&self.readiness_path, &ready)
                .map_err(|error| TransportError::Io(error.to_string().into()))?;
            (
                StatusCode::OK,
                serde_json::to_vec(&ready)
                    .map(Bytes::from)
                    .map_err(|error| TransportError::Io(error.to_string().into()))?,
            )
        } else {
            (
                StatusCode::NOT_IMPLEMENTED,
                Bytes::from_static(NOT_IMPLEMENTED_BODY),
            )
        };
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(CONTENT_LENGTH, HeaderValue::from(body.len() as u64));
        headers.insert(CONNECTION, HeaderValue::from_static("close"));
        session
            .write_response_head(GatewayResponseHead { status, headers })
            .await?;
        session.write_response_body(body, true).await?;
        Ok(SessionReuse::Close)
    }
}

fn write_ready<T: Serialize>(path: &Path, value: &T) -> Result<(), GatewayLauncherError> {
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(GatewayLauncherError::ReadinessIo)?;
    let mut bytes = serde_json::to_vec(value).map_err(GatewayLauncherError::FixtureJson)?;
    bytes.push(b'\n');
    file.write_all(&bytes)
        .map_err(GatewayLauncherError::ReadinessIo)?;
    file.sync_all().map_err(GatewayLauncherError::ReadinessIo)
}

fn canonical_digest(value: &Value) -> String {
    let bytes = serde_json::to_vec(&canonicalize(value)).expect("JSON serialization cannot fail");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn contains_forbidden_case_key(value: &Value) -> bool {
    match value {
        Value::Object(values) => values.iter().any(|(key, value)| {
            matches!(key.as_str(), "case" | "case_id" | "cases")
                || contains_forbidden_case_key(value)
        }),
        Value::Array(values) => values.iter().any(contains_forbidden_case_key),
        _ => false,
    }
}

fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Error)]
pub enum GatewayLauncherError {
    #[error("hirouted supports only --role gateway, not {0}")]
    UnsupportedRole(String),
    #[error("gateway listener must use a reserved nonzero IPv4 address, got {0}")]
    ListenerMustUseReservedAddress(SocketAddr),
    #[error(transparent)]
    Publication(#[from] PublicationInstallError),
    #[error("cannot read production gateway publication: {0}")]
    ProductionPublicationIo(std::io::Error),
    #[error("production gateway publication is invalid JSON: {0}")]
    ProductionPublicationJson(serde_json::Error),
    #[error("production runtime authority configuration is invalid: {0}")]
    RuntimeAuthority(#[from] PortError),
    #[error("Tool continuation process authority key is unavailable")]
    ContinuationKeyUnavailable,
    #[cfg(feature = "e2e-test-control")]
    #[error(transparent)]
    TestControl(#[from] E2eControlError),
    #[error("cannot read gateway launch fixture: {0}")]
    FixtureIo(std::io::Error),
    #[error("gateway launch fixture is invalid JSON: {0}")]
    FixtureJson(serde_json::Error),
    #[error("gateway launch fixture envelope is incomplete")]
    FixtureEnvelope,
    #[error("gateway publication digest or SUT-visible case isolation failed")]
    PublicationDigest,
    #[error("gateway listener failed: {0}")]
    Listener(String),
    #[error("gateway readiness handshake failed: {0}")]
    ReadinessIo(std::io::Error),
    #[error(
        "{LAUNCHER_EXECUTABLE_SHA256_ENV} must be a lowercase sha256:<64 hex characters> digest"
    )]
    InvalidExecutableDigest,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_implemented_body_is_exact_typed_json() {
        let value: Value = serde_json::from_slice(NOT_IMPLEMENTED_BODY).unwrap();
        assert_eq!(value["code"], NOT_IMPLEMENTED_CODE);
    }

    #[test]
    fn launcher_identity_accepts_only_canonical_sha256() {
        assert!(
            GatewayLaunchIdentity::default()
                .executable_sha256()
                .is_none()
        );
        let digest = format!("sha256:{}", "a".repeat(64));
        assert!(GatewayLaunchIdentity::from_executable_sha256(digest).is_ok());
        for invalid in [
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}", "a".repeat(63)),
            "a".repeat(64),
        ] {
            assert!(matches!(
                GatewayLaunchIdentity::from_executable_sha256(invalid),
                Err(GatewayLauncherError::InvalidExecutableDigest)
            ));
        }
    }
}
