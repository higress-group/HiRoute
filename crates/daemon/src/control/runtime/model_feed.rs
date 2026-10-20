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
                |catalog, prepared| self.activate_catalog_prices(catalog, prepared),
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

    fn feed_status(&self) -> Result<ModelFeedStatusV1, ErrorCode> {
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
        Ok(ModelFeedStatusV1 {
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
            counts: ModelFeedCountsV1 {
                applied: status.counts.applied,
                ignored_embedded: status.counts.ignored_embedded,
                newer_client: status.counts.newer_client,
                unresolved: status.counts.unresolved,
            },
            next_automatic_attempt_at: status.next_automatic_attempt_at,
            disclosure: MODEL_FEED_DISCLOSURE.into(),
            disclosure_acknowledged: status.disclosure_acknowledged,
        })
    }
    fn feed_refresh(&self) -> Result<ModelFeedStatusV1, ErrorCode> {
        // Retrieval failures are represented by last_attempt; the last verified copy survives.
        let _ = self.refresh_model_feed();
        self.feed_status()
    }
    fn feed_settings(
        &self,
        request: ModelFeedSettingsRequestV1,
    ) -> Result<ModelFeedStatusV1, ErrorCode> {
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
    fn feed_remove_preview(&self) -> Result<ModelFeedRemovalPreviewV1, ErrorCode> {
        self.feed_removal_context().map(|(preview, _)| preview)
    }
    fn feed_remove(
        &self,
        request: ModelFeedRemoveRequestV1,
    ) -> Result<ModelFeedStatusV1, ErrorCode> {
        let (preview, state_digest) = self.feed_removal_context()?;
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
                |catalog, prepared| self.activate_catalog_prices(catalog, prepared),
            )
            .map_err(feed_error)?;
        self.feed_status()
    }
    fn model_availability(&self) -> Result<ModelAvailabilityResultV1, ErrorCode> {
        availability(self)
    }
}

impl LocalControlAdapter {
    fn feed_removal_context(
        &self,
    ) -> Result<(ModelFeedRemovalPreviewV1, CanonicalDigest), ErrorCode> {
        let feed = self
            .model_feed
            .as_ref()
            .ok_or(ErrorCode::DaemonUnavailable)?;
        // Capture feed before the storage lock; feed activation takes locks in that order.
        let before = feed.removal_digest().map_err(feed_error)?;
        let status = feed.status().map_err(feed_error)?;
        let catalog = self.current_catalog().ok_or(ErrorCode::DaemonUnavailable)?;
        let stores = self
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
        let mut sources = Vec::new();
        let mut published_revisions = Vec::new();
        for source in &management.sources {
            let models = source
                .models
                .iter()
                .filter(|model| {
                    model.catalog_configuration_id.as_ref().is_some_and(|id| {
                        !feed.is_embedded_model(id) && catalog.model_data().model(id).is_some()
                    })
                })
                .collect::<Vec<_>>();
            if models.is_empty() {
                continue;
            }
            let references = versions
                .iter()
                .filter(|version| {
                    candidates(version).any(|candidate| {
                        models
                            .iter()
                            .any(|model| model.binding_id == candidate.binding_id)
                    })
                })
                .map(|version| version.reference.clone())
                .collect::<Vec<_>>();
            for reference in &references {
                if !published_revisions.contains(reference) {
                    published_revisions.push(reference.clone());
                }
            }
            sources.push(ModelFeedAffectedSourceV1 {
                source_id: source.source_id.clone(),
                display_name: source.display_name.clone(),
                model_configuration_ids: models
                    .iter()
                    .filter_map(|m| m.catalog_configuration_id.clone())
                    .collect(),
                published_revisions: references,
            });
        }
        let revisions = management.revisions;
        drop(stores);
        if before != feed.removal_digest().map_err(feed_error)? {
            return Err(ErrorCode::ChangePreviewStale);
        }
        let digest = CanonicalDigest::of(&(&before, &revisions, &sources, &published_revisions))
            .map_err(|_| ErrorCode::Internal)?;
        Ok((
            ModelFeedRemovalPreviewV1 {
                sources,
                published_revisions,
                retained_sequence: status.highest_accepted_sequence,
                retrieval_enabled: status.retrieval_enabled,
                digest,
            },
            before,
        ))
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

fn availability(adapter: &LocalControlAdapter) -> Result<ModelAvailabilityResultV1, ErrorCode> {
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
            for fact in facts {
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
            }) => ModelFactOriginV1::Feed {
                sequence,
                included_on,
            },
            _ => ModelFactOriginV1::Embedded {
                client_version: env!("CARGO_PKG_VERSION").into(),
            },
        };
        models.push(AvailableModelV1 {
            model_configuration_id: id.clone(),
            display_name: model.display_name.clone(),
            origin,
            identity_stability: Some(catalog.model_identity_stability(id).into()),
            availability: if executable.is_empty() {
                ModelAvailabilityV1::NeedsConnection
            } else {
                ModelAvailabilityV1::Executable
            },
            binding_ids: executable,
            facts: Some(model_facts(
                &model.publisher_id,
                &model.capabilities,
                reasoning(catalog.native_reasoning(), id),
            )),
        });
    }
    for newer in catalog.newer_feed_models() {
        models.push(AvailableModelV1 {
            model_configuration_id: newer.model_configuration_id.clone(),
            display_name: newer.display_name.clone(),
            origin: ModelFactOriginV1::Feed {
                sequence: newer.sequence,
                included_on: newer.included_on.clone(),
            },
            identity_stability: Some(newer.identity_stability.clone()),
            availability: ModelAvailabilityV1::NeedsNewerClient {
                version: newer.minimum_client.clone(),
            },
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
        models.push(AvailableModelV1 {
            display_name: id.clone(),
            model_configuration_id: id,
            origin: ModelFactOriginV1::Published,
            identity_stability: None,
            availability: ModelAvailabilityV1::UsedByPublishedPlans { count },
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
    Ok(ModelAvailabilityResultV1 { models, featured })
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
