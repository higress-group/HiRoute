use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_application::control::{
    AgentConfigPermissionFindingV1, AgentDiscoveryPort, AgentModelCatalogMetadataSourceV1,
    AgentModelSourceCoverageStateV1, ApplicationClockPort, ApplicationPorts, ControlReadError,
    ControlStatePort, ControlStateSnapshotV1, DiscoveredAgentConfigurationV1,
    DiscoveredAgentModelCatalogV1, DiscoveredAgentModelSourceV1, DiscoveredAgentModelV1,
    DiscoveredAgentV1, DiscoveredCredentialInputV1, ValuePlanScopeV1, ValueScopePort,
};
use hiroute_application::delegation::safety::RunSafetyProjection;
use hiroute_application::publication::PublicationTargetPort;
use hiroute_application_api::{
    AgentPlanId, CanonicalDigest, PrepareDiscoveredModelConnectionRequestV1, PrincipalKind,
    RevisionSetV1,
};
use hiroute_cpa_bridge::{BorrowedCodexEvidence, CpaRegisteredSourcePort, ManagedCpaRuntime};
use hiroute_diagnostics::runtime::DiagnosticsPort;
use hiroute_domain::{
    AgentConnectionEffectRoleV1, AgentConnectionTransactionSubjectV1, AgentKindV1,
    ComputeRuntimeStateStoreV1, ControlRepositoryPort, HeaderSecretLeaseRequestV1,
    MaterializationState, NativeCredentialAuthorityV1, NativeCredentialLeaseRequestV1,
    NativeCredentialLeaseV1, OperationId, OperationV1, PortError, PortErrorCode, PortResult,
    ProtectedApplyCapability, ProtectedSecret, RuntimeProbeAcquireOutcomeV1,
    RuntimeProbeLeaseRequestV1, RuntimeProbeLeaseV1, RuntimeStateIdentityV1, RuntimeStateV1,
    WorkspaceId,
};
use hiroute_integrations::{
    AgentDiscoveryOutcomeV1, AgentFilesystemLayoutV1, CLAUDE_INTEGRATION_PROFILE_REF_V1,
    CLAUDE_PROFILE_ID_V1, CODEX_INTEGRATION_PROFILE_REF_V1, CODEX_PROFILE_ID_V1,
    ClaudeRegistrationIndexV1, DiscoveredCredentialRefV1, FilesystemAgentDiscoveryV1,
    FilesystemAgentScannerV1, PermissionHardeningRequiredV1, ProtectedAgentSubscriptionSourceV1,
    TrustedReleaseCatalog,
};
use hiroute_local_storage::{ApplyCapabilityRegistrationV1, LocalStorageSet, ManagedArtifactStore};
use hiroute_observation::{DigestAuthority, LocalObservationStore};
use zeroize::Zeroizing;

use crate::delegation::dispatcher::DelegationCancellationDispatcher;
use crate::delegation::executor::{DelegationRunExecutor, UnavailableWorkerProfileSource};
use crate::delegation::finalization::DelegationFinalization;
use crate::delegation::installation::{
    WorkerExecutorAvailabilityRegistry, WorkerInstallationConfig, WorkerInstallationSelection,
    WorkerInstallationSelectionSource, managed_profile_source,
};
use crate::delegation::local_worker::LocalWorkerPlatform;
use crate::delegation::run_authority::DelegationRunAuthority;

mod agent_connection;
mod agent_live_check;
mod candidate_execution;
mod client_access;
mod collaboration_installation;
mod collaboration_status;
mod compute_routing;
mod delegation_continue;
mod delegation_cursors;
mod delegation_maintenance;
mod delegation_reads;
mod delegation_results;
mod delegation_runtime_port;
mod delegation_task_queries;
mod delegation_tasks;
mod delegation_worker;
mod effects;
mod model_catalog;
mod model_connections;
mod mutation;
mod native_claude_model;
mod native_model;
mod plan_content;
mod plan_content_snapshot;
mod plan_versions;
mod prices;
mod protected_inputs;
mod publication;
mod release_install;
mod routing;
mod settings_facts;
mod settings_status;
mod source_authorization;
mod subscriptions;
mod work_plans;

pub struct ProductionControlRuntime {
    _observation_maintenance: Option<hiroute_observation::maintenance::ObservationMaintenance>,
    adapter: Arc<LocalControlAdapter>,
    observation: Arc<LocalObservationStore>,
    observation_workspace_key: Zeroizing<[u8; 32]>,
    delegation_executor: Arc<DelegationRunExecutor>,
    delegation_storage_root: PathBuf,
}

#[derive(Clone)]
struct CodexSubscriptionContextV1 {
    source: ProtectedAgentSubscriptionSourceV1,
    evidence: Option<BorrowedCodexEvidence>,
    candidate_revision: u64,
}

#[derive(Default)]
struct RuntimeOpenOverrides {
    scanner: Option<FilesystemAgentScannerV1>,
    model_transport: Option<Arc<dyn hiroute_integrations::ModelDirectoryTransportV1>>,
    codex_desktop_engine: Option<PathBuf>,
}

impl ProductionControlRuntime {
    pub fn open(storage_root: impl AsRef<Path>) -> Result<Self, String> {
        let catalog = crate::release_catalog::load_production_release_catalog()?;
        Self::open_inner(storage_root.as_ref(), catalog)
    }

    /// Opens the same production composition with an already client-bound ReleaseFacts
    /// catalog. Raw bundle bytes and bundle-validation decisions never enter Application or Local
    /// Control.
    pub fn open_with_release_catalog(
        storage_root: impl AsRef<Path>,
        catalog: TrustedReleaseCatalog,
    ) -> Result<Self, String> {
        Self::open_inner(storage_root.as_ref(), catalog)
    }

    fn open_inner(
        storage_root: &Path,
        release_catalog: TrustedReleaseCatalog,
    ) -> Result<Self, String> {
        Self::open_inner_with_cpa(storage_root, release_catalog, None, None, true)
    }

