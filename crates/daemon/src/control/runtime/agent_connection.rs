use std::collections::BTreeMap;

use hiroute_application::agent_connection::{
    AgentConnectionBeforeFingerprintsV1, AgentConnectionPlanningFactsV1,
    AgentConnectionPlanningInputV1, RegisteredGatewayEndpointV1,
};
use hiroute_application::control::{AgentConnectionControlPort, ControlReadError};
use hiroute_application_api::{
    AGENT_CONNECTION_STATUS_SCHEMA_V1, AgentConnectSpecV1, AgentConnectionStateV1,
    AgentConnectionStatusRequestV1, AgentConnectionStatusV1, AgentLaunchDescriptorRequestV1,
    ManagedClaudeLaunchDescriptorV2, WarningV1,
};
use hiroute_domain::{
    ActiveAgentConnectionV1, AgentAccessGrantMaterial, AgentAccessGrantRefV1,
    AgentActivationModeV1, AgentConfigDocumentV1, AgentConnectionEffectRoleV1,
    AgentConnectionTransactionSubjectV1, AgentPlanGrantV1, CanonicalDigest, ControlRepositoryPort,
    PublicationRecordV1, PublicationRepositoryPort, SecretStorePort, WorkspaceId,
};
use hiroute_integrations::AgentDiscoveryOutcomeV1;

use super::LocalControlAdapter;

#[path = "agent_connection_discovery.rs"]
mod discovery;

const APPLY_OPERATION: &str = "ApplyAgentConnectionChange";
const ACCESS_POINT_REF: &str = "access-point/local/default";

struct ActiveJoinV1 {
    connection: ActiveAgentConnectionV1,
    grant: AgentAccessGrantRefV1,
    publication_record: PublicationRecordV1,
}

impl LocalControlAdapter {
    fn active_join(&self, connection_id: &str) -> Result<ActiveJoinV1, ControlReadError> {
        // The scanner is deliberately invoked for every join. A descriptor or raw-token request
        // never inherits a prior Preview's executable or auth-precedence observation.
        let (connection, operation) = self.latest_connection_operation(connection_id)?;
        let discovery = self.exact_discovery(
            &connection.connection.agent_id,
            &connection.connection.profile_id,
            &connection.installed_version,
        )?;
        let AgentDiscoveryOutcomeV1::Supported { installation } = &discovery.outcome else {
            return Err(ControlReadError::NotFound);
        };
        match connection.connection.activation_mode {
            AgentActivationModeV1::ManagedLaunch => {
                if installation.observation_digest != connection.observation_digest
                    && !self.scanner.matches_legacy_claude_observation(
                        &connection.installed_version,
                        &connection.observation_digest,
                    )
                {
                    return Err(ControlReadError::Denied);
                }
                let preflight = discovery.managed_launch.ok_or(ControlReadError::Denied)?;
                if !preflight.is_launchable() || preflight.executable.is_empty() {
                    return Err(ControlReadError::Denied);
                }
            }
            AgentActivationModeV1::ManagedConfiguration => {
                let intent = operation
                    .plan
                    .external()
                    .iter()
                    .find(|intent| intent.effect_id() == "agent-connection-managed-configuration")
                    .ok_or(ControlReadError::Corrupt)?;
                let change = self
                    .native_claude_change(intent)
                    .map_err(super::map_port)?
                    .ok_or(ControlReadError::Corrupt)?;
                if !self
                    .artifacts
                    .rendered_external_is_recoverable(&operation.operation_id, intent)
                    .map_err(super::map_port)?
                    || !self
                        .scanner
                        .claude_user_config_change_is_applied(&change)
                        .map_err(|_| ControlReadError::Denied)?
                {
                    return Err(ControlReadError::Denied);
                }
            }
        }

        let stores = self
            .stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        let grant = stores
            .secrets()
            .inspect_agent_access_grant(WorkspaceId::DEFAULT, connection_id)
            .map_err(super::map_port)?
            .ok_or(ControlReadError::Denied)?;
        let publication_record = stores
            .control()
            .active_publication(&WorkspaceId::default())
            .map_err(super::map_port)?
            .ok_or(ControlReadError::Denied)?;
        let publication = publication_record
            .verify()
            .map_err(|_| ControlReadError::Corrupt)?;
        let published_plans = publication
            .published_agent_plans()
            .map_err(|_| ControlReadError::Corrupt)?;
        let rebound_grant = AgentPlanGrantV1::derive_with_scope(
            connection.connection.protocol,
            connection.connection.grant.allowed_scope,
            connection.connection.grant.default_agent_plan_id.clone(),
            connection.connection.grant.allowed_agent_plan_ids.clone(),
            &published_plans,
        )
        .map_err(|_| ControlReadError::Denied)?;
        if rebound_grant != connection.connection.grant {
            return Err(ControlReadError::Denied);
        }
        let model_grant = hiroute_domain::AgentModelGrantV2::from_plan_ids(
            connection.connection.protocol,
            rebound_grant.allowed_agent_plan_ids.clone(),
            &publication,
        )
        .map_err(|_| ControlReadError::Denied)?;
        let published_grant = publication
            .grants
            .iter()
            .find(|candidate| candidate.grant_id == grant.grant_id())
            .ok_or(ControlReadError::Denied)?;
        if grant.owner_scope() != WorkspaceId::DEFAULT
            || grant.connection_id() != connection_id
            || grant.scope().protocol() != connection.connection.protocol
            || grant.scope().model_grant() != &model_grant
            || published_grant.generation != grant.generation()
            || published_grant.bearer_token_sha256 != *grant.material_sha256()
            || published_grant.model_grant != model_grant
        {
            return Err(ControlReadError::Denied);
        }
        if self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .is_none()
        {
            return Err(ControlReadError::Unavailable);
        }
        Ok(ActiveJoinV1 {
            connection,
            grant,
            publication_record,
        })
    }

