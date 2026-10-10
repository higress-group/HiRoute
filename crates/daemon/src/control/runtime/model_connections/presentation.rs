//! Safe catalog and price facts captured before the management storage lock.

use super::LocalControlAdapter;
use hiroute_application::compute_management::{
    ComputeCandidateCapabilityFactsV2, ComputeCandidateFactBasisV2, ComputeCandidateFactValueV2,
    ComputeCandidateFactsV2, ComputeCandidateModelFactsV2, ComputeCredentialBindingV2,
    ComputeManagementModelPresentationFactV1, ComputeManagementModelRuntimeAvailabilityFactV1,
    ComputeManagementPresentationFactsV1, ComputeManagementSourcePresentationFactV1,
};
use hiroute_application::control::ComputeManagementControlError;
use hiroute_application::prices::SourcePriceControlPort;
use hiroute_application_api::{
    ComputeConnectionAccessKindV1, ComputeConnectionIdentityV1, ComputePriceContextV1,
};
use hiroute_cpa_bridge::{
    CpaAccountKind, CpaSubscriptionAvailability, cpa_subscription_availability,
};
use hiroute_domain::{
    AuthenticationKind, BillingClass, CanonicalDigest, ComputeManagedModelV2,
    ComputeManagementFactBasisV2, ComputeManagementMembershipV2, ComputeManagementProvenanceV2,
    ComputeManagementRepositoryPort, ComputeManagementSourceV2, ConnectionOrigin,
    ConnectorRuntimeKind, InventoryDisposition, MaterializationState, PriceBillingContextV1,
    PriceModelIdentityV1, TokenRateV1, WorkspaceId,
};
use hiroute_integrations::{CpaRegisteredSourceV1, TrustedReleaseCatalog};

use crate::control::runtime::subscriptions::maintenance::SubscriptionMaintenancePresentation;

enum ConnectorRuntimeRead {
    Sources(Vec<CpaRegisteredSourceV1>),
    AuthenticationRequired,
    RuntimeUnavailable,
}

