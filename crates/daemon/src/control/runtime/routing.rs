use hiroute_application::compiler::{
    AgentPlanCompilationFactsV1, CandidateCompilationFactV1, CandidateFactScope,
    FreeCandidateEvidenceV1, OrderingPriceFactV1,
};
use hiroute_application::control::{
    ControlReadError, RoutingCompilationSnapshotV1, RoutingFactsPort,
};
use hiroute_domain::{
    AGENT_PLAN_COMPILER_REVISION_V1, AGENT_PLAN_FACTS_SCHEMA_V1, AgentPlanFactRefsV1,
    AuthenticationKind, CanonicalDigest, ComputeManagementRepositoryPort, ControlRepositoryPort,
    MaterializationState, PriceTrackingMode, PublicationRepositoryPort, SourceOrigin, WorkspaceId,
};

use super::LocalControlAdapter;
use hiroute_application_api::PlanCandidateUnavailableReasonV1 as UnavailableReason;

#[path = "routing_candidates.rs"]
mod candidates;
use candidates::CandidateDiagnostics;

impl RoutingFactsPort for LocalControlAdapter {
    fn plan_lifecycle_snapshot(
        &self,
        workspace: &WorkspaceId,
        change: &hiroute_application_api::PlanLifecycleChangeV1,
    ) -> Result<hiroute_application::routing::PlanLifecycleSnapshotV1, ControlReadError> {
        use hiroute_domain::{AgentPlanReferenceKind, AgentPlanReferenceReadPort};
        // All Plan, Agent and retention writers share this store owner. Apply repeats this
        // read under the existing writer/admission barrier before installing the publication.
        let stores = self.stores_lock().map_err(super::map_port)?;
        let control = stores.control();
        let head = control
            .plan_head(workspace, &change.plan_id)
            .map_err(|_| ControlReadError::Corrupt)?
            .ok_or(ControlReadError::NotFound)?;
        let version = control
            .lookup_exact_plan_version(&head.reference)
            .map_err(|_| ControlReadError::Corrupt)?;
        let publication = control
            .active_publication(workspace)
            .map_err(super::map_port)?
            .ok_or(ControlReadError::Unavailable)?
            .verify()
            .map_err(|_| ControlReadError::Corrupt)?;
        let references = control
            .agent_plan_references(workspace, &change.plan_id)
            .map_err(super::map_port)?;
        let has_version_holds = control
            .plan_has_retained_versions(workspace, &change.plan_id)
            .map_err(|_| ControlReadError::Unavailable)?;
        Ok(hiroute_application::routing::PlanLifecycleSnapshotV1 {
            head,
            version,
            publication,
            expected_revisions: control
                .current_revisions(workspace)
                .map_err(super::map_port)?,
            references_digest: references.facts_digest,
            has_agent_references: !references.references.is_empty(),
            has_default_model_reference: references
                .references
                .iter()
                .any(|reference| reference.kind == AgentPlanReferenceKind::DefaultModel),
            has_version_holds,
        })
    }

    fn decision_services(
        &self,
        workspace: &WorkspaceId,
    ) -> Result<Vec<hiroute_domain::DecisionServiceV1>, ControlReadError> {
        self.stores_lock()
            .map_err(super::map_port)?
            .control()
            .decision_services(workspace)
            .map_err(super::map_port)
    }
    fn claude_client_capability_preview(
        &self,
        plan: &hiroute_domain::CompiledAgentPlanV1,
    ) -> Result<hiroute_application_api::ClaudeClientCapabilityPreviewV1, ControlReadError> {
        Ok(hiroute_integrations::agents::claude_plan_capability_preview(plan))
    }

    fn codex_client_capability_preview(
        &self,
        plan: &hiroute_domain::CompiledAgentPlanV1,
    ) -> Result<hiroute_application_api::CodexClientCapabilityPreviewV1, ControlReadError> {
        Ok(hiroute_integrations::agents::codex_plan_capability_preview(
            plan,
        ))
    }

