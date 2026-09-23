//! `hirouted --role=all` composition with one Product truth and one exact Gateway runtime.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hiroute_application::ApplicationService;
use hiroute_application::control::{ClassifierDiagnosticPort, ControlReadError};
use hiroute_application_api::{
    CLASSIFIER_DECISION_TEST_LATEST_USER, CLASSIFIER_DECISION_TEST_RESULT_SCHEMA_V1,
    ClassifierDecisionTestOutcomeV1, ClassifierDecisionTestRequestV1,
    ClassifierDecisionTestResultV1,
};
use hiroute_cpa_bridge::{
    BorrowedCodexAuthSpec, CpaAccountKind, CpaAttemptError, CpaDownstreamCredentialPort,
    CpaProfileBinding, CpaRuntimeSpec, ExactCpaCredentialRequest, MANAGED_CPA_ARTIFACT_VERSION,
    ManagedCpaRuntime, PinnedCpaArtifact, PinnedCpaBinaryLocator, RestartPolicy,
};
use hiroute_diagnostics::event::{StageOutcome, StartupFailureCode, StartupStage};
use hiroute_diagnostics::runtime::DiagnosticsPort;
use hiroute_gateway::server::composition::{ProductionPorts, PublicationPlannerInputAuthority};
use hiroute_gateway::server::core_runtime::ProductionGatewayRuntime;
use hiroute_gateway::server::core_runtime::observation::{
    GatewayObservation, GatewayObservationSinks, OtelContentPolicy,
};
use hiroute_gateway::server::publication::GatewayPublicationInstaller;
use hiroute_gateway::server::{GatewayLauncher, ManagedGatewayHandle, ManagedGatewayPhase};
use hiroute_host_runtime::{connect_address, validate_gateway_listen};
use hiroute_local_storage::ApplyCapabilityRegistrationV1;
use hiroute_observation::BoundedLifecycleReceiverV2;

use crate::control::{
    ManagedControlHandle, ManagedControlPhase, ProductionControlRuntime,
    start_control_with_agent_grants,
};
use crate::gateway_ports::{
    GatewayConversationContentSink, GatewayCredentialResolver, GatewayExecutionFactSink,
    GatewayLifecycleTelemetrySink, GatewayPublicationAdapter, GatewayRequestPriceSource,
    GatewayRunRelationSink, GatewayRuntimeStateStore,
};

const OBSERVATION_CHANNEL_BYTES: usize = 4 * 1024 * 1024;
const LIFECYCLE_RECORD_CAPACITY: usize = 2_048;
const COMPONENT_JOIN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct RoleAllConfig {
    pub storage_root: PathBuf,
    pub runtime_root: PathBuf,
    pub gateway_listen: SocketAddr,
    pub gateway_lkg: PathBuf,
    pub cpa: Option<RoleAllCpaConfig>,
    /// Codex Desktop's application-owned engine as resolved by the native host. CLI-only
    /// compositions leave it absent and never substitute PATH for the Desktop surface.
    pub codex_desktop_engine: Option<PathBuf>,
    /// Optional staged diagnostics; `None` is a no-op port for tests and embedded hosts.
    pub diagnostics: DiagnosticsPort,
    /// Standalone exposes only the current public release manifest at Local Control.
    pub released_commands_only: bool,
}

#[derive(Clone, Debug)]
pub struct RoleAllCpaConfig {
    pub binary: PathBuf,
    pub expected_sha256_hex: String,
}

impl RoleAllConfig {
    pub fn new(
        storage_root: impl Into<PathBuf>,
        runtime_root: impl Into<PathBuf>,
        gateway_listen: SocketAddr,
        gateway_lkg: impl Into<PathBuf>,
    ) -> Self {
        Self {
            storage_root: storage_root.into(),
            runtime_root: runtime_root.into(),
            gateway_listen,
            gateway_lkg: gateway_lkg.into(),
            cpa: None,
            codex_desktop_engine: None,
            diagnostics: DiagnosticsPort::default(),
            released_commands_only: false,
        }
    }

