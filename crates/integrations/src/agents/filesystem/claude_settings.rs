use super::*;

impl FilesystemAgentScannerV1 {
    pub fn claude_context_override(&self) -> Result<bool, AgentFilesystemScanError> {
        Ok(self.claude_observations()?.iter().any(|observation| {
            observation
                .settings
                .env
                .context_environment
                .keys()
                .any(|key| {
                    observation.layer != ConfigLayerV1::User
                        || !hiroute_domain::CLAUDE_CONTEXT_ENVIRONMENT.contains(&key.as_str())
                })
        }))
    }

    /// User-file edits cannot override another active settings source or inherited process
    /// authentication. Report that boundary before promising ordinary-CLI routing.
    pub fn claude_native_routing_conflict(&self) -> Result<bool, AgentFilesystemScanError> {
        Ok(self.claude_observations()?.iter().any(|observation| {
            if observation.layer == ConfigLayerV1::User {
                return false;
            }
            let env = &observation.settings.env;
            observation.settings.model.is_some()
                || env.base_url.is_some()
                || env.model.is_some()
                || env.default_opus_model.is_some()
                || env.default_sonnet_model.is_some()
                || env.default_haiku_model.is_some()
                || env.small_fast_model.is_some()
                || !env.present_environment_fields.is_empty()
                || observation.settings.api_key_helper_present
        }))
    }

    /// Claude's explicit initial model, not the three independently configurable preset
    /// mappings. An environment selection wins over settings.model; absent both, Claude's
    /// account-dependent Default remains unknown and must not be inferred from a preset.
    pub fn claude_explicit_model_selection(
        &self,
    ) -> Result<Option<String>, AgentFilesystemScanError> {
        let observations = self.claude_observations()?;
        Ok(
            select_effective(&observations, |settings| settings.env.model.as_deref())
                .or_else(|| select_effective(&observations, |settings| settings.model.as_deref()))
                .map(|(model, _)| model.to_owned()),
        )
    }

    /// Native-host fact used to prove Agent discovery and Worker selection resolve the same
    /// explicitly selected Claude installation. This path is never serialized to a client.
    pub fn claude_executable_target(&self) -> PathBuf {
        self.layout.claude_executable.clone()
    }

    /// Returns the bounded semantic fields from Claude's registered *user* settings file.
    /// Higher-precedence process/project/managed layers are deliberately not flattened here:
    /// the settings planner checks those independently before it asks the native renderer to
    /// change this one owned file.
    pub fn claude_user_config_document(
        &self,
    ) -> Result<AgentConfigDocumentV1, AgentFilesystemScanError> {
        let observations = self.claude_observations()?;
        let user = observations
            .iter()
            .find(|observation| observation.layer == ConfigLayerV1::User)
            .ok_or(AgentFilesystemScanError::InvalidConfig)?;
        let env = &user.settings.env;
        let mut fields = BTreeMap::new();
        for key in hiroute_domain::CLAUDE_CONTEXT_ENVIRONMENT {
            if let Some(value) = env.context_environment.get(key) {
                fields.insert(format!("env.{key}"), json!(value));
            }
        }
        for (path, value) in [
            ("env.ANTHROPIC_BASE_URL", env.base_url.as_deref()),
            ("env.ANTHROPIC_MODEL", env.model.as_deref()),
            (
                "env.ANTHROPIC_DEFAULT_OPUS_MODEL",
                env.default_opus_model.as_deref(),
            ),
            (
                "env.ANTHROPIC_DEFAULT_SONNET_MODEL",
                env.default_sonnet_model.as_deref(),
            ),
            (
                "env.ANTHROPIC_DEFAULT_HAIKU_MODEL",
                env.default_haiku_model.as_deref(),
            ),
            (
                "env.ANTHROPIC_SMALL_FAST_MODEL",
                env.small_fast_model.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                fields.insert(path.to_owned(), json!(value));
            }
        }
        if user.settings.api_key_helper_present {
            fields.insert("apiKeyHelper".to_owned(), json!({"configured":true}));
        }
        if CLAUDE_AUTH_ENVIRONMENT_FIELDS
            .iter()
            .any(|name| env.present_environment_fields.contains(*name))
        {
            fields.insert(
                "hiroute.auth_environment".to_owned(),
                json!({"configured":true}),
            );
        }
        Ok(AgentConfigDocumentV1 { fields })
    }

    /// Renders the registered semantic field change into the exact native Claude settings file.
    /// The returned bytes may contain unrelated user secrets and therefore remain zeroizing and
    /// must be staged only by the encrypted managed-artifact store.
    pub fn render_claude_user_config_change(
        &self,
        change: &hiroute_domain::AgentConfigChangeV1,
    ) -> Result<Zeroizing<Vec<u8>>, AgentFilesystemScanError> {
        render_claude_user_change(&self.layout.claude_user_settings, change)
    }

    /// Checks the registered managed fields semantically while allowing Claude Code to rewrite
    /// formatting or unrelated settings after activation.
    pub fn claude_user_config_change_is_applied(
        &self,
        change: &hiroute_domain::AgentConfigChangeV1,
    ) -> Result<bool, AgentFilesystemScanError> {
        claude_user_change_is_applied(&self.layout.claude_user_settings, change)
    }

    /// Reconstructs settings facts only for a daemon-validated, currently applied user-file
    /// change. The local Gateway is not an upstream in the discovery registry. Repeated saves
    /// and restoration therefore use the active Operation's ownership proof, while retaining
    /// native layer and target checks without running the diagnostic version probe.
    pub fn claude_installation_for_applied_user_change(
        &self,
        change: &hiroute_domain::AgentConfigChangeV1,
    ) -> Result<(hiroute_domain::SupportedAgentInstallationV1, String), AgentFilesystemScanError>
    {
        if !self.claude_user_config_change_is_applied(change)? {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        let executable = crate::agents::executable::resolve(&self.layout.claude_executable)
            .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
        let executable_path = executable
            .to_str()
            .ok_or(AgentFilesystemScanError::SourceUnavailable)?
            .to_owned();
        let observations = self.claude_observations()?;
        let observations =
            main_observation::resolve_project_local_fields(&self.layout, &observations);
        let (outcome, conflict) =
            main_observation::main_claude_observation("not-probed", &observations);
        if conflict {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        let AgentDiscoveryOutcomeV1::Supported { mut installation } = outcome else {
            return Err(AgentFilesystemScanError::InvalidConfig);
        };
        crate::agents::observed_capabilities::attach_target_file_capabilities(
            &mut installation,
            &self.layout.claude_user_settings,
        );
        #[cfg(unix)]
        if let Ok(cache) = self.claude_ingress.lock()
            && let Some(evidence) = cache.as_ref()
        {
            evidence.attach(&executable, &mut installation);
        }
        Ok((*installation, executable_path))
    }

    /// Identifies whether an opaque discovered credential came from the exact user settings file
    /// that the persistent Claude adapter owns. Process, project, launch, and enterprise-managed
    /// sources cannot be made safe by rewriting the user file and therefore stay launch-only.
    pub fn is_claude_user_credential(&self, descriptor: &DiscoveredCredentialRefV1) -> bool {
        validate_descriptor(descriptor).is_ok()
            && descriptor.discovered_source_ref
                == ClaudeSource::File(
                    ConfigLayerV1::User,
                    self.layout.claude_user_settings.clone(),
                )
                .source_ref()
    }
}