    fn plan_draft_snapshot(
        &self,
        workspace: &WorkspaceId,
        change: &hiroute_domain::PlanDraftChangeV1,
    ) -> Result<hiroute_application::routing::PlanDraftSnapshotV1, ControlReadError> {
        let stores = self.stores_lock().map_err(super::map_port)?;
        let control = stores.control();
        let first = control
            .current_revisions(workspace)
            .map_err(super::map_port)?;
        let draft = control
            .plan_draft(workspace, &change.draft_id)
            .map_err(|_| ControlReadError::Corrupt)?;
        let legacy_source = match &change.action {
            hiroute_domain::PlanDraftActionV1::Save { draft } => match &draft.plan_id {
                Some(plan_id)
                    if control
                        .plan_head(workspace, plan_id)
                        .map_err(|_| ControlReadError::Corrupt)?
                        .is_none() =>
                {
                    let record = control
                        .active_publication(workspace)
                        .map_err(super::map_port)?
                        .ok_or(ControlReadError::Unavailable)?;
                    let publication = record.verify().map_err(|_| ControlReadError::Corrupt)?;
                    let compiled = publication
                        .plans
                        .iter()
                        .find(|p| p.agent_plan_id() == plan_id)
                        .ok_or(ControlReadError::Unavailable)?;
                    Some(
                        hiroute_domain::PlanVersionV1::from_unversioned_compiled_recovery(
                            workspace.clone(),
                            compiled.clone(),
                        )
                        .map_err(|_| ControlReadError::Corrupt)?,
                    )
                }
                _ => None,
            },
            _ => None,
        };
        let second = control
            .current_revisions(workspace)
            .map_err(super::map_port)?;
        if first != second {
            return Err(ControlReadError::SnapshotChanged);
        }
        Ok(hiroute_application::routing::PlanDraftSnapshotV1 {
            workspace: workspace.clone(),
            draft,
            legacy_source,
            expected_revisions: second,
        })
    }

    fn resolve_model_ratings(
        &self,
        query: &hiroute_application_api::ResolveModelRatingsV1,
    ) -> Result<
        hiroute_application_api::ResolveModelRatingsResultV1,
        hiroute_application::model_catalog::RatingQueryError,
    > {
        self.plan_model_ratings()?.resolve(query)
    }

    fn free_plan_suggestions(
        &self,
        workspace: &WorkspaceId,
        requirements: &hiroute_domain::CapabilityRequirementsV1,
        selections: &std::collections::BTreeMap<String, hiroute_domain::ReasoningSelectionV1>,
        snapshot: hiroute_application_api::RatingSnapshotSelectionV1,
    ) -> Result<hiroute_application::routing::FreeSuggestionsV1, ControlReadError> {
        let facts = self.routing_compilation_snapshot(workspace)?;
        // Missing rating data does not remove otherwise legal suggestions.
        let ratings = self.plan_model_ratings().unwrap_or_default();
        hiroute_application::routing::suggest_free(
            &facts.facts,
            requirements,
            selections,
            &ratings,
            snapshot,
        )
        .map_err(|error| match error {
            hiroute_application::compiler::AgentPlanCompilerError::RatingSnapshotUnavailable => {
                ControlReadError::Unavailable
            }
            _ => ControlReadError::Corrupt,
        })
    }

    fn plan_authoring_snapshot(
        &self,
        workspace: &WorkspaceId,
        change: &hiroute_application_api::PlanContentChangeV2,
    ) -> Result<hiroute_application::routing::PlanAuthoringSnapshotV2, ControlReadError> {
        self.plan_content_snapshot(workspace, change)
    }

