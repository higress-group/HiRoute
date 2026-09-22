use super::*;

impl LocalControlAdapter {
    pub(super) fn codex_restore_model_facts(
        &self,
        original: &hiroute_domain::OperationId,
        target: &str,
    ) -> Result<CodexRestoreModelFacts, ControlReadError> {
        let stores = self.stores_lock().map_err(super::map_port)?;
        let operation = stores
            .control()
            .load_operation(original)
            .map_err(super::map_port)?
            .ok_or(ControlReadError::Corrupt)?;
        let intent = operation
            .plan
            .external()
            .iter()
            .find(|intent| {
                super::native_model::is_settings_codex_model(intent) && intent.target() == target
            })
            .ok_or(ControlReadError::Corrupt)?;
        let record = self
            .artifacts
            .load_native_restore(original, intent)
            .map_err(super::map_port)?
            .ok_or(ControlReadError::Corrupt)?;
        let baseline = super::native_model::codex_catalog_baseline(
            stores.control(),
            &self.artifacts,
            Some(original),
            target,
        )
        .map_err(super::map_port)?;
        drop(stores);
        if record.len() < 3 || record[0] != 1 || record[1] > 1 {
            return Err(ControlReadError::Corrupt);
        }
        let restore = CodexNativeRestore::decode_protected(&record[2..])
            .map_err(|_| ControlReadError::Corrupt)?;
        let current = self
            .artifacts
            .read_native_target(target)
            .map_err(super::map_port)?;
        let text = std::str::from_utf8(current.as_deref().map_or(&[], |bytes| bytes.as_slice()))
            .map_err(|_| ControlReadError::Corrupt)?;
        let restored = match restore_codex_native(text, &restore) {
            Ok(restored) => restored,
            Err(_) => {
                return Ok(CodexRestoreModelFacts {
                    ids: Some(Vec::new()),
                    model: None,
                    catalog: None,
                });
            }
        };
        let restored_model =
            codex_explicit_model(&restored).map_err(|_| ControlReadError::Corrupt)?;
        let scope = CodexConfigurationScope::user_file(self.scanner.codex_user_config_target());
        let catalog = sample_codex_catalog_plan(
            &scope,
            &[],
            CodexDefaultPolicy {
                explicit_model: None,
                uses_codex_backend: true,
                allow_provider_model_fallback: false,
            },
            &baseline,
            None,
        );
        let Ok(catalog) = catalog else {
            return Ok(CodexRestoreModelFacts {
                ids: Some(Vec::new()),
                model: restored_model,
                catalog: None,
            });
        };
        let default = catalog
            .selection
            .effective_default(CodexDefaultPolicy {
                explicit_model: restored_model.as_deref(),
                uses_codex_backend: true,
                allow_provider_model_fallback: false,
            })
            .map_err(|_| ControlReadError::Corrupt)?
            .to_owned();
        let reasoning =
            codex_explicit_reasoning_effort(&restored).map_err(|_| ControlReadError::Corrupt)?;
        let models = catalog.selection.original()["models"]
            .as_array()
            .ok_or(ControlReadError::Corrupt)?
            .iter()
            .map(|model| {
                Ok(hiroute_integrations::CodexCatalogModelSummaryV1 {
                    client_model_id: model["slug"]
                        .as_str()
                        .ok_or(ControlReadError::Corrupt)?
                        .to_owned(),
                    display_name: model["display_name"]
                        .as_str()
                        .ok_or(ControlReadError::Corrupt)?
                        .to_owned(),
                    native_reasoning_profile: reasoning
                        .clone()
                        .or_else(|| model["default_reasoning_level"].as_str().map(str::to_owned)),
                })
            })
            .collect::<Result<Vec<_>, ControlReadError>>()?;
        let ids = models
            .iter()
            .map(|model| model.client_model_id.clone())
            .collect();
        let summary = CodexCatalogSummaryV1 {
            metadata_source: catalog.producer.metadata_source,
            native_default_model: default,
            models,
        };
        Ok(CodexRestoreModelFacts {
            ids: Some(ids),
            model: restored_model,
            catalog: Some(summary),
        })
    }