impl LocalControlAdapter {
    pub(super) fn compute_management_presentation_facts(
        &self,
    ) -> Result<ComputeManagementPresentationFactsV1, ComputeManagementControlError> {
        let prices = self
            .price_control_facts()
            .map_err(|_| ComputeManagementControlError::Unavailable)?;
        let catalog = self
            .release_catalog
            .as_ref()
            .ok_or(ComputeManagementControlError::Unavailable)?;
        let evaluated_at_ms = prices
            .evaluated_at
            .checked_mul(1_000)
            .ok_or(ComputeManagementControlError::Corrupt)?;
        let mut source_facts = std::collections::BTreeMap::new();
        let mut model_facts = std::collections::BTreeMap::new();
        // Direct Registered connections do not necessarily have a legacy price projection. Seed
        // every saved row from its durable identity and the verified catalog, then enrich any
        // matching rows with price-controller facts below.
        let management = self
            .stores_lock()
            .map_err(|_| ComputeManagementControlError::Unavailable)?
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .map_err(super::map_port)?;
        let complete = management.revisions == prices.revisions;
        let mut connector_runtime = std::collections::BTreeMap::new();
        for source in &management.sources {
            if source.state == MaterializationState::Ready
                && let ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } =
                    &source.provenance
                && let Some(kind) = CpaAccountKind::from_connector(connector_id)
            {
                connector_runtime
                    .entry(kind)
                    .or_insert_with(|| connector_runtime_read(self, kind));
            }
        }
        for source in &management.sources {
            let resolved = match &source.provenance {
                ComputeManagementProvenanceV2::Registered {
                    connection_option_id,
                    ..
                } => {
                    match catalog.registry().resolve_option(connection_option_id) {
                        Ok(resolved)
                            if super::registered_source_matches_current_option(
                                source, &resolved,
                            ) =>
                        {
                            Some((
                                identity_from_option(connection_option_id, &resolved.option),
                                resolved.option.billing_class,
                            ))
                        }
                        // A Registered identity is catalog-owned. Omitting its facts lets the
                        // management projection retain the row but mark only this source partial.
                        Ok(_) | Err(_) => None,
                    }
                }
                ComputeManagementProvenanceV2::UserConfigured { .. } => Some((
                    ComputeConnectionIdentityV1 {
                        access_kind: ComputeConnectionAccessKindV1::Api,
                        connection_option_id: None,
                        product_label: Some(source.display_name.clone()),
                    },
                    BillingClass::Unknown,
                )),
                // Connector ownership alone is insufficient. A saved native subscription may be
                // presented before publication only when its retained validation, successful
                // save and current client-bundled catalog still agree exactly.
                ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } => {
                    if let Some(kind) = CpaAccountKind::from_connector(connector_id) {
                        self.retained_subscription_candidate_for_source(source)?
                            .filter(|checked| {
                                self.subscription_logical_target(kind).is_ok_and(|target| {
                                    trusted_subscription(catalog, source, checked, &target, kind)
                                })
                            })
                            .and_then(|_| {
                                catalog
                                    .registry()
                                    .resolve_option(kind.connection_option_id())
                                    .ok()
                            })
                            .map(|resolved| {
                                (
                                    identity_from_option(
                                        kind.connection_option_id(),
                                        &resolved.option,
                                    ),
                                    resolved.option.billing_class,
                                )
                            })
                    } else {
                        None
                    }
                }
            };
            let Some((identity, billing_class)) = resolved else {
                continue;
            };
            source_facts.insert(
                source.source_id.clone(),
                ComputeManagementSourcePresentationFactV1 {
                    source_id: source.source_id.clone(),
                    identity,
                },
            );
            for model in &source.models {
                let maintenance = self.subscription_maintenance_presentation(&source.source_id);
                model_facts.insert(
                    model.binding_id.clone(),
                    ComputeManagementModelPresentationFactV1 {
                        binding_id: model.binding_id.clone(),
                        billing_class,
                        price_contexts: Vec::new(),
                        runtime_availability: connector_model_runtime_availability(
                            catalog,
                            source,
                            model,
                            CpaAccountKind::from_candidate(&source.last_candidate_ref)
                                .and_then(|kind| connector_runtime.get(&kind)),
                            maintenance,
                        ),
                    },
                );
            }
        }
        for binding in &prices.source_bindings {
            let Some(management_source) = management.sources.iter().find(|candidate| {
                candidate.source_id == binding.source_id
                    && candidate
                        .models
                        .iter()
                        .any(|model| model.binding_id == binding.binding_id)
            }) else {
                continue;
            };
            let source_identity_is_trusted =
                source_facts.get(&binding.source_id).is_some_and(|fact| {
                    match binding.connection_option_id.as_deref() {
                        Some(connection_option_id) => {
                            fact.identity.connection_option_id.as_deref()
                                == Some(connection_option_id)
                                || (fact.identity.access_kind == ComputeConnectionAccessKindV1::Api
                                    && fact.identity.connection_option_id.is_none())
                        }
                        None => true,
                    }
                });
            if !source_identity_is_trusted {
                continue;
            }

            let mut price_contexts = prices
                .entries
                .iter()
                .filter(|entry| {
                    entry.target.source_id == binding.source_id
                        && match &entry.target.model_identity {
                            PriceModelIdentityV1::CatalogModel(id)
                            | PriceModelIdentityV1::LocalModel(id) => {
                                id == &binding.model_configuration_id
                            }
                        }
                })
                .filter_map(|entry| {
                    let quote = prices
                        .effective
                        .freeze_price(
                            &entry.target,
                            prices.evaluated_at,
                            PriceBillingContextV1::StandardTokens,
                        )
                        .ok()?;
                    matches!(quote.rates.input_uncached, TokenRateV1::Known { .. }).then_some(())?;
                    matches!(quote.rates.output, TokenRateV1::Known { .. }).then_some(())?;
                    Some(ComputePriceContextV1 {
                        currency: entry.target.currency.clone(),
                        valuation_kind: entry.target.valuation_kind,
                    })
                })
                .collect::<Vec<_>>();
            price_contexts.sort();
            price_contexts.dedup();
            model_facts
                .entry(binding.binding_id.clone())
                .and_modify(|fact| {
                    fact.billing_class = binding.billing_class;
                    fact.price_contexts = price_contexts.clone();
                })
                .or_insert(ComputeManagementModelPresentationFactV1 {
                    binding_id: binding.binding_id.clone(),
                    billing_class: binding.billing_class,
                    price_contexts,
                    runtime_availability: management_source
                        .models
                        .iter()
                        .find(|model| model.binding_id == binding.binding_id)
                        .map_or(
                            ComputeManagementModelRuntimeAvailabilityFactV1::BindingAndKeys,
                            |model| {
                                connector_model_runtime_availability(
                                    catalog,
                                    management_source,
                                    model,
                                    CpaAccountKind::from_candidate(
                                        &management_source.last_candidate_ref,
                                    )
                                    .and_then(|kind| connector_runtime.get(&kind)),
                                    self.subscription_maintenance_presentation(
                                        &management_source.source_id,
                                    ),
                                )
                            },
                        ),
                });
        }
        Ok(ComputeManagementPresentationFactsV1 {
            evaluated_at_ms,
            complete,
            revisions: Some(prices.revisions),
            sources: source_facts.into_values().collect(),
            models: model_facts.into_values().collect(),
        })
    }
}