    #[cfg(test)]
    fn open_with_release_catalog_and_model_transport(
        storage_root: impl AsRef<Path>,
        release_catalog: TrustedReleaseCatalog,
        model_transport: Arc<dyn hiroute_integrations::ModelDirectoryTransportV1>,
    ) -> Result<Self, String> {
        Self::open_inner_with_cpa_and_overrides(
            storage_root.as_ref(),
            release_catalog,
            None,
            None,
            true,
            RuntimeOpenOverrides {
                model_transport: Some(model_transport),
                ..RuntimeOpenOverrides::default()
            },
        )
    }

    /// Opens Local Control with the managed CPA target authority owned by the `role=all`
    /// composition. Control-only mode intentionally omits CPA candidates: without this authority
    /// it cannot truthfully publish a live loopback target or its runtime epochs.
    pub fn open_with_release_catalog_and_cpa(
        storage_root: impl AsRef<Path>,
        release_catalog: TrustedReleaseCatalog,
        cpa: Arc<ManagedCpaRuntime>,
    ) -> Result<Self, String> {
        let cpa_sources: Arc<dyn CpaRegisteredSourcePort + Send + Sync> = cpa.clone();
        Self::open_inner_with_cpa(
            storage_root.as_ref(),
            release_catalog,
            Some(cpa_sources),
            Some(cpa),
            true,
        )
    }

    #[cfg(test)]
    pub(crate) fn prepare_for_role_all(
        storage_root: &Path,
        catalog: TrustedReleaseCatalog,
        cpa: Option<Arc<ManagedCpaRuntime>>,
    ) -> Result<Self, String> {
        Self::prepare_for_role_all_with_codex_desktop_engine(storage_root, catalog, cpa, None)
    }

    pub(crate) fn prepare_for_role_all_with_codex_desktop_engine(
        storage_root: &Path,
        catalog: TrustedReleaseCatalog,
        cpa: Option<Arc<ManagedCpaRuntime>>,
        codex_desktop_engine: Option<PathBuf>,
    ) -> Result<Self, String> {
        let sources = cpa
            .as_ref()
            .map(|cpa| cpa.clone() as Arc<dyn CpaRegisteredSourcePort + Send + Sync>);
        Self::open_inner_with_cpa_and_overrides(
            storage_root,
            catalog,
            sources,
            cpa,
            false,
            RuntimeOpenOverrides {
                codex_desktop_engine,
                ..RuntimeOpenOverrides::default()
            },
        )
    }

    fn open_inner_with_cpa(
        storage_root: &Path,
        release_catalog: TrustedReleaseCatalog,
        cpa_sources: Option<Arc<dyn CpaRegisteredSourcePort + Send + Sync>>,
        cpa_runtime: Option<Arc<ManagedCpaRuntime>>,
        recover: bool,
    ) -> Result<Self, String> {
        Self::open_inner_with_cpa_and_overrides(
            storage_root,
            release_catalog,
            cpa_sources,
            cpa_runtime,
            recover,
            RuntimeOpenOverrides::default(),
        )
    }

    #[cfg(test)]
    fn prepare_for_role_all_with_scanner(
        storage_root: &Path,
        catalog: TrustedReleaseCatalog,
        cpa: Option<Arc<ManagedCpaRuntime>>,
        scanner: FilesystemAgentScannerV1,
    ) -> Result<Self, String> {
        let sources = cpa
            .as_ref()
            .map(|cpa| cpa.clone() as Arc<dyn CpaRegisteredSourcePort + Send + Sync>);
        Self::open_inner_with_cpa_and_overrides(
            storage_root,
            catalog,
            sources,
            cpa,
            false,
            RuntimeOpenOverrides {
                scanner: Some(scanner),
                ..RuntimeOpenOverrides::default()
            },
        )
    }

