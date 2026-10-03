//! One CODEX_HOME slot, two explicit targets. Reuse existing durable Operations and grants.
use super::settings_facts::SettingsAgentClass;
use super::*;
use hiroute_application::control::CodexAccessViewV1;
use hiroute_domain::{
    AgentFacetIntent, AgentSettingsSpecV2, ControlRepositoryPort, SecretStorePort,
};

impl LocalControlAdapter {
    pub(super) fn guard_codex_pending_change(
        &self,
        same_operation: Option<&hiroute_domain::OperationId>,
    ) -> hiroute_domain::PortResult<()> {
        let stores = self.stores_lock()?;
        for op in stores.control().recoverable_operations()? {
            if same_operation == Some(&op.operation_id) {
                continue;
            }
            let has_receipt = op
                .step(hiroute_domain::OperationStepKind::Activate)
                .terminal_result
                .as_deref()
                .and_then(hiroute_domain::SettingsServiceCompletionV1::parse)
                .is_some();
            if has_receipt
                && op
                    .plan
                    .external()
                    .iter()
                    .any(super::native_model::is_settings_codex_model)
            {
                return Err(hiroute_domain::PortError::new(
                    hiroute_domain::PortErrorCode::Conflict,
                    "codex.pending_file_tail.change",
                ));
            }
        }
        Ok(())
    }

    fn codex_slot_state(
        &self,
    ) -> Result<
        (
            Option<SettingsAgentClass>,
            Option<hiroute_domain::OperationV1>,
        ),
        ControlReadError,
    > {
        let stores = self.stores_lock().map_err(super::map_port)?;
        let root = self.settings_context(SettingsAgentClass::Codex);
        let profile = self.settings_context(SettingsAgentClass::CodexProfile);
        let classify = |context: &str| {
            if context == root {
                Some(SettingsAgentClass::Codex)
            } else if context == profile {
                Some(SettingsAgentClass::CodexProfile)
            } else {
                None
            }
        };
        // A parked file tail releases the global writer but still owns this home slot.
        for op in stores
            .control()
            .recoverable_operations()
            .map_err(super::map_port)?
        {
            if op.plan.spec().command_id == "agents.settings.apply"
                && let Some(context) = op.plan.spec().resource_id.as_deref()
                && let Some(class) = classify(context)
            {
                let spec: AgentSettingsSpecV2 =
                    serde_json::from_value(op.plan.spec().desired_state.clone())
                        .map_err(|_| ControlReadError::Corrupt)?;
                if !matches!(spec.model, AgentFacetIntent::Keep) {
                    return Ok((Some(class), Some(op)));
                }
            }
        }
        let mut active = None;
        for (context, class) in [
            (&root, SettingsAgentClass::Codex),
            (&profile, SettingsAgentClass::CodexProfile),
        ] {
            if stores
                .secrets()
                .inspect_agent_access_grant(
                    WorkspaceId::DEFAULT,
                    &format!("agent-connection/{context}"),
                )
                .map_err(super::map_port)?
                .is_some()
            {
                if active.is_some() {
                    return Err(ControlReadError::Corrupt);
                }
                active = Some(class);
            }
        }
        Ok((active, None))
    }

    pub(super) fn check_codex_slot(
        &self,
        spec: &AgentSettingsSpecV2,
        class: SettingsAgentClass,
    ) -> Result<(), ControlReadError> {
        if !class.is_codex() {
            return Ok(());
        }
        if class == SettingsAgentClass::CodexProfile
            && (!matches!(spec.collaboration, AgentFacetIntent::Keep)
                || spec.restore_native_model.is_some())
        {
            return Err(ControlReadError::Denied);
        }
        if matches!(spec.model, AgentFacetIntent::Keep) {
            return Ok(());
        }
        if class == SettingsAgentClass::CodexProfile
            && matches!(spec.model, AgentFacetIntent::Configure { .. })
            && !self
                .scanner
                .available_model_surfaces(class.agent_id())
                .contains(&hiroute_domain::AgentModelSurfaceV2::CodexCli)
        {
            return Err(ControlReadError::Denied);
        }
        let (active, pending) = self.codex_slot_state()?;
        if pending.is_some() || active.is_some_and(|owner| owner != class) {
            return Err(ControlReadError::SnapshotChanged);
        }
        Ok(())
    }