    fn routing_compilation_snapshot(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<RoutingCompilationSnapshotV1, ControlReadError> {
        let first = self.routing_snapshot(workspace_id)?;
        let second = self.routing_snapshot(workspace_id)?;
        // Both reads are independent. Compare the complete inputs without temporary encoding.
        if first.unavailable_candidates != second.unavailable_candidates
            || first.facts != second.facts
            || first.expected_revisions != second.expected_revisions
            || first.active_publication != second.active_publication
        {
            return Err(ControlReadError::SnapshotChanged);
        }
        Ok(second)
    }
}

impl LocalControlAdapter {
    fn plan_model_ratings(
        &self,
    ) -> Result<
        hiroute_application::model_catalog::ModelRatings,
        hiroute_application::model_catalog::RatingQueryError,
    > {
        use hiroute_application::model_catalog::{ModelRatings, RatingQueryError};
        let snapshot = self
            .release_catalog
            .as_ref()
            .ok_or(RatingQueryError::SnapshotUnavailable)?
            .rating_snapshot()
            .clone();
        let ratings = ModelRatings::default();
        ratings
            .install(snapshot)
            .map_err(|_| RatingQueryError::SnapshotUnavailable)?;
        Ok(ratings)
    }

    fn routing_snapshot(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<RoutingCompilationSnapshotV1, ControlReadError> {
        hiroute_diagnostics::publication::measure(
            &self
                .publication_diagnostics
                .lock()
                .map(|p| p.clone())
                .unwrap_or_default(),
            hiroute_diagnostics::publication::PublicationStage::RoutingSnapshot,
            None,
            None,
            || {
                let catalog = self
                    .release_catalog
                    .as_ref()
                    .ok_or(ControlReadError::Unavailable)?;
                let provenance = catalog
                    .compute_catalog_provenance()
                    .map_err(|_| ControlReadError::Corrupt)?;
                let (management, rows) = {
                    let stores = self.stores_lock().map_err(super::map_port)?;
                    let control = stores.control();
                    (
                        control
                            .compute_management_snapshot(workspace_id)
                            .map_err(super::map_port)?,
                        control.compute_projection_rows().map_err(super::map_port)?,
                    )
                };
                let mut diagnostics =
                    CandidateDiagnostics::new(&management.sources, &rows, catalog);
                let needs_cpa = rows
                    .iter()
                    .any(|(source, _, _)| source.origin == SourceOrigin::Cpa)
                    || management.sources.iter().any(|source| {
                        source.state == MaterializationState::Ready
                            && matches!(
                                source.provenance,
                                hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned { .. }
                            )
                    });
                let cpa_batch = if needs_cpa {
                    self.cpa_sources
                        .as_ref()
                        .and_then(|authority| authority.begin_routing_batch().ok())
                } else {
                    None
                };
                let cpa_sources = cpa_batch
                    .as_ref()
                    .map(|batch| self.cpa_candidates_from_sources(batch.sources().to_vec()))
                    .transpose()?
                    .unwrap_or_default();
                let cpa_targets = cpa_batch.as_ref();
                let stores = self.stores_lock().map_err(super::map_port)?;
                let control = stores.control();
                let mut inventory_refs = Vec::new();
                let mut candidates = Vec::new();
                for (source, binding, inventory) in rows {
                    source
                        .validate_shape()
                        .map_err(|_| ControlReadError::Corrupt)?;
                    if !hiroute_domain::valid_upstream_model_id(&binding.upstream_model_id) {
                        diagnostics
                            .legacy_reason(&binding.binding_id, UnavailableReason::InvalidModelId);
                        continue;
                    }
                    binding
                        .validate_shape()
                        .map_err(|_| ControlReadError::Corrupt)?;
                    if catalog.validate_source(&source, true).is_err()
                        || binding.validate(&source, catalog.model_data()).is_err()
                    {
                        // A structurally valid durable projection can legitimately become stale when a
                        // newer client-bundled catalog removes or revises its exact connector/model/Offer facts.
                        // Exclude only that candidate; persisted corruption was already rejected above.
                        diagnostics
                            .legacy_reason(&binding.binding_id, UnavailableReason::CatalogMismatch);
                        continue;
                    }
                    let Some(model) = catalog
                        .model_data()
                        .model(&binding.model_configuration_id)
                        .cloned()
                    else {
                        diagnostics
                            .legacy_reason(&binding.binding_id, UnavailableReason::CatalogMismatch);
                        continue;
                    };
                    let Some(capability) = catalog
                        .model_data()
                        .model_endpoint_capabilities
                        .iter()
                        .find(|capability| capability.capability_id == binding.capability_id)
                        .cloned()
                    else {
                        diagnostics
                            .legacy_reason(&binding.binding_id, UnavailableReason::CatalogMismatch);
                        continue;
                    };
                    let Ok(resolved) =
                        catalog.resolve_connection_option(&source.connection_option_id)
                    else {
                        diagnostics
                            .legacy_reason(&binding.binding_id, UnavailableReason::CatalogMismatch);
                        continue;
                    };
                    let Some(protocol_endpoint) = resolved
                        .endpoint_profile
                        .protocol_endpoints
                        .iter()
                        .find(|endpoint| {
                            endpoint.protocol_endpoint_id == capability.protocol_endpoint_id
                        })
                        .cloned()
                    else {
                        diagnostics
                            .legacy_reason(&binding.binding_id, UnavailableReason::CatalogMismatch);
                        continue;
                    };
                    let Some(offer) = catalog.model_data().offer(&binding.offer_ref) else {
                        diagnostics
                            .legacy_reason(&binding.binding_id, UnavailableReason::CatalogMismatch);
                        continue;
                    };
                    let mut pool = binding
                        .credential_pool_id
                        .as_deref()
                        .map(|pool_id| control.credential_pool(pool_id))
                        .transpose()
                        .map_err(super::map_port)?
                        .flatten();
                    if pool.is_none() && source.origin == SourceOrigin::Cpa {
                        pool = cpa_sources
                            .iter()
                            .find(|(registered, model_id)| {
                                registered.source.source_id == source.source_id
                                    && model_id == &binding.model_configuration_id
                            })
                            .and_then(|(registered, _)| {
                                catalog
                                    .transient_cpa_credential_pool(registered, &source, &binding)
                                    .ok()
                            });
                    }
                    if let Some(pool) = &pool
                        && (catalog.validate_pool(pool, &source).is_err()
                            || pool.validate_against_binding(&binding).is_err())
                    {
                        diagnostics.legacy_reason(
                            &binding.binding_id,
                            UnavailableReason::CredentialUnavailable,
                        );
                        continue;
                    }
                    let Some(reasoning) = catalog
                        .native_reasoning()
                        .iter()
                        .find(|reasoning| {
                            reasoning.model_configuration_id == binding.model_configuration_id
                        })
                        .cloned()
                    else {
                        diagnostics.legacy_reason(
                            &binding.binding_id,
                            UnavailableReason::CapabilityUnavailable,
                        );
                        continue;
                    };
                    let source_state = match resolved.connector.authentication {
                        AuthenticationKind::None => source.state,
                        _ if source.state == MaterializationState::Disabled => source.state,
                        _ if pool.as_ref().is_some_and(|pool| {
                            pool.credentials.iter().any(|entry| entry.enabled)
                        }) =>
                        {
                            MaterializationState::Ready
                        }
                        _ => MaterializationState::NeedsCredential,
                    };
                    let Some(execution) =
                        super::candidate_execution::materialize_candidate_execution(
                            &resolved,
                            &model,
                            &capability,
                            &protocol_endpoint,
                            &reasoning,
                            pool.as_ref(),
                            cpa_targets,
                        )
                    else {
                        let reason = if source.origin == SourceOrigin::Cpa && cpa_targets.is_none()
                        {
                            UnavailableReason::RuntimeUnavailable
                        } else if source_state == MaterializationState::NeedsCredential {
                            UnavailableReason::CredentialUnavailable
                        } else {
                            UnavailableReason::InvalidConfiguration
                        };
                        diagnostics.legacy_reason(&binding.binding_id, reason);
                        continue;
                    };
                    // The current catalog keeps configuration-scoped ratings separate. Explicit
                    // compilation must not collapse them into this legacy model-level field.
                    let rating = None;
                    let ordering_price = catalog
                        .model_data()
                        .price_rates
                        .iter()
                        .find(|price| {
                            price.offer_ref == binding.offer_ref
                                && price.model_configuration_id == binding.model_configuration_id
                        })
                        .map(|price| {
                            Ok(OrderingPriceFactV1 {
                                price_rate_id: price.price_rate_id.clone(),
                                price_rate_revision: price.revision,
                                offer_ref: price.offer_ref.clone(),
                                model_configuration_id: price.model_configuration_id.clone(),
                                currency: price.currency.clone(),
                                input_micros_per_million: price.input_micros_per_million,
                                output_micros_per_million: price.output_micros_per_million,
                                frozen_digest: CanonicalDigest::of(price)
                                    .map_err(|_| ControlReadError::Corrupt)?,
                            })
                        })
                        .transpose()?;
                    let free_evidence = if binding.billing_class
                        == hiroute_domain::BillingClass::Free
                    {
                        let Some(free) = catalog.model_data().free_offers.iter().find(|free| {
                            free.offer_ref == binding.offer_ref
                                && free
                                    .model_configuration_ids
                                    .contains(&binding.model_configuration_id)
                        }) else {
                            continue;
                        };
                        Some(FreeCandidateEvidenceV1 {
                            free_offer_id: free.free_offer_id.clone(),
                            free_offer_revision: free.revision,
                            offer_ref: free.offer_ref.clone(),
                            access: free.access,
                            evidence_digest: free
                                .direct_verification_evidence
                                .as_ref()
                                .map(|value| CanonicalDigest::of_bytes(value.as_bytes()))
                                .unwrap_or_else(|| CanonicalDigest::of_bytes(b"api-key-required")),
                        })
                    } else {
                        None
                    };
                    inventory_refs.push((
                        inventory.source_id.clone(),
                        inventory.endpoint_profile_id.clone(),
                        inventory.inventory_revision,
                        inventory.inventory_digest.clone(),
                    ));
                    let inventory_model_matched = inventory
                        .observed_models
                        .iter()
                        .any(|model| model.upstream_model_id == binding.upstream_model_id);
                    candidates.push(CandidateCompilationFactV1 {
                authority:
                    hiroute_application::compiler::CandidateFactAuthorityV1::RegisteredCatalog,
                connection_option_id: source.connection_option_id.clone(),
                offer_revision: offer.revision,
                binding,
                model,
                capability,
                protocol_endpoint,
                connector_runtime: execution.connector_runtime,
                operational_target: execution.operational_target,
                native_transport_model: execution.native_transport_model,
                protocol_profiles: execution.protocol_profiles,
                credential_refs: execution.credential_refs,
                credential_destination_ref: None,
                source_state,
                inventory_model_matched,
                reasoning: reasoning.capability,
                rating,
                ordering_price,
                free_evidence,
            });
                }
                for source in management.sources {
                    inventory_refs.push((
                        source.source_id.clone(),
                        "compute-management/v2".into(),
                        source.revision,
                        source.digest().map_err(|_| ControlReadError::Corrupt)?,
                    ));
                    if let hiroute_domain::ComputeManagementProvenanceV2::Registered {
                        connection_option_id,
                        ..
                    } = &source.provenance
                    {
                        let Ok(resolved) = catalog.resolve_connection_option(connection_option_id)
                        else {
                            diagnostics.catalog_mismatch(&source);
                            continue;
                        };
                        if !super::model_connections::registered_source_matches_current_option(
                            &source, &resolved,
                        ) {
                            diagnostics.catalog_mismatch(&source);
                            continue;
                        }
                    }
                    let compilation = diagnostics.compile_models(&source);
                    for fact in &compilation {
                        let candidate = match &fact.provenance {
                    hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
                        connector_id,
                        account_ref,
                    } => cpa_targets
                        .and_then(|batch| {
                            unique_live_cpa_source(batch.sources(), connector_id, account_ref)
                        })
                        .and_then(|registered| {
                            super::candidate_execution::materialize_connector_management_candidate(
                                fact,
                                registered,
                                catalog,
                                cpa_targets,
                            )
                        }),
                    hiroute_domain::ComputeManagementProvenanceV2::Registered { .. } => {
                        if fact.eligibility
                            == hiroute_application::compute_management::ComputeManagementEligibilityV2::RuntimeQualified
                        {
                            super::candidate_execution::materialize_management_candidate(
                                fact,
                                Some(catalog),
                            )
                        } else {
                            super::candidate_execution::materialize_registered_management_candidate(
                                fact, catalog,
                            )
                        }
                    }
                    hiroute_domain::ComputeManagementProvenanceV2::UserConfigured { .. } => {
                        super::candidate_execution::materialize_management_candidate(
                            fact,
                            Some(catalog),
                        )
                    }
                };
                        if let Some(candidate) = candidate {
                            candidates.push(candidate);
                        } else {
                            let runtime_unavailable = match &fact.provenance {
                                hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
                                    connector_id,
                                    account_ref,
                                } => cpa_targets
                                    .and_then(|batch| {
                                        unique_live_cpa_source(
                                            batch.sources(),
                                            connector_id,
                                            account_ref,
                                        )
                                    })
                                    .is_none(),
                                _ => false,
                            };
                            diagnostics.materialization_failed(fact, runtime_unavailable);
                        }
                    }
                }
                inventory_refs.sort();
                candidates
                    .sort_by(|left, right| left.binding.binding_id.cmp(&right.binding.binding_id));
                let inventory_revision = inventory_refs
                    .iter()
                    .map(|value| value.2)
                    .max()
                    .unwrap_or(0);
                let rating_snapshot = catalog.rating_snapshot().clone();
                let facts = AgentPlanCompilationFactsV1 {
                    schema: AGENT_PLAN_FACTS_SCHEMA_V1.into(),
                    compiler_revision: AGENT_PLAN_COMPILER_REVISION_V1.into(),
                    candidate_scope: CandidateFactScope::AllMaterializedBindings,
                    refs: AgentPlanFactRefsV1 {
                        connector_registry_version: catalog.registry().registry_version.clone(),
                        connector_registry_digest: provenance.connector_registry_digest,
                        model_data_bundle_version: catalog.model_data().bundle_version.clone(),
                        model_data_digest: provenance.model_data_digest,
                        capability_slice_version: catalog
                            .model_data()
                            .capability_slice_version
                            .clone(),
                        capability_slice_digest: CanonicalDigest::of(
                            &catalog.model_data().model_endpoint_capabilities,
                        )
                        .map_err(|_| ControlReadError::Corrupt)?,
                        ratings_slice_version: rating_snapshot.version,
                        ratings_slice_digest: rating_snapshot.digest,
                        free_offers_slice_version: catalog
                            .model_data()
                            .free_offers_slice_version
                            .clone(),
                        free_offers_slice_digest: CanonicalDigest::of(
                            &catalog.model_data().free_offers,
                        )
                        .map_err(|_| ControlReadError::Corrupt)?,
                        inventory_revision,
                        inventory_digest: CanonicalDigest::of(&inventory_refs)
                            .map_err(|_| ControlReadError::Corrupt)?,
                        price_tracking: PriceTrackingMode::FollowLatest,
                    },
                    ordering_price_version: catalog.model_data().prices_slice_version.clone(),
                    ordering_price_digest: CanonicalDigest::of(&catalog.model_data().price_rates)
                        .map_err(|_| ControlReadError::Corrupt)?,
                    candidates,
                };
                facts.validate_snapshot().map_err(|error| {
                    eprintln!("routing compilation facts are invalid: {error}");
                    ControlReadError::Corrupt
                })?;
                let revisions = control
                    .current_revisions(workspace_id)
                    .map_err(super::map_port)?;
                let active = control
                    .active_publication(workspace_id)
                    .map_err(super::map_port)?;
                let active_publication = active
                    .as_ref()
                    .map(hiroute_domain::PublicationRecordV1::verify)
                    .transpose()
                    .map_err(|_| ControlReadError::Corrupt)?;
                let mut expected_revisions = revisions;
                expected_revisions
                    .dependencies
                    .insert("release.registry".into(), provenance.release_sequence);
                expected_revisions
                    .dependencies
                    .insert("release.model_data".into(), provenance.release_sequence);
                if let Some(batch) = cpa_batch
                    && !batch.finish().unwrap_or(false)
                {
                    return Err(ControlReadError::SnapshotChanged);
                }
                Ok(RoutingCompilationSnapshotV1 {
                    unavailable_candidates: diagnostics.finish(&facts.candidates),
                    facts,
                    expected_revisions,
                    active_publication,
                })
            },
        )
    }