    fn open_inner_with_cpa_and_overrides(
        storage_root: &Path,
        release_catalog: TrustedReleaseCatalog,
        cpa_sources: Option<Arc<dyn CpaRegisteredSourcePort + Send + Sync>>,
        cpa_runtime: Option<Arc<ManagedCpaRuntime>>,
        recover: bool,
        overrides: RuntimeOpenOverrides,
    ) -> Result<Self, String> {
        let stores = LocalStorageSet::open_for_daemon_startup(storage_root)
            .map_err(|error| error.to_string())?;
        stores
            .control()
            .begin_plan_version_recovery()
            .map_err(|error| error.to_string())?;
        release_install::activate_release_catalog_revisions(&stores, &release_catalog)?;
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is unavailable for Agent discovery".to_owned())?;
        let project = std::env::current_dir().map_err(|error| error.to_string())?;
        let scanner = match overrides.scanner {
            Some(scanner) => scanner,
            None => {
                let selected_claude = stores
                    .control()
                    .worker_dependency_selection(
                        &WorkspaceId::default(),
                        hiroute_domain::delegation::WorkerHarnessV1::ClaudeCode,
                    )
                    .map_err(|error| error.to_string())?
                    .map(|(selection, _)| PathBuf::from(selection.cli_path));
                release_agent_scanner(
                    &home,
                    &project,
                    &release_catalog,
                    overrides.codex_desktop_engine,
                    selected_claude,
                )?
            }
        };
        let claude_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_claude_default",
            CLAUDE_PROFILE_ID_V1,
            CLAUDE_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|error| error.to_string())?;
        let claude_managed_target = AgentConnectionEffectRoleV1::ManagedConfiguration
            .target_for(&claude_subject)
            .map_err(|error| error.to_string())?;
        let codex_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_codex_default",
            CODEX_PROFILE_ID_V1,
            CODEX_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|error| error.to_string())?;
        let codex_managed_target = AgentConnectionEffectRoleV1::ManagedConfiguration
            .target_for(&codex_subject)
            .map_err(|error| error.to_string())?;
        let artifacts = stores
            .open_managed_artifacts_with_external_targets(
                storage_root.join("managed-artifacts"),
                storage_root.join("artifact-restores"),
                [
                    (claude_managed_target, scanner.claude_user_settings_target()),
                    (codex_managed_target, scanner.codex_user_config_target()),
                    (
                        AgentConnectionEffectRoleV1::RoutingSkill
                            .target_for(&codex_subject)
                            .map_err(|error| error.to_string())?,
                        home.join(".agents/skills/hiroute-collaboration/SKILL.md"),
                    ),
                    (
                        AgentConnectionEffectRoleV1::RoutingSkill
                            .target_for(&claude_subject)
                            .map_err(|error| error.to_string())?,
                        home.join(".claude/skills/hiroute-collaboration/SKILL.md"),
                    ),
                ],
            )
            .map_err(|error| error.to_string())?;
        let observation_workspace_key = observation_workspace_key(storage_root)?;
        let digest_authority = DigestAuthority::new(*observation_workspace_key);
        let observation = Arc::new(
            LocalObservationStore::open(storage_root.join("observation"), digest_authority.clone())
                .map_err(|error| error.to_string())?,
        );
        let delegation_epoch = delegation_epoch()?;
        let plan_admission =
            Arc::new(hiroute_application::publication::admission::SharedAdmissionGate::new());
        let delegation_safety = Arc::new(
            RunSafetyProjection::new(plan_admission.clone(), delegation_epoch.clone())
                .map_err(|error| error.to_string())?,
        );
        let delegation_finalization = Arc::new(DelegationFinalization::default());
        let adapter = Arc::new(LocalControlAdapter {
            publication_diagnostics: Mutex::new(Default::default()),
            stores: Mutex::new(stores),
            price_snapshot: Arc::new(hiroute_application::prices::PriceSnapshotSlot::default()),
            delegation_digest_authority: digest_authority,
            delegation_observation: observation.clone(),
            observation_workspace_key: Zeroizing::new(*observation_workspace_key),
            delegation_epoch,
            delegation_safety,
            delegation_run_authority: Arc::new(DelegationRunAuthority::default()),
            delegation_finalization: delegation_finalization.clone(),
            scanner,
            artifacts,
            release_catalog: Some(release_catalog),
            protected_inputs: Mutex::new(BTreeMap::new()),
            manual_protected_inputs: Mutex::new(BTreeMap::new()),
            agent_token_inputs: Mutex::new(BTreeMap::new()),
            model_connections: hiroute_integrations::NativeModelConnectionServiceV1::new(
                hiroute_application::compute_management::TrustedComputeCandidateRegistry::new(),
                overrides.model_transport.unwrap_or_else(|| {
                    Arc::new(hiroute_integrations::ReqwestModelDirectoryTransportV1)
                }),
            ),
            model_connection_cancellations: Mutex::new(BTreeMap::new()),
            prepared_discoveries: Mutex::new(BTreeMap::new()),
            permission_findings: Mutex::new(BTreeMap::new()),
            admission: hiroute_application::TransactionRuntime::default(),
            plan_admission,
            observation_activity_path: storage_root.join("observation/activity.db"),
            cpa_sources,
            cpa_runtime,
            subscription_sources: Mutex::new(BTreeMap::new()),
            subscription_targets: Mutex::new(BTreeMap::new()),
            subscription_maintenance: Mutex::new(subscriptions::SubscriptionMaintenance::new()?),
            managed_agent_runtime: Mutex::new(None),
            publication_target: Mutex::new(None),
            delegation_native_cleanup_cursor: Mutex::new(None),
            delegation_task_maintenance_cursor: Mutex::new(None),
        });
        let worker_platform = Arc::new(LocalWorkerPlatform::default());
        let delegation_executor = Arc::new(DelegationRunExecutor::new(
            adapter.clone(),
            adapter.clone(),
            adapter.plan_admission.clone(),
            adapter.delegation_safety.clone(),
            observation.clone(),
            adapter.delegation_run_authority.clone(),
            Arc::new(DelegationCancellationDispatcher::default()),
            delegation_finalization,
            worker_platform.clone(),
            Arc::new(UnavailableWorkerProfileSource),
        ));
        // Prime opaque scanner descriptors before journal recovery. A recovered Secret step can
        // resolve only the same exact revision; changed or missing sources fail closed.
        adapter.refresh_discovery()?;
        // Price availability alone does not reject an otherwise authorized model request.
        // Pending price Operations perform mandatory installation during recovery below.
        let _ = adapter.rebuild_price_snapshot();
        if recover {
            adapter.reconcile_startup_and_open()?;
            adapter.reconcile_cpa_runtime_from_management()?;
            adapter.reconcile_delegation_plan_versions()?;
            adapter.recover_delegation_authorizations()?;
            adapter.enable_subscription_maintenance();
        }
        Ok(Self {
            _observation_maintenance:
                hiroute_observation::maintenance::ObservationMaintenance::start_with_hook(
                    &observation,
                    Some(adapter.clone()),
                )
                .ok(),
            adapter,
            observation,
            observation_workspace_key,
            delegation_executor,
            delegation_storage_root: storage_root.to_path_buf(),
        })
    }

    /// Makes the role's diagnostic port visible to delegation work. The default no-op port
    /// records nothing and must never change delegation behavior.
    pub fn set_diagnostics(&self, diagnostics: DiagnosticsPort) {
        if let Ok(mut port) = self.adapter.publication_diagnostics.lock() {
            *port = diagnostics.clone();
        }
        if let Ok(stores) = self.adapter.stores.lock() {
            stores.control().set_diagnostics(diagnostics.clone());
        }
        self.delegation_executor
            .set_diagnostics(diagnostics.clone());
    }

    /// The sole control admission gate; grant/permit/run composition must reuse this instance.
    pub fn plan_admission_gate(
        &self,
    ) -> Arc<hiroute_application::publication::admission::SharedAdmissionGate> {
        self.adapter.plan_admission.clone()
    }

    pub fn plan_version_port(
        &self,
    ) -> Arc<dyn hiroute_application::publication::versions::ExactPlanVersionPort> {
        self.adapter.clone()
    }

    pub fn application_ports(&self) -> ApplicationPorts {
        ApplicationPorts::new(
            self.adapter.clone(),
            self.adapter.clone(),
            self.observation.clone(),
            self.adapter.clone(),
            self.adapter.clone(),
        )
        .with_work_plans(Arc::new(
            hiroute_application::delegation::work_plans::WorkPlanDirectory::new(
                self.adapter.clone(),
                self.adapter.clone(),
            ),
        ))
        .with_delegation_tasks(Arc::new(
            hiroute_application::delegation::tasks::DelegationTasks::new(Arc::new(
                delegation_tasks::ScheduledDelegationTaskPort::new(
                    self.adapter.clone(),
                    self.delegation_executor.clone(),
                ),
            )),
        ))
        .with_client_access(self.adapter.clone())
        .with_mutation(self.adapter.clone())
        .with_prices(self.adapter.clone())
        .with_model_catalog(self.adapter.clone())
        .with_compute_management(self.adapter.clone())
        .with_compute_routing(self.adapter.clone())
        .with_agent_connection(self.adapter.clone())
    }

    /// Registers only composition-owned runtime facts. The exact scanner, product publication,
    /// and grant stores remain the authorities used for every descriptor and raw resolution.
    pub fn configure_managed_agent_runtime(
        &self,
        gateway_base_url: String,
        trusted_hiroute_executable: String,
        publication_target: Option<Arc<dyn PublicationTargetPort + Send + Sync>>,
    ) -> Result<(), String> {
        self.configure_managed_agent_runtime_with_resident_service(
            gateway_base_url,
            trusted_hiroute_executable,
            publication_target,
            false,
        )
    }

    pub(crate) fn configure_managed_agent_runtime_with_resident_service(
        &self,
        gateway_base_url: String,
        trusted_hiroute_executable: String,
        publication_target: Option<Arc<dyn PublicationTargetPort + Send + Sync>>,
        resident_service_ready: bool,
    ) -> Result<(), String> {
        let worker_gateway = gateway_base_url
            .strip_prefix("http://")
            .and_then(|value| value.strip_suffix("/v1"))
            .ok_or_else(|| "managed Agent Gateway base must be an HTTP IPv4 /v1 URL".to_owned())?
            .parse()
            .map_err(|_| "managed Agent Gateway base has an invalid listener address".to_owned())?;
        self.delegation_executor
            .set_gateway(worker_gateway)
            .map_err(|error| error.to_string())?;
        let selection_source: Arc<dyn WorkerInstallationSelectionSource> = self.adapter.clone();
        let managed = managed_profile_source(
            &self.delegation_storage_root,
            selection_source,
            self.adapter.clone(),
        );
        let source = managed.source;
        self.delegation_executor
            .set_profile_source(source)
            .map_err(|error| error.to_string())?;
        let claude_gateway_origin = gateway_base_url
            .strip_suffix("/v1")
            .ok_or_else(|| "managed Agent Gateway base must end in /v1".to_owned())?;
        hiroute_domain::ManagedClaudeLaunchDescriptorV2::trusted(
            "agent-connection/validation",
            "claude-messages-v1",
            "/validation/claude",
            CanonicalDigest::of_bytes(b"validation-snapshot"),
            1,
            CanonicalDigest::of_bytes(b"validation-publication"),
            claude_gateway_origin,
            hiroute_domain::AgentClaudePresetValuesV2 {
                opus: Some("validation-opus".to_owned()),
                sonnet: None,
                haiku: None,
            },
            trusted_hiroute_executable.clone(),
        )
        .map_err(|error| error.to_string())?;
        *self
            .adapter
            .managed_agent_runtime
            .lock()
            .map_err(|_| "managed Agent runtime is unavailable".to_owned())? =
            Some(ManagedAgentRuntimeV1 {
                gateway_base_url,
                trusted_hiroute_executable,
                worker_executor_availability: managed.availability,
                resident_service_ready,
            });
        *self
            .adapter
            .publication_target
            .lock()
            .map_err(|_| "publication target is unavailable".to_owned())? = publication_target;
        self.adapter.reconcile_startup_and_open()?;
        self.adapter.reconcile_cpa_runtime_from_management()?;
        self.adapter.reconcile_delegation_plan_versions()?;
        self.adapter.recover_delegation_authorizations()?;
        self.adapter.enable_subscription_maintenance();
        self.adapter
            .reconcile_active_publication()
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub(crate) fn observation_store(&self) -> Arc<LocalObservationStore> {
        Arc::clone(&self.observation)
    }

    pub(crate) fn observation_workspace_key(&self) -> [u8; 32] {
        *self.observation_workspace_key
    }

    pub(crate) fn native_credential_authority(&self) -> ControlNativeCredentialAuthority {
        ControlNativeCredentialAuthority {
            adapter: Arc::clone(&self.adapter),
        }
    }

    pub(crate) fn compute_runtime_state_store(&self) -> ControlRuntimeStateStore {
        ControlRuntimeStateStore {
            adapter: Arc::clone(&self.adapter),
        }
    }

    /// The Gateway receives only this narrow in-memory authority.  It does not receive Control
    /// storage or a mutable publication lookup, and unknown run tokens therefore cannot route to
    /// ordinary aggregate authorization.
    pub(crate) fn delegation_run_authority(&self) -> Arc<DelegationRunAuthority> {
        Arc::clone(&self.adapter.delegation_run_authority)
    }

    #[cfg(unix)]
    pub(crate) fn agent_grant_resolver(&self) -> Arc<dyn super::AgentGrantResolverPort> {
        self.adapter.clone()
    }

    /// Persists one already launcher-validated, exact grant. Neither Local Control nor
    /// Application receives this registrar or can mint a capability.
    pub(crate) fn register_apply_capability(
        &self,
        registration: ApplyCapabilityRegistrationV1,
    ) -> Result<(), String> {
        self.adapter
            .stores
            .lock()
            .map_err(|_| "capability registrar is unavailable".to_owned())?
            .apply_capability_registrar()
            .register(registration)
            .map_err(|error| error.to_string())
    }
}