    /// Protected daemon-only resolver used by the raw owner socket. Material is opened only
    /// after the complete fresh scanner/connection/publication/grant join succeeds.
    pub(super) fn resolve_active_agent_grant(
        &self,
        connection_id: &str,
    ) -> Result<AgentAccessGrantMaterial, ControlReadError> {
        let grant = if connection_id
            .strip_prefix("agent-connection/")
            .is_some_and(|context| self.settings_agent_for_context(context).is_some())
        {
            self.active_model_settings_grant(connection_id)?
        } else {
            self.active_join(connection_id)?.grant
        };
        self.stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .secrets()
            .resolve_agent_access_grant(&grant)
            .map_err(super::map_port)
    }

    fn before_fingerprints(
        &self,
        subject: &AgentConnectionTransactionSubjectV1,
        publication_digest: CanonicalDigest,
    ) -> Result<AgentConnectionBeforeFingerprintsV1, ControlReadError> {
        let fingerprint = |role: AgentConnectionEffectRoleV1| {
            let target = role
                .target_for(subject)
                .map_err(|_| ControlReadError::Corrupt)?;
            self.artifacts
                .current_external_fingerprint(&target)
                .map_err(super::map_port)
        };
        Ok(AgentConnectionBeforeFingerprintsV1 {
            grant_publication: Some(publication_digest),
            managed_configuration: fingerprint(AgentConnectionEffectRoleV1::ManagedConfiguration)?,
            model_catalog: fingerprint(AgentConnectionEffectRoleV1::ModelCatalog)?,
            routing_skill: fingerprint(AgentConnectionEffectRoleV1::RoutingSkill)?,
            instruction_overlay: fingerprint(AgentConnectionEffectRoleV1::InstructionOverlay)?,
            spawn_guidance: fingerprint(AgentConnectionEffectRoleV1::SpawnGuidanceRewrite)?,
        })
    }
}