    pub(super) fn codex_profile_dependencies(
        &self,
        class: SettingsAgentClass,
        spec: &AgentSettingsSpecV2,
        owner: Option<&hiroute_domain::OperationId>,
    ) -> Result<Option<CanonicalDigest>, ControlReadError> {
        if class != SettingsAgentClass::CodexProfile
            || !matches!(spec.model, AgentFacetIntent::Configure { .. })
        {
            return Ok(None);
        }
        hiroute_integrations::codex_profile_dependency_digest(
            &self.scanner.codex_user_config_target(),
            owner.is_some(),
        )
        .map(Some)
        .map_err(|_| ControlReadError::SnapshotChanged)
    }

    pub(super) fn codex_access_view(&self) -> Result<CodexAccessViewV1, ControlReadError> {
        let root = self.scanner.codex_user_config_target();
        let home = root.parent().ok_or(ControlReadError::Corrupt)?;
        let (owner, pending) = self.codex_slot_state()?;
        let class = owner.unwrap_or(SettingsAgentClass::CodexProfile);
        let context = self.settings_context(class);
        let grant = self
            .stores_lock()
            .map_err(super::map_port)?
            .secrets()
            .inspect_agent_access_grant(
                WorkspaceId::DEFAULT,
                &format!("agent-connection/{context}"),
            )
            .map_err(super::map_port)?;
        let conflict_fields = if let Some(op) = pending.as_ref() {
            self.codex_conflicts(op, true)?
        } else if let Some(grant) = grant.as_ref() {
            self.active_codex_conflicts(&context, grant)?
        } else {
            Vec::new()
        };
        let commands = if class == SettingsAgentClass::CodexProfile {
            hiroute_integrations::codex_profile_commands(home)
                .map_err(|_| ControlReadError::Corrupt)?
        } else {
            Default::default()
        };
        Ok(CodexAccessViewV1 {
            codex_home: home.to_string_lossy().into_owned(),
            slot_id: format!(
                "codex-home/{}",
                CanonicalDigest::of_bytes(home.as_os_str().as_encoded_bytes()).as_str()
            ),
            profile_context_id: self.settings_context(SettingsAgentClass::CodexProfile),
            root_context_id: self.settings_context(SettingsAgentClass::Codex),
            selected_mode: if class == SettingsAgentClass::CodexProfile {
                "profile"
            } else {
                "root"
            }
            .into(),
            slot_occupied: owner.is_some(),
            target_file: class
                .native_path(&self.scanner)
                .to_string_lossy()
                .into_owned(),
            profile_name: hiroute_integrations::CODEX_MANAGED_PROFILE_NAME.into(),
            commands,
            pending_operation: pending.as_ref().map(|op| op.operation_id.to_string()),
            access_revoked: grant.is_none(),
            conflict_fields,
        })
    }
}

impl LocalControlAdapter {
    fn active_codex_conflicts(
        &self,
        context: &str,
        grant: &hiroute_domain::AgentAccessGrantRefV1,
    ) -> Result<Vec<String>, ControlReadError> {
        let stores = self.stores_lock().map_err(super::map_port)?;
        for original in stores
            .control()
            .succeeded_agent_operations_for_kind(
                &WorkspaceId::default(),
                "ApplyAgentConnectionChange",
            )
            .map_err(super::map_port)?
        {
            if original.plan.spec().resource_id.as_deref() != Some(context) {
                continue;
            }
            let [mutation] = original.plan.agent_access_grants() else {
                continue;
            };
            let Some(effect) = original
                .step(hiroute_domain::OperationStepKind::ApplySecrets)
                .effects
                .iter()
                .find(|effect| hiroute_domain::is_agent_access_grant_effect(effect))
            else {
                continue;
            };
            let reference =
                hiroute_domain::AgentAccessGrantRefV1::from_ensure_effect(effect, mutation)
                    .map_err(|_| ControlReadError::Corrupt)?;
            if &reference == grant {
                let op = stores
                    .control()
                    .load_operation(&original.operation_id)
                    .map_err(super::map_port)?
                    .ok_or(ControlReadError::Corrupt)?;
                drop(stores);
                return self.codex_conflicts(&op, false);
            }
        }
        Ok(Vec::new())
    }