    pub fn with_diagnostics(mut self, diagnostics: DiagnosticsPort) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    pub fn with_cpa(mut self, cpa: RoleAllCpaConfig) -> Self {
        self.cpa = Some(cpa);
        self
    }

    pub fn with_codex_desktop_engine(mut self, engine: PathBuf) -> Self {
        self.codex_desktop_engine = Some(engine);
        self
    }

    pub fn with_released_commands_only(mut self) -> Self {
        self.released_commands_only = true;
        self
    }
}

pub struct RoleAllHandle {
    control: ManagedControlHandle,
    gateway: ManagedGatewayHandle,
    cpa: Option<Arc<ManagedCpaRuntime>>,
    _runtime: ProductionControlRuntime,
}

struct GatewayClassifierDiagnostic {
    runtime: Arc<ProductionGatewayRuntime>,
}

impl ClassifierDiagnosticPort for GatewayClassifierDiagnostic {
    fn test_classifier_decision(
        &self,
        request: &ClassifierDecisionTestRequestV1,
    ) -> Result<ClassifierDecisionTestResultV1, ControlReadError> {
        debug_assert_eq!(
            CLASSIFIER_DECISION_TEST_LATEST_USER,
            hiroute_gateway::server::core_runtime::CLASSIFIER_DIAGNOSTIC_LATEST_USER
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| ControlReadError::Unavailable)?;
        let result = runtime.block_on(self.runtime.test_classifier_decision(&request.classifier));
        Ok(match result {
            Ok(outcome) => ClassifierDecisionTestResultV1 {
                schema: CLASSIFIER_DECISION_TEST_RESULT_SCHEMA_V1.into(),
                outcome: ClassifierDecisionTestOutcomeV1::Passed,
                branch_id: Some(outcome.branch_id),
                duration_millis: Some(outcome.duration_millis),
                failure_code: None,
            },
            Err(error) => ClassifierDecisionTestResultV1 {
                schema: CLASSIFIER_DECISION_TEST_RESULT_SCHEMA_V1.into(),
                outcome: ClassifierDecisionTestOutcomeV1::Failed,
                branch_id: None,
                duration_millis: None,
                failure_code: Some(error.code().into()),
            },
        })
    }
}

impl RoleAllHandle {
    pub fn control_endpoint(&self) -> &crate::ControlEndpoint {
        self.control.endpoint()
    }

    pub fn gateway_address(&self) -> SocketAddr {
        self.gateway.listen_address()
    }

    pub fn phases(&self) -> (ManagedControlPhase, ManagedGatewayPhase) {
        (self.control.phase(), self.gateway.phase())
    }

    /// Accepts only the sealed registration value produced after validation of the inherited
    /// launcher channel. The socket API has no route to this method.
    pub fn register_apply_capability(
        &self,
        registration: ApplyCapabilityRegistrationV1,
    ) -> Result<(), String> {
        self._runtime.register_apply_capability(registration)
    }

    pub fn register_manual_protected_input(
        &self,
        candidate: hiroute_application_api::ComputeCandidateRefV2,
        secret: hiroute_domain::ProtectedSecret,
    ) -> Result<(), String> {
        self._runtime
            .register_manual_protected_input(candidate, secret)
    }

    pub fn release_manual_protected_input(&self, candidate_ref: &str) -> Result<(), String> {
        self._runtime.release_manual_protected_input(candidate_ref)
    }

    pub fn shutdown(&self) {
        // Reverse startup order: reject new control work before stopping request traffic. The
        // Gateway-owned bounded observation producers drain as their runtime is dropped.
        self.control.shutdown();
        self.gateway.shutdown();
    }

