//! Model-settings status from the original Operation, current native file/grant and Gateway.
use super::settings_facts::SettingsAgentClass;
use super::*;
use hiroute_application::agent_connection::{
    ClaudeModelFileAction, CodexModelFileAction, codex_model_restore_point_ref,
    settings_claude_model_file_for_operation, settings_codex_model_file_for_operation,
};
use hiroute_application_api::{
    AgentModelCheckStateV2, AgentModelSettingsStateV2 as State, AgentModelSettingsStatusV2,
    AgentModelSurfaceResultV2, AgentSettingsStatusRequestV2,
};
use hiroute_diagnostics::agent_read::{
    AgentReadCategory as Category, AgentReadStage as Stage, Timer,
};
use hiroute_domain::{
    AgentAccessGrantRefV1, OperationState, SecretStorePort, is_agent_access_grant_effect,
};
use hiroute_local_storage::{
    ActivePublicationIdentity, AgentLocatorScope, AgentLocatorWitness, AgentNativeTargetWitness,
    AgentReadStamp, CapturedAgentLocator,
};

#[cfg(test)]
thread_local! {
    pub(super) static MODEL_VERIFY_BARRIER: std::cell::RefCell<Option<(
        std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>,
    )>> = const { std::cell::RefCell::new(None) };
    pub(super) static MODEL_NATIVE_READ_BARRIER: std::cell::RefCell<Option<(
        std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>,
    )>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn wait_before_model_verify() {
    MODEL_VERIFY_BARRIER.with(|slot| {
        if let Some((reached, release)) = slot.borrow_mut().take() {
            reached.send(()).unwrap();
            release.recv().unwrap();
        }
    });
}

#[cfg(test)]
fn wait_before_model_native_read() {
    MODEL_NATIVE_READ_BARRIER.with(|slot| {
        if let Some((reached, release)) = slot.borrow_mut().take() {
            reached.send(()).unwrap();
            release.recv().unwrap();
        }
    });
}

enum ModelAction {
    Configure,
    Restore,
}

/// The configured Claude/Codex settings join: the applied managed-configuration intent, the
/// active grant reference, and the installed publication the configuration was sealed against.
pub(super) struct ConfiguredModelSettings {
    pub(super) operation_id: hiroute_domain::OperationId,
    pub(super) intent: hiroute_domain::ExternalEffectIntentV1,
    pub(super) grant: AgentAccessGrantRefV1,
    pub(super) publication_digest: CanonicalDigest,
}

impl LocalControlAdapter {
    pub(super) fn agent_settings_status(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<hiroute_application_api::AgentSettingsStatusV2, ControlReadError> {
        self.model_settings_status(request)
            .map(Box::new)
            .map(hiroute_application_api::AgentSettingsStatusV2::Model)
    }

    pub(super) fn validate_model_check_target(
        &self,
        request: &hiroute_application_api::AgentCheckRequestV1,
    ) -> Result<(), ControlReadError> {
        let target = request.target.as_ref().ok_or(ControlReadError::Denied)?;
        if request.scope != hiroute_application_api::AgentCheckScopeV1::Live
            || !request.allow_model_call
            || !request.valid_target()
            || self
                .settings_agent_for_context(&target.context_id)
                .is_none_or(|class| class.agent_id() != request.agent_id)
        {
            return Err(ControlReadError::Denied);
        }
        let (status, grant, _) = self.evaluate_model_settings(&AgentSettingsStatusRequestV2 {
            projection: Default::default(),
            effective: None,
            include_skills: false,
            native_target: None,
            schema_version: hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2,
            context_id: target.context_id.clone(),
        })?;
        if status.state != State::Configured
            || status.applied_revision != Some(target.expected_applied_revision)
        {
            return Err(ControlReadError::SnapshotChanged);
        }
        if grant.is_none()
            || !status
                .live_check_targets
                .iter()
                .any(|exact| exact == target)
        {
            return Err(ControlReadError::Denied);
        }
        Ok(())
    }

    pub(super) fn model_settings_status(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<AgentModelSettingsStatusV2, ControlReadError> {
        if request.projection == hiroute_application_api::AgentSettingsStatusProjectionV2::Model
            && (request.include_skills || request.effective.is_some())
        {
            return Err(ControlReadError::Denied);
        }
        let (mut status, _, join) = self.evaluate_model_settings(request)?;
        if request.projection == hiroute_application_api::AgentSettingsStatusProjectionV2::Model {
            return Ok(status);
        }
        let collaboration_request = if self.settings_agent_for_context(&request.context_id)
            == Some(SettingsAgentClass::CodexProfile)
        {
            AgentSettingsStatusRequestV2 {
                projection: Default::default(),
                effective: None,
                include_skills: false,
                native_target: None,
                schema_version: request.schema_version,
                context_id: self.settings_context(SettingsAgentClass::Codex),
            }
        } else {
            request.clone()
        };
        let desktop = self.settings_agent_for_context(&request.context_id)
            == Some(SettingsAgentClass::DshDesktop);
        let hermes = self.settings_agent_for_context(&request.context_id)
            == Some(SettingsAgentClass::Hermes);
        if !desktop && (hermes || request.include_skills) {
            status.management_skill =
                Some(self.management_settings_status(&collaboration_request)?);
        }
        if !desktop && (!hermes || request.include_skills) {
            status.collaboration =
                Some(self.collaboration_settings_status(&collaboration_request)?);
        }
        if let Some(effective) = &request.effective {
            status.effective =
                Some(self.agent_effective_state(&status, join.as_ref(), effective)?);
        }
        Ok(status)
    }

    /// The configured settings join for one context, when the owned file, SecretStore grant,
    /// and installed Gateway publication all still agree with the succeeded configure.
    pub(super) fn configured_model_settings_join(
        &self,
        context_id: &str,
    ) -> Result<Option<ConfiguredModelSettings>, ControlReadError> {
        let (_, _, join) = self.evaluate_model_settings(&AgentSettingsStatusRequestV2 {
            projection: Default::default(),
            effective: None,
            include_skills: false,
            native_target: None,
            schema_version: hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2,
            context_id: context_id.to_owned(),
        })?;
        Ok(join)
    }

    /// Resolve the exact model grant only when the same owned file, SecretStore reference and
    /// installed Gateway publication that make V2 status `configured` still agree. This is the
    /// V2 counterpart of the legacy AgentConnection aggregate join; bearer bytes remain inside
    /// the protected daemon resolver.
    pub(super) fn active_model_settings_grant(
        &self,
        connection_id: &str,
    ) -> Result<AgentAccessGrantRefV1, ControlReadError> {
        let context_id = connection_id
            .strip_prefix("agent-connection/")
            .filter(|context| format!("agent-connection/{context}") == connection_id)
            .ok_or(ControlReadError::NotFound)?;
        let request = AgentSettingsStatusRequestV2 {
            projection: Default::default(),
            effective: None,
            include_skills: false,
            native_target: None,
            schema_version: hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2,
            context_id: context_id.to_owned(),
        };
        let (status, grant, _) = self.evaluate_model_settings(&request)?;
        if status.state != State::Configured {
            return Err(ControlReadError::Denied);
        }
        grant.ok_or(ControlReadError::Corrupt)
    }

    fn evaluate_model_settings(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<
        (
            AgentModelSettingsStatusV2,
            Option<AgentAccessGrantRefV1>,
            Option<ConfiguredModelSettings>,
        ),
        ControlReadError,
    > {
        let diagnostics = self.agent_read_diagnostics();
        let _total = Timer::start(&diagnostics, Category::Model, Stage::Total);
        match self.evaluate_model_settings_once(request) {
            Err(ControlReadError::SnapshotChanged) => self.evaluate_model_settings_once(request),
            result => result,
        }
    }

    fn fence_model_read(
        &self,
        request: &AgentSettingsStatusRequestV2,
        stamp: &AgentReadStamp,
        witness: &AgentLocatorWitness,
        native_target: Option<&AgentNativeTargetWitness>,
        dependencies: Option<(
            &Option<AgentAccessGrantRefV1>,
            &Option<ActivePublicationIdentity>,
            &[hiroute_domain::AgentSurfaceCheckRecordV1],
        )>,
    ) -> Result<(), ControlReadError> {
        let diagnostics = self.agent_read_diagnostics();
        let waiting = Timer::start(&diagnostics, Category::Model, Stage::LockWait);
        let stores = self.stores_lock().map_err(super::map_port)?;
        drop(waiting);
        let _held = Timer::start(&diagnostics, Category::Model, Stage::LockHeld);
        let _fence = Timer::start(&diagnostics, Category::Model, Stage::Fence);
        stores
            .fence_agent_read(stamp, |control, secrets| {
                let changed = || PortError::new(PortErrorCode::Conflict, "agent.model.read-fence");
                if !witness.matches_current(control)? {
                    return Err(changed());
                }
                if let Some(target) = native_target
                    && !target.matches_current(control)?
                {
                    return Err(changed());
                }
                if let Some((grant, publication, checks)) = dependencies
                    && (control.active_publication_identity(&WorkspaceId::default())?
                        != *publication
                        || control
                            .agent_surface_checks(&WorkspaceId::default(), &request.context_id)?
                            != checks
                        || !secrets.ok_or_else(changed)?.current_agent_grant_matches(
                            WorkspaceId::DEFAULT,
                            &format!("agent-connection/{}", request.context_id),
                            grant.as_ref(),
                        )?)
                {
                    return Err(changed());
                }
                Ok(())
            })
            .map_err(super::map_port)
    }

    fn evaluate_model_settings_once(
        &self,
        request: &AgentSettingsStatusRequestV2,
    ) -> Result<
        (
            AgentModelSettingsStatusV2,
            Option<AgentAccessGrantRefV1>,
            Option<ConfiguredModelSettings>,
        ),
        ControlReadError,
    > {
        let diagnostics = self.agent_read_diagnostics();
        let class = self
            .settings_agent_for_context(&request.context_id)
            .ok_or(ControlReadError::NotFound)?;
        let requested_target =
            if class == SettingsAgentClass::Hermes && request.native_target.is_some() {
                Some(self.hermes_target_for_context(
                    &request.context_id,
                    request.native_target.as_ref(),
                )?)
            } else if class != SettingsAgentClass::Hermes && request.native_target.is_some() {
                return Err(ControlReadError::Denied);
            } else {
                None
            };
        let mut status = AgentModelSettingsStatusV2 {
            effective: None,
            native_target: None,
            native_adapter: None,
            schema: "hiroute.agent-model-settings-status/v2".into(),
            context_id: request.context_id.clone(),
            state: State::NotConfigured,
            operation_id: None,
            operation_state: None,
            restore_point_ref: None,
            applied_revision: None,
            surface_results: Vec::new(),
            live_check_targets: Vec::new(),
            model_verified: false,
            current_selection: None,
            protected_native_model_ids: Vec::new(),
            collaboration: None,
            management_skill: None,
        };
        let scope = AgentLocatorScope::ContextFacet {
            context_id: &request.context_id,
            facet: hiroute_domain::AgentSettingsFacet::Model,
        };
        let (capture, stamp, current, publication, checks, native_target) = {
            let waiting = Timer::start(&diagnostics, Category::Model, Stage::LockWait);
            let stores = self.stores_lock().map_err(super::map_port)?;
            drop(waiting);
            let _held = Timer::start(&diagnostics, Category::Model, Stage::LockHeld);
            let _copy = Timer::start(&diagnostics, Category::Model, Stage::SnapshotCopy);
            let include_secrets = stores
                .control()
                .may_need_agent_model_grant(&WorkspaceId::default(), &request.context_id)
                .map_err(super::map_port)?;
            let ((capture, current, publication, checks, native_target), stamp) = stores
                .capture_agent_read(include_secrets, |control, secrets| {
                    let mut locator = Timer::start(&diagnostics, Category::Model, Stage::Locator);
                    let capture = control.capture_agent_locator(&WorkspaceId::default(), scope)?;
                    super::agent_read_diagnostics::count_locator(&mut locator, &capture);
                    drop(locator);
                    let native_target = if class == SettingsAgentClass::Hermes {
                        Some(control.capture_agent_native_target(
                            &WorkspaceId::default(),
                            &request.context_id,
                        )?)
                    } else {
                        None
                    };
                    let found = matches!(capture, CapturedAgentLocator::Found(_));
                    if found != include_secrets {
                        return Err(PortError::new(
                            PortErrorCode::Conflict,
                            "agent.model.capture-changed",
                        ));
                    }
                    if !found {
                        return Ok((capture, None, None, Vec::new(), native_target));
                    }
                    let mut authentication =
                        Timer::start(&diagnostics, Category::Model, Stage::GrantAuthentication);
                    authentication.counts.grant_authentications = 1;
                    let current = secrets
                        .ok_or_else(|| {
                            PortError::new(PortErrorCode::Conflict, "agent.model.secret-snapshot")
                        })?
                        .inspect_agent_access_grant(
                            WorkspaceId::DEFAULT,
                            &format!("agent-connection/{}", request.context_id),
                        )?;
                    drop(authentication);
                    let publication = control.capture_active_publication(&WorkspaceId::default())?;
                    let checks = control
                        .agent_surface_checks(&WorkspaceId::default(), &request.context_id)?;
                    Ok((capture, current, publication, checks, native_target))
                })
                .map_err(super::map_port)?;
            (capture, stamp, current, publication, checks, native_target)
        };
        let verify = Timer::start(&diagnostics, Category::Model, Stage::Verify);
        let witness = capture.witness().map_err(super::map_port)?;
        let native_witness = native_target.as_ref().map(|target| target.witness());
        status.native_target = match native_target {
            Some(target) => target
                .verify()
                .map_err(super::map_port)?
                .or(requested_target),
            None => requested_target,
        };
        let operation = match capture {
            CapturedAgentLocator::Pending(pending, _) => {
                let pending = pending.verify().map_err(super::map_port)?;
                drop(verify);
                status.state = if pending.state == OperationState::NeedsAttention {
                    State::NeedsAttention
                } else {
                    State::Pending
                };
                status.operation_id = Some(pending.operation_id.to_string());
                status.operation_state = Some(pending.state.as_str().into());
                self.fence_model_read(request, &stamp, &witness, native_witness.as_ref(), None)?;
                return Ok((status, None, None));
            }
            CapturedAgentLocator::AbsentWithCompleteCoverage(_) => {
                drop(verify);
                self.fence_model_read(request, &stamp, &witness, native_witness.as_ref(), None)?;
                return Ok((status, None, None));
            }
            CapturedAgentLocator::Found(capture) => {
                #[cfg(test)]
                wait_before_model_verify();
                capture.verify().map_err(super::map_port)?
            }
        };
        let publication_identity = publication.as_ref().map(|record| record.identity());
        let publication = publication
            .map(|record| record.verify())
            .transpose()
            .map_err(super::map_port)?;
        let intent = model_intent(class, operation.as_ref()).ok_or(ControlReadError::Corrupt)?;
        let action = model_action(class, operation.as_ref(), intent).map_err(super::map_port)?;
        drop(verify);
        status.operation_id = Some(operation.operation_id.to_string());
        status.operation_state = Some(operation.state.as_str().into());
        let native = Timer::start(&diagnostics, Category::Model, Stage::NativeRead);
        let (active_grant, configured_join) = match action {
            ModelAction::Configure => {
                #[cfg(test)]
                wait_before_model_native_read();
                let file_applied = if matches!(
                    class,
                    SettingsAgentClass::Qoder
                        | SettingsAgentClass::Pi
                        | SettingsAgentClass::Dsh
                        | SettingsAgentClass::DshDesktop
                        | SettingsAgentClass::OpenCode
                ) {
                    let model_applied =
                        hiroute_integrations::additional_native_configuration_is_applied(
                            &self.artifacts,
                            &operation.operation_id,
                            intent,
                        )
                        .map_err(super::map_port)?;
                    // Published operations predating routing adapters retain their
                    // original contract. New operations also own the adapter bytes.
                    let mut adapter_applied = true;
                    for plugin in operation
                        .plan
                        .external()
                        .iter()
                        .filter(|i| i.effect_id() == "agent-connection-native-routing-adapter")
                    {
                        hiroute_application::agent_connection::settings_native_plugin_file_for_operation(
                            operation.as_ref(), plugin,
                        ).map_err(super::map_port)?;
                        adapter_applied &= hiroute_integrations::native_plugin_is_applied(
                            &self.artifacts,
                            &operation.operation_id,
                            plugin,
                        )
                        .map_err(super::map_port)?;
                    }
                    model_applied && adapter_applied
                } else if class == SettingsAgentClass::Hermes {
                    self.hermes_operation_files_applied(operation.as_ref(), intent)
                        .map_err(super::map_port)?
                } else if class == SettingsAgentClass::Claude {
                    let policy =
                        hiroute_application::agent_connection::decode_settings_claude_model_file(
                            intent,
                        )
                        .map_err(super::map_port)?;
                    let window_owned = matches!(policy.change, hiroute_application::agent_connection::ClaudeModelFileAction::Configure { snapshot, .. } if snapshot.context_window_tokens.is_some());
                    let context_conflict = window_owned
                        && self
                            .scanner
                            .claude_context_override()
                            .map_err(|_| ControlReadError::Corrupt)?;
                    !context_conflict
                        && hiroute_integrations::claude_native_configuration_is_applied(
                            &self.artifacts,
                            &operation.operation_id,
                            intent,
                        )
                        .map_err(super::map_port)?
                        && !self
                            .scanner
                            .claude_native_routing_conflict()
                            .map_err(|_| ControlReadError::Corrupt)?
                } else {
                    (class != SettingsAgentClass::CodexProfile
                        || hiroute_integrations::codex_profile_dependency_digest(
                            &self.scanner.codex_user_config_target(),
                            true,
                        )
                        .is_ok())
                        && hiroute_integrations::codex_native_configuration_is_applied(
                            &self.artifacts,
                            &operation.operation_id,
                            intent,
                        )
                        .map_err(super::map_port)?
                };
                let file_applied = file_applied
                    && (class != SettingsAgentClass::OpenCode || {
                        use hiroute_application::agent_connection::{
                            AdditionalModelFileAction, decode_settings_additional_model_file,
                        };
                        let payload = decode_settings_additional_model_file(intent)
                            .map_err(super::map_port)?;
                        let AdditionalModelFileAction::Configure {
                            provider_id,
                            models,
                            ..
                        } = payload.change
                        else {
                            return Err(ControlReadError::Corrupt);
                        };
                        // Recheck effective ownership/allowlists, without treating unrelated
                        // document changes as drift or rejecting a retained default selector.
                        self.scanner
                            .opencode_model_facts(&provider_id, &models)
                            .is_ok()
                    });
                let [mutation] = operation.plan.agent_access_grants() else {
                    return Err(ControlReadError::Corrupt);
                };
                let effect = operation
                    .grant_effects()
                    .iter()
                    .find(|effect| is_agent_access_grant_effect(effect))
                    .ok_or(ControlReadError::Corrupt)?;
                let expected = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                    .map_err(|_| ControlReadError::Corrupt)?;
                let published = match &publication {
                    Some(record) => {
                        self.publication_is_installed(record)
                            .map_err(super::map_port)?
                            && record
                                .verify()
                                .map_err(|_| ControlReadError::Corrupt)?
                                .grants
                                .iter()
                                .any(|grant| {
                                    grant.grant_id == expected.grant_id()
                                        && grant.generation == expected.generation()
                                        && grant.bearer_token_sha256 == *expected.material_sha256()
                                        && &grant.model_grant == expected.scope().model_grant()
                                })
                    }
                    None => false,
                };
                let endpoint_current = {
                    let runtime = self
                        .managed_agent_runtime
                        .lock()
                        .map_err(|_| ControlReadError::Unavailable)?;
                    let runtime = runtime.as_ref().ok_or(ControlReadError::Unavailable)?;
                    model_endpoint_matches_runtime(
                        class,
                        operation.as_ref(),
                        intent,
                        &runtime.gateway_base_url,
                    )
                    .map_err(super::map_port)?
                };
                if class == SettingsAgentClass::Hermes {
                    let resources_installed = self
                        .hermes_operation_plugin_files_applied(operation.as_ref())
                        .map_err(super::map_port)?;
                    status.native_adapter = Some(
                        hiroute_application_api::AgentNativeAdapterStatusV1 {
                            resources_installed,
                            known_channel_models: status.native_target.as_ref().and_then(
                                |target| {
                                    hiroute_integrations::hermes_known_channel_models(target).ok()
                                },
                            ),
                            runtime_loading:
                                hiroute_application_api::AgentNativeRuntimeLoadingV1::Unobserved,
                            next_action: if resources_installed {
                                hiroute_application_api::AgentNativeAdapterNextActionV1::ReloadPluginsIfRunning
                            } else {
                                hiroute_application_api::AgentNativeAdapterNextActionV1::RepairOwnedResources
                            },
                        },
                    );
                }
                let configured = file_applied
                    && endpoint_current
                    && current.as_ref() == Some(&expected)
                    && published;
                status.state = if configured {
                    State::Configured
                } else {
                    State::Drift
                };
                status.restore_point_ref =
                    Some(codex_model_restore_point_ref(&operation.operation_id));
                if configured {
                    let spec: hiroute_domain::AgentSettingsSpecV2 =
                        serde_json::from_value(operation.plan.spec().desired_state.clone())
                            .map_err(|_| ControlReadError::Corrupt)?;
                    let hiroute_domain::AgentFacetIntent::Configure { settings } = spec.model
                    else {
                        return Err(ControlReadError::Corrupt);
                    };
                    status.protected_native_model_ids = spec.protected_native_model_ids;
                    let surfaces = match (&settings, class) {
                        (
                            hiroute_domain::AgentModelSelectionV2::CodexDefault { .. },
                            SettingsAgentClass::Codex,
                        ) => self.scanner.available_model_surfaces(class.agent_id()),
                        (
                            hiroute_domain::AgentModelSelectionV2::CodexDefault { .. },
                            SettingsAgentClass::CodexProfile,
                        ) => [hiroute_domain::AgentModelSurfaceV2::CodexCli].into(),
                        (
                            hiroute_domain::AgentModelSelectionV2::ClaudeLauncher {
                                surfaces, ..
                            },
                            SettingsAgentClass::Claude,
                        ) => surfaces.clone(),
                        (
                            hiroute_domain::AgentModelSelectionV2::QoderAdditional { .. },
                            SettingsAgentClass::Qoder,
                        ) => [hiroute_domain::AgentModelSurfaceV2::QoderCli].into(),
                        (
                            hiroute_domain::AgentModelSelectionV2::PiAdditional { .. },
                            SettingsAgentClass::Pi,
                        ) => [hiroute_domain::AgentModelSurfaceV2::PiCli].into(),
                        (
                            hiroute_domain::AgentModelSelectionV2::OpenCodeAdditional { .. },
                            SettingsAgentClass::OpenCode,
                        ) => [hiroute_domain::AgentModelSurfaceV2::OpenCodeCli].into(),
                        (
                            hiroute_domain::AgentModelSelectionV2::DshAdditional { .. },
                            SettingsAgentClass::Dsh,
                        ) => [hiroute_domain::AgentModelSurfaceV2::DshCli].into(),
                        (
                            hiroute_domain::AgentModelSelectionV2::DshAdditional { .. },
                            SettingsAgentClass::DshDesktop,
                        ) => [hiroute_domain::AgentModelSurfaceV2::DshDesktop].into(),
                        (
                            hiroute_domain::AgentModelSelectionV2::HermesDefault { .. },
                            SettingsAgentClass::Hermes,
                        ) => [hiroute_domain::AgentModelSurfaceV2::HermesCli].into(),
                        _ => return Err(ControlReadError::Corrupt),
                    };
                    // The applied revision is the installed publication this configuration was
                    // sealed against; surface evidence from any other revision is not shown.
                    let applied = publication
                        .as_ref()
                        .map(|record| record.publication_revision);
                    status.applied_revision = applied;
                    let applied = applied.ok_or(ControlReadError::Corrupt)?;
                    status.surface_results = surfaces
                        .iter()
                        .map(|surface| {
                            let matched = checks.iter().find(|check| {
                                &check.surface == surface && check.applied_revision == applied
                            });
                            let (state, reason_code) = match matched {
                                Some(check) => (
                                    match check.state {
                                        hiroute_domain::AgentSurfaceCheckStateV1::Passed => {
                                            AgentModelCheckStateV2::Passed
                                        }
                                        hiroute_domain::AgentSurfaceCheckStateV1::Failed => {
                                            AgentModelCheckStateV2::Failed
                                        }
                                    },
                                    check.reason_code.clone(),
                                ),
                                None => (AgentModelCheckStateV2::NotVerified, None),
                            };
                            AgentModelSurfaceResultV2 {
                                surface: *surface,
                                applied_revision: applied,
                                state,
                                reason_code,
                            }
                        })
                        .collect();
                    let client_model_ids = expected
                        .scope()
                        .model_grant()
                        .routes
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>();
                    status.live_check_targets = surfaces
                        .iter()
                        .map(|surface| hiroute_application_api::AgentModelCheckTargetV2 {
                            context_id: request.context_id.clone(),
                            surface: *surface,
                            expected_applied_revision: applied,
                            client_model_ids: client_model_ids.clone(),
                        })
                        .collect();
                    status.current_selection = Some(settings);
                    status.model_verified = status.derive_model_verified(Some(applied));
                }
                let join = configured.then(|| {
                    let publication_digest = publication
                        .as_ref()
                        .map(|record| record.digest.clone())
                        .ok_or(ControlReadError::Corrupt)?;
                    Ok::<_, ControlReadError>(ConfiguredModelSettings {
                        operation_id: operation.operation_id.clone(),
                        intent: intent.clone(),
                        grant: expected.clone(),
                        publication_digest,
                    })
                });
                (configured.then_some(expected), join.transpose()?)
            }
            ModelAction::Restore => {
                let publication_intent = operation
                    .plan
                    .external()
                    .iter()
                    .find(|intent| intent.kind() == hiroute_domain::OwnedEffectKind::Publication)
                    .ok_or(ControlReadError::Corrupt)?;
                let reference: AgentAccessGrantRefV1 = serde_json::from_value(
                    publication_intent.desired()["payload"]["restore_grant"].clone(),
                )
                .map_err(|_| ControlReadError::Corrupt)?;
                reference
                    .validate()
                    .map_err(|_| ControlReadError::Corrupt)?;
                let withdrawn = match &publication {
                    Some(record) => {
                        self.publication_is_installed(record)
                            .map_err(super::map_port)?
                            && !record
                                .verify()
                                .map_err(|_| ControlReadError::Corrupt)?
                                .grants
                                .iter()
                                .any(|grant| grant.grant_id == reference.grant_id())
                    }
                    None => false,
                };
                // A succeeded restore relinquishes the native file. Later edits by Codex or
                // the user are no longer HiRoute drift; only a surviving grant/publication
                // means the connection was not actually withdrawn.
                status.state = if current.is_none() && withdrawn {
                    State::NotConfigured
                } else {
                    State::Drift
                };
                (None, None)
            }
        };
        drop(native);
        self.fence_model_read(
            request,
            &stamp,
            &witness,
            native_witness.as_ref(),
            Some((&current, &publication_identity, &checks)),
        )?;
        Ok((status, active_grant, configured_join))
    }
}

fn model_intent(
    class: super::settings_facts::SettingsAgentClass,
    operation: &(impl hiroute_domain::AgentOperationRead + ?Sized),
) -> Option<&hiroute_domain::ExternalEffectIntentV1> {
    operation
        .agent_input()
        .external
        .iter()
        .find(|intent| match class {
            super::settings_facts::SettingsAgentClass::Codex
            | super::settings_facts::SettingsAgentClass::CodexProfile => {
                super::native_model::is_settings_codex_model(intent)
            }
            super::settings_facts::SettingsAgentClass::Hermes => {
                hiroute_application::agent_connection::is_settings_hermes_model(intent)
            }
            super::settings_facts::SettingsAgentClass::Claude => {
                super::native_claude_model::is_settings_claude_model(intent)
            }
            super::settings_facts::SettingsAgentClass::Qoder
            | super::settings_facts::SettingsAgentClass::Pi
            | super::settings_facts::SettingsAgentClass::Dsh
            | super::settings_facts::SettingsAgentClass::DshDesktop
            | super::settings_facts::SettingsAgentClass::OpenCode => {
                super::native_additional_model::is_settings_additional_model(intent)
            }
        })
}

fn model_action(
    class: super::settings_facts::SettingsAgentClass,
    operation: &(impl hiroute_domain::AgentOperationRead + ?Sized),
    intent: &hiroute_domain::ExternalEffectIntentV1,
) -> hiroute_domain::PortResult<ModelAction> {
    match class {
        super::settings_facts::SettingsAgentClass::Hermes => match hiroute_application::agent_connection::settings_hermes_model_file_for_operation(operation, intent)?.change {
            hiroute_application::agent_connection::HermesModelFileAction::Configure { .. } => Ok(ModelAction::Configure),
            hiroute_application::agent_connection::HermesModelFileAction::Restore { .. } => Ok(ModelAction::Restore),
        },
        super::settings_facts::SettingsAgentClass::Qoder | super::settings_facts::SettingsAgentClass::Pi | super::settings_facts::SettingsAgentClass::Dsh | super::settings_facts::SettingsAgentClass::DshDesktop | super::settings_facts::SettingsAgentClass::OpenCode => {
            match hiroute_application::agent_connection::settings_additional_model_file_for_operation(
                operation, intent,
            )?
            .change
            {
                hiroute_application::agent_connection::AdditionalModelFileAction::Configure {
                    ..
                } => Ok(ModelAction::Configure),
                hiroute_application::agent_connection::AdditionalModelFileAction::Restore { .. } => {
                    Ok(ModelAction::Restore)
                }
            }
        }
        super::settings_facts::SettingsAgentClass::Codex
        | super::settings_facts::SettingsAgentClass::CodexProfile => {
            match settings_codex_model_file_for_operation(operation, intent)?.change {
                CodexModelFileAction::Configure { .. } => Ok(ModelAction::Configure),
                CodexModelFileAction::Restore { .. } => Ok(ModelAction::Restore),
            }
        }
        super::settings_facts::SettingsAgentClass::Claude => {
            match settings_claude_model_file_for_operation(operation, intent)?.change {
                ClaudeModelFileAction::Configure { .. } => Ok(ModelAction::Configure),
                ClaudeModelFileAction::Restore { .. } => Ok(ModelAction::Restore),
            }
        }
    }
}

// The native adapter proves the owned file still matches the sealed configuration.
// That historical endpoint must also match today's running Gateway.
fn model_endpoint_matches_runtime(
    class: SettingsAgentClass,
    operation: &(impl hiroute_domain::AgentOperationRead + ?Sized),
    intent: &hiroute_domain::ExternalEffectIntentV1,
    gateway_base_url: &str,
) -> hiroute_domain::PortResult<bool> {
    use hiroute_application::agent_connection::{
        AdditionalModelFileAction, additional_model_kind,
        settings_additional_model_file_for_operation,
    };
    match class {
        SettingsAgentClass::Hermes => Ok(
            matches!(hiroute_application::agent_connection::settings_hermes_model_file_for_operation(operation, intent)?.change, hiroute_application::agent_connection::HermesModelFileAction::Configure { endpoint, .. } if endpoint == gateway_base_url),
        ),
        SettingsAgentClass::Codex | SettingsAgentClass::CodexProfile => Ok(matches!(
            settings_codex_model_file_for_operation(operation, intent)?.change,
            CodexModelFileAction::Configure { endpoint, .. } if endpoint == gateway_base_url
        )),
        SettingsAgentClass::Claude => Ok(matches!(
            settings_claude_model_file_for_operation(operation, intent)?.change,
            ClaudeModelFileAction::Configure { gateway_base_url: endpoint, .. }
                if endpoint == gateway_base_url
        )),
        SettingsAgentClass::Qoder
        | SettingsAgentClass::Pi
        | SettingsAgentClass::Dsh
        | SettingsAgentClass::DshDesktop
        | SettingsAgentClass::OpenCode => {
            let endpoint = super::native_additional_model::additional_model_endpoint(
                additional_model_kind(intent)?,
                gateway_base_url,
            )?;
            Ok(matches!(
                settings_additional_model_file_for_operation(operation, intent)?.change,
                AdditionalModelFileAction::Configure { endpoint: configured, .. }
                    if configured == endpoint
            ))
        }
    }
}
