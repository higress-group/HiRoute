use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

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
use hiroute_cpa_bridge::{
    BorrowedSubscriptionEvidence, CpaRegisteredSourcePort, ManagedCpaRuntime, ManagedCpaRuntimeSet,
};
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
    FilesystemAgentScannerV1, PermissionHardeningRequiredV1, QODER_INTEGRATION_PROFILE_REF_V1,
    QODER_PROFILE_ID_V1, TrustedReleaseCatalog,
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

mod additional_model_budget;
mod agent_connection;
mod agent_effective_state;
mod agent_live_check;
mod candidate_execution;
mod client_access;
mod clock;
mod collaboration_installation;
mod collaboration_status;
mod compute_routing;
mod delegation_continue;
mod delegation_cursors;
mod delegation_maintenance;
mod delegation_reads;
mod delegation_residual;
mod delegation_results;
mod delegation_runtime_port;
mod delegation_task_queries;
mod delegation_tasks;
mod delegation_worker;
mod effects;
mod grant_attribution;
mod hermes_management_status;
mod model_catalog;
mod model_connections;
mod model_feed;
mod mutation;
mod native_additional_model;
mod native_claude_model;
mod native_hermes;
mod native_model;
mod observation_key;
mod trust;
use observation_key::observation_workspace_key;
mod plan_content;
mod plan_content_snapshot;
mod plan_versions;
mod prices;
mod protected_inputs;
mod publication;
mod release_install;
mod routing;
mod settings_codex_profile;
mod settings_facts;
mod settings_retry;
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
struct SubscriptionContextV1 {
    source: subscriptions::source::SubscriptionSource,
    evidence: Option<BorrowedSubscriptionEvidence>,
    candidate_revision: u64,
}

#[derive(Default)]
struct RuntimeOpenOverrides {
    scanner: Option<FilesystemAgentScannerV1>,
    model_transport: Option<Arc<dyn hiroute_integrations::ModelDirectoryTransportV1>>,
    codex_desktop_engine: Option<PathBuf>,
    storage_startup: hiroute_local_storage::StorageStartupOptions,
}

pub(crate) fn native_agent_home() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        // This selector exists only in the libtest child launched by test_support.
        // Production always discovers the actual account profile through Windows.
        #[cfg(test)]
        if std::env::var_os("HIROUTE_ISOLATED_DAEMON_TEST")
            .is_some_and(|selector| !selector.is_empty())
        {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| {
                    "isolated Agent discovery requires an absolute test HOME".to_owned()
                })?;
            hiroute_platform::windows::security::verify_owner_only(&home, true).map_err(
                |error| format!("isolated Agent discovery HOME is not private: {error}"),
            )?;
            return Ok(home);
        }
        hiroute_platform::windows::background::profile().map_err(|error| {
            format!("Windows account profile is unavailable for Agent discovery: {error}")
        })
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| "HOME is unavailable for Agent discovery".to_owned())
    }
}

#[cfg(all(test, windows))]
#[test]
fn windows_agent_discovery_uses_the_private_isolated_test_home() {
    if crate::test_support::isolated_agent_home(
        "control::runtime::windows_agent_discovery_uses_the_private_isolated_test_home",
    ) {
        return;
    }
    let home = native_agent_home().unwrap();
    assert_eq!(home, PathBuf::from(std::env::var_os("HOME").unwrap()));
    assert_ne!(
        home,
        hiroute_platform::windows::background::profile().unwrap()
    );
    hiroute_platform::windows::security::verify_owner_only(&home, true).unwrap();
}