    pub fn join(&mut self, timeout: Duration) -> Result<(), RoleAllError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(RoleAllError::JoinTimeout("role-all"))?;
        self.control
            .join(remaining(deadline, "Local Control")?)
            .map_err(|error| RoleAllError::Component("Local Control", error.to_string()))?;
        self.gateway
            .join(remaining(deadline, "Gateway")?)
            .map_err(|error| RoleAllError::Component("Gateway", error.to_string()))?;
        if let Some(cpa) = &self.cpa {
            match cpa.shutdown() {
                Ok(_) | Err(hiroute_cpa_bridge::CpaLifecycleError::NotStarted) => {}
                Err(error) => return Err(RoleAllError::Component("CPA", error.to_string())),
            }
        }
        Ok(())
    }
}

impl Drop for RoleAllHandle {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(cpa) = &self.cpa {
            let _ = cpa.shutdown();
        }
    }
}

pub fn start_role_all(config: RoleAllConfig) -> Result<RoleAllHandle, RoleAllError> {
    if validate_gateway_listen(config.gateway_listen).is_err() {
        return Err(RoleAllError::InvalidConfiguration);
    }

    // Verify one catalog before either CPA or Local Control can consume it. CPA construction is
    // side-effect free; an approved subscription check starts it lazily.
    config.diagnostics.stage_begin(StartupStage::CatalogLoad);
    let catalog = match crate::release_catalog::load_production_release_catalog() {
        Ok(catalog) => {
            config
                .diagnostics
                .stage_end(StartupStage::CatalogLoad, StageOutcome::Completed);
            catalog
        }
        Err(error) => {
            config.diagnostics.stage_end(
                StartupStage::CatalogLoad,
                StageOutcome::Failed {
                    code: StartupFailureCode::ReleaseFactsInvalid,
                },
            );
            return Err(RoleAllError::Component("Release facts", error));
        }
    };
    config
        .diagnostics
        .stage_begin(StartupStage::ArtifactValidate);
    let cpa = match config
        .cpa
        .as_ref()
        .map(|cpa| start_cpa(&config, cpa, catalog.clone()))
        .transpose()
    {
        Ok(cpa) => {
            config
                .diagnostics
                .stage_end(StartupStage::ArtifactValidate, StageOutcome::Completed);
            cpa
        }
        Err(error) => {
            config.diagnostics.stage_end(
                StartupStage::ArtifactValidate,
                StageOutcome::Failed {
                    code: StartupFailureCode::DependencyUnavailable,
                },
            );
            return Err(error);
        }
    };
    config
        .diagnostics
        .stage_begin(StartupStage::PublicationReconcile);
    let runtime = ProductionControlRuntime::prepare_for_role_all_with_codex_desktop_engine(
        &config.storage_root,
        catalog,
        cpa.clone(),
        config.codex_desktop_engine.clone(),
    )
    .map_err(|error| RoleAllError::Component("Storage", error));
    let runtime = match runtime {
        Ok(runtime) => {
            config
                .diagnostics
                .stage_end(StartupStage::PublicationReconcile, StageOutcome::Completed);
            runtime
        }
        Err(error) => {
            config.diagnostics.stage_end(
                StartupStage::PublicationReconcile,
                StageOutcome::Failed {
                    code: StartupFailureCode::StorageUnavailable,
                },
            );
            if let Some(cpa) = &cpa {
                let _ = cpa.shutdown();
            }
            return Err(error);
        }
    };
    runtime.set_diagnostics(config.diagnostics.clone());
    start_role_all_with_runtime(config, runtime, cpa)
}