fn delegation_epoch() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| "delegation epoch entropy is unavailable".to_owned())?;
    let mut epoch = String::from("delegation-");
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut epoch, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(epoch)
}

struct LocalControlAdapter {
    publication_diagnostics: Mutex<DiagnosticsPort>,
    price_snapshot: Arc<hiroute_application::prices::PriceSnapshotSlot>,
    stores: Mutex<LocalStorageSet>,
    delegation_digest_authority: DigestAuthority,
    delegation_observation: Arc<LocalObservationStore>,
    observation_workspace_key: Zeroizing<[u8; 32]>,
    delegation_epoch: String,
    delegation_safety: Arc<RunSafetyProjection>,
    delegation_run_authority: Arc<DelegationRunAuthority>,
    delegation_finalization: Arc<DelegationFinalization>,
    scanner: FilesystemAgentScannerV1,
    artifacts: ManagedArtifactStore,
    release_catalog: Option<TrustedReleaseCatalog>,
    protected_inputs: Mutex<BTreeMap<String, DiscoveredCredentialRefV1>>,
    manual_protected_inputs: Mutex<BTreeMap<String, ProtectedSecret>>,
    agent_token_inputs: Mutex<BTreeMap<String, ProtectedSecret>>,
    model_connections: hiroute_integrations::NativeModelConnectionServiceV1<
        hiroute_application::compute_management::TrustedComputeCandidateRegistry,
        Arc<dyn hiroute_integrations::ModelDirectoryTransportV1>,
    >,
    model_connection_cancellations:
        Mutex<BTreeMap<String, hiroute_integrations::ModelConnectionProbeCancellationV1>>,
    prepared_discoveries: Mutex<BTreeMap<String, PrepareDiscoveredModelConnectionRequestV1>>,
    permission_findings: Mutex<BTreeMap<String, PermissionHardeningRequiredV1>>,
    admission: hiroute_application::TransactionRuntime,
    plan_admission: Arc<hiroute_application::publication::admission::SharedAdmissionGate>,
    observation_activity_path: PathBuf,
    cpa_sources: Option<Arc<dyn CpaRegisteredSourcePort + Send + Sync>>,
    cpa_runtime: Option<Arc<ManagedCpaRuntime>>,
    subscription_sources: Mutex<BTreeMap<String, CodexSubscriptionContextV1>>,
    subscription_targets: Mutex<BTreeMap<String, CodexSubscriptionContextV1>>,
    subscription_maintenance: Mutex<subscriptions::SubscriptionMaintenance>,
    managed_agent_runtime: Mutex<Option<ManagedAgentRuntimeV1>>,
    publication_target: Mutex<Option<Arc<dyn PublicationTargetPort + Send + Sync>>>,
    delegation_native_cleanup_cursor:
        Mutex<Option<hiroute_observation::managed_text::ManagedTextNativeCleanupCursor>>,
    delegation_task_maintenance_cursor:
        Mutex<Option<hiroute_domain::delegation::DelegationTaskMaintenanceCursorV1>>,
}