    /// Bind the native Codex catalog to exactly one already saved source/account. Catalog names
    /// alone never authorize calls: subscription identity comes from the selected auth file's
    /// account evidence; API identity requires the exact endpoint and keyed credential fingerprint.
    pub(super) fn preserved_codex_model_selections(
        &self,
        catalog: &CodexCatalogSummaryV1,
        candidates: &mut Vec<hiroute_application::compiler::CandidateCompilationFactV1>,
    ) -> Option<(Vec<AgentFixedModelSelectionV2>, Vec<String>)> {
        let mut native_candidates = self
            .scanner
            .codex_source_candidates(&CodexSelectionTarget::Root)
            .ok()?;
        if native_candidates.len() != 1 {
            return None;
        }
        let native = native_candidates.pop()?;
        let stores = self.stores_lock().ok()?;
        let management = stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .ok()?;
        enum NativeIdentity {
            ConnectorAccount(String),
            Api {
                endpoint: zeroize::Zeroizing<String>,
                credential_fingerprint: CanonicalDigest,
            },
        }
        let identity = match native.authentication {
            DiscoveredAuthSource::NativeSessionNeedsConfirmation => {
                let account_ref = self
                    .scanner
                    .codex_subscription_source()
                    .ok()
                    .flatten()
                    .and_then(|source| {
                        BorrowedCodexAuthSpec::new(source.source_path())
                            .inspect()
                            .ok()
                    })?
                    .account_ref();
                NativeIdentity::ConnectorAccount(account_ref)
            }
            DiscoveredAuthSource::EnvironmentKey | DiscoveredAuthSource::InlineToken => {
                let protected = self
                    .scanner
                    .read_selected_codex_source(&CodexSelectionTarget::Root, &native)
                    .ok()?;
                let fingerprint = stores
                    .secrets()
                    .fingerprint(protected.credential.as_ref()?)
                    .ok()?;
                NativeIdentity::Api {
                    endpoint: protected.endpoint,
                    credential_fingerprint: fingerprint,
                }
            }
            DiscoveredAuthSource::EnvironmentToken
            | DiscoveredAuthSource::HelperNeedsInput
            | DiscoveredAuthSource::Missing
            | DiscoveredAuthSource::Ambiguous => return None,
        };
        let mut matching_sources = management.sources.iter().filter(|source| {
            if source.state != hiroute_domain::MaterializationState::Ready {
                return false;
            }
            match &identity {
                NativeIdentity::ConnectorAccount(account_ref) => {
                    connector_account_matches(&source.provenance, account_ref)
                }
                NativeIdentity::Api {
                    endpoint,
                    credential_fingerprint,
                } => {
                    source.provenance.is_native()
                        && endpoint_matches_target(endpoint, &source.target)
                        && source
                            .enabled_credentials()
                            .any(|credential| &credential.fingerprint == credential_fingerprint)
                }
            }
        });
        let source = matching_sources.next()?.clone();
        if matching_sources.next().is_some() {
            return None;
        }
        drop(stores);
        let mut proven_names = source
            .models
            .iter()
            .filter(|model| model.execution_eligible)
            .map(|model| model.upstream_model_id.clone())
            .collect::<BTreeSet<_>>();

        // The saved management source may select only one model. Additional Codex catalog names
        // are connection-local facts from the retained, checked account and current CPA batch;
        // they never enter the source or the generic Plan candidate snapshot.
        if let NativeIdentity::ConnectorAccount(_) = identity
            && let (Ok(Some(checked)), Some(catalog_facts), Some(authority)) = (
                self.retained_subscription_candidate_for_source(&source),
                self.release_catalog.as_ref(),
                self.cpa_sources.as_ref(),
            )
            && let Ok(batch) = authority.begin_routing_batch()
        {
            let live = self
                .cpa_candidates_from_sources(batch.sources().to_vec())
                .unwrap_or_default();
            for native_model in &catalog.models {
                let mut checked_models = checked.models.iter().filter(|model| {
                    model.upstream_model_id == native_model.client_model_id
                        && model.selectable
                        && model.reason.is_none()
                });
                let Some(checked_model) = checked_models.next() else {
                    continue;
                };
                if checked_models.next().is_some() {
                    continue;
                }
                proven_names.insert(native_model.client_model_id.clone());
                if candidates.iter().any(|candidate| {
                    candidate.binding.source_id == source.source_id
                        && candidate.binding.upstream_model_id == native_model.client_model_id
                }) {
                    continue;
                }
                let Ok(fact) =
                    hiroute_application::compute_management::compile_connection_only_codex_model(
                        &source,
                        &checked,
                        checked_model,
                    )
                else {
                    continue;
                };
                let mut registered = live.iter().filter(|(registered, model_id)| {
                    registered.source.identity.account_subject_ref.as_str()
                        == match &source.provenance {
                            hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
                                account_ref,
                                ..
                            } => account_ref.as_str(),
                            _ => return false,
                        }
                        && fact.catalog_configuration_id.as_deref() == Some(model_id.as_str())
                });
                let Some((registered_source, _)) = registered.next() else {
                    continue;
                };
                if registered.next().is_some() {
                    continue;
                }
                if let Some(candidate) =
                    super::candidate_execution::materialize_connector_management_candidate(
                        &fact,
                        registered_source,
                        catalog_facts,
                        Some(&batch),
                    )
                {
                    candidates.push(candidate);
                }
            }
        }