fn start_role_all_with_runtime(
    config: RoleAllConfig,
    runtime: ProductionControlRuntime,
    cpa: Option<Arc<ManagedCpaRuntime>>,
) -> Result<RoleAllHandle, RoleAllError> {
    let installer = Arc::new(
        GatewayPublicationInstaller::open(&config.gateway_lkg)
            .map_err(|error| RoleAllError::Component("Gateway publication", error.to_string()))?,
    );
    let publications = Arc::new(GatewayPublicationAdapter::new(installer));
    let observation_store = runtime.observation_store();
    let lifecycle = Arc::new(
        BoundedLifecycleReceiverV2::new(LIFECYCLE_RECORD_CAPACITY)
            .map_err(|error| RoleAllError::Component("Lifecycle telemetry", error.to_string()))?,
    );
    let lifecycle_sink = Arc::new(GatewayLifecycleTelemetrySink::new(lifecycle));
    let execution_sink = Arc::new(GatewayExecutionFactSink::new(Arc::clone(
        &observation_store,
    )));
    let content_sink = Arc::new(GatewayConversationContentSink::new(Arc::clone(
        &observation_store,
    )));
    let run_relation_sink = Arc::new(GatewayRunRelationSink::new(observation_store));
    let otel_sink = GatewayObservationSinks::discard().otel;
    // Consume the same instance installed by the price control writer/recovery.
    // Exact source/model/profile identities come from each request's pinned
    // publication; this port supplies only the independent current generation.
    let price_source = GatewayRequestPriceSource::new(runtime.price_snapshot_slot());
    let observation = Arc::new(
        GatewayObservation::with_sinks_and_policy_and_workspace_key(
            true,
            OBSERVATION_CHANNEL_BYTES,
            GatewayObservationSinks {
                lifecycle: Arc::clone(&lifecycle_sink) as _,
                execution_fact: Arc::clone(&execution_sink) as _,
                conversation_content: Arc::clone(&content_sink) as _,
                run_relation: run_relation_sink,
                otel: otel_sink,
            },
            OtelContentPolicy::Disabled,
            runtime.observation_workspace_key(),
        )
        .with_price_source(Arc::new(price_source))
        .with_diagnostics(config.diagnostics.clone()),
    );
    let credentials = Arc::new(GatewayCredentialResolver::new(
        Arc::new(runtime.native_credential_authority()),
        Arc::new(RoleAllCpaCredentials(cpa.clone())),
    ));
    let runtime_state = Arc::new(
        GatewayRuntimeStateStore::new(runtime.compute_runtime_state_store())
            .map_err(|error| RoleAllError::Component("Runtime state", error.to_string()))?,
    );
    let ports = ProductionPorts {
        publications: Arc::clone(&publications) as _,
        credentials,
        runtime_state,
        lifecycle: lifecycle_sink,
        execution_facts: execution_sink,
        conversation_content: content_sink,
    };
    let gateway_runtime = Arc::new(
        ProductionGatewayRuntime::compose_with_planner_and_observation(
            ports,
            Arc::new(PublicationPlannerInputAuthority),
            observation,
        )
        .with_run_request_authority(runtime.delegation_run_authority()),
    );
    config.diagnostics.stage_begin(StartupStage::GatewayStart);
    let gateway =
        match GatewayLauncher::from_runtime(config.gateway_listen, Arc::clone(&gateway_runtime))
            .map_err(|error| RoleAllError::Component("Gateway", error.to_string()))
            .and_then(|launcher| {
                launcher
                    .start_managed()
                    .map_err(|error| RoleAllError::Component("Gateway", error.to_string()))
            }) {
            Ok(gateway) => {
                config
                    .diagnostics
                    .stage_end(StartupStage::GatewayStart, StageOutcome::Completed);
                gateway
            }
            Err(error) => {
                config.diagnostics.stage_end(
                    StartupStage::GatewayStart,
                    StageOutcome::Failed {
                        code: StartupFailureCode::GatewayUnavailable,
                    },
                );
                return Err(error);
            }
        };
    let bundled_hiroute = std::env::current_exe()
        .map_err(|error| RoleAllError::Component("Local Control", error.to_string()))?
        .with_file_name("hiroute");
    let standalone_home = if config.released_commands_only {
        Some(
            hiroute_host_runtime::StandaloneLayout::from_environment()
                .map_err(|_| RoleAllError::InvalidConfiguration)?
                .home,
        )
    } else {
        None
    };
    let trusted_hiroute = managed_cli_entry(&bundled_hiroute, standalone_home.as_deref())?
        .into_os_string()
        .into_string()
        .map_err(|_| RoleAllError::InvalidConfiguration)?;
    let managed_agent_gateway = connect_address(gateway.listen_address())
        .map_err(|_| RoleAllError::InvalidConfiguration)?;
    if let Err(error) = runtime.configure_managed_agent_runtime_with_resident_service(
        format!("http://{managed_agent_gateway}/v1"),
        trusted_hiroute,
        Some(publications),
        config.released_commands_only,
    ) {
        gateway.shutdown();
        let mut gateway = gateway;
        let _ = gateway.join(COMPONENT_JOIN_TIMEOUT);
        return Err(RoleAllError::Component("Local Control", error));
    }
    let application = ApplicationService::new(
        runtime
            .application_ports()
            .with_role_all_ready()
            .with_classifier_diagnostic(Arc::new(GatewayClassifierDiagnostic {
                runtime: gateway_runtime,
            })),
    );
    config.diagnostics.stage_begin(StartupStage::ControlBind);
    let control = match start_control_with_agent_grants(
        application,
        &config.runtime_root,
        runtime.agent_grant_resolver(),
        config.released_commands_only,
    ) {
        Ok(control) => {
            config
                .diagnostics
                .stage_end(StartupStage::ControlBind, StageOutcome::Completed);
            control
        }
        Err(error) => {
            config.diagnostics.stage_end(
                StartupStage::ControlBind,
                StageOutcome::Failed {
                    code: StartupFailureCode::ControlUnavailable,
                },
            );
            gateway.shutdown();
            let mut gateway = gateway;
            let _ = gateway.join(COMPONENT_JOIN_TIMEOUT);
            return Err(RoleAllError::Component("Local Control", error.to_string()));
        }
    };
    Ok(RoleAllHandle {
        control,
        gateway,
        cpa,
        _runtime: runtime,
    })
}