impl LocalControlAdapter {
    /// Restore all persisted authorization denials before making this daemon epoch eligible for
    /// admission.  The projection intentionally remains fail-closed if recovery cannot finish.
    fn recover_delegation_authorizations(&self) -> Result<(), String> {
        let stores = self.stores_lock().map_err(|error| error.to_string())?;
        crate::delegation::authorization_recovery::recover_authorizations(
            &stores,
            &WorkspaceId::default(),
            self.plan_admission.clone(),
            self.delegation_safety.as_ref(),
        )
        .map_err(|error| error.to_string())?;
        drop(stores);
        self.delegation_safety.finish_startup_recovery();
        Ok(())
    }
}

/// Narrow, shareable view of the coordinated Secret authority used by the Gateway. It preserves
/// one storage owner in `role=all`; opening a second Secret store would split generations and
/// violate key rotation semantics.
pub(crate) struct ControlNativeCredentialAuthority {
    adapter: Arc<LocalControlAdapter>,
}

impl NativeCredentialAuthorityV1 for ControlNativeCredentialAuthority {
    fn lease_native_credential(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        self.adapter
            .stores
            .lock()
            .map_err(|_| {
                PortError::new(
                    PortErrorCode::Unavailable,
                    "control.runtime.native_credential_lock",
                )
            })?
            .secrets()
            .lease_native_credential_exact(request)
    }

    fn lease_header_secret(
        &self,
        request: &HeaderSecretLeaseRequestV1,
    ) -> PortResult<Option<NativeCredentialLeaseV1>> {
        self.adapter
            .stores
            .lock()
            .map_err(|_| {
                PortError::new(
                    PortErrorCode::Unavailable,
                    "control.runtime.classifier_credential_lock",
                )
            })?
            .secrets()
            .lease_header_secret_exact(request)
    }
}

/// Narrow, shareable view of the coordinated compute-runtime authority. Cooldown, key rotation,
/// and half-open probe state therefore have the same durable truth for Control and Gateway.
pub(crate) struct ControlRuntimeStateStore {
    adapter: Arc<LocalControlAdapter>,
}

impl ComputeRuntimeStateStoreV1 for ControlRuntimeStateStore {
    fn runtime_state(
        &self,
        identity: &RuntimeStateIdentityV1,
    ) -> PortResult<Option<RuntimeStateV1>> {
        self.adapter
            .stores
            .lock()
            .map_err(runtime_store_lock)?
            .runtime()
            .runtime_state(identity)
    }

    fn compare_and_set_runtime_state(
        &self,
        expected_generation: u64,
        state: &RuntimeStateV1,
    ) -> PortResult<()> {
        self.adapter
            .stores
            .lock()
            .map_err(runtime_store_lock)?
            .runtime()
            .compare_and_set_runtime_state(expected_generation, state)
    }

    fn acquire_runtime_probe(
        &self,
        identity: &RuntimeStateIdentityV1,
        expected_generation: u64,
        request: &RuntimeProbeLeaseRequestV1,
    ) -> PortResult<RuntimeProbeAcquireOutcomeV1> {
        self.adapter
            .stores
            .lock()
            .map_err(runtime_store_lock)?
            .runtime()
            .acquire_runtime_probe(identity, expected_generation, request)
    }

    fn complete_runtime_probe(
        &self,
        lease: &RuntimeProbeLeaseV1,
        state: &RuntimeStateV1,
    ) -> PortResult<()> {
        self.adapter
            .stores
            .lock()
            .map_err(runtime_store_lock)?
            .runtime()
            .complete_runtime_probe(lease, state)
    }
}

