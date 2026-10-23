//! Catalog capture and daemon-owned retrieval. Gateway execution keeps its frozen Plan facts.
use super::*;
use crate::model_feed::ReleaseCatalogSnapshot;

/// Embedded fallback is private to this module; runtime consumers capture current_catalog().
#[derive(Default)]
pub(super) struct FallbackCatalog(Option<Arc<ReleaseCatalogSnapshot>>);
impl FallbackCatalog {
    pub(super) fn new(catalog: TrustedReleaseCatalog) -> Self {
        Self(Some(Arc::new(ReleaseCatalogSnapshot {
            catalog,
            generation: 0,
            accepted_sequence: 0,
        })))
    }
}

impl LocalControlAdapter {
    pub(super) fn current_catalog(&self) -> Option<Arc<ReleaseCatalogSnapshot>> {
        match &self.model_feed {
            Some(feed) => feed.available_catalog(),
            None => self.catalog_fallback.0.clone(),
        }
    }

    fn refresh_model_feed(&self) -> Result<(), String> {
        self.model_feed
            .as_ref()
            .ok_or("feed_unavailable")?
            .refresh_prepared(
                |catalog| self.prepare_catalog_prices(catalog),
                |catalog, prepared, commit| self.activate_catalog_prices(catalog, prepared, commit),
            )
    }
}

pub(super) fn start_retrieval(adapter: &Arc<LocalControlAdapter>) {
    let weak = Arc::downgrade(adapter);
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(15));
            let Some(adapter) = weak.upgrade() else {
                break;
            };
            if adapter.startup_recovery_complete.load(Ordering::Acquire)
                && adapter
                    .model_feed
                    .as_ref()
                    .is_some_and(|feed| feed.automatic_attempt_due())
            {
                let _ = adapter.refresh_model_feed();
            }
        }
    });
}

use hiroute_application::model_feed::ModelFeedPort;
use hiroute_application_api::*;
use hiroute_domain::{ComputeManagementRepositoryPort, PlanVersionV1};

fn feed_error(reason: String) -> ErrorCode {
    if reason == "feed_preview_changed" {
        ErrorCode::ChangePreviewStale
    } else {
        ErrorCode::DaemonUnavailable
    }
}

mod metadata;
mod removal;
impl ModelFeedPort for LocalControlAdapter {
    fn list_model_metadata(
        &self,
        request: ListModelMetadataV1,
    ) -> Result<ModelMetadataPageV1, ErrorCode> {
        metadata::list(self, request)
    }
    fn get_model_metadata(
        &self,
        request: GetModelMetadataV1,
    ) -> Result<ModelMetadataDetailV1, ErrorCode> {
        metadata::get(self, request)
    }

