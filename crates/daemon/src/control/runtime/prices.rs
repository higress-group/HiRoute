use super::*;
mod official;
use hiroute_application::prices::{
    CatalogPriceRuleV2, PriceControlFactsV1, PriceIndexEntryV1, PriceSnapshot, PriceSnapshotSlot,
    PriceSourceBinding, SourcePriceControlPort,
};
use hiroute_domain::{
    BillingClass, ComputeManagementProvenanceV2, ComputeManagementRepositoryPort, ConnectionOrigin,
    PriceFactRefV1, PriceGenerationRefV1, PriceModelIdentityV1, PriceTargetV1,
    PriceValuationKindV1, SourceOrigin, SourcePriceOverrideV1, TokenRatesV1,
};
use std::collections::BTreeSet;

impl ProductionControlRuntime {
    /// The role=all producer and control writer must share this exact slot. A consumer captures
    /// it once at LogicalRequest admission and retains that handle across all Attempts.
    pub fn price_snapshot_slot(&self) -> Arc<PriceSnapshotSlot> {
        self.adapter.price_snapshot.clone()
    }
}

fn source_origin(origin: ConnectionOrigin) -> SourceOrigin {
    match origin {
        ConnectionOrigin::NativeApi => SourceOrigin::NativeApi,
        ConnectionOrigin::AgentSubscription => SourceOrigin::Cpa,
        ConnectionOrigin::FreeCatalog => SourceOrigin::ReleaseFree,
    }
}

fn unique_offer<'a>(
    catalog: &'a hiroute_integrations::TrustedReleaseCatalog,
    resolved: &hiroute_domain::ResolvedConnectionOptionV1,
    model_configuration_id: &str,
) -> Option<&'a hiroute_domain::OfferV1> {
    let mut offers = catalog.model_data().offers.iter().filter(|offer| {
        offer.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
            && offer.endpoint_profile_revision == resolved.endpoint_profile.revision
            && offer.service_offering_id == resolved.endpoint_profile.service_offering_id
            && offer.entitlement_id == resolved.endpoint_profile.entitlement_id
            && offer.usage_scope == resolved.endpoint_profile.usage_scope
            && offer.region_id == resolved.endpoint_profile.region_id
            && offer.billing_class == resolved.option.billing_class
            && offer
                .model_configuration_ids
                .iter()
                .any(|id| id == model_configuration_id)
    });
    let first = offers.next()?;
    offers.next().is_none().then_some(first)
}