impl ProductionControlRuntime {
    pub(crate) fn upgrade_active_tasks(&self) -> Result<u64, String> {
        let occupied = self
            .adapter
            .stores
            .lock()
            .map_err(|_| "upgrade task storage unavailable".to_owned())?
            .runtime()
            .upgrade_occupied_tasks()
            .map_err(|_| "upgrade task storage unavailable".to_owned())?;
        let executing = self
            .delegation_executor
            .upgrade_active_count()
            .map_err(|_| "upgrade task executor unavailable".to_owned())?;
        Ok(occupied.max(executing))
    }

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
        let cpa = Arc::new(ManagedCpaRuntimeSet::from(cpa));
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
        Self::prepare_for_role_all_with_codex_desktop_engine(
            storage_root,
            catalog,
            cpa.map(|runtime| Arc::new(ManagedCpaRuntimeSet::from(runtime))),
            None,
            Default::default(),
        )
    }

    pub(crate) fn prepare_for_role_all_with_codex_desktop_engine(
        storage_root: &Path,
        catalog: TrustedReleaseCatalog,
        cpa: Option<Arc<ManagedCpaRuntimeSet>>,
        codex_desktop_engine: Option<PathBuf>,
        storage_startup: hiroute_local_storage::StorageStartupOptions,
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
                storage_startup,
                ..RuntimeOpenOverrides::default()
            },
        )
    }

    fn open_inner_with_cpa(
        storage_root: &Path,
        release_catalog: TrustedReleaseCatalog,
        cpa_sources: Option<Arc<dyn CpaRegisteredSourcePort + Send + Sync>>,
        cpa_runtime: Option<Arc<ManagedCpaRuntimeSet>>,
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
        let cpa = cpa.map(|runtime| Arc::new(ManagedCpaRuntimeSet::from(runtime)));
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
        cpa_runtime: Option<Arc<ManagedCpaRuntimeSet>>,
        recover: bool,
        overrides: RuntimeOpenOverrides,
    ) -> Result<Self, String> {
        observation_key::validate_existing_authority(storage_root).map_err(|error| {
            overrides.storage_startup.report_failure(error.recovery());
            error.to_string()
        })?;
        let mut startup = overrides.storage_startup.clone();
        startup.preserve_observation_source =
            LocalObservationStore::legacy_history_requires_seal(storage_root.join("observation"))
                .map_err(|error| {
                    startup.report_failure(hiroute_host_runtime::StorageRecovery::new(
                        hiroute_host_runtime::StorageRecoveryCause::UnsupportedFormat,
                        hiroute_host_runtime::StorageRecoveryAction::PreserveAndDiagnose,
                    ));
                    format!("{error}; preserve all storage generations and use the original whole-group recovery guide")
                })?
                || LocalStorageSet::observation_cutover_pending(storage_root);
        if startup.preserve_observation_source
            && let Some(progress) = &startup.upgrade_progress
        {
            progress.report(hiroute_host_runtime::StorageUpgradePhase::SourceCheck);
        }
        let stores = LocalStorageSet::open_for_daemon_startup_with_options(storage_root, &startup)
            .map_err(|error| error.to_string())?;
        stores
            .control()
            .begin_plan_version_recovery()
            .map_err(|error| error.to_string())?;
        release_install::activate_release_catalog_revisions(&stores, &release_catalog)?;
        let feed_revisions = stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .map_err(|error| error.to_string())?
            .dependencies;
        let feed_generation = feed_revisions.get("feed.model_data").copied().unwrap_or(0);
        let feed_sequence = feed_revisions.get("feed.v2_sequence").copied().unwrap_or(0);
        let model_feed = Arc::new(crate::model_feed::ModelFeedService::open(
            storage_root,
            release_catalog.clone(),
            feed_generation,
            feed_sequence,
        )?);
        stores
            .control()
            .activate_model_feed_generation(
                &WorkspaceId::default(),
                model_feed.current().generation,
                model_feed.current().accepted_sequence,
            )
            .map_err(|error| error.to_string())?;
        let home = native_agent_home()?;
        let project = std::env::current_dir().map_err(|error| error.to_string())?;
        let mut scanner = match overrides.scanner {
            Some(scanner) => scanner,
            None => {
                use hiroute_domain::delegation::WorkerHarnessV1;
                let mut selected = Vec::new();
                for (kind, harness) in [
                    (AgentKindV1::ClaudeCode, WorkerHarnessV1::ClaudeCode),
                    (AgentKindV1::Qoder, WorkerHarnessV1::QoderCli),
                    (AgentKindV1::Pi, WorkerHarnessV1::Pi),
                    (AgentKindV1::OpenCode, WorkerHarnessV1::OpenCode),
                    (
                        AgentKindV1::DeepseekHarness,
                        WorkerHarnessV1::DeepseekHarness,
                    ),
                ] {
                    if let Some((selection, _)) = stores
                        .control()
                        .worker_dependency_selection(&WorkspaceId::default(), harness)
                        .map_err(|error| error.to_string())?
                    {
                        selected.push((kind, PathBuf::from(selection.cli_path)));
                    }
                }
                release_agent_scanner(
                    &home,
                    &project,
                    &release_catalog,
                    overrides.codex_desktop_engine,
                    &selected,
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
        let profile_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_codex_default",
            hiroute_integrations::CODEX_STANDALONE_PROFILE_ID,
            CODEX_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|error| error.to_string())?;
        let profile_target = AgentConnectionEffectRoleV1::ManagedConfiguration
            .target_for(&profile_subject)
            .map_err(|error| error.to_string())?;
        let qoder_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_qoder_default",
            QODER_PROFILE_ID_V1,
            QODER_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|error| error.to_string())?;
        let pi_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_pi_default",
            hiroute_integrations::PI_PROFILE_ID_V1,
            hiroute_integrations::PI_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|error| error.to_string())?;
        let dsh_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_dsh_default",
            hiroute_integrations::DSH_PROFILE_ID_V1,
            hiroute_integrations::DSH_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|e| e.to_string())?;
        let dsh_desktop_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_dsh_desktop",
            hiroute_integrations::DSH_PROFILE_ID_V1,
            hiroute_integrations::DSH_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|e| e.to_string())?;
        let opencode_subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            "agent_opencode_default",
            hiroute_integrations::OPENCODE_PROFILE_ID_V1,
            hiroute_integrations::OPENCODE_INTEGRATION_PROFILE_REF_V1,
        )
        .map_err(|e| e.to_string())?;
        let opencode_model_target = AgentConnectionEffectRoleV1::ManagedConfiguration
            .target_for(&opencode_subject)
            .map_err(|e| e.to_string())?;
        let mut scoped_targets = Vec::new();
        let mut hermes_scan_targets = BTreeMap::new();
        for target in stores
            .control()
            .agent_settings_native_targets(&WorkspaceId::default())
            .map_err(|e| e.to_string())?
        {
            let context = hiroute_integrations::hermes_context_id(target.config_root());
            if !context.starts_with("agent-context/hermes/") {
                return Err("invalid native target".into());
            }
            hermes_scan_targets.insert(context.clone(), target.clone());
            scoped_targets.extend(
                hiroute_integrations::hermes_external_targets(target.config_root())
                    .map_err(|e| e.to_string())?,
            );
        }
        for (subject, collaboration) in [
            (
                &codex_subject,
                home.join(".agents/skills/hiroute-collaboration/SKILL.md"),
            ),
            (
                &claude_subject,
                home.join(".claude/skills/hiroute-collaboration/SKILL.md"),
            ),
            (&qoder_subject, scanner.qoder_user_skill_target()),
            (&pi_subject, scanner.pi_user_skill_target()),
            (&dsh_subject, scanner.dsh_user_skill_target()),
            (&opencode_subject, scanner.opencode_user_skill_target()),
        ]
        .into_iter()
        .filter(|(_, collaboration)| collaboration.is_absolute())
        {
            let root = collaboration
                .parent()
                .and_then(|p| p.parent())
                .ok_or("invalid Skill root")?;
            scoped_targets.push((
                AgentConnectionEffectRoleV1::ManagementSkill
                    .settings_target_for(subject)
                    .map_err(|e| e.to_string())?,
                root.join("hiroute-management/SKILL.md"),
            ));
        }
        // An unavailable native context is local to that Agent. Register only
        // resolved adapter targets so it cannot prevent daemon/control startup.
        for (subject, kind) in [
            (&opencode_subject, hiroute_domain::AgentKindV1::OpenCode),
            (&dsh_subject, hiroute_domain::AgentKindV1::DeepseekHarness),
            (&pi_subject, hiroute_domain::AgentKindV1::Pi),
        ] {
            if let Some(path) = scanner.native_routing_adapter_target(kind) {
                scoped_targets.push((
                    AgentConnectionEffectRoleV1::NativeRoutingAdapter
                        .target_for(subject)
                        .map_err(|e| e.to_string())?,
                    path,
                ));
            }
        }
        scoped_targets.push((
            AgentConnectionEffectRoleV1::NativeRoutingAdapter
                .target_for(&dsh_desktop_subject)
                .map_err(|e| e.to_string())?,
            scanner.dsh_desktop_routing_adapter_target(),
        ));
        scoped_targets.push((
            AgentConnectionEffectRoleV1::ManagedConfiguration
                .target_for(&dsh_desktop_subject)
                .map_err(|e| e.to_string())?,
            scanner.dsh_desktop_models_target(),
        ));
        let mut external_targets = [
            (
                opencode_model_target.clone(),
                scanner.opencode_user_models_target(),
            ),
            (
                AgentConnectionEffectRoleV1::RoutingSkill
                    .target_for(&opencode_subject)
                    .map_err(|e| e.to_string())?,
                scanner.opencode_user_skill_target(),
            ),
            (
                AgentConnectionEffectRoleV1::ManagedConfiguration
                    .target_for(&dsh_subject)
                    .map_err(|e| e.to_string())?,
                scanner.dsh_user_models_target(),
            ),
            (
                AgentConnectionEffectRoleV1::RoutingSkill
                    .target_for(&dsh_subject)
                    .map_err(|e| e.to_string())?,
                scanner.dsh_user_skill_target(),
            ),
            (
                AgentConnectionEffectRoleV1::ManagedConfiguration
                    .target_for(&pi_subject)
                    .map_err(|error| error.to_string())?,
                scanner.pi_user_models_target(),
            ),
            (
                AgentConnectionEffectRoleV1::RoutingSkill
                    .target_for(&pi_subject)
                    .map_err(|error| error.to_string())?,
                scanner.pi_user_skill_target(),
            ),
            (
                AgentConnectionEffectRoleV1::ManagedConfiguration
                    .target_for(&qoder_subject)
                    .map_err(|error| error.to_string())?,
                scanner.qoder_user_config_target(),
            ),
            (claude_managed_target, scanner.claude_user_settings_target()),
            (codex_managed_target, scanner.codex_user_config_target()),
            (profile_target, scanner.codex_profile_config_target()),
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
            (
                AgentConnectionEffectRoleV1::RoutingSkill
                    .target_for(&qoder_subject)
                    .map_err(|error| error.to_string())?,
                scanner.qoder_user_skill_target(),
            ),
        ]
        .into_iter()
        .chain(scoped_targets)
        .filter(|(_, path)| path.is_absolute())
        .collect::<Vec<_>>();
        let mut alternatives = scanner
            .opencode_model_target_candidates()
            .into_iter()
            .skip(1);
        let artifacts = loop {
            match stores.open_managed_artifacts_with_external_targets(
                storage_root.join("managed-artifacts"),
                storage_root.join("artifact-restores"),
                external_targets.clone(),
            ) {
                Ok(artifacts) => break artifacts,
                Err(error) => {
                    // An added JSONC layer must not redirect an existing JSON recovery target.
                    // The same store validates each required marker against the exact candidate path;
                    // no new binding metadata or permissive recovery reader is introduced.
                    if matches!(&error, hiroute_local_storage::LocalStorageError::Locked)
                        && let Some(path) = alternatives.next()
                    {
                        let target = external_targets
                            .iter_mut()
                            .find(|(target, _)| target == &opencode_model_target)
                            .ok_or_else(|| error.to_string())?;
                        target.1 = path.clone();
                        scanner
                            .bind_opencode_model_target(path)
                            .map_err(|e| e.to_string())?;
                        continue;
                    }
                    return Err(error.to_string());
                }
            }
        };
        let observation_workspace_key = observation_workspace_key(storage_root)?;
        let digest_authority = DigestAuthority::new(*observation_workspace_key);
        let observation_root = storage_root.join("observation");
        let observation = Arc::new(
            if startup.preserve_observation_source {
                LocalObservationStore::open_fresh_after_seal(
                    observation_root,
                    digest_authority.clone(),
                )
            } else {
                LocalObservationStore::open(observation_root, digest_authority.clone())
            }
            .map_err(|error| {
                startup.report_failure(stores.record_observation_startup_failure(
                    storage_root,
                    hiroute_host_runtime::StorageRecoveryCause::MigrationFailed,
                ));
                error.to_string()
            })?,
        );
        if startup.preserve_observation_source
            && let Some(progress) = &startup.upgrade_progress
        {
            progress.report(hiroute_host_runtime::StorageUpgradePhase::Validation);
        }
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
            publication_marker_cache: Mutex::new(None),
            settings_publication_candidate: Mutex::new(None),
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
            catalog_fallback: model_feed::FallbackCatalog::new(release_catalog),
            model_feed: Some(model_feed),
            protected_inputs: Mutex::new(BTreeMap::new()),
            manual_protected_inputs: Mutex::new(BTreeMap::new()),
            agent_token_inputs: Mutex::new(BTreeMap::new()),
            model_connections: hiroute_integrations::NativeModelConnectionServiceV1::new(
                hiroute_application::compute_management::TrustedComputeCandidateRegistry::new(),
                overrides.model_transport.unwrap_or_else(|| {
                    #[cfg(feature = "integration-test-hooks")]
                    {
                        crate::model_feed::product_fixture::directory_transport()
                    }
                    #[cfg(not(feature = "integration-test-hooks"))]
                    Arc::new(hiroute_integrations::ReqwestModelDirectoryTransportV1)
                }),
            ),
            model_connection_cancellations: Mutex::new(BTreeMap::new()),
            prepared_discoveries: Mutex::new(BTreeMap::new()),
            hermes_scan_targets: Mutex::new(hermes_scan_targets),
            permission_findings: Mutex::new(BTreeMap::new()),
            admission: hiroute_application::TransactionRuntime::default(),
            plan_admission,
            observation_activity_path: storage_root.join("observation/activity.db"),
            cpa_sources,
            cpa_runtime,
            subscription_sources: Mutex::new(BTreeMap::new()),
            subscription_targets: Mutex::new(BTreeMap::new()),
            subscription_lifecycle: Mutex::new(()),
            subscription_maintenance: Mutex::new(subscriptions::SubscriptionMaintenance::new()?),
            managed_agent_runtime: Mutex::new(None),
            publication_target: Mutex::new(None),
            startup_recovery_complete: AtomicBool::new(false),
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
        // Recovery needs exact configuration descriptors, not the informational native-version
        // inventory. A recovered Secret step still rejects a changed or missing source revision.
        if let Some(progress) = &startup.upgrade_progress
            && progress.started()
        {
            progress.report(hiroute_host_runtime::StorageUpgradePhase::ServiceRecovery);
        }
        adapter.prime_discovery()?;
        // Price availability alone does not reject an otherwise authorized model request.
        // Pending price Operations perform mandatory installation during recovery below.
        let _ = adapter.rebuild_price_snapshot();
        if recover {
            adapter.reconcile_startup_and_open()?;
            adapter.reconcile_cpa_runtime_from_management()?;
            adapter.reconcile_delegation_plan_versions()?;
            adapter.recover_delegation_authorizations()?;
            adapter.enable_subscription_maintenance();
            adapter
                .finish_startup_publication_recovery()
                .map_err(|error| error.to_string())?;
        }
        model_feed::start_retrieval(&adapter);
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
        .with_model_feed(self.adapter.clone())
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
        self.adapter
            .startup_recovery_complete
            .store(false, Ordering::Release);
        if let Some(target) = self
            .adapter
            .publication_target()
            .map_err(|e| e.to_string())?
        {
            target.suspend_requests().map_err(|e| e.to_string())?;
        }
        if let Some(target) = &publication_target {
            target.suspend_requests().map_err(|e| e.to_string())?;
        }
        let worker_gateway = gateway_base_url
            .strip_prefix("http://")
            .and_then(|value| value.strip_suffix("/v1"))
            .ok_or_else(|| "managed Agent Gateway base must be an HTTP IPv4 /v1 URL".to_owned())?
            .parse()
            .map_err(|_| "managed Agent Gateway base has an invalid listener address".to_owned())?;
        self.delegation_executor
            .set_gateway(worker_gateway)
            .map_err(|error| error.to_string())?;
        // Availability is retained by the adapter, so its selection callback must not
        // own that adapter in return. The enclosing runtime owns the live adapter.
        let selection_source: Arc<dyn WorkerInstallationSelectionSource> =
            Arc::new(WorkerSelections(Arc::downgrade(&self.adapter)));
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
        self.adapter
            .restore_active_publication()
            .map_err(|error| error.to_string())?;
        self.adapter.reconcile_startup_and_open()?;
        self.adapter.reconcile_cpa_runtime_from_management()?;
        self.adapter.reconcile_delegation_plan_versions()?;
        self.adapter.recover_delegation_authorizations()?;
        self.adapter.enable_subscription_maintenance();
        self.adapter
            .finish_startup_publication_recovery()
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

    #[cfg(any(unix, windows))]
    pub(crate) fn agent_grant_resolver(&self) -> Arc<dyn super::AgentGrantResolverPort> {
        self.adapter.clone()
    }

    /// Persists one already launcher-validated, exact grant. Neither Local Control nor
    /// Application receives this registrar or can mint a capability.
    pub(crate) fn register_apply_capability(
        &self,
        registration: hiroute_local_storage::ApplyCapabilityRegistrationV1,
    ) -> Result<(), String> {
        self.protected_authority()
            .register_apply_capability(registration)
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
    publication_marker_cache: Mutex<Option<publication::DecodedMarker>>,
    settings_publication_candidate: Mutex<Option<publication::SettingsCandidate>>,
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
    catalog_fallback: model_feed::FallbackCatalog,
    model_feed: Option<Arc<crate::model_feed::ModelFeedService>>,
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
    hermes_scan_targets: Mutex<BTreeMap<String, hiroute_domain::AgentNativeTargetV1>>,
    permission_findings: Mutex<BTreeMap<String, PermissionHardeningRequiredV1>>,
    admission: hiroute_application::TransactionRuntime,
    plan_admission: Arc<hiroute_application::publication::admission::SharedAdmissionGate>,
    observation_activity_path: PathBuf,
    cpa_sources: Option<Arc<dyn CpaRegisteredSourcePort + Send + Sync>>,
    cpa_runtime: Option<Arc<ManagedCpaRuntimeSet>>,
    subscription_sources: Mutex<BTreeMap<String, SubscriptionContextV1>>,
    subscription_targets: Mutex<BTreeMap<String, SubscriptionContextV1>>,
    /// Lock order: subscription lifecycle -> transaction writer -> registry/stores. Held
    /// through all Apply effects and projection, or Forget's snapshot/disable/withdrawal.
    subscription_lifecycle: Mutex<()>,
    subscription_maintenance: Mutex<subscriptions::SubscriptionMaintenance>,
    managed_agent_runtime: Mutex<Option<ManagedAgentRuntimeV1>>,
    publication_target: Mutex<Option<Arc<dyn PublicationTargetPort + Send + Sync>>>,
    startup_recovery_complete: AtomicBool,
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
    fn current_native_credential_refs(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> Result<Option<Vec<String>>, hiroute_domain::NativeCredentialLeaseErrorV1> {
        self.adapter
            .stores
            .lock()
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "control.runtime.pool_lock"))?
            .current_native_credential_refs(request)
    }

    fn lease_native_credential(
        &self,
        request: &NativeCredentialLeaseRequestV1,
    ) -> Result<Option<NativeCredentialLeaseV1>, hiroute_domain::NativeCredentialLeaseErrorV1> {
        self.adapter
            .stores
            .lock()
            .map_err(|_| {
                PortError::new(
                    PortErrorCode::Unavailable,
                    "control.runtime.native_credential_lock",
                )
            })?
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

struct WorkerSelections(Weak<LocalControlAdapter>);

impl WorkerInstallationSelectionSource for WorkerSelections {
    fn selection(
        &self,
        harness: hiroute_domain::delegation::WorkerHarnessV1,
    ) -> Result<Option<WorkerInstallationSelection>, hiroute_domain::delegation::DelegationErrorV1>
    {
        self.0
            .upgrade()
            .ok_or(hiroute_domain::delegation::DelegationErrorV1::StorageUnavailable)?
            .selection(harness)
    }
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
        selected
            .map(|(selection, revision)| {
                Ok(WorkerInstallationSelection {
                    config: WorkerInstallationConfig::from_record(&selection)?,
                    revision,
                })
            })
            .transpose()
    }
}