fn start_cpa(
    config: &RoleAllConfig,
    cpa: &RoleAllCpaConfig,
    catalog: hiroute_integrations::TrustedReleaseCatalog,
) -> Result<Arc<ManagedCpaRuntime>, RoleAllError> {
    if !config.storage_root.is_absolute()
        || !cpa.binary.is_absolute()
        || cpa.expected_sha256_hex.len() != 64
    {
        return Err(RoleAllError::InvalidConfiguration);
    }
    let binary_name = cpa
        .binary
        .file_name()
        .ok_or(RoleAllError::InvalidConfiguration)?;
    let trusted_root = cpa
        .binary
        .parent()
        .ok_or(RoleAllError::InvalidConfiguration)?;
    let artifact = PinnedCpaArtifact::new(
        trusted_root,
        binary_name,
        MANAGED_CPA_ARTIFACT_VERSION
            .parse()
            .map_err(|_| RoleAllError::InvalidConfiguration)?,
        &cpa.expected_sha256_hex,
    );
    let spec = CpaRuntimeSpec {
        instance_id: "hiroute-codex".into(),
        state_root: config.storage_root.join("cpa/state"),
        auth_dir: config.storage_root.join("cpa/auth"),
        borrowed_codex_auth: Some(BorrowedCodexAuthSpec::new(selected_codex_auth()?)),
        bindings: vec![CpaProfileBinding {
            account_kind: CpaAccountKind::Codex,
            connector_id: "connector.cpa.codex".into(),
            connection_option_id: "codex.subscription.global.v1".into(),
            endpoint_profile_id: "endpoint.cpa.codex".into(),
        }],
        startup_timeout: Duration::from_secs(20),
        control_timeout: Duration::from_secs(5),
        shutdown_timeout: Duration::from_secs(5),
        restart_policy: RestartPolicy::default(),
    };
    let runtime = Arc::new(
        ManagedCpaRuntime::new(
            spec,
            Arc::new(catalog),
            Arc::new(PinnedCpaBinaryLocator::new(artifact)),
        )
        .map_err(|error| RoleAllError::Component("CPA", error.to_string()))?
        .with_diagnostics(config.diagnostics.clone()),
    );
    Ok(runtime)
}