    #[cfg(test)]
    pub(super) fn routing_cpa_candidates(
        &self,
        needed: bool,
    ) -> Vec<(hiroute_integrations::CpaRegisteredSourceV1, String)> {
        if !needed {
            return Vec::new();
        }
        // A managed connector is an independent execution dependency. Its process or borrowed
        // authorization becoming temporarily unavailable must remove only those candidates, not
        // make unrelated native/API candidates or the plan editor unreadable.
        self.registered_cpa_candidates().unwrap_or_default()
    }
}

fn unique_live_cpa_source<'a>(
    sources: &'a [hiroute_integrations::CpaRegisteredSourceV1],
    connector_id: &str,
    account_ref: &str,
) -> Option<&'a hiroute_integrations::CpaRegisteredSourceV1> {
    let mut matching = sources.iter().filter(|registered| {
        registered.source.connector_id == connector_id
            && registered.source.connection_option_id == super::subscriptions::CONNECTION_OPTION_ID
            && registered.source.identity.account_subject_ref == account_ref
    });
    let source = matching.next()?;
    matching.next().is_none().then_some(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_domain::{CredentialRefV1, InventoryDisposition};
    use hiroute_integrations::{
        CpaAccountMaterializationV1, TrustedReleaseCatalog, register_cpa_account,
    };

    #[test]
    fn observed_cpa_model_without_catalog_projection_keeps_its_live_account_source() {
        const MANIFEST: &[u8] =
            include_bytes!("../../../../../assets/release-facts/current/bundle/manifest.json");
        let catalog = TrustedReleaseCatalog::load_bundled_release_facts(
            MANIFEST,
            MANIFEST,
            include_bytes!(
                "../../../../../assets/release-facts/current/bundle/connector-registry.json"
            ),
            include_bytes!("../../../../../assets/release-facts/current/bundle/model-data.json"),
        )
        .unwrap();
        let source_id = "cpa/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let account_ref =
            "account/cpa/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let materialization = CpaAccountMaterializationV1 {
            connector_id: "connector.cpa.codex".into(),
            connection_option_id: super::super::subscriptions::CONNECTION_OPTION_ID.into(),
            endpoint_profile_id: "endpoint.cpa.codex".into(),
            source_id: source_id.into(),
            account_subject: account_ref.into(),
            credential_ref: CredentialRefV1::new(
                "credential/cpa/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                format!("source/{source_id}"),
                "connector/connector.cpa.codex",
                "provider-auth",
                ["connection-option/codex.subscription.global.v1".into()],
                1,
            )
            .unwrap(),
            observed_model_ids: ["gpt-6-future-text".into()].into_iter().collect(),
        };
        let registered = register_cpa_account(&catalog, &materialization).unwrap();
        assert_eq!(registered.inventory.len(), 1);
        assert_eq!(
            registered.inventory[0].disposition,
            InventoryDisposition::InventoryOnly
        );
        assert!(registered.inventory[0].model_configuration_id.is_none());
        assert_eq!(
            unique_live_cpa_source(
                std::slice::from_ref(&registered),
                "connector.cpa.codex",
                account_ref
            ),
            Some(&registered),
        );
        assert!(
            unique_live_cpa_source(
                &[registered.clone(), registered],
                "connector.cpa.codex",
                account_ref
            )
            .is_none()
        );
    }
}