fn identity_from_option(
    connection_option_id: &str,
    option: &hiroute_domain::ConnectionOptionV1,
) -> ComputeConnectionIdentityV1 {
    ComputeConnectionIdentityV1 {
        access_kind: match option.origin {
            ConnectionOrigin::AgentSubscription => ComputeConnectionAccessKindV1::Subscription,
            ConnectionOrigin::NativeApi | ConnectionOrigin::FreeCatalog => {
                ComputeConnectionAccessKindV1::Api
            }
        },
        connection_option_id: Some(connection_option_id.to_owned()),
        product_label: Some(option.display_name.clone()),
    }
}

fn connector_runtime_read(
    adapter: &LocalControlAdapter,
    kind: CpaAccountKind,
) -> ConnectorRuntimeRead {
    let Some(authority) = adapter
        .cpa_runtime
        .as_ref()
        .and_then(|runtimes| runtimes.for_connector(kind.connector_id()))
    else {
        return ConnectorRuntimeRead::RuntimeUnavailable;
    };
    if authority.inspect_subscription().is_err() {
        return ConnectorRuntimeRead::AuthenticationRequired;
    }
    match authority.discover_registered_sources() {
        Ok(sources) => ConnectorRuntimeRead::Sources(sources),
        Err(error) => match cpa_subscription_availability(Err(&error)) {
            CpaSubscriptionAvailability::NeedsAuthentication => {
                ConnectorRuntimeRead::AuthenticationRequired
            }
            CpaSubscriptionAvailability::Ready
            | CpaSubscriptionAvailability::Stopped
            | CpaSubscriptionAvailability::RuntimeUnavailable
            | CpaSubscriptionAvailability::ArtifactUnavailable => {
                ConnectorRuntimeRead::RuntimeUnavailable
            }
        },
    }
}

fn connector_model_runtime_availability(
    catalog: &TrustedReleaseCatalog,
    source: &ComputeManagementSourceV2,
    model: &ComputeManagedModelV2,
    runtime: Option<&ConnectorRuntimeRead>,
    maintenance: Option<SubscriptionMaintenancePresentation>,
) -> ComputeManagementModelRuntimeAvailabilityFactV1 {
    if let Some(maintenance) = maintenance {
        return match maintenance {
            SubscriptionMaintenancePresentation::Updating => {
                ComputeManagementModelRuntimeAvailabilityFactV1::SubscriptionUpdating
            }
            SubscriptionMaintenancePresentation::AuthenticationRequired => {
                ComputeManagementModelRuntimeAvailabilityFactV1::AuthenticationRequired
            }
            SubscriptionMaintenancePresentation::RuntimeUnavailable => {
                ComputeManagementModelRuntimeAvailabilityFactV1::RuntimeUnavailable
            }
        };
    }
    if !model.execution_eligible {
        return ComputeManagementModelRuntimeAvailabilityFactV1::ModelNotAllowed;
    }
    let ComputeManagementProvenanceV2::ConnectorOwned {
        connector_id,
        account_ref,
    } = &source.provenance
    else {
        return ComputeManagementModelRuntimeAvailabilityFactV1::BindingAndKeys;
    };
    match runtime {
        Some(ConnectorRuntimeRead::AuthenticationRequired) => {
            ComputeManagementModelRuntimeAvailabilityFactV1::AuthenticationRequired
        }
        Some(ConnectorRuntimeRead::Sources(sources)) => {
            let available = sources.iter().any(|registered| {
                let exact_source = registered.source.connector_id == *connector_id
                    && CpaAccountKind::from_connector(connector_id).is_some_and(|kind| {
                        registered.source.connection_option_id == kind.connection_option_id()
                    })
                    && registered.source.identity.account_subject_ref == *account_ref;
                exact_source
                    && if let Some(model_configuration_id) =
                        model.catalog_configuration_id.as_deref()
                    {
                        registered.inventory.iter().any(|inventory| {
                            inventory.disposition == InventoryDisposition::CatalogMatched
                                && inventory.model_configuration_id.as_deref()
                                    == Some(model_configuration_id)
                                && inventory.upstream_model_id == model.upstream_model_id
                        }) && catalog
                            .cpa_compute_projection_ids(registered, model_configuration_id)
                            .is_ok()
                    } else {
                        runtime_fallback_saved_model(model)
                            && catalog
                                .runtime_fallback_allows_observed_text(&model.upstream_model_id)
                            && registered.inventory.iter().any(|inventory| {
                                inventory.disposition == InventoryDisposition::InventoryOnly
                                    && inventory.model_configuration_id.is_none()
                                    && inventory.upstream_model_id == model.upstream_model_id
                            })
                    }
            });
            if available {
                ComputeManagementModelRuntimeAvailabilityFactV1::Available
            } else if !sources.iter().any(|registered| {
                registered.source.connector_id == *connector_id
                    && CpaAccountKind::from_connector(connector_id).is_some_and(|kind| {
                        registered.source.connection_option_id == kind.connection_option_id()
                    })
                    && registered.source.identity.account_subject_ref == *account_ref
            }) {
                ComputeManagementModelRuntimeAvailabilityFactV1::AuthenticationRequired
            } else {
                ComputeManagementModelRuntimeAvailabilityFactV1::ModelNotAllowed
            }
        }
        Some(ConnectorRuntimeRead::RuntimeUnavailable) | None => {
            ComputeManagementModelRuntimeAvailabilityFactV1::RuntimeUnavailable
        }
    }
}