fn selected_codex_auth() -> Result<PathBuf, RoleAllError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(RoleAllError::InvalidConfiguration)?;
    let root = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    if !root.is_absolute() {
        return Err(RoleAllError::InvalidConfiguration);
    }
    Ok(root.join("auth.json"))
}

fn managed_cli_entry(
    bundled: &std::path::Path,
    standalone_home: Option<&std::path::Path>,
) -> Result<PathBuf, RoleAllError> {
    let Some(home) = standalone_home else {
        return Ok(bundled.to_owned());
    };
    // Persist the installer-owned entry, whose target changes on upgrade, only
    // after proving it identifies the CLI shipped with this running daemon.
    let stable = home.join(".local/bin/hiroute");
    let target = std::fs::canonicalize(&stable).map_err(|_| RoleAllError::InvalidConfiguration)?;
    let expected =
        std::fs::canonicalize(bundled).map_err(|_| RoleAllError::InvalidConfiguration)?;
    if target != expected {
        return Err(RoleAllError::InvalidConfiguration);
    }
    Ok(stable)
}

fn remaining(deadline: Instant, component: &'static str) -> Result<Duration, RoleAllError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(RoleAllError::JoinTimeout(component))
}

struct RoleAllCpaCredentials(Option<Arc<ManagedCpaRuntime>>);

impl CpaDownstreamCredentialPort for RoleAllCpaCredentials {
    fn lease_downstream_capability(
        &self,
        request: ExactCpaCredentialRequest<'_>,
    ) -> Result<Option<hiroute_cpa_bridge::CpaDownstreamCredentialCapability>, CpaAttemptError>
    {
        self.0
            .as_ref()
            .ok_or(CpaAttemptError::Unavailable)?
            .lease_downstream_capability(request)
    }
}