impl ControlStatePort for LocalControlAdapter {
    fn try_observation_revision(
        &self,
        workspace: &WorkspaceId,
        deadline: std::time::Instant,
    ) -> Result<hiroute_application::control::ControlReadAttempt<u64>, ControlReadError> {
        use hiroute_application::control::ControlReadAttempt;
        let stores = match self.stores.try_lock() {
            Ok(stores) => stores,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(ControlReadAttempt::Busy),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(ControlReadError::Unavailable),
        };
        if std::time::Instant::now() >= deadline {
            return Err(ControlReadError::Unavailable);
        }
        stores
            .control()
            .current_revisions(workspace)
            .map(|r| ControlReadAttempt::Ready(r.target))
            .map_err(map_port)
    }

    fn try_validate_observation_capability(
        &self,
        raw: &str,
        workspace: &WorkspaceId,
        principal: PrincipalKind,
        operation: &str,
        digest: &CanonicalDigest,
        deadline: std::time::Instant,
    ) -> Result<hiroute_application::control::ControlReadAttempt<u64>, ControlReadError> {
        use hiroute_application::control::ControlReadAttempt;
        let capability =
            ProtectedApplyCapability::new(raw.to_owned()).map_err(|_| ControlReadError::Denied)?;
        let principal = match principal {
            PrincipalKind::InteractiveUser => "interactive-user",
            PrincipalKind::Desktop => "desktop",
            PrincipalKind::Skill | PrincipalKind::SealedCollaboration => {
                return Err(ControlReadError::Denied);
            }
        };
        let stores = match self.stores.try_lock() {
            Ok(stores) => stores,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(ControlReadAttempt::Busy),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(ControlReadError::Unavailable),
        };
        if std::time::Instant::now() >= deadline {
            return Err(ControlReadError::Unavailable);
        }
        let revisions = stores
            .control()
            .current_revisions(workspace)
            .map_err(map_port)?;
        stores
            .control()
            .verify_apply_authorization(
                &capability,
                workspace,
                principal,
                operation,
                digest,
                &revisions,
            )
            .map_err(map_port)?;
        Ok(ControlReadAttempt::Ready(revisions.target))
    }
    fn trust_recovery_targets(
        &self,
        plan_id: &str,
    ) -> Result<Vec<hiroute_application::agent_connection::PreviousRoutingTargetV1>, ControlReadError>
    {
        hiroute_application::agent_connection::previous_routing_targets(self, self, plan_id)
    }

