//! Fresh Agent discovery with an explicit current-context read boundary.
use super::*;

impl AgentDiscoveryPort for LocalControlAdapter {
    fn list(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        let mut agents = self.discover()?;
        let targets = self
            .stores_lock()
            .map_err(map_port)?
            .control()
            .agent_settings_native_targets(&WorkspaceId::default())
            .map_err(map_port)?;
        for target in targets {
            match self.discover_target(&target) {
                Ok(rows) => agents.extend(rows),
                Err(_) => {
                    let subject = hiroute_integrations::hermes_subject(target.config_root())
                        .map_err(|_| ControlReadError::Corrupt)?;
                    agents.push(DiscoveredAgentV1 {
                        native_target: Some(target.clone()),
                        codex_access: None,
                        codex_desktop: None,
                        context_id: Some(hiroute_integrations::hermes_context_id(
                            target.config_root(),
                        )),
                        agent_id: subject.agent_id().into(),
                        profile_id: subject.profile_id().into(),
                        version: "unknown".into(),
                        supported: false,
                        configuration_state: "selected_target_unavailable".into(),
                        available_surfaces: Default::default(),
                        native_model_catalog: None,
                        registered_configuration: None,
                        discovered_credential: None,
                        permission_hardening: None,
                    });
                }
            }
        }
        Ok(agents)
    }
    fn discover_target(
        &self,
        target: &hiroute_domain::AgentNativeTargetV1,
    ) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        let target = hiroute_integrations::canonical_hermes_target(target)
            .map_err(|_| ControlReadError::Denied)?;
        let (installation, config_available) =
            match hiroute_integrations::hermes_settings_installation(&target) {
                Ok(installation) => (installation, true),
                Err(_) => (
                    hiroute_integrations::hermes_management_installation(&target)
                        .map_err(|_| ControlReadError::Denied)?,
                    false,
                ),
            };
        let supported = config_available
            && installation
                .require_action(hiroute_domain::AgentAction::ConfigureModel)
                .is_ok();
        let context_id = hiroute_integrations::hermes_context_id(target.config_root());
        Ok(vec![DiscoveredAgentV1 {
            native_target: Some(target),
            context_id: Some(context_id),
            codex_access: None,
            codex_desktop: None,
            agent_id: installation.agent_id,
            profile_id: installation.profile.profile_id,
            version: installation.version,
            supported,
            configuration_state: if supported {
                "registered"
            } else {
                "unsupported_native_feature"
            }
            .into(),
            available_surfaces: [hiroute_domain::AgentModelSurfaceV2::HermesCli].into(),
            native_model_catalog: None,
            registered_configuration: None,
            discovered_credential: None,
            permission_hardening: None,
        }])
    }
    fn discover(&self) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        self.discover_entries(None)
    }

    fn discover_context(
        &self,
        context_id: &str,
        target: Option<&hiroute_domain::AgentNativeTargetV1>,
    ) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        use settings_facts::SettingsAgentClass;
        let class = self
            .settings_agent_for_context(context_id)
            .ok_or(ControlReadError::NotFound)?;
        let agents = if class == SettingsAgentClass::Hermes {
            let target = self.hermes_target_for_context(context_id, target)?;
            self.discover_target(&target)?
        } else {
            if target.is_some() {
                return Err(ControlReadError::Denied);
            }
            self.discover_entries(Some(class.agent_id()))?
        };
        // Codex selection can move between root/profile while this page is open.
        // An old context is never silently rebound; explicit discovery recovers it.
        if agents.len() != 1 || agents[0].context_id.as_deref() != Some(context_id) {
            return Err(ControlReadError::SnapshotChanged);
        }
        Ok(agents)
    }
}

impl LocalControlAdapter {
    pub(super) fn refresh_codex_desktop_location(&self) {
        #[cfg(not(target_os = "macos"))]
        self.scanner
            .refresh_codex_desktop_location(hiroute_desktop_host_effects::codex_desktop_engine);
    }

    fn discover_entries(
        &self,
        selected: Option<&str>,
    ) -> Result<Vec<DiscoveredAgentV1>, ControlReadError> {
        let wants_codex = selected.is_none_or(|agent| agent == "agent_codex_default");
        let wants_desktop = selected.is_none_or(|agent| agent == "agent_codex_desktop");
        if wants_desktop {
            self.refresh_codex_desktop_location();
        }
        let desktop_catalog_source = if wants_desktop {
            self.scanner
                .codex_desktop_scanner()
                .and_then(|scanner| scanner.codex_catalog_summary().ok())
                .filter(|catalog| {
                    catalog.metadata_source
                        != hiroute_integrations::CodexCatalogMetadataSourceV1::HirouteGenerated
                })
        } else {
            None
        };
        // Fresh and HiRoute-only Desktop instances display no native source bindings.
        // Their targeted read must not join an unrelated, growing compute inventory.
        let fixed_candidates = if wants_codex || desktop_catalog_source.is_some() {
            hiroute_application::control::RoutingFactsPort::routing_compilation_snapshot(
                self,
                &WorkspaceId::default(),
            )
            .map(|snapshot| snapshot.facts.candidates)
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        let catalog_view = |catalog: hiroute_integrations::CodexCatalogSummaryV1| {
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
        };
        let codex_catalog = if wants_codex {
            self.scanner.codex_catalog_summary().ok()
        } else {
            None
        }
        .and_then(&catalog_view);
        let desktop_catalog = desktop_catalog_source.and_then(catalog_view);
        let discoveries = match selected {
            Some(agent) => vec![
                self.scanner
                    .scan_agent(agent)
                    .ok_or(ControlReadError::NotFound)?,
            ],
            None => self
                .refresh_discovery()
                .map_err(|_| ControlReadError::Unavailable)?,
        };
        discoveries
            .into_iter()
            .map(|discovery| {
                let mut agent = discovered_agent(discovery)?;
                agent.context_id = self.settings_context_for_agent(&agent.agent_id);
                agent.available_surfaces = self.scanner.available_model_surfaces(&agent.agent_id);
                if agent.agent_id == "agent_codex_default" {
                    agent.native_model_catalog = codex_catalog.clone();
                    let access = self.codex_access_view()?;
                    agent.context_id = Some(if access.selected_mode == "profile" {
                        access.profile_context_id.clone()
                    } else {
                        access.root_context_id.clone()
                    });
                    agent.codex_access = Some(access);
                }
                if agent.agent_id == hiroute_integrations::CODEX_DESKTOP_AGENT_ID {
                    agent.native_model_catalog = desktop_catalog.clone();
                    let paths = self
                        .scanner
                        .codex_desktop_instance_paths()
                        .ok_or(ControlReadError::Corrupt)?;
                    let source = self.scanner.codex_user_config_target();
                    agent.codex_desktop =
                        Some(hiroute_application_api::CodexDesktopInstanceViewV1 {
                            codex_home: paths.codex_home.to_string_lossy().into_owned(),
                            user_data: paths.user_data.to_string_lossy().into_owned(),
                            source_codex_home: source
                                .parent()
                                .ok_or(ControlReadError::Corrupt)?
                                .to_string_lossy()
                                .into_owned(),
                            shared_skills: native_agent_home()
                                .map_err(|_| ControlReadError::Unavailable)?
                                .join(".agents/skills")
                                .to_string_lossy()
                                .into_owned(),
                        });
                }
                Ok(agent)
            })
            .collect()
    }
}