#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum RoleAllError {
    #[error("role=all configuration is invalid")]
    InvalidConfiguration,
    #[error("{0} component failed: {1}")]
    Component(&'static str, String),
    #[error("{0} did not stop before the role=all deadline")]
    JoinTimeout(&'static str),
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::os::unix::net::UnixStream;

    use hiroute_application_api::{
        CanonicalDigest, ClientHelloV1, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2,
        MACHINE_ENVELOPE_SCHEMA_V2, MachineEnvelopeV2, PrincipalKind,
    };
    use hiroute_domain::{RevisionSetV1, WorkspaceId};
    use hiroute_local_storage::ApplyCapabilityRegistrationV1;
    use serde_json::json;

    use super::*;

    #[test]
    fn standalone_agent_helper_uses_verified_stable_entry_across_upgrade() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        let stable = home.join(".local/bin/hiroute");
        std::fs::create_dir_all(stable.parent().unwrap()).unwrap();
        let first = home.join("v1/hiroute");
        let second = home.join("v2/hiroute");
        for binary in [&first, &second] {
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            std::fs::write(binary, b"binary").unwrap();
        }
        assert!(managed_cli_entry(&first, Some(home)).is_err());
        std::os::unix::fs::symlink(&first, &stable).unwrap();
        let persisted = managed_cli_entry(&first, Some(home)).unwrap();
        assert_eq!(persisted, stable);
        assert!(managed_cli_entry(&second, Some(home)).is_err());
        std::fs::remove_file(&stable).unwrap();
        std::os::unix::fs::symlink(&second, &stable).unwrap();
        std::fs::remove_file(&first).unwrap();
        assert_eq!(managed_cli_entry(&second, Some(home)).unwrap(), persisted);
        assert_eq!(std::fs::canonicalize(persisted).unwrap(), second);
        assert_eq!(managed_cli_entry(&second, None).unwrap(), second);
    }

    fn reserve_loopback() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        address
    }

    #[test]
    fn production_role_all_owns_real_gateway_control_and_observation_lifetimes() {
        #[cfg(unix)]
        if crate::test_support::isolated_agent_home(
            "role_all::tests::production_role_all_owns_real_gateway_control_and_observation_lifetimes",
        ) {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let config = RoleAllConfig::new(
            directory.path().join("storage"),
            directory.path().join("runtime"),
            reserve_loopback(),
            directory.path().join("gateway-lkg.json"),
        );
        let runtime = ProductionControlRuntime::open_with_release_catalog(
            &config.storage_root,
            crate::release_catalog::fixture_catalog(),
        )
        .unwrap();
        let mut role = start_role_all_with_runtime(config, runtime, None).unwrap();
        assert_eq!(
            role.phases(),
            (ManagedControlPhase::Ready, ManagedGatewayPhase::Ready)
        );
        let capability = "0123456789abcdef0123456789abcdef0123456789abcdef";
        let revisions = RevisionSetV1 {
            target: 0,
            dependencies: Default::default(),
        };
        let accepted = CanonicalDigest::of_bytes(b"role-all-exact-change");
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 60;
        role.register_apply_capability(
            ApplyCapabilityRegistrationV1::from_protected_launcher(
                capability.to_owned(),
                "interactive-user",
                WorkspaceId::default(),
                "ApplySetup",
                accepted.clone(),
                revisions.clone(),
                expiry,
            )
            .unwrap(),
        )
        .unwrap();
        role._runtime
            .application_ports()
            .control
            .validate_protected_capability(
                capability,
                &WorkspaceId::default(),
                PrincipalKind::InteractiveUser,
                "ApplySetup",
                &accepted,
                &revisions,
            )
            .unwrap();
        let endpoint = role.control_endpoint().path().to_path_buf();
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        serde_json::to_writer(
            &mut stream,
            &ClientHelloV1 {
                api_version: LOCAL_CONTROL_SCHEMA_V2,
                machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
                client_name: "role-all-test".to_owned(),
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        assert!(!response.is_empty());
        let _: hiroute_application_api::ServerHelloV1 = serde_json::from_str(response.trim())
            .unwrap_or_else(|error| panic!("invalid server hello {response:?}: {error}"));
        serde_json::to_writer(
            &mut stream,
            &LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "role-all-status".to_owned(),
                operation_id: "GetSystemStatus".to_owned(),
                payload: json!({}),
                protected_grant: None,
            },
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        response.clear();
        reader.read_line(&mut response).unwrap();
        let envelope: MachineEnvelopeV2<serde_json::Value> =
            serde_json::from_str(response.trim()).unwrap();
        assert_eq!(
            envelope.status,
            hiroute_application_api::MachineStatus::Succeeded
        );
        let mut stream = UnixStream::connect(&endpoint).unwrap();
        serde_json::to_writer(
            &mut stream,
            &ClientHelloV1 {
                api_version: LOCAL_CONTROL_SCHEMA_V2,
                machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
                client_name: "role-all-worker-test".to_owned(),
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
            },
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        response.clear();
        reader.read_line(&mut response).unwrap();
        let _: hiroute_application_api::ServerHelloV1 =
            serde_json::from_str(response.trim()).unwrap();
        serde_json::to_writer(
            &mut stream,
            &LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: "worker-availability".to_owned(),
                operation_id: "WorkerExecutorAvailability".to_owned(),
                payload: json!({}),
                protected_grant: None,
            },
        )
        .unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        response.clear();
        reader.read_line(&mut response).unwrap();
        let availability: MachineEnvelopeV2<
            hiroute_application_api::WorkerExecutorAvailabilityListV1,
        > = serde_json::from_str(response.trim()).unwrap();
        assert_eq!(
            availability.status,
            hiroute_application_api::MachineStatus::Succeeded
        );
        let availability = availability.data.unwrap();
        assert_eq!(
            availability.executors[0].state,
            hiroute_application_api::WorkerExecutorAvailabilityStateV1::Unavailable
        );
        assert_eq!(
            availability.executors[0].reason,
            Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured)
        );
        assert_eq!(
            availability.executors[1].reason,
            Some(hiroute_application_api::WorkerExecutorAvailabilityReasonV1::InstallationNotConfigured)
        );

        role.shutdown();
        role.join(Duration::from_secs(15)).unwrap();
        assert!(!endpoint.exists());
    }
}