impl LocalControlAdapter {
    fn capture_planning_facts(
        &self,
        spec: &AgentConnectSpecV1,
    ) -> Result<AgentConnectionPlanningInputV1, ControlReadError> {
        let discovery =
            self.exact_discovery(&spec.agent_id, &spec.profile_id, &spec.installed_version)?;
        let AgentDiscoveryOutcomeV1::Supported { installation } = discovery.outcome else {
            return Err(ControlReadError::NotFound);
        };
        let runtime = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .clone()
            .ok_or(ControlReadError::Unavailable)?;
        let stores = self
            .stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        let active_record = stores
            .control()
            .active_publication(&WorkspaceId::default())
            .map_err(super::map_port)?
            .ok_or(ControlReadError::NotFound)?;
        let active = active_record
            .verify()
            .map_err(|_| ControlReadError::Corrupt)?;
        let plans = active
            .published_agent_plans()
            .map_err(|_| ControlReadError::Corrupt)?;
        let expected_revisions = stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .map_err(super::map_port)?;
        let connection_id = format!("agent-connection/{}/{}", spec.agent_id, spec.profile_id);
        let expected_grant_generation = stores
            .secrets()
            .inspect_agent_access_grant(WorkspaceId::DEFAULT, &connection_id)
            .map_err(super::map_port)?
            .map_or(0, |reference| reference.generation());
        drop(stores);

        let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            spec.agent_id.clone(),
            spec.profile_id.clone(),
            installation.profile.integration_profile_ref.clone(),
        )
        .map_err(|_| ControlReadError::Corrupt)?;
        // A readable exact Claude credential has already been materialized as an independent
        // ComputeSource before Agent connection. In that case ordinary `claude` is the primary
        // product path, so Apply owns the user config; isolated managed launch remains the safe
        // fallback when the third-party auth source cannot be materialized.
        let persistent_claude_source = spec.profile_id == "claude-messages-v1"
            && discovery
                .discovered_credential
                .as_ref()
                .is_some_and(|descriptor| self.scanner.is_claude_user_credential(descriptor));
        let activation_mode =
            if installation.profile.supports_managed_launch() && !persistent_claude_source {
                AgentActivationModeV1::ManagedLaunch
            } else {
                AgentActivationModeV1::ManagedConfiguration
            };
        let current_config = AgentConfigDocumentV1 {
            fields: installation
                .effective_config
                .iter()
                .map(|(path, field)| (path.clone(), field.value.clone()))
                .collect::<BTreeMap<_, _>>(),
        };
        let mut warnings = Vec::new();
        let mut blockers = Vec::new();
        if activation_mode == AgentActivationModeV1::ManagedLaunch {
            let preflight = discovery.managed_launch.ok_or(ControlReadError::Corrupt)?;
            warnings.extend(preflight.warnings.iter().map(|warning| WarningV1 {
                code: warning.code.clone(),
                details_schema: "hiroute.managed-launch-warning/v1".to_owned(),
            }));
            blockers.extend(
                preflight
                    .conflicts
                    .iter()
                    .map(|conflict| conflict.code.clone()),
            );
        }
        Ok(AgentConnectionPlanningInputV1 {
            facts: AgentConnectionPlanningFactsV1 {
                installation: *installation,
                gateway: RegisteredGatewayEndpointV1 {
                    access_point_ref: ACCESS_POINT_REF.to_owned(),
                    base_endpoint: runtime.gateway_base_url,
                    trusted_cli_executable: runtime.trusted_hiroute_executable,
                },
                publication_digest: active_record.digest.clone(),
                plans,
                plan_route_digests: active
                    .plans
                    .iter()
                    .map(|plan| {
                        // The scope sealed here must equal the grant the next publication seal
                        // rebinds to the current compiler contract, so derive both digests from
                        // the upgraded Plan form.
                        let upgraded = plan
                            .clone()
                            .into_current()
                            .map_err(|_| ControlReadError::Corrupt)?;
                        Ok((
                            upgraded.agent_plan_id().clone(),
                            upgraded.body.materialized_route_digest.clone(),
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()?,
                current_config,
                expected_revisions,
                before_fingerprints: self.before_fingerprints(&subject, active_record.digest)?,
                registered_tools: Vec::new(),
                activation_mode,
                expected_grant_generation,
            },
            warnings,
            blockers,
        })
    }
}

impl AgentConnectionControlPort for LocalControlAdapter {
    fn check_collaboration(&self, agent_id: &str) -> Result<(), ControlReadError> {
        #[cfg(unix)]
        {
            // A sibling from the running installation, never a PATH-selected replacement.
            let cli = std::env::current_exe()
                .map_err(|_| ControlReadError::Unavailable)?
                .with_file_name("hiroute");
            let result = match agent_id {
                "agent_codex_default" => self.scanner.check_codex_collaboration(&cli),
                "agent_claude_default" => self.scanner.check_claude_collaboration(&cli),
                _ => return Err(ControlReadError::NotFound),
            };
            result.map_err(|error| {
                eprintln!("native collaboration check: {error}");
                ControlReadError::Unavailable
            })
        }
        #[cfg(not(unix))]
        {
            let _ = agent_id;
            Err(ControlReadError::Unavailable)
        }
    }
    fn check_native_authentication(&self, agent_id: &str) -> Result<(), ControlReadError> {
        #[cfg(unix)]
        {
            let result = match agent_id {
                "agent_codex_default" => self.scanner.check_codex_native_authentication(),
                "agent_claude_default" => self.scanner.check_claude_native_authentication(),
                _ => return Err(ControlReadError::NotFound),
            };
            result.map_err(|error| {
                eprintln!("native authentication check: {error}");
                ControlReadError::Unavailable
            })
        }
        #[cfg(not(unix))]
        {
            let _ = agent_id;
            Err(ControlReadError::NotFound)
        }
    }

    fn validate_live_check_target(
        &self,
        request: &hiroute_application_api::AgentCheckRequestV1,
    ) -> Result<(), ControlReadError> {
        self.validate_model_check_target(request)
    }

    fn execute_live_check(
        &self,
        request: &hiroute_application_api::AgentCheckRequestV1,
        request_digest: &CanonicalDigest,
    ) -> Result<hiroute_domain::AgentSurfaceCheckRecordV1, ControlReadError> {
        self.execute_model_live_check(request, request_digest)
    }

    fn save_live_check_result(
        &self,
        record: &hiroute_domain::AgentSurfaceCheckRecordV1,
    ) -> Result<bool, ControlReadError> {
        self.save_model_live_check_result(record)
    }

    fn settings_status(
        &self,
        request: &hiroute_application_api::AgentSettingsStatusRequestV2,
    ) -> Result<hiroute_application_api::AgentModelSettingsStatusV2, ControlReadError> {
        self.model_settings_status(request)
    }

    fn settings_facts(
        &self,
        spec: &hiroute_application_api::AgentSettingsSpecV2,
    ) -> Result<hiroute_application::agent_connection::AgentSettingsPlanningInput, ControlReadError>
    {
        self.capture_settings_facts(spec)
    }

    fn planning_facts(
        &self,
        spec: &AgentConnectSpecV1,
    ) -> Result<AgentConnectionPlanningInputV1, ControlReadError> {
        let first = self.capture_planning_facts(spec)?;
        let second = self.capture_planning_facts(spec)?;
        if !first.facts.same_snapshot(&second.facts) || first.blockers != second.blockers {
            return Err(ControlReadError::SnapshotChanged);
        }
        Ok(second)
    }

    fn connection_status(
        &self,
        request: &AgentConnectionStatusRequestV1,
    ) -> Result<AgentConnectionStatusV1, ControlReadError> {
        let joined = match self.active_join(&request.connection_id) {
            Ok(joined) => Some(joined),
            Err(ControlReadError::NotFound | ControlReadError::Denied) => None,
            Err(error) => return Err(error),
        };
        // On the ready path, project every field from the one exact join rather than performing
        // a second completed-operation read that could race a newer connection generation.
        let connection = joined.as_ref().map_or_else(
            || self.latest_connection(&request.connection_id),
            |joined| Ok(joined.connection.clone()),
        )?;
        let grant_exists = if joined.is_some() {
            true
        } else {
            self.stores
                .lock()
                .map_err(|_| ControlReadError::Unavailable)?
                .secrets()
                .inspect_agent_access_grant(WorkspaceId::DEFAULT, &request.connection_id)
                .map_err(super::map_port)?
                .is_some()
        };
        Ok(AgentConnectionStatusV1 {
            schema: AGENT_CONNECTION_STATUS_SCHEMA_V1.to_owned(),
            connection_id: connection.connection_id,
            agent_id: connection.connection.agent_id,
            profile_id: connection.connection.profile_id,
            installed_version: connection.installed_version,
            state: if joined.is_some() {
                AgentConnectionStateV1::Active
            } else if grant_exists {
                AgentConnectionStateV1::Disconnected
            } else {
                AgentConnectionStateV1::Revoked
            },
            activation_mode: connection.connection.activation_mode,
            revision: connection.connection.revision,
            launch_ready: joined.is_some(),
            publication_digest: joined
                .as_ref()
                .map(|value| value.publication_record.digest.clone()),
            grant_generation: joined.as_ref().map(|value| value.grant.generation()),
        })
    }

    fn managed_launch_descriptor(
        &self,
        request: &AgentLaunchDescriptorRequestV1,
    ) -> Result<ManagedClaudeLaunchDescriptorV2, ControlReadError> {
        let context_id = request
            .connection_id
            .strip_prefix("agent-connection/")
            .filter(|context| format!("agent-connection/{context}") == request.connection_id)
            .ok_or(ControlReadError::NotFound)?;
        if self.settings_agent_for_context(context_id)
            != Some(super::settings_facts::SettingsAgentClass::Claude)
        {
            return Err(ControlReadError::NotFound);
        }
        let Some(join) = self.configured_model_settings_join(context_id)? else {
            return Err(ControlReadError::Denied);
        };
        let payload =
            hiroute_application::agent_connection::decode_settings_claude_model_file(&join.intent)
                .map_err(super::map_port)?;
        let hiroute_application::agent_connection::ClaudeModelFileAction::Configure {
            snapshot: expected_snapshot,
            gateway_base_url,
            trusted_hiroute_executable,
            ..
        } = payload.change
        else {
            return Err(ControlReadError::Corrupt);
        };
        if payload.context_id != context_id {
            return Err(ControlReadError::Corrupt);
        }
        let snapshot_digest =
            CanonicalDigest::of(&expected_snapshot).map_err(|_| ControlReadError::Corrupt)?;
        let presets = expected_snapshot.presets;
        let snapshot_gateway = gateway_base_url
            .strip_suffix("/v1")
            .ok_or(ControlReadError::Corrupt)?;
        let snapshot_helper = trusted_hiroute_executable.as_str();
        let runtime = self
            .managed_agent_runtime
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?
            .clone()
            .ok_or(ControlReadError::Unavailable)?;
        if runtime.gateway_base_url != gateway_base_url
            || runtime.trusted_hiroute_executable != snapshot_helper
        {
            return Err(ControlReadError::Denied);
        }
        let executable = expected_snapshot.executable.as_str();
        let mut descriptor = ManagedClaudeLaunchDescriptorV2::trusted(
            request.connection_id.clone(),
            "claude-messages-v1",
            executable,
            snapshot_digest,
            join.grant.generation(),
            join.publication_digest,
            snapshot_gateway.to_owned(),
            presets,
            runtime.trusted_hiroute_executable,
        )
        .map_err(|_| ControlReadError::Corrupt)?;
        descriptor.context_window_tokens = expected_snapshot.context_window_tokens;
        descriptor
            .validate()
            .map_err(|_| ControlReadError::Corrupt)?;
        Ok(descriptor)
    }
}

#[cfg(unix)]
impl crate::control::AgentGrantResolverPort for LocalControlAdapter {
    fn resolve_agent_grant(&self, connection_id: &str) -> Result<AgentAccessGrantMaterial, ()> {
        self.resolve_active_agent_grant(connection_id)
            .map_err(|_| ())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};

    use hiroute_application::agent_connection::{
        AgentConnectionPlanner, AgentConnectionRestorePlanningFactsV1,
        AgentConnectionRestorePointV1, AgentConnectionRestorePreviewOutcomeV1, AgentRestoreSpecV1,
        preview_agent_connection_restore, seal_agent_connection_restore,
    };
    use hiroute_application::publication::AtomicPublicationTarget;
    use hiroute_application::{TransactionCoordinator, TransactionRuntime};
    use hiroute_application_api::AGENT_CONNECT_SPEC_SCHEMA_V1;
    use hiroute_domain::{
        AgentConfigRestorePointV1, AgentPlanAllowedScopeV1, BeginOperationOutcome,
        ConnectorRegistryBundleV1, GatewayPublicationRevision, GatewayPublicationV1,
        IdempotencyScopeV1, OperationId, OperationV1, ProtectedApplyCapability,
        PublicationRecordV1, ReleaseModelDataBundleV2,
    };
    use hiroute_integrations::{
        AgentFilesystemLayoutV1, ClaudeRegistrationIndexV1, FilesystemAgentScannerV1,
    };
    use hiroute_local_storage::{ApplyCapabilityRegistrationV1, LocalStorageSet};

    use super::super::ManagedAgentRuntimeV1;
    use super::*;

    fn write_claude(path: &std::path::Path, version: &str) {
        fs::write(
            path,
            format!("#!/bin/sh\nprintf '%s\\n' '{version} (Claude Code)'\n"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn planning_facts_rescan_the_exact_managed_executable_on_every_request() {
        #[cfg(unix)]
        if crate::test_support::isolated_agent_home(
            "control::runtime::agent_connection::tests::planning_facts_rescan_the_exact_managed_executable_on_every_request",
        ) {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let home = directory.path().join("home");
        let project = directory.path().join("project");
        let bin = directory.path().join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let claude = bin.join("claude");
        write_claude(&claude, "2.1.231");

        let registry: ConnectorRegistryBundleV1 = serde_json::from_slice(include_bytes!(
            "../../../../../assets/connector-registry/current/registry-seed.json"
        ))
        .unwrap();
        let model_data: ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
            "../../../../../assets/release-facts/current/bundle/model-data.json"
        ))
        .unwrap();
        let mut layout = AgentFilesystemLayoutV1::from_process(&home, &project);
        layout.codex_executable = bin.join("missing-codex");
        layout.claude_executable = claude.clone();
        layout.claude_launch_settings = None;
        layout.claude_project_settings.clear();
        layout.claude_user_settings = home.join("missing-settings.json");
        layout.claude_managed_settings.clear();
        let scanner = FilesystemAgentScannerV1::new(
            layout,
            ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &model_data.data)
                .unwrap(),
        );
        let storage = directory.path().join("storage");
        let stores = LocalStorageSet::open_for_daemon_startup(&storage).unwrap();
        let artifacts = stores
            .open_managed_artifacts(storage.join("artifacts"), storage.join("restores"))
            .unwrap();
        let mut publication = GatewayPublicationV1::decode_persisted(include_bytes!(
            "../../../../../e2e/product/golden/routing/compiled-publication.v2.json"
        ))
        .unwrap();
        publication.publication_revision = GatewayPublicationRevision::new(1).unwrap();
        publication.aliases.clear();
        publication.grants.clear();
        let record =
            PublicationRecordV1::from_publication(WorkspaceId::default(), &publication).unwrap();
        stores.control().prepare_publication(&record, None).unwrap();
        stores
            .control()
            .mark_publication_active(
                &WorkspaceId::default(),
                record.publication_revision,
                &record.digest,
            )
            .unwrap();
        let target = Arc::new(AtomicPublicationTarget::default());
        let plan_admission =
            Arc::new(hiroute_application::publication::admission::SharedAdmissionGate::new());
        let delegation_safety = Arc::new(
            hiroute_application::delegation::safety::RunSafetyProjection::new(
                plan_admission.clone(),
                "test-delegation-epoch".to_owned(),
            )
            .unwrap(),
        );
        delegation_safety.finish_startup_recovery();
        let adapter = LocalControlAdapter {
            price_snapshot: std::sync::Arc::new(
                hiroute_application::prices::PriceSnapshotSlot::default(),
            ),
            publication_diagnostics: Mutex::new(Default::default()),
            stores: Mutex::new(stores),
            delegation_digest_authority: hiroute_observation::DigestAuthority::new([7; 32]),
            delegation_observation: Arc::new(
                hiroute_observation::LocalObservationStore::open(
                    storage.join("delegation-observation"),
                    hiroute_observation::DigestAuthority::new([7; 32]),
                )
                .unwrap(),
            ),
            observation_workspace_key: zeroize::Zeroizing::new([7; 32]),
            delegation_epoch: "test-delegation-epoch".to_owned(),
            delegation_safety,
            delegation_run_authority: Arc::new(
                crate::delegation::run_authority::DelegationRunAuthority::default(),
            ),
            delegation_finalization: Arc::new(
                crate::delegation::finalization::DelegationFinalization::default(),
            ),
            scanner,
            artifacts,
            release_catalog: None,
            protected_inputs: Mutex::new(BTreeMap::new()),
            manual_protected_inputs: Mutex::new(BTreeMap::new()),
            agent_token_inputs: Mutex::new(BTreeMap::new()),
            model_connections: hiroute_integrations::NativeModelConnectionServiceV1::new(
                hiroute_application::compute_management::TrustedComputeCandidateRegistry::new(),
                Arc::new(hiroute_integrations::ReqwestModelDirectoryTransportV1),
            ),
            model_connection_cancellations: Mutex::new(BTreeMap::new()),
            prepared_discoveries: Mutex::new(BTreeMap::new()),
            permission_findings: Mutex::new(BTreeMap::new()),
            admission: TransactionRuntime::default(),
            plan_admission,
            observation_activity_path: storage.join("observation/activity.db"),
            cpa_sources: None,
            cpa_runtime: None,
            subscription_sources: Mutex::new(BTreeMap::new()),
            subscription_targets: Mutex::new(BTreeMap::new()),
            subscription_maintenance: Mutex::new(
                crate::control::runtime::subscriptions::SubscriptionMaintenance::new().unwrap(),
            ),
            managed_agent_runtime: Mutex::new(Some(ManagedAgentRuntimeV1 {
                gateway_base_url: "http://127.0.0.1:5837/v1".to_owned(),
                trusted_hiroute_executable: "/fixture/hiroute".to_owned(),
                worker_executor_availability: Arc::new(
                    crate::delegation::installation::WorkerExecutorAvailabilityRegistry::unconfigured(),
                ),
                resident_service_ready: false,
            })),
            publication_target: Mutex::new(Some(target.clone())),
            delegation_native_cleanup_cursor: Mutex::new(None),
            delegation_task_maintenance_cursor: Mutex::new(None),
        };
        let plan = publication
            .published_agent_plans()
            .unwrap()
            .into_iter()
            .find(|plan| {
                plan.supported_ingress
                    .contains(&hiroute_domain::AgentIngressProtocolV1::Messages)
            })
            .unwrap();
        let spec = AgentConnectSpecV1 {
            schema_version: AGENT_CONNECT_SPEC_SCHEMA_V1,
            agent_id: "agent_claude_default".to_owned(),
            profile_id: "claude-messages-v1".to_owned(),
            installed_version: "2.1.231".to_owned(),
            default_agent_plan_id: plan.agent_plan_id.clone(),
            allowed_scope: AgentPlanAllowedScopeV1::Selected,
            allowed_agent_plan_ids: BTreeSet::from([plan.agent_plan_id]),
            native_subagent_routing: false,
            dynamic_catalog_available: false,
        };
        let observed = adapter
            .exact_discovery(&spec.agent_id, &spec.profile_id, &spec.installed_version)
            .unwrap();
        assert!(observed.managed_launch.as_ref().unwrap().is_launchable());
        let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
            spec.agent_id.clone(),
            spec.profile_id.clone(),
            "builtin/claude-messages/v1",
        )
        .unwrap();
        assert!(
            adapter
                .before_fingerprints(&subject, record.digest.clone())
                .is_ok()
        );
        let mut facts = adapter.planning_facts(&spec).unwrap();
        assert_eq!(
            facts.facts.activation_mode,
            AgentActivationModeV1::ManagedLaunch
        );
        assert!(facts.blockers.is_empty());

        // This executable fixture only prints a version; discovery must not claim auth proof.
        assert!(matches!(
            AgentConnectionPlanner::preview_connect(spec.clone(), facts.facts.clone()),
            Err(hiroute_application::agent_connection::AgentConnectionPlanningError::UnprovenCapabilities(_))
        ));
        // The remaining assertions exercise the legacy journal/publication join with explicitly
        // synthetic component evidence. They do not prove native authentication or live readiness.
        for proof in &mut facts.facts.installation.capability_evidence {
            if hiroute_domain::AgentAction::ConfigureModel
                .requires()
                .contains(&proof.capability)
            {
                proof.state = hiroute_domain::CapabilityState::Proven;
                proof.reason = None;
            }
        }
        let preview = AgentConnectionPlanner::preview_connect(spec.clone(), facts.facts).unwrap();
        let plan = AgentConnectionPlanner::seal_apply(
            &preview,
            &preview.change_digest,
            &preview.expected_revisions,
            &preview.expected_revisions,
        )
        .unwrap();
        let workspace = WorkspaceId::default();
        let scope =
            IdempotencyScopeV1::new("interactive-user", APPLY_OPERATION, "active-managed-join")
                .unwrap();
        let request_digest = CanonicalDigest::of_bytes(b"active-managed-join-request");
        let operation = OperationV1::new(
            OperationId::derive(&workspace, &scope, &request_digest),
            workspace.clone(),
            scope,
            request_digest,
            preview.change_digest.clone(),
            preview.expected_revisions.clone(),
            plan,
        )
        .unwrap();
        let capability = "agent-connection-capability-0123456789abcdef";
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 60;
        {
            let stores = adapter.stores.lock().unwrap();
            stores
                .apply_capability_registrar()
                .register(
                    ApplyCapabilityRegistrationV1::from_protected_launcher(
                        capability.to_owned(),
                        operation.idempotency.principal.clone(),
                        workspace.clone(),
                        operation.idempotency.operation_kind.clone(),
                        operation.accepted_digest.clone(),
                        operation.expected_revisions.clone(),
                        expires,
                    )
                    .unwrap(),
                )
                .unwrap();
            let authorization = stores
                .control()
                .verify_apply_authorization(
                    &ProtectedApplyCapability::new(capability.to_owned()).unwrap(),
                    &workspace,
                    &operation.idempotency.principal,
                    &operation.idempotency.operation_kind,
                    &operation.accepted_digest,
                    &operation.expected_revisions,
                )
                .unwrap();
            assert_eq!(
                stores
                    .control()
                    .begin_operation(&operation, &authorization)
                    .unwrap(),
                BeginOperationOutcome::Created
            );
        }
        let completed = TransactionCoordinator::new(
            &adapter,
            &adapter,
            &adapter,
            &adapter,
            &adapter,
            &adapter.admission,
        )
        .run(&operation.operation_id)
        .unwrap();
        assert_eq!(
            completed.state,
            hiroute_domain::OperationState::Succeeded,
            "{completed:#?}"
        );

        let connection_id = preview.change_spec.resource_id.clone().unwrap();
        // The legacy V1 managed-launch descriptor entry is retired: only a V2 settings
        // connection with a published launch snapshot serves launches now.
        assert_eq!(
            adapter
                .managed_launch_descriptor(&AgentLaunchDescriptorRequestV1 {
                    connection_id: connection_id.clone(),
                })
                .unwrap_err(),
            ControlReadError::NotFound
        );
        let material = adapter.resolve_active_agent_grant(&connection_id).unwrap();
        let (grant_generation, publication_digest) = {
            let stores = adapter.stores.lock().unwrap();
            (
                stores
                    .secrets()
                    .inspect_agent_access_grant(WorkspaceId::DEFAULT, &connection_id)
                    .unwrap()
                    .unwrap()
                    .generation(),
                stores
                    .control()
                    .active_publication(&workspace)
                    .unwrap()
                    .unwrap()
                    .digest,
            )
        };
        let installed = target.pin_request().unwrap();
        let published = installed
            .grants
            .iter()
            .find(|grant| grant.generation == grant_generation)
            .unwrap();
        assert_eq!(published.bearer_token_sha256, material.sha256());
        assert_eq!(installed.digest().unwrap(), publication_digest);

        // Restore carries the exact revoke and publication transition through the composed
        // Gateway target. It must roll back to the old complete state instead of claiming
        // success while the executable snapshot remains installed.
        let restore_point = AgentConnectionRestorePointV1::from_preview(
            "restore/claude/active-managed-join",
            "2.1.231",
            &preview,
            AgentConfigRestorePointV1::from_change(
                preview.connection.profile_id.clone(),
                preview.config_change.clone(),
            )
            .unwrap(),
        )
        .unwrap();
        let (restore_revisions, restore_publication, restore_generation) = {
            let stores = adapter.stores.lock().unwrap();
            (
                stores.control().current_revisions(&workspace).unwrap(),
                stores
                    .control()
                    .active_publication(&workspace)
                    .unwrap()
                    .unwrap(),
                stores
                    .secrets()
                    .inspect_agent_access_grant(WorkspaceId::DEFAULT, &connection_id)
                    .unwrap()
                    .unwrap()
                    .generation(),
            )
        };
        let discovery = adapter
            .exact_discovery(&spec.agent_id, &spec.profile_id, &spec.installed_version)
            .unwrap();
        let AgentDiscoveryOutcomeV1::Supported { installation } = discovery.outcome else {
            panic!("exact managed Claude installation must remain supported");
        };
        let restore = preview_agent_connection_restore(
            AgentRestoreSpecV1 {
                schema_version: AGENT_CONNECT_SPEC_SCHEMA_V1,
                agent_id: spec.agent_id.clone(),
                profile_id: spec.profile_id.clone(),
                installed_version: spec.installed_version.clone(),
                restore_point_ref: restore_point.restore_point_ref.clone(),
            },
            restore_point,
            AgentConnectionRestorePlanningFactsV1 {
                installation: *installation,
                current_revisions: restore_revisions.clone(),
                current_fingerprints: adapter
                    .before_fingerprints(&subject, restore_publication.digest.clone())
                    .unwrap(),
                expected_grant_generation: restore_generation,
            },
        )
        .unwrap();
        let AgentConnectionRestorePreviewOutcomeV1::Ready(restore) = restore else {
            panic!("exact restore point must be ready");
        };
        let restore_plan = seal_agent_connection_restore(
            &restore,
            &restore.change_digest,
            &restore.expected_revisions,
            &restore.expected_revisions,
        )
        .unwrap();
        let restore_scope = IdempotencyScopeV1::new(
            "interactive-user",
            "ApplyAgentConnectionRestore",
            "restore-active-managed-join",
        )
        .unwrap();
        let restore_request_digest = CanonicalDigest::of_bytes(b"restore-active-managed-request");
        let restore_operation = OperationV1::new(
            OperationId::derive(&workspace, &restore_scope, &restore_request_digest),
            workspace.clone(),
            restore_scope,
            restore_request_digest,
            restore.change_digest.clone(),
            restore.expected_revisions.clone(),
            restore_plan,
        )
        .unwrap();
        let restore_capability = "agent-restore-capability-0123456789abcdef";
        {
            let stores = adapter.stores.lock().unwrap();
            stores
                .apply_capability_registrar()
                .register(
                    ApplyCapabilityRegistrationV1::from_protected_launcher(
                        restore_capability.to_owned(),
                        restore_operation.idempotency.principal.clone(),
                        workspace.clone(),
                        restore_operation.idempotency.operation_kind.clone(),
                        restore_operation.accepted_digest.clone(),
                        restore_operation.expected_revisions.clone(),
                        expires,
                    )
                    .unwrap(),
                )
                .unwrap();
            let authorization = stores
                .control()
                .verify_apply_authorization(
                    &ProtectedApplyCapability::new(restore_capability.to_owned()).unwrap(),
                    &workspace,
                    &restore_operation.idempotency.principal,
                    &restore_operation.idempotency.operation_kind,
                    &restore_operation.accepted_digest,
                    &restore_operation.expected_revisions,
                )
                .unwrap();
            assert_eq!(
                stores
                    .control()
                    .begin_operation(&restore_operation, &authorization)
                    .unwrap(),
                BeginOperationOutcome::Created
            );
        }
        let expected_red = TransactionCoordinator::new(
            &adapter,
            &adapter,
            &adapter,
            &adapter,
            &adapter,
            &adapter.admission,
        )
        .run(&restore_operation.operation_id)
        .unwrap();
        assert_eq!(
            expected_red.state,
            hiroute_domain::OperationState::RolledBack,
            "{expected_red:#?}"
        );
        assert_eq!(
            expected_red.safe_error_code.as_deref(),
            Some("ADAPTER_FAILURE")
        );
        let still_active = adapter.resolve_active_agent_grant(&connection_id).unwrap();
        assert_eq!(still_active.sha256(), material.sha256());
        assert_eq!(
            adapter
                .stores
                .lock()
                .unwrap()
                .control()
                .active_publication(&workspace)
                .unwrap()
                .unwrap()
                .digest,
            publication_digest
        );

        write_claude(&claude, "2.1.230");
        let changed = adapter.planning_facts(&spec).unwrap();
        assert_eq!(changed.facts.installation.version, "2.1.230");
        assert!(
            changed
                .facts
                .installation
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_err()
        );
        // Replacing the executable invalidates its identity proof, independently of the
        // diagnostic version printed by that executable.
        assert!(adapter.resolve_active_agent_grant(&connection_id).is_err());
    }
}