    fn feed_status(&self) -> Result<ModelFeedStatusV2, ErrorCode> {
        let status = self
            .model_feed
            .as_ref()
            .ok_or(ErrorCode::DaemonUnavailable)?
            .status()
            .map_err(feed_error)?;
        let verification = match status.verification.as_str() {
            "validated" => ModelFeedVerificationV1::Validated,
            "none" => ModelFeedVerificationV1::None,
            reason => ModelFeedVerificationV1::Discarded {
                reason: reason.into(),
            },
        };
        let catalog = self
            .current_catalog()
            .ok_or(ErrorCode::ChangePreviewStale)?;
        let snapshot = catalog.official_snapshot();
        let qualified = catalog
            .model_data()
            .models
            .iter()
            .filter(|m| {
                catalog
                    .model_data()
                    .model_endpoint_capabilities
                    .iter()
                    .any(|cap| {
                        cap.model_configuration_id == m.model_configuration_id
                            && catalog.model_data().offers.iter().any(|o| {
                                o.endpoint_profile_id == cap.endpoint_profile_id
                                    && o.model_configuration_ids
                                        .contains(&m.model_configuration_id)
                            })
                    })
            })
            .count() as u64;
        let data = &snapshot.data;
        let descriptive = data.models.len()
            + data.model_endpoint_capabilities.len()
            + data.offers.len()
            + data.price_rates.len()
            + data.pricing_facts.len()
            + data.connection_hints.len()
            + data.rating_snapshot.records.len()
            + data.rating_snapshot.models.len()
            + data.metadata_catalog.evidence_sources.len()
            + data.metadata_catalog.client_runs.len()
            + data.metadata_catalog.access_products.len()
            + data.metadata_catalog.canonical_models.len()
            + data.metadata_catalog.endpoint_bindings.len()
            + data.metadata_catalog.dynamic_routes.len()
            + data.metadata_catalog.inference_rules.len()
            + data.metadata_catalog.provider_records.len()
            + data.metadata_catalog.model_records.len();
        Ok(ModelFeedStatusV2 {
            selected: ModelMetadataSnapshotRefV1 {
                generation: catalog.generation,
                data_revision: snapshot.data_revision,
                data_digest: snapshot.data_digest.clone(),
            },
            selected_origin: if catalog.official_data_from_feed() {
                "website_cache"
            } else {
                "embedded"
            }
            .into(),
            highest_accepted_data_revision: status.highest_accepted_data_revision,
            last_success_at: status.last_success_at_ms,
            recovery_available: status.recovery_available,
            retrieval_enabled: status.retrieval_enabled,
            held: status.held.map(|held| ModelFeedHeldV1 {
                sequence: held.sequence,
                issued_at: held.issued_at,
                retrieved_at: held.retrieved_at_ms,
            }),
            highest_accepted_sequence: status.highest_accepted_sequence,
            last_attempt: status.last_attempt.map(|attempt| ModelFeedAttemptV1 {
                at: attempt.at_ms,
                result: attempt.result,
                reason: attempt.reason,
            }),
            verification,
            counts: ModelFeedCountsV2 {
                descriptive_records: descriptive as u64,
                qualified_models: qualified,
                restricted_models: data.models.len() as u64 - qualified,
            },
            next_automatic_attempt_at: status.next_automatic_attempt_at,
            disclosure: MODEL_FEED_DISCLOSURE.into(),
            disclosure_acknowledged: status.disclosure_acknowledged,
        })
    }
    fn feed_refresh(&self) -> Result<ModelFeedStatusV2, ErrorCode> {
        // Retrieval failures are represented by last_attempt; the last verified copy survives.
        let _ = self.refresh_model_feed();
        self.feed_status()
    }
    fn feed_settings(
        &self,
        request: ModelFeedSettingsRequestV1,
    ) -> Result<ModelFeedStatusV2, ErrorCode> {
        if request.retrieval_enabled.is_none() && request.acknowledge_disclosure.is_none() {
            return Err(ErrorCode::InvalidArguments);
        }
        self.model_feed
            .as_ref()
            .ok_or(ErrorCode::DaemonUnavailable)?
            .settings(request.retrieval_enabled, request.acknowledge_disclosure)
            .map_err(feed_error)?;
        self.feed_status()
    }
    fn feed_remove_preview(
        &self,
        query: ModelFeedRemovalQueryV2,
    ) -> Result<ModelFeedRemovalPreviewV2, ErrorCode> {
        self.feed_removal_context(query).map(|(preview, _)| preview)
    }
    fn feed_remove(
        &self,
        request: ModelFeedRemoveRequestV1,
    ) -> Result<ModelFeedStatusV2, ErrorCode> {
        let (preview, state_digest) =
            self.feed_removal_context(ModelFeedRemovalQueryV2::default())?;
        if preview.digest != request.accepted_digest {
            return Err(ErrorCode::ChangePreviewStale);
        }
        self.model_feed
            .as_ref()
            .ok_or(ErrorCode::DaemonUnavailable)?
            .remove_prepared(
                &state_digest,
                request.also_disable_retrieval,
                |catalog| self.prepare_catalog_prices(catalog),
                |catalog, prepared, commit| self.activate_catalog_prices(catalog, prepared, commit),
            )
            .map_err(feed_error)?;
        self.feed_status()
    }
    fn model_availability(&self) -> Result<ModelAvailabilityResultV2, ErrorCode> {
        availability(self)
    }
}

impl LocalControlAdapter {
    fn feed_removal_context(
        &self,
        query: ModelFeedRemovalQueryV2,
    ) -> Result<(ModelFeedRemovalPreviewV2, CanonicalDigest), ErrorCode> {
        removal::preview(self, query)
    }
}

fn candidates(
    version: &PlanVersionV1,
) -> impl Iterator<Item = &hiroute_domain::AttemptOwnedCandidateV1> {
    version
        .compiled
        .body
        .materialized
        .attempt_owned
        .groups
        .iter()
        .flat_map(|group| &group.candidates)
}