    fn trust_plan_ids(&self, workspace: &WorkspaceId) -> Result<Vec<String>, ControlReadError> {
        let stores = self.stores_lock().map_err(map_port)?;
        let heads = stores
            .control()
            .plan_heads(workspace)
            .map_err(|_| ControlReadError::Unavailable)?;
        Ok(heads
            .into_iter()
            .filter(|h| h.status == hiroute_domain::PlanLifecycleV1::Enabled)
            .map(|h| h.reference.plan_id.as_str().to_owned())
            .collect())
    }

    fn trust_context(
        &self,
        workspace: &WorkspaceId,
        plan_id: &str,
    ) -> Result<hiroute_application::trust::context::TrustContextV1, ControlReadError> {
        self.read_trust_context(workspace, plan_id)
    }
    fn trust_primary_bindings(
        &self,
        workspace: &WorkspaceId,
        plan_id: &str,
        revision: Option<u64>,
        branch_id: Option<&str>,
    ) -> Result<Vec<String>, ControlReadError> {
        self.read_trust_primary_bindings(workspace, plan_id, revision, branch_id)
    }
    fn trust_task_population(
        &self,
        workspace: &WorkspaceId,
        query: &hiroute_domain::TrustPopulationQueryV1,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<hiroute_domain::TrustTaskPopulationV1, ControlReadError> {
        let stores = self.stores_lock().map_err(map_port)?;
        stores
            .runtime()
            .trust_task_population(workspace, &query.plan_id, query.revision, from_ms, to_ms)
            .map_err(|_| ControlReadError::Unavailable)
    }
    fn trust_runs(
        &self,
        workspace: &WorkspaceId,
        ids: &[String],
    ) -> Result<
        std::collections::BTreeMap<String, hiroute_application::trust::context::TrustRunV1>,
        ControlReadError,
    > {
        self.read_trust_runs(workspace, ids)
    }

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

mod agent_discovery;

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
    selected: &[(AgentKindV1, PathBuf)],
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
    for (kind, executable) in selected {
        let target = match kind {
            AgentKindV1::Codex => &mut layout.codex_executable,
            AgentKindV1::ClaudeCode => &mut layout.claude_executable,
            AgentKindV1::Qoder => &mut layout.qoder_executable,
            AgentKindV1::Pi => &mut layout.pi_executable,
            AgentKindV1::DeepseekHarness => &mut layout.dsh_executable,
            AgentKindV1::OpenCode => &mut layout.opencode_executable,
            AgentKindV1::Hermes => continue,
        };
        *target = executable.clone();
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
                AgentKindV1::Qoder => QODER_PROFILE_ID_V1,
                AgentKindV1::Pi => hiroute_integrations::PI_PROFILE_ID_V1,
                AgentKindV1::DeepseekHarness => hiroute_integrations::DSH_PROFILE_ID_V1,
                AgentKindV1::OpenCode => hiroute_integrations::OPENCODE_PROFILE_ID_V1,
                AgentKindV1::Hermes => hiroute_integrations::HERMES_PROFILE_ID_V1,
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
        native_target: None,
        codex_access: None,
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

#[cfg(test)]
#[path = "runtime_observation_tests.rs"]
mod tests;

#[cfg(test)]
mod history_cutover_tests;

#[cfg(test)]
#[path = "runtime_startup_progress_tests.rs"]
mod startup_progress_tests;

#[cfg(test)]
#[path = "runtime_worker_dependencies_tests.rs"]
mod worker_dependencies_tests;

#[cfg(test)]
#[path = "runtime_early_cancel_tests.rs"]
mod early_cancel_tests;

pub(crate) use protected_inputs::RuntimeAuthority;
impl RuntimeAuthority {
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