impl SourcePriceControlPort for LocalControlAdapter {
    fn price_control_facts(&self) -> Result<PriceControlFactsV1, ControlReadError> {
        let stores = self
            .stores
            .lock()
            .map_err(|_| ControlReadError::Unavailable)?;
        self.price_facts_locked(&stores, None)
    }
    fn try_price_control_facts(
        &self,
        deadline: std::time::Instant,
    ) -> Result<
        hiroute_application::control::ControlReadAttempt<PriceControlFactsV1>,
        ControlReadError,
    > {
        use hiroute_application::control::ControlReadAttempt;
        match self.stores.try_lock() {
            Ok(stores) => self
                .price_facts_locked(&stores, Some(deadline))
                .map(ControlReadAttempt::Ready),
            Err(std::sync::TryLockError::WouldBlock) => Ok(ControlReadAttempt::Busy),
            Err(std::sync::TryLockError::Poisoned(_)) => Err(ControlReadError::Unavailable),
        }
    }
}
impl LocalControlAdapter {
    fn price_facts_locked(
        &self,
        stores: &hiroute_local_storage::LocalStorageSet,
        deadline: Option<std::time::Instant>,
    ) -> Result<PriceControlFactsV1, ControlReadError> {
        let catalog = self
            .current_catalog()
            .ok_or(ControlReadError::Unavailable)?;
        self.price_facts_for(stores, deadline, &catalog)
    }
    pub(super) fn price_facts_for(
        &self,
        stores: &hiroute_local_storage::LocalStorageSet,
        deadline: Option<std::time::Instant>,
        catalog: &crate::model_feed::ReleaseCatalogSnapshot,
    ) -> Result<PriceControlFactsV1, ControlReadError> {
        let check = || {
            if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                Err(ControlReadError::Unavailable)
            } else {
                Ok(())
            }
        };
        check()?;
        let control = stores.control();
        let legacy_source_bindings = control.price_source_bindings().map_err(map_port)?;
        let management = control
            .compute_management_snapshot(&WorkspaceId::default())
            .map_err(map_port)?;
        let mut catalog_model_ids = catalog
            .model_data()
            .models
            .iter()
            .map(|m| m.model_configuration_id.clone())
            .collect::<BTreeSet<_>>();
        catalog_model_ids.extend(
            management
                .sources
                .iter()
                .flat_map(|s| &s.models)
                .filter_map(|m| m.catalog_configuration_id.clone()),
        );
        catalog_model_ids.extend(
            legacy_source_bindings
                .iter()
                .map(|(_, binding)| binding.model_configuration_id.clone()),
        );
        check()?;
        let overrides = control
            .source_price_overrides(&WorkspaceId::default())
            .map_err(map_port)?;
        check()?;
        let legacy = control.price_overrides().map_err(map_port)?;
        let revisions = control
            .current_revisions(&WorkspaceId::default())
            .map_err(map_port)?;
        let configuration_revision = revisions.target;
        let identity = catalog.official_snapshot();
        let catalog_refs = vec![PriceFactRefV1 {
            id: "official-model-data".into(),
            revision: identity.data_revision,
            digest: identity.data_digest,
        }];
        check()?;
        let mut source_bindings = Vec::new();
        let mut identities = BTreeSet::new();
        for source in &management.sources {
            check()?;
            // The current management aggregate owns an exact source/binding identity even when
            // its retained catalog proof is stale. In that case the binding must disappear from
            // price facts instead of falling back to a pre-migration projection with the same
            // identity.
            identities.extend(
                source
                    .models
                    .iter()
                    .map(|model| (source.source_id.clone(), model.binding_id.clone())),
            );
            let registered = match &source.provenance {
                ComputeManagementProvenanceV2::Registered {
                    connection_option_id,
                    ..
                } => catalog
                    .resolve_connection_option(connection_option_id)
                    .ok()
                    .and_then(|resolved| {
                        super::model_connections::registered_source_matches_current_option(
                            source, &resolved,
                        )
                        .then_some((connection_option_id.clone(), resolved))
                    }),
                _ => None,
            };
            let (source_origin, billing_class, connection_option_id) = match &source.provenance {
                ComputeManagementProvenanceV2::Registered { .. } => {
                    let Some((connection_option_id, resolved)) = registered.as_ref() else {
                        continue;
                    };
                    (
                        source_origin(resolved.option.origin),
                        resolved.option.billing_class,
                        Some(connection_option_id.clone()),
                    )
                }
                ComputeManagementProvenanceV2::UserConfigured { .. } => {
                    (SourceOrigin::NativeApi, BillingClass::Unknown, None)
                }
                ComputeManagementProvenanceV2::ConnectorOwned { .. } => {
                    (SourceOrigin::Cpa, BillingClass::Subscription, None)
                }
            };
            let execution_models =
                hiroute_application::compute_management::compile_compute_management_source(source)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|fact| {
                        super::candidate_execution::management_execution_model_id(&fact)
                            .map(|id| (fact.binding_id, id))
                    })
                    .collect::<std::collections::BTreeMap<_, _>>();
            for model in &source.models {
                let model_configuration_id = model
                    .catalog_configuration_id
                    .clone()
                    .unwrap_or_else(|| model.model_ref.clone());
                let offer_ref = registered.as_ref().and_then(|(_, resolved)| {
                    unique_offer(&catalog, resolved, &model_configuration_id)
                        .map(|offer| offer.offer_id.clone())
                });
                source_bindings.push(PriceSourceBinding {
                    source_id: source.source_id.clone(),
                    source_revision: source.revision,
                    source_identity_digest: source.lineage_digest.clone(),
                    source_origin,
                    connection_option_id: connection_option_id.clone(),
                    binding_id: model.binding_id.clone(),
                    binding_revision: model.revision,
                    model_configuration_id,
                    execution_model_configuration_id: execution_models
                        .get(&model.binding_id)
                        .cloned(),
                    billing_class,
                    offer_ref,
                });
            }
        }
        for (source, binding) in legacy_source_bindings {
            if source.identity_digest != binding.source_identity_digest {
                continue;
            }
            if !identities.insert((source.source_id.clone(), binding.binding_id.clone())) {
                continue;
            }
            let offer_ref = catalog
                .model_data()
                .offer(&binding.offer_ref)
                .filter(|offer| {
                    offer.evidence_digest == binding.offer_evidence_digest
                        && offer.endpoint_profile_id == source.identity.endpoint_profile_id
                        && offer.service_offering_id == source.identity.service_offering_id
                        && offer.entitlement_id == source.identity.entitlement_id
                        && offer.usage_scope == source.identity.usage_scope
                        && offer.region_id == source.identity.region_id
                        && offer
                            .model_configuration_ids
                            .contains(&binding.model_configuration_id)
                })
                .map(|offer| offer.offer_id.clone());
            source_bindings.push(PriceSourceBinding {
                source_id: source.source_id,
                source_revision: source.revision,
                source_identity_digest: source.identity_digest,
                source_origin: source.origin,
                connection_option_id: Some(source.connection_option_id),
                binding_id: binding.binding_id,
                binding_revision: binding.revision,
                model_configuration_id: binding.model_configuration_id,
                execution_model_configuration_id: None,
                billing_class: binding.billing_class,
                offer_ref,
            });
        }
        let mut entries = Vec::new();
        for binding in &source_bindings {
            check()?;
            let model = if catalog_model_ids.contains(&binding.model_configuration_id) {
                PriceModelIdentityV1::CatalogModel(binding.model_configuration_id.clone())
            } else {
                PriceModelIdentityV1::LocalModel(binding.model_configuration_id.clone())
            };
            let offer = binding
                .offer_ref
                .as_deref()
                .and_then(|offer_ref| catalog.model_data().offer(offer_ref));
            let rates = catalog
                .model_data()
                .price_rates
                .iter()
                .filter(|r| {
                    offer.is_some()
                        && Some(r.offer_ref.as_str()) == binding.offer_ref.as_deref()
                        && r.model_configuration_id == binding.model_configuration_id
                })
                .collect::<Vec<_>>();
            let documented = official::documented_prices(catalog, binding);
            let mut currencies = rates
                .iter()
                .map(|r| r.currency.clone())
                .collect::<BTreeSet<_>>();
            currencies.extend(documented.iter().map(|p| p.currency.clone()));
            currencies.insert("USD".into());
            currencies.extend(
                overrides
                    .iter()
                    .filter(|o| {
                        o.target.source_id == binding.source_id
                            && o.target.source_identity_digest == binding.source_identity_digest
                            && o.target.model_identity == model
                    })
                    .map(|o| o.target.currency.clone()),
            );
            for currency in currencies {
                for kind in [
                    PriceValuationKindV1::UsageEstimate,
                    PriceValuationKindV1::ApiEquivalent,
                ] {
                    let target = PriceTargetV1 {
                        workspace_id: WorkspaceId::default(),
                        source_id: binding.source_id.clone(),
                        source_identity_digest: binding.source_identity_digest.clone(),
                        model_identity: model.clone(),
                        currency: currency.clone(),
                        valuation_kind: kind,
                    };
                    // Subscription reference mappings belong to 19. Until supplied, API equivalent
                    // has no guessed catalog reference; a source-specific manual value can exist.
                    let catalog_rules = if kind == PriceValuationKindV1::UsageEstimate {
                        if let Some(mut rules) = official::rules(
                            &documented,
                            &currency,
                            catalog.official_snapshot().data_revision,
                        ) {
                            // Enriched components retain the exact current legacy base identity.
                            let bases = rates
                                .iter()
                                .filter(|r| r.currency == currency && r.schedule.is_none())
                                .collect::<Vec<_>>();
                            if let [base] = bases.as_slice() {
                                rules[0].rule_ref.id = base.price_rate_id.clone();
                            }
                            rules
                        } else {
                            rates
                                .iter()
                                .filter(|r| r.currency == currency)
                                .map(|r| {
                                    Ok(CatalogPriceRuleV2 {
                                        rule_ref: PriceFactRefV1 {
                                            id: r.price_rate_id.clone(),
                                            revision: r.revision,
                                            digest: CanonicalDigest::of(*r)
                                                .map_err(|_| ControlReadError::Corrupt)?,
                                        },
                                        rates: TokenRatesV1::from_legacy(
                                            r.input_micros_per_million,
                                            r.output_micros_per_million,
                                        ),
                                        schedule: r.schedule.clone(),
                                    })
                                })
                                .collect::<Result<Vec<_>, ControlReadError>>()?
                        }
                    } else {
                        vec![]
                    };
                    let legacy_overrides = if kind == PriceValuationKindV1::UsageEstimate
                        && offer.is_some()
                    {
                        legacy
                            .iter()
                            .filter(|o| {
                                Some(o.offer_ref.as_str()) == binding.offer_ref.as_deref()
                                    && o.model_configuration_id == binding.model_configuration_id
                                    && o.currency == currency
                                    && o.target_rule_id.as_ref().is_none_or(|id| {
                                        catalog_rules.iter().any(|r| &r.rule_ref.id == id)
                                    })
                            })
                            .cloned()
                            .collect()
                    } else {
                        vec![]
                    };
                    let source_override = overrides.iter().find(|o| o.target == target).cloned();
                    entries.push(PriceIndexEntryV1 {
                        target,
                        actual_offer_ref: offer.map(|o| o.offer_id.clone()),
                        reference_model_offer_ref: None,
                        catalog_rules,
                        legacy_overrides,
                        source_override,
                    });
                }
            }
        }
        Ok(PriceControlFactsV1 {
            configuration_revision,
            revisions,
            source_bindings,
            entries,
            catalog_refs,
            effective: self.price_snapshot.capture_current_price_snapshot(),
            evaluated_at: self.now_ms()?.div_euclid(1000),
            catalog_model_ids,
        })
    }
}
pub(super) struct PreparedCatalogPrices {
    snapshot: PriceSnapshot,
    revisions: RevisionSetV1,
    previous_price: Option<PriceGenerationRefV1>,
}
impl LocalControlAdapter {
    pub(super) fn prepare_catalog_prices(
        &self,
        catalog: &crate::model_feed::ReleaseCatalogSnapshot,
    ) -> Result<PreparedCatalogPrices, String> {
        let stores = self.stores_lock().map_err(|e| e.to_string())?;
        let revisions = stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .map_err(|e| e.to_string())?;
        let facts = self
            .price_facts_for(&stores, None, catalog)
            .map_err(|_| "prices.candidate.facts")?;
        drop(stores);
        let snapshot = PriceSnapshot::build(
            facts.configuration_revision,
            facts.catalog_refs,
            facts.entries,
        )
        .map_err(|_| "prices.candidate.build")?;
        Ok(PreparedCatalogPrices {
            snapshot,
            revisions,
            previous_price: self
                .price_snapshot
                .capture_current_price_snapshot()
                .generation_ref()
                .cloned(),
        })
    }
    pub(super) fn activate_catalog_prices(
        &self,
        catalog: &crate::model_feed::ReleaseCatalogSnapshot,
        prepared: PreparedCatalogPrices,
        commit: &mut dyn FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let stores = self.stores_lock().map_err(|e| e.to_string())?;
        if stores
            .control()
            .current_revisions(&WorkspaceId::default())
            .map_err(|e| e.to_string())?
            != prepared.revisions
            || self
                .price_snapshot
                .capture_current_price_snapshot()
                .generation_ref()
                != prepared.previous_price.as_ref()
        {
            return Err("prices.candidate.dependencies_changed".into());
        }
        commit()?;
        stores
            .control()
            .activate_model_feed_generation(
                &WorkspaceId::default(),
                catalog.generation,
                catalog.accepted_sequence,
            )
            .map_err(|e| e.to_string())?;
        if self
            .price_snapshot
            .install(prepared.snapshot, prepared.previous_price.as_ref())
            .is_err()
        {
            self.price_snapshot.invalidate_current();
            return Err("prices.candidate.restart_required".into());
        }
        Ok(())
    }
    pub(super) fn rebuild_price_snapshot(&self) -> PortResult<PriceGenerationRefV1> {
        let facts = self
            .price_control_facts()
            .map_err(|_| PortError::new(PortErrorCode::Unavailable, "prices.snapshot.facts"))?;
        let snapshot = PriceSnapshot::build(
            facts.configuration_revision,
            facts.catalog_refs,
            facts.entries,
        )
        .map_err(|_| PortError::new(PortErrorCode::InvalidData, "prices.snapshot.build"))?;
        let generation = snapshot.generation_ref().clone();
        let current = self.price_snapshot.capture_current_price_snapshot();
        if current.generation_ref() == Some(&generation) {
            return Ok(generation);
        }
        // Serialize the final generation check and installation with feed activation. An old
        // builder must not reinstall withdrawn default prices after the slot was invalidated.
        let stores = self.stores_lock()?;
        if stores
            .control()
            .current_revisions(&WorkspaceId::default())?
            != facts.revisions
        {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "prices.feed.changed",
            ));
        }
        self.price_snapshot
            .install(snapshot, current.generation_ref())
            .map_err(|_| PortError::new(PortErrorCode::Conflict, "prices.snapshot.install"))?;
        Ok(generation)
    }
    pub(super) fn install_committed_price_snapshot(
        &self,
        operation: &OperationV1,
    ) -> PortResult<PriceGenerationRefV1> {
        let Some(change) = operation.plan.source_price_change() else {
            return self.rebuild_price_snapshot();
        };
        let persisted: Option<SourcePriceOverrideV1> = self
            .stores_lock()?
            .control()
            .source_price_override(&change.target)?;
        if persisted != Some(change.after) {
            return Err(PortError::new(
                PortErrorCode::Conflict,
                "prices.operation.committed",
            ));
        }
        #[cfg(test)]
        if tests::take_install_failure() {
            return Err(PortError::new(
                PortErrorCode::Unavailable,
                "prices.test.after_commit",
            ));
        }
        self.rebuild_price_snapshot()
    }
}

#[cfg(test)]
mod tests;