fn model_facts(
    publisher_id: &str,
    capabilities: &hiroute_domain::CapabilityFactsV1,
    reasoning: Option<AvailableModelReasoningV1>,
) -> AvailableModelFactsV1 {
    AvailableModelFactsV1 {
        publisher_id: publisher_id.into(),
        context_tokens: capabilities.context_tokens,
        tool: capabilities.tool,
        vision: capabilities.vision,
        reasoning,
    }
}

/// A fixed native profile is adaptation metadata, not a choice, so it is not presented.
fn reasoning(
    facts: &[hiroute_domain::ModelNativeReasoningV1],
    id: &str,
) -> Option<AvailableModelReasoningV1> {
    use hiroute_domain::NativeReasoningCapabilityV1 as Native;
    match &facts
        .iter()
        .find(|fact| fact.model_configuration_id == id)?
        .capability
    {
        Native::Fixed { .. } => None,
        Native::Toggle { .. } => Some(AvailableModelReasoningV1::Toggle),
        Native::Discrete { profiles, .. } => Some(AvailableModelReasoningV1::Profiles {
            profiles: profiles.clone(),
        }),
        Native::Budget { .. } => Some(AvailableModelReasoningV1::Budget),
    }
}

fn availability(adapter: &LocalControlAdapter) -> Result<ModelAvailabilityResultV2, ErrorCode> {
    use hiroute_integrations::model_feed::ModelFactOrigin;
    let catalog = adapter
        .current_catalog()
        .ok_or(ErrorCode::ChangePreviewStale)?;
    let stores = adapter
        .stores_lock()
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let management = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    let versions = stores
        .control()
        .published_plan_versions(&WorkspaceId::default())
        .map_err(|_| ErrorCode::DaemonUnavailable)?;
    if adapter.model_feed.is_some()
        && management
            .revisions
            .dependencies
            .get("feed.model_data")
            .copied()
            .unwrap_or(0)
            != catalog.generation
    {
        return Err(ErrorCode::ChangePreviewStale);
    }
    let preferences = management
        .sources
        .iter()
        .map(|source| {
            stores
                .control()
                .source_default_preferences(&source.source_id)
                .map(|preferences| (source.source_id.clone(), preferences))
                .map_err(|_| ErrorCode::DaemonUnavailable)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    drop(stores);
    let mut bindings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Pure local admission: never probe a provider or refresh a subscription from this read.
    for source in &management.sources {
        let mut selected = source.clone();
        for model in &source.models {
            selected.models = vec![model.clone()];
            let Ok(facts) =
                hiroute_application::compute_management::compile_compute_management_source(
                    &selected,
                )
            else {
                continue;
            };
            for mut fact in facts {
                fact.default_preferences = preferences
                    .get(&source.source_id)
                    .and_then(|preferences| preferences.get(&fact.binding_id))
                    .cloned()
                    .unwrap_or_default();
                let candidate =
                    super::candidate_execution::materialize_registered_management_candidate(
                        &fact, &catalog,
                    );
                if let Some(candidate) = candidate {
                    bindings
                        .entry(candidate.model.model_configuration_id)
                        .or_default()
                        .push(model.binding_id.clone());
                }
            }
        }
    }
    let mut models = Vec::new();
    for model in &catalog.model_data().models {
        let id = &model.model_configuration_id;
        let executable = bindings.remove(id).unwrap_or_default();
        let origin = match catalog.model_fact_origin(id) {
            Some(ModelFactOrigin::Feed {
                sequence,
                included_on,
            }) => ModelFactOriginV2::Feed {
                sequence,
                as_of: included_on,
            },
            _ => ModelFactOriginV2::Embedded {
                client_version: env!("CARGO_PKG_VERSION").into(),
            },
        };
        models.push(AvailableModelV2 {
            model_configuration_id: id.clone(),
            display_name: model.display_name.clone(),
            origin,
            identity_stability: Some(catalog.model_identity_stability(id).into()),
            availability: if !catalog
                .model_data()
                .model_endpoint_capabilities
                .iter()
                .any(|cap| {
                    &cap.model_configuration_id == id
                        && catalog.model_data().offers.iter().any(|offer| {
                            offer.endpoint_profile_id == cap.endpoint_profile_id
                                && offer.model_configuration_ids.contains(id)
                        })
                }) {
                ModelAvailabilityV2::DescriptiveOnly
            } else if executable.is_empty() {
                ModelAvailabilityV2::NeedsConnection
            } else {
                ModelAvailabilityV2::Executable
            },
            published_plan_references: versions
                .iter()
                .filter(|v| candidates(v).any(|c| &c.model_configuration_id == id))
                .count(),
            binding_ids: executable,
            facts: Some(model_facts(
                &model.publisher_id,
                &model.capabilities,
                reasoning(catalog.native_reasoning(), id),
            )),
        });
    }
    for newer in catalog.newer_feed_models() {
        models.push(AvailableModelV2 {
            model_configuration_id: newer.model_configuration_id.clone(),
            display_name: newer.display_name.clone(),
            origin: ModelFactOriginV2::Feed {
                sequence: newer.sequence,
                as_of: newer.included_on.clone(),
            },
            identity_stability: Some(newer.identity_stability.clone()),
            availability: ModelAvailabilityV2::NeedsNewerClient {
                version: newer.minimum_client.clone(),
            },
            published_plan_references: 0,
            binding_ids: vec![],
            facts: Some(model_facts(&newer.publisher_id, &newer.capabilities, None)),
        });
    }
    let mut historical: BTreeMap<String, usize> = BTreeMap::new();
    for version in &versions {
        let ids = candidates(version)
            .map(|c| c.model_configuration_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        for id in ids {
            if !models
                .iter()
                .any(|model| model.model_configuration_id == id)
            {
                *historical.entry(id.into()).or_default() += 1;
            }
        }
    }
    for (id, count) in historical {
        models.push(AvailableModelV2 {
            display_name: id.clone(),
            model_configuration_id: id,
            origin: ModelFactOriginV2::Published,
            identity_stability: None,
            availability: ModelAvailabilityV2::UsedByPublishedPlans { count },
            published_plan_references: count,
            binding_ids: vec![],
            facts: None,
        });
    }
    models.sort_by(|a, b| a.model_configuration_id.cmp(&b.model_configuration_id));
    let featured = catalog
        .featured_models()
        .iter()
        .filter(|entry| {
            models
                .iter()
                .any(|model| model.model_configuration_id == entry.model_configuration_id)
        })
        .map(|entry| ModelAnnouncementV1 {
            id: entry.id.clone(),
            kind: match entry.kind {
                hiroute_integrations::model_feed::FeaturedKind::NewModel => "new_model",
                hiroute_integrations::model_feed::FeaturedKind::ModelUpdate => "model_update",
            }
            .into(),
            model_configuration_id: entry.model_configuration_id.clone(),
            headline: ModelFeedTextV1 {
                en: entry.headline.en.clone(),
                zh: entry.headline.zh.clone(),
            },
            note: ModelFeedTextV1 {
                en: entry.note.en.clone(),
                zh: entry.note.zh.clone(),
            },
            links: entry.links.clone(),
            included_on: entry.included_on.clone(),
        })
        .collect();
    Ok(ModelAvailabilityResultV2 { models, featured })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{ModelNativeReasoningV1, NativeReasoningCapabilityV1 as Native};

    fn fact(id: &str, capability: Native) -> ModelNativeReasoningV1 {
        ModelNativeReasoningV1 {
            model_configuration_id: id.into(),
            capability,
            native_render_convention: None,
        }
    }

    #[test]
    fn presented_reasoning_hides_native_parameters_and_fixed_profiles() {
        let facts = [
            fact(
                "model.profiles",
                Native::Discrete {
                    parameter: "output_config.effort".into(),
                    profiles: vec!["low".into(), "high".into()],
                    default_profile: None,
                },
            ),
            fact(
                "model.toggle",
                Native::Toggle {
                    parameter: "thinking".into(),
                },
            ),
            fact(
                "model.fixed",
                Native::Fixed {
                    profile: "none".into(),
                },
            ),
        ];
        assert_eq!(
            reasoning(&facts, "model.profiles"),
            Some(AvailableModelReasoningV1::Profiles {
                profiles: vec!["low".into(), "high".into()]
            })
        );
        assert_eq!(
            reasoning(&facts, "model.toggle"),
            Some(AvailableModelReasoningV1::Toggle)
        );
        assert_eq!(reasoning(&facts, "model.fixed"), None);
        assert_eq!(reasoning(&facts, "model.unknown"), None);
    }
}