fn trusted_subscription(
    catalog: &TrustedReleaseCatalog,
    source: &ComputeManagementSourceV2,
    checked: &ComputeCandidateFactsV2,
    logical_target: &hiroute_application_api::ComputeCandidateTargetV2,
    kind: CpaAccountKind,
) -> bool {
    let Ok(resolved) = catalog.resolve_connection_option(kind.connection_option_id()) else {
        return false;
    };
    if resolved.option.origin != ConnectionOrigin::AgentSubscription
        || resolved.option.billing_class != BillingClass::Subscription
        || resolved.connector.connector_id != kind.connector_id()
        || resolved.connector.runtime_kind != ConnectorRuntimeKind::CpaBridge
        || resolved.connector.authentication != AuthenticationKind::ConnectorOwnedOpaque
        || checked.target.as_ref() != Some(logical_target)
    {
        return false;
    }
    let (
        ComputeManagementProvenanceV2::ConnectorOwned {
            connector_id,
            account_ref,
        },
        ComputeCredentialBindingV2::CpaOwned {
            account_ref: binding_account,
            validation,
        },
    ) = (&source.provenance, &checked.credential_binding)
    else {
        return false;
    };
    if connector_id != kind.connector_id()
        || account_ref != binding_account
        || checked.validation.as_ref() != Some(validation)
    {
        return false;
    }
    source.models.iter().all(|saved| {
        let checked = checked
            .models
            .iter()
            .find(|candidate| candidate.model_ref == saved.model_ref);
        if saved.execution_eligible {
            checked
                .is_some_and(|candidate| trusted_subscription_model(catalog, &resolved, candidate))
        } else {
            checked.is_none_or(|candidate| !candidate.selectable || candidate.reason.is_some())
        }
    })
}

