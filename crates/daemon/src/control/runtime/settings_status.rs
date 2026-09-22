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
use hiroute_domain::{
    AgentAccessGrantRefV1, OperationState, OperationStepKind, PublicationRepositoryPort,
    SecretStorePort, is_agent_access_grant_effect,
};

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
    pub(super) fn validate_model_check_target(
        &self,
        request: &hiroute_application_api::AgentCheckRequestV1,
    ) -> Result<(), ControlReadError> {
        let target = request.target.as_ref().ok_or(ControlReadError::Denied)?;
        if request.scope != hiroute_application_api::AgentCheckScopeV1::Live
            || !request.allow_model_call
            || !request.valid_target()
            || self
                .settings_context_for_agent(&request.agent_id)
                .as_deref()
                != Some(target.context_id.as_str())
        {
            return Err(ControlReadError::Denied);
        }
        let (status, grant, _) = self.evaluate_model_settings(&AgentSettingsStatusRequestV2 {
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
        let (mut status, _, _) = self.evaluate_model_settings(request)?;
        status.collaboration = Some(self.collaboration_settings_status(request)?);
        Ok(status)
    }

    /// The configured settings join for one context, when the owned file, SecretStore grant,
    /// and installed Gateway publication all still agree with the succeeded configure.
    pub(super) fn configured_model_settings_join(
        &self,
        context_id: &str,
    ) -> Result<Option<ConfiguredModelSettings>, ControlReadError> {
        let (_, _, join) = self.evaluate_model_settings(&AgentSettingsStatusRequestV2 {
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
        let class = self
            .settings_agent_for_context(&request.context_id)
            .ok_or(ControlReadError::NotFound)?;
        let mut status = AgentModelSettingsStatusV2 {
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
        };
        let stores = self.stores_lock().map_err(super::map_port)?;
        let belongs = |operation: &OperationV1| {
            operation.plan.spec().command_id == "agents.settings.apply"
                && operation.plan.spec().resource_id.as_deref() == Some(&request.context_id)
                && model_intent(class, operation).is_some()
        };
        if let Some(pending) = stores
            .control()
            .writer_claim_operation()
            .map_err(super::map_port)?
            && belongs(&pending)
        {
            status.state = if pending.state == OperationState::NeedsAttention {
                State::NeedsAttention
            } else {
                State::Pending
            };
            status.operation_id = Some(pending.operation_id.to_string());
            status.operation_state = Some(pending.state.as_str().into());
            return Ok((status, None, None));
        }
        let operations = stores
            .control()
            .succeeded_operations_for_kinds(
                &WorkspaceId::default(),
                &["ApplyAgentConnectionChange", "ApplyAgentConnectionRestore"],
            )
            .map_err(super::map_port)?;
        let Some(operation) = operations.into_iter().find(belongs) else {
            return Ok((status, None, None));
        };
        let intent = model_intent(class, &operation).ok_or(ControlReadError::Corrupt)?;
        let action = model_action(class, &operation, intent).map_err(super::map_port)?;
        let current = stores
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{}", request.context_id),
            )
            .map_err(super::map_port)?;
        status.operation_id = Some(operation.operation_id.to_string());
        status.operation_state = Some(operation.state.as_str().into());
        let (active_grant, configured_join) = match action {
            ModelAction::Configure => {
                let file_applied = if class == SettingsAgentClass::Claude {
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
                    hiroute_integrations::codex_native_configuration_is_applied(
                        &self.artifacts,
                        &operation.operation_id,
                        intent,
                    )
                    .map_err(super::map_port)?
                };
                let [mutation] = operation.plan.agent_access_grants() else {
                    return Err(ControlReadError::Corrupt);
                };
                let effect = operation
                    .step(OperationStepKind::ApplySecrets)
                    .effects
                    .iter()
                    .find(|effect| is_agent_access_grant_effect(effect))
                    .ok_or(ControlReadError::Corrupt)?;
                let expected = AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                    .map_err(|_| ControlReadError::Corrupt)?;
                let publication = stores
                    .control()
                    .active_publication(&operation.workspace_id)
                    .map_err(super::map_port)?;
                let checks = stores
                    .control()
                    .agent_surface_checks(&operation.workspace_id, &request.context_id)
                    .map_err(super::map_port)?;
                drop(stores);
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
                let configured = file_applied && current.as_ref() == Some(&expected) && published;
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
                            hiroute_domain::AgentModelSelectionV2::ClaudeLauncher {
                                surfaces, ..
                            },
                            SettingsAgentClass::Claude,
                        ) => surfaces.clone(),
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
                let publication = stores
                    .control()
                    .active_publication(&operation.workspace_id)
                    .map_err(super::map_port)?;
                drop(stores);
                let withdrawn = match publication {
                    Some(record) => {
                        self.publication_is_installed(&record)
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
        Ok((status, active_grant, configured_join))
    }
}

fn model_intent(
    class: super::settings_facts::SettingsAgentClass,
    operation: &OperationV1,
) -> Option<&hiroute_domain::ExternalEffectIntentV1> {
    operation.plan.external().iter().find(|intent| match class {
        super::settings_facts::SettingsAgentClass::Codex => {
            super::native_model::is_settings_codex_model(intent)
        }
        super::settings_facts::SettingsAgentClass::Claude => {
            super::native_claude_model::is_settings_claude_model(intent)
        }
    })
}

fn model_action(
    class: super::settings_facts::SettingsAgentClass,
    operation: &OperationV1,
    intent: &hiroute_domain::ExternalEffectIntentV1,
) -> hiroute_domain::PortResult<ModelAction> {
    match class {
        super::settings_facts::SettingsAgentClass::Codex => {
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