    fn codex_conflicts(
        &self,
        op: &hiroute_domain::OperationV1,
        pending: bool,
    ) -> Result<Vec<String>, ControlReadError> {
        use hiroute_application::agent_connection::{
            CodexModelFileAction, settings_codex_model_file_for_operation,
        };
        use hiroute_domain::NativeAgentArtifactPort;
        let intent = op
            .plan
            .external()
            .iter()
            .find(|i| super::native_model::is_settings_codex_model(i))
            .ok_or(ControlReadError::Corrupt)?;
        let payload =
            settings_codex_model_file_for_operation(op, intent).map_err(super::map_port)?;
        let restoring = matches!(&payload.change, CodexModelFileAction::Restore { .. });
        let original_id = match payload.change {
            CodexModelFileAction::Restore {
                original_operation, ..
            } => original_operation,
            CodexModelFileAction::Configure { .. } => op.operation_id.clone(),
        };
        let stores = self.stores_lock().map_err(super::map_port)?;
        let original = stores
            .control()
            .load_operation(&original_id)
            .map_err(super::map_port)?
            .ok_or(ControlReadError::Corrupt)?;
        let original_intent = original
            .plan
            .external()
            .iter()
            .find(|i| i.target() == intent.target())
            .ok_or(ControlReadError::Corrupt)?;
        // Diagnose the live path first: restore AAD validation also rejects unsafe targets.
        let current = match self.artifacts.read_native_target(intent.target()) {
            Ok(current) => current,
            Err(_) => return Ok(vec!["configuration_unreadable".into()]),
        };
        let Ok(text) = std::str::from_utf8(current.as_deref().map_or(&[], |v| v.as_slice())) else {
            return Ok(vec!["configuration_encoding".into()]);
        };
        let record = self
            .artifacts
            .load_native_restore(&original_id, original_intent)
            .map_err(super::map_port)?;
        let Some(record) = record else {
            return Ok(vec!["configuration_snapshot".into()]);
        };
        if record.len() < 3 || record[0] != 1 || record[1] > 1 {
            return Err(ControlReadError::Corrupt);
        }
        let restore = hiroute_integrations::CodexNativeRestore::decode_protected(&record[2..])
            .map_err(|_| ControlReadError::Corrupt)?;
        if restoring {
            return Ok(restore.pending_restoration_fields(text));
        }
        let mut fields = restore.conflicting_fields(text);
        if pending && fields.is_empty() {
            fields.push("configuration_snapshot".into());
        }
        Ok(fields)
    }

    pub(super) fn retry_codex_operation(
        &self,
        request: &hiroute_application_api::AgentSettingsRetryV1,
    ) -> Result<hiroute_domain::OperationV1, ControlReadError> {
        if request.schema != "hiroute.agent-settings-retry/v1"
            || self
                .settings_agent_for_context(&request.context_id)
                .is_none_or(|class| !class.is_codex())
        {
            return Err(ControlReadError::Denied);
        }
        let stores = self.stores_lock().map_err(super::map_port)?;
        let op = stores
            .control()
            .load_operation(&request.operation_id)
            .map_err(super::map_port)?
            .ok_or(ControlReadError::NotFound)?;
        if op.workspace_id != WorkspaceId::default()
            || op.plan.spec().command_id != "agents.settings.apply"
            || op.plan.spec().resource_id.as_deref() != Some(&request.context_id)
        {
            return Err(ControlReadError::Denied);
        }
        let spec: AgentSettingsSpecV2 =
            serde_json::from_value(op.plan.spec().desired_state.clone())
                .map_err(|_| ControlReadError::Corrupt)?;
        if matches!(spec.model, AgentFacetIntent::Keep) {
            return Err(ControlReadError::Denied);
        }
        drop(stores);
        hiroute_application::TransactionCoordinator::new(
            self,
            self,
            self,
            self,
            self,
            &self.admission,
        )
        .run(&request.operation_id)
        .map_err(|_| ControlReadError::Unavailable)
    }
}