fn trusted_subscription_model(
    catalog: &TrustedReleaseCatalog,
    resolved: &hiroute_domain::ResolvedConnectionOptionV1,
    candidate: &ComputeCandidateModelFactsV2,
) -> bool {
    let Some(model_configuration_id) = candidate.catalog_configuration_id.as_deref() else {
        return candidate.membership == hiroute_application_api::ComputeModelMembershipV2::Observed
            && candidate.selectable
            && candidate.reason.is_none()
            && catalog.runtime_fallback_allows_observed_text(&candidate.upstream_model_id)
            && candidate_runtime_fallback(&candidate.capabilities);
    };
    let Some(definition) = catalog.model_data().model(model_configuration_id) else {
        return false;
    };
    let capabilities = catalog
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .filter(|capability| {
            capability.model_configuration_id == model_configuration_id
                && capability.connector_id == resolved.connector.connector_id
                && capability.connector_revision == resolved.connector.revision
                && capability.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                && capability.endpoint_profile_revision == resolved.endpoint_profile.revision
                && resolved
                    .endpoint_profile
                    .protocol_endpoints
                    .iter()
                    .any(|endpoint| endpoint.protocol == capability.upstream_protocol)
                && capability.upstream_model_id == candidate.upstream_model_id
        })
        .collect::<Vec<_>>();
    let offers = catalog
        .model_data()
        .offers
        .iter()
        .filter(|offer| {
            offer.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                && offer.endpoint_profile_revision == resolved.endpoint_profile.revision
                && offer.billing_class == BillingClass::Subscription
                && offer
                    .model_configuration_ids
                    .iter()
                    .any(|id| id == model_configuration_id)
        })
        .collect::<Vec<_>>();
    if capabilities.len() != 1
        || offers.len() != 1
        || candidate.model_ref != format!("cpa-model/{}", candidate.upstream_model_id)
        || candidate.display_name != definition.display_name
        || candidate.membership != hiroute_application_api::ComputeModelMembershipV2::Observed
        || !candidate.selectable
        || candidate.reason.is_some()
        || !registered_fact_matches(&candidate.capabilities.tool, &definition.capabilities.tool)
        || !registered_fact_matches(
            &candidate.capabilities.vision,
            &definition.capabilities.vision,
        )
        || !registered_fact_matches(
            &candidate.capabilities.streaming,
            &definition.capabilities.streaming,
        )
        || !registered_fact_matches(
            &candidate.capabilities.context_tokens,
            &definition.capabilities.context_tokens,
        )
        || !registered_fact_matches(
            &candidate.capabilities.max_output_tokens,
            &definition.capabilities.max_output_tokens,
        )
    {
        return false;
    }
    let native_reasoning = catalog
        .native_reasoning()
        .iter()
        .find(|model| model.model_configuration_id == model_configuration_id)
        .map(|model| model.capability.clone());
    let reasoning_matches = match native_reasoning.as_ref() {
        Some(reasoning) => {
            registered_fact_matches(&candidate.capabilities.native_reasoning, reasoning)
        }
        None => {
            candidate.capabilities.native_reasoning.value.is_none()
                && candidate.capabilities.native_reasoning.basis
                    == ComputeCandidateFactBasisV2::Unknown
        }
    };
    let digest = CanonicalDigest::of(&(
        "hiroute.cpa-subscription-catalog-capability/v1",
        &capabilities[0].evidence_digest,
        definition,
        &native_reasoning,
    ));
    reasoning_matches && digest.is_ok_and(|digest| digest == candidate.capability_evidence_digest)
}

fn candidate_runtime_fallback(capabilities: &ComputeCandidateCapabilityFactsV2) -> bool {
    capabilities.tool.value == Some(true)
        && capabilities.vision.value == Some(false)
        && capabilities.streaming.value == Some(true)
        && capabilities.context_tokens.value == Some(131_072)
        && capabilities.max_output_tokens.value == Some(8_192)
        && capabilities.native_reasoning.value
            == Some(hiroute_domain::NativeReasoningCapabilityV1::Fixed {
                profile: "non-thinking".into(),
            })
        && [
            capabilities.tool.basis,
            capabilities.vision.basis,
            capabilities.streaming.basis,
            capabilities.context_tokens.basis,
            capabilities.max_output_tokens.basis,
            capabilities.native_reasoning.basis,
        ]
        .into_iter()
        .all(|basis| basis == ComputeCandidateFactBasisV2::RuntimeFallback)
}

fn runtime_fallback_saved_model(model: &ComputeManagedModelV2) -> bool {
    model.membership == ComputeManagementMembershipV2::Observed
        && model.catalog_configuration_id.is_none()
        && [
            model.capabilities.tool.basis,
            model.capabilities.vision.basis,
            model.capabilities.streaming.basis,
            model.capabilities.context_tokens.basis,
            model.capabilities.max_output_tokens.basis,
            model.capabilities.native_reasoning.basis,
        ]
        .into_iter()
        .all(|basis| basis == ComputeManagementFactBasisV2::RuntimeFallback)
}

fn registered_fact_matches<T: Eq>(fact: &ComputeCandidateFactValueV2<T>, expected: &T) -> bool {
    fact.value.as_ref() == Some(expected)
        && fact.basis == ComputeCandidateFactBasisV2::RegisteredCatalog
}