fn runtime_store_lock<T>(_: std::sync::PoisonError<T>) -> PortError {
    PortError::new(
        PortErrorCode::Unavailable,
        "control.runtime.compute_state_lock",
    )
}

#[derive(Clone)]
struct ManagedAgentRuntimeV1 {
    gateway_base_url: String,
    trusted_hiroute_executable: String,
    worker_executor_availability: Arc<WorkerExecutorAvailabilityRegistry>,
    /// Standalone is already the installed resident user service. Desktop compositions leave
    /// this false and retain their native login-item confirmation contract.
    resident_service_ready: bool,
}

impl WorkerInstallationSelectionSource for LocalControlAdapter {
    fn selection(
        &self,
        harness: hiroute_domain::delegation::WorkerHarnessV1,
    ) -> Result<Option<WorkerInstallationSelection>, hiroute_domain::delegation::DelegationErrorV1>
    {
        let selected = self
            .stores_lock()
            .map_err(|_| hiroute_domain::delegation::DelegationErrorV1::StorageUnavailable)?
            .control()
            .worker_dependency_selection(&WorkspaceId::default(), harness)
            .map_err(|_| hiroute_domain::delegation::DelegationErrorV1::StorageUnavailable)?;
        Ok(
            selected.map(|(selection, revision)| WorkerInstallationSelection {
                config: WorkerInstallationConfig {
                    harness: selection.harness,
                    adapter: PathBuf::from(selection.adapter_path),
                    harness_binary: PathBuf::from(selection.cli_path),
                    node_binary: selection.node_path.map(PathBuf::from),
                },
                revision,
            }),
        )
    }
}

impl ControlStatePort for LocalControlAdapter {
    fn snapshot(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<ControlStateSnapshotV1, ControlReadError> {
        let stores = self
            .stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        let control = stores.control();
        Ok(ControlStateSnapshotV1 {
            revisions: control.current_revisions(workspace_id).map_err(map_port)?,
            desired_state: control.desired_state(workspace_id).map_err(map_port)?,
            recoverable_operations: control.recoverable_operations().map_err(map_port)?,
        })
    }

    fn operation(
        &self,
        operation_id: &OperationId,
    ) -> Result<Option<OperationV1>, ControlReadError> {
        self.stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .control()
            .load_operation(operation_id)
            .map_err(map_port)
    }

    fn operation_for_idempotency(
        &self,
        workspace_id: &WorkspaceId,
        principal: PrincipalKind,
        operation_kind: &str,
        idempotency_key: &str,
    ) -> Result<Option<OperationV1>, ControlReadError> {
        let principal = match principal {
            PrincipalKind::InteractiveUser => "interactive-user",
            PrincipalKind::Desktop => "desktop",
            PrincipalKind::Skill | PrincipalKind::SealedCollaboration => {
                return Err(ControlReadError::Denied);
            }
        };
        let scope =
            hiroute_domain::IdempotencyScopeV1::new(principal, operation_kind, idempotency_key)
                .map_err(|_| ControlReadError::Corrupt)?;
        self.stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .control()
            .operation_for_idempotency(workspace_id, &scope)
            .map_err(map_port)
    }

    fn validate_protected_capability(
        &self,
        raw_capability: &str,
        workspace_id: &WorkspaceId,
        principal: PrincipalKind,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
    ) -> Result<(), ControlReadError> {
        let capability = ProtectedApplyCapability::new(raw_capability.to_owned())
            .map_err(|_| ControlReadError::Denied)?;
        let principal = match principal {
            PrincipalKind::InteractiveUser => "interactive-user",
            PrincipalKind::Desktop => "desktop",
            PrincipalKind::Skill | PrincipalKind::SealedCollaboration => {
                return Err(ControlReadError::Denied);
            }
        };
        self.stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .control()
            .verify_apply_authorization(
                &capability,
                workspace_id,
                principal,
                operation_kind,
                accepted_digest,
                expected_revisions,
            )
            .map(|_| ())
            .map_err(map_port)
    }

    fn consume_agent_live_check_capability(
        &self,
        raw_capability: &str,
        workspace_id: &WorkspaceId,
        principal: PrincipalKind,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
    ) -> Result<(), ControlReadError> {
        let capability = ProtectedApplyCapability::new(raw_capability.to_owned())
            .map_err(|_| ControlReadError::Denied)?;
        let principal = match principal {
            PrincipalKind::InteractiveUser => "interactive-user",
            PrincipalKind::Desktop => "desktop",
            PrincipalKind::Skill | PrincipalKind::SealedCollaboration => {
                return Err(ControlReadError::Denied);
            }
        };
        self.stores_lock()
            .map_err(map_port)?
            .control()
            .consume_agent_live_check_capability(
                &capability,
                workspace_id,
                principal,
                accepted_digest,
                expected_revisions,
            )
            .map_err(map_port)
    }
}

impl AgentDiscoveryPort for LocalControlAdapter {
    fn discover(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        let fixed_candidates =
            hiroute_application::control::RoutingFactsPort::routing_compilation_snapshot(
                self,
                &WorkspaceId::default(),
            )
            .map(|snapshot| snapshot.facts.candidates)
            .unwrap_or_default();
        let codex_catalog = self.scanner.codex_catalog_summary().ok().and_then(|catalog| {
            let metadata_source = match catalog.metadata_source {
                hiroute_integrations::CodexCatalogMetadataSourceV1::UserConfigured => {
                    AgentModelCatalogMetadataSourceV1::UserConfigured
                }
                hiroute_integrations::CodexCatalogMetadataSourceV1::TargetCache => {
                    AgentModelCatalogMetadataSourceV1::TargetCache
                }
                hiroute_integrations::CodexCatalogMetadataSourceV1::TargetBundled => {
                    AgentModelCatalogMetadataSourceV1::TargetBundled
                }
                // This is an output artifact, never a native metadata source.
                hiroute_integrations::CodexCatalogMetadataSourceV1::HirouteGenerated => {
                    return None;
                }
            };
            let models = catalog
                .models
                .into_iter()
                .map(|model| {
                    let mut source_options = fixed_candidates
                        .iter()
                        .filter(|candidate| {
                            candidate.native_transport_model == model.client_model_id
                                || candidate.binding.upstream_model_id == model.client_model_id
                        })
                        .map(|candidate| {
                            let state = if candidate.is_routable() {
                                AgentModelSourceCoverageStateV1::Ready
                            } else if !candidate.inventory_model_matched
                                && candidate.authority
                                    != hiroute_application::compiler::CandidateFactAuthorityV1::SourceLocalUser
                            {
                                AgentModelSourceCoverageStateV1::ModelUnconfirmed
                            } else {
                                match candidate.source_state {
                                    MaterializationState::NeedsCredential => {
                                        AgentModelSourceCoverageStateV1::CredentialRequired
                                    }
                                    MaterializationState::NeedsAuthorization => {
                                        AgentModelSourceCoverageStateV1::AuthorizationRequired
                                    }
                                    MaterializationState::Disabled => {
                                        AgentModelSourceCoverageStateV1::Disabled
                                    }
                                    MaterializationState::Ready => {
                                        AgentModelSourceCoverageStateV1::ModelUnconfirmed
                                    }
                                }
                            };
                            DiscoveredAgentModelSourceV1 {
                                binding_id: candidate.binding.binding_id.clone(),
                                source_label: candidate.connection_option_id.clone(),
                                account_scope_ref: candidate.binding.source_id.clone(),
                                account_scope_digest: candidate
                                    .binding
                                    .source_identity_digest
                                    .clone(),
                                state,
                                reasoning: candidate.reasoning.clone(),
                            }
                        })
                        .collect::<Vec<_>>();
                    source_options.sort_by(|left, right| {
                        (&left.source_label, &left.account_scope_ref, &left.binding_id).cmp(&(
                            &right.source_label,
                            &right.account_scope_ref,
                            &right.binding_id,
                        ))
                    });
                    DiscoveredAgentModelV1 {
                        client_model_id: model.client_model_id,
                        display_name: model.display_name,
                        source_options,
                    }
                })
                .collect();
            Some(DiscoveredAgentModelCatalogV1 {
                metadata_source,
                native_default_model: catalog.native_default_model,
                models,
            })
        });
        self.refresh_discovery()
            .map_err(|_| ControlReadError::Unavailable)?
            .into_iter()
            .map(|discovery| {
                let mut agent = discovered_agent(discovery)?;
                agent.context_id = self.settings_context_for_agent(&agent.agent_id);
                agent.available_surfaces = self.scanner.available_model_surfaces(&agent.agent_id);
                if agent.agent_id == "agent_codex_default" {
                    agent.native_model_catalog = codex_catalog.clone();
                }
                Ok(agent)
            })
            .collect()
    }
}

impl ApplicationClockPort for LocalControlAdapter {
    fn local_day_start_ms(&self, at_ms: i64) -> Result<i64, ControlReadError> {
        if at_ms < 0 {
            return Err(ControlReadError::Corrupt);
        }
        // SQLite delegates local calendar conversion to the host timezone. No
        // observation rows or configurable Plan timezone affect "Today".
        let clock =
            rusqlite::Connection::open_in_memory().map_err(|_| ControlReadError::Unavailable)?;
        clock.query_row("SELECT CAST(strftime('%s',?1,'unixepoch','localtime','start of day','utc') AS INTEGER)*1000",[at_ms/1000],|r|r.get(0)).map_err(|_|ControlReadError::Unavailable)
    }