        // Every original name is checked again by the application before the provider switch.
        let mut selections = Vec::with_capacity(catalog.models.len());
        for model in &catalog.models {
            let mut matching = candidates.iter().filter(|candidate| {
                candidate.binding.source_id == source.source_id
                    && candidate.is_routable()
                    && candidate.binding.upstream_model_id == model.client_model_id
            });
            let Some(candidate) = matching.next() else {
                continue;
            };
            if matching.next().is_some() {
                continue;
            }
            let reasoning = match &candidate.reasoning {
                hiroute_domain::NativeReasoningCapabilityV1::Fixed { .. } => None,
                hiroute_domain::NativeReasoningCapabilityV1::Discrete { profiles, .. } => {
                    let Some(profile) = model.native_reasoning_profile.as_ref() else {
                        continue;
                    };
                    profiles
                        .contains(profile)
                        .then(|| ReasoningSelectionV1::Profile {
                            profile: profile.clone(),
                        })
                }
                hiroute_domain::NativeReasoningCapabilityV1::Toggle { .. }
                | hiroute_domain::NativeReasoningCapabilityV1::Budget { .. } => None,
            };
            if !matches!(
                candidate.reasoning,
                hiroute_domain::NativeReasoningCapabilityV1::Fixed { .. }
            ) && reasoning.is_none()
            {
                continue;
            }
            selections.push(AgentFixedModelSelectionV2 {
                client_model_id: model.client_model_id.clone(),
                candidate: CandidateSelectionV1 {
                    binding_id: candidate.binding.binding_id.clone(),
                    reasoning,
                },
            });
        }
        let proven = catalog
            .models
            .iter()
            .filter(|model| proven_names.contains(&model.client_model_id))
            .map(|model| model.client_model_id.clone())
            .collect();
        Some((selections, proven))
    }

    /// Derive the immutable catalog facts for a plan-carrying Codex selection. Any derivation
    /// failure leaves the catalog absent, so Preview blocks the selection instead of sealing
    /// an unfulfillable plan. Capability evidence here describes structural catalog support,
    /// not a Codex executable identity or version precheck.
    pub(super) fn codex_catalog_facts(
        &self,
        settings: &hiroute_domain::AgentModelSelectionV2,
        publication: Option<&hiroute_domain::GatewayPublicationV1>,
        baseline: &hiroute_integrations::CodexCatalogBaseline,
        installation: &mut SupportedAgentInstallationV1,
    ) -> Option<SettingsModelCatalogFacts> {
        let publication = publication?;
        let derived = super::native_model::codex_catalog_plan_for(
            &self.scanner.codex_user_config_target(),
            settings,
            publication,
            baseline,
        );
        match derived {
            Ok(plan) => {
                mark_catalog_structure_proven(installation);
                let producer_kind = match plan.producer.metadata_source {
                    hiroute_integrations::CodexCatalogMetadataSourceV1::UserConfigured => {
                        hiroute_application::agent_connection::CodexCatalogProducerKindV1::UserConfigured
                    }
                    hiroute_integrations::CodexCatalogMetadataSourceV1::TargetCache => {
                        hiroute_application::agent_connection::CodexCatalogProducerKindV1::TargetCache
                    }
                    hiroute_integrations::CodexCatalogMetadataSourceV1::TargetBundled => {
                        hiroute_application::agent_connection::CodexCatalogProducerKindV1::TargetBundled
                    }
                    hiroute_integrations::CodexCatalogMetadataSourceV1::HirouteGenerated => {
                        hiroute_application::agent_connection::CodexCatalogProducerKindV1::HirouteGenerated
                    }
                };
                Some(SettingsModelCatalogFacts {
                    source_revision: hiroute_integrations::CODEX_CATALOG_SOURCE_REVISION.to_owned(),
                    content_digest: plan.content_digest,
                    before_fingerprint: None,
                    producer_kind,
                    producer_path: plan.producer.path.to_str()?.to_owned(),
                    producer_content_digest: plan.producer.content_digest,
                    producer_context_digest: plan.producer.context_digest,
                    producer_dependency_digest: plan.producer.dependency_digest,
                })
            }
            Err(_) => {
                mark_catalog_structure_proven(installation);
                None
            }
        }
    }
}