    fn now_ms(&self) -> Result<i64, ControlReadError> {
        let milliseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ControlReadError::Corrupt)?
            .as_millis();
        i64::try_from(milliseconds).map_err(|_| ControlReadError::Corrupt)
    }
}

impl ValueScopePort for LocalControlAdapter {
    fn value_plan_ids(
        &self,
        workspace_id: &WorkspaceId,
        scope: &ValuePlanScopeV1,
    ) -> Result<Vec<AgentPlanId>, ControlReadError> {
        use rusqlite::{Connection, OpenFlags, params};

        let connection = Connection::open_with_flags(
            &self.observation_activity_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| ControlReadError::Unavailable)?;
        let mut raw = Vec::new();
        if let Some(session_id) = scope.session_id.as_ref() {
            let mut statement = connection
                .prepare(
                    "SELECT DISTINCT agent_plan_id FROM value_ledger_entries
                     WHERE workspace_id=?1 AND currency=?2 AND frozen_at_ms>=?3
                       AND frozen_at_ms<?4 AND session_id=?5 ORDER BY agent_plan_id LIMIT 201",
                )
                .map_err(|_| ControlReadError::Unavailable)?;
            let rows = statement
                .query_map(
                    params![
                        workspace_id.as_str(),
                        scope.currency,
                        scope.from_ms,
                        scope.to_ms,
                        session_id.as_str(),
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|_| ControlReadError::Unavailable)?;
            for row in rows {
                raw.push(row.map_err(|_| ControlReadError::Corrupt)?);
            }
        } else {
            let first_day = scope.from_ms.div_euclid(86_400_000);
            let last_day = (scope.to_ms - 1).div_euclid(86_400_000);
            let mut statement = connection
                .prepare(
                    "SELECT agent_plan_id FROM value_ledger_entries
                     WHERE workspace_id=?1 AND currency=?2 AND frozen_at_ms>=?3
                       AND frozen_at_ms<?4
                     UNION
                     SELECT agent_plan_id FROM daily_value_rollups
                     WHERE workspace_id=?1 AND currency=?2 AND day_number>=?5
                       AND day_number<=?6
                     ORDER BY agent_plan_id LIMIT 201",
                )
                .map_err(|_| ControlReadError::Unavailable)?;
            let rows = statement
                .query_map(
                    params![
                        workspace_id.as_str(),
                        scope.currency,
                        scope.from_ms,
                        scope.to_ms,
                        first_day,
                        last_day,
                    ],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|_| ControlReadError::Unavailable)?;
            for row in rows {
                raw.push(row.map_err(|_| ControlReadError::Corrupt)?);
            }
        }
        if raw.len() > 200 {
            return Err(ControlReadError::Unavailable);
        }
        raw.into_iter()
            .map(|value| AgentPlanId::parse(value).map_err(|_| ControlReadError::Corrupt))
            .collect()
    }
}

fn release_agent_scanner(
    home: &Path,
    project: &Path,
    catalog: &TrustedReleaseCatalog,
    codex_desktop_engine: Option<PathBuf>,
    selected_claude: Option<PathBuf>,
) -> Result<FilesystemAgentScannerV1, String> {
    // ReleaseFacts verification already validated the registered connector/model metadata.
    // Agent registration consumes that payload after the current rating snapshot and all
    // cross-references have passed fail-closed validation; it is not a Codex target catalog.
    let index = ClaudeRegistrationIndexV1::from_verified_model_data(
        catalog.registry(),
        catalog.model_data(),
    )
    .map_err(|_| "verified Agent discovery facts are inconsistent".to_owned())?;
    let mut layout = AgentFilesystemLayoutV1::from_process(home, project);
    layout.codex_desktop_executable = codex_desktop_engine;
    if let Some(executable) = selected_claude {
        layout.claude_executable = executable;
    }
    Ok(FilesystemAgentScannerV1::new(layout, index))
}

fn discovered_agent(
    discovery: FilesystemAgentDiscoveryV1,
) -> Result<DiscoveredAgentV1, ControlReadError> {
    let (agent_id, profile_id, version, supported, configuration_state) = match &discovery.outcome {
        AgentDiscoveryOutcomeV1::Supported { installation } => (
            installation.agent_id.clone(),
            installation.profile.profile_id.clone(),
            installation.version.clone(),
            true,
            if discovery.discovered_credential.is_some() {
                "registered_with_protected_input".to_owned()
            } else {
                "registered".to_owned()
            },
        ),
        AgentDiscoveryOutcomeV1::ReportOnly {
            agent_id,
            kind,
            version,
            reason,
            ..
        } => (
            agent_id.clone(),
            match kind {
                AgentKindV1::Codex => "codex-responses-v1",
                AgentKindV1::ClaudeCode => "claude-messages-v1",
            }
            .to_owned(),
            version.clone(),
            false,
            serde_json::to_value(reason)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or(ControlReadError::Corrupt)?,
        ),
    };
    let discovered_credential = discovery
        .discovered_credential
        .map(|credential| {
            let protected_input_slot = mutation::protected_input_slot(&credential)
                .map_err(|_| ControlReadError::Corrupt)?;
            Ok(DiscoveredCredentialInputV1 {
                source: credential.source,
                scanner_id: credential.scanner_id,
                scanner_version: credential.scanner_version,
                discovered_source_ref: credential.discovered_source_ref,
                field_selector: credential.field_selector,
                observed_revision: credential.observed_revision,
                protected_input_slot,
            })
        })
        .transpose()?;
    Ok(DiscoveredAgentV1 {
        context_id: None,
        agent_id,
        profile_id,
        version,
        supported,
        configuration_state,
        available_surfaces: Default::default(),
        native_model_catalog: None,
        registered_configuration: discovery.claude_configuration.map(|configuration| {
            DiscoveredAgentConfigurationV1 {
                connection_option_id: configuration.connection_option_id,
                endpoint_profile_id: configuration.endpoint_profile_id,
                endpoint_profile_revision: configuration.endpoint_profile_revision,
                model_configuration_id: configuration.model_configuration_id,
                base_url: configuration.base_url,
                observed_model_id: configuration.observed_model_id,
                provider_model_alias_hints: configuration.provider_model_alias_hints,
                configuration_revision: configuration.configuration_revision,
            }
        }),
        discovered_credential,
        permission_hardening: discovery.permission_hardening.map(|finding| {
            AgentConfigPermissionFindingV1 {
                scanner_id: finding.scanner_id,
                scanner_version: finding.scanner_version,
                discovered_source_ref: finding.discovered_source_ref,
                observed_identity: finding.observed_identity,
                observed_revision: finding.observed_revision,
                display_path: finding.display_path,
                required_mode: finding.required_mode,
            }
        }),
    })
}

fn map_port(error: PortError) -> ControlReadError {
    match error.code {
        PortErrorCode::Conflict => ControlReadError::SnapshotChanged,
        PortErrorCode::NotFound => ControlReadError::NotFound,
        PortErrorCode::PermissionDenied => ControlReadError::Denied,
        PortErrorCode::Corrupt | PortErrorCode::InvalidData => ControlReadError::Corrupt,
        _ => ControlReadError::Unavailable,
    }
}

fn observation_workspace_key(storage_root: &Path) -> Result<Zeroizing<[u8; 32]>, String> {
    let key_path = storage_root.join("observation-hmac-key");
    let mut key = [0_u8; 32];
    match OpenOptions::new().read(true).open(&key_path) {
        Ok(mut file) => {
            file.read_exact(&mut key)
                .map_err(|error| error.to_string())?;
            let mut trailing = [0_u8; 1];
            if file
                .read(&mut trailing)
                .map_err(|error| error.to_string())?
                != 0
            {
                return Err("observation authority key has an invalid length".to_owned());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(storage_root).map_err(|error| error.to_string())?;
            getrandom::fill(&mut key).map_err(|error| error.to_string())?;
            create_owner_key(&key_path, &key)?;
        }
        Err(error) => return Err(error.to_string()),
    }
    validate_owner_key(&key_path)?;
    Ok(Zeroizing::new(key))
}

#[cfg(unix)]
fn create_owner_key(path: &Path, key: &[u8; 32]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(key).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn create_owner_key(path: &Path, key: &[u8; 32]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(key).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}

#[cfg(unix)]
fn validate_owner_key(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != nix::unistd::geteuid().as_raw()
    {
        return Err("observation authority key is not owner-only".to_owned());
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_owner_key(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
#[path = "runtime_observation_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_worker_dependencies_tests.rs"]
mod worker_dependencies_tests;
