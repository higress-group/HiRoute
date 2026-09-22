//! Client-bound catalog intersection for non-secret filesystem discovery facts.

use std::collections::BTreeMap;

use hiroute_domain::{
    AuthenticationKind, BillingClass, COMPUTE_CONTROL_PROJECTION_SCHEMA_V1,
    COMPUTE_STATE_SCHEMA_V1, CanonicalDigest, ComputeCatalogProvenanceV1,
    ComputeControlProjectionV1, ComputeInventorySnapshotV1, ComputeProjectionExpectationV1,
    ComputeScannerEvidenceV1, ComputeSourceV1, ConnectionOrigin, ConnectorRuntimeKind,
    CredentialPoolIdentityV1, CredentialPoolV1, InventoryDisposition, MaterializationState,
    ModelEndpointCapabilityV1, ObservedModelV1, OfferV1, PreparedComputeProjectionV1,
    ResolvedConnectionOptionV1, SourceBindingV1, SourceIdentityV1, SourceOrigin, UpstreamProtocol,
};
use thiserror::Error;

use super::{CpaRegisteredSourceV1, ReleaseVerificationError, TrustedReleaseCatalog};

/// Exact scanner facts already stripped of Secret bytes. This adapter accepts no URL or model
/// authority from the caller: every field is intersected with the verified Release catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredComputeDiscoveryFactV1 {
    pub agent_id: String,
    pub scanner_id: String,
    pub scanner_version: String,
    pub discovered_source_ref: String,
    pub configuration_revision: u64,
    pub connection_option_id: String,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub registered_base_url: String,
    pub observed_model_id: String,
    pub model_configuration_id: String,
    pub protected_credential_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredComputeCandidateV1 {
    pub fact: RegisteredComputeDiscoveryFactV1,
    pub billing_class: BillingClass,
    pub authentication: AuthenticationKind,
    pub inventory_eligible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredConnectionOptionFactV1 {
    pub connection_option_id: String,
    pub display_name: String,
    pub connector_id: String,
    pub connector_revision: u64,
    pub endpoint_profile_id: String,
    pub endpoint_profile_revision: u64,
    pub origin: ConnectionOrigin,
    pub billing_class: BillingClass,
    pub authentication: AuthenticationKind,
    pub model_configuration_ids: Vec<String>,
    pub registered_check_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComputeProjectionIdsV1 {
    pub source_id: String,
    pub binding_id: String,
    pub endpoint_profile_id: String,
    /// Exact pre-v8 identity for this trusted discovery fact. This is upgrade provenance only;
    /// it is never accepted from a Local Control request or exposed as a product resource ID.
    legacy_v7_source_id: String,
}

impl ComputeProjectionIdsV1 {
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub fn endpoint_profile_id(&self) -> &str {
        &self.endpoint_profile_id
    }

    pub fn legacy_v7_source_id(&self) -> &str {
        &self.legacy_v7_source_id
    }
}

impl TrustedReleaseCatalog {
    pub fn compute_catalog_provenance(
        &self,
    ) -> Result<ComputeCatalogProvenanceV1, ComputeControlPlaneError> {
        let manifest = self.release_facts_manifest();
        let (registry_key, registry_sequence, registry_digest) = self.registry_provenance();
        let (model_key, model_sequence, model_digest) = self.model_data_provenance();
        if registry_key != model_key
            || registry_sequence != model_sequence
            || manifest.catalog_id != registry_key
            || manifest.sequence != registry_sequence
        {
            return Err(ComputeControlPlaneError::CatalogProvenance);
        }
        Ok(ComputeCatalogProvenanceV1 {
            product_release: manifest.product_release.clone(),
            catalog_binding_id: manifest.catalog_id.clone(),
            release_sequence: manifest.sequence,
            connector_registry_version: self.registry().registry_version.clone(),
            connector_registry_digest: registry_digest.clone(),
            model_data_bundle_version: self.model_data().bundle_version.clone(),
            model_data_digest: model_digest.clone(),
            cross_reference_digest: manifest.cross_reference_digest.clone(),
        })
    }

    pub fn registered_connection_options(
        &self,
    ) -> Result<Vec<RegisteredConnectionOptionFactV1>, ComputeControlPlaneError> {
        self.compute_catalog_provenance()?;
        let mut options = Vec::new();
        for option in &self.registry().connection_options {
            let resolved = self
                .resolve_connection_option(&option.connection_option_id)
                .map_err(ComputeControlPlaneError::Release)?;
            let exact_current_capability = |capability: &ModelEndpointCapabilityV1| {
                capability.endpoint_profile_id == resolved.endpoint_profile.endpoint_profile_id
                    && capability.endpoint_profile_revision == resolved.endpoint_profile.revision
                    && capability.connector_id == resolved.connector.connector_id
                    && capability.connector_revision == resolved.connector.revision
                    && resolved
                        .endpoint_profile
                        .protocol_endpoints
                        .iter()
                        .any(|endpoint| {
                            endpoint.authentication_semantics.is_some()
                                && capability.protocol_endpoint_id == endpoint.protocol_endpoint_id
                                && capability.upstream_protocol == endpoint.protocol
                        })
            };
            let mut model_configuration_ids = self
                .model_data()
                .model_endpoint_capabilities
                .iter()
                .filter(|capability| exact_current_capability(capability))
                .map(|capability| capability.model_configuration_id.clone())
                .collect::<Vec<_>>();
            model_configuration_ids.sort();
            model_configuration_ids.dedup();
            let has_configured_endpoint = resolved
                .endpoint_profile
                .protocol_endpoints
                .iter()
                .any(|endpoint| endpoint.authentication_semantics.is_some());
            let registered_check_available = option.origin != ConnectionOrigin::AgentSubscription
                && resolved.connector.runtime_kind == ConnectorRuntimeKind::BuiltinNative
                && resolved.connector.authentication == AuthenticationKind::ProviderApiKey
                && has_configured_endpoint;
            options.push(RegisteredConnectionOptionFactV1 {
                connection_option_id: option.connection_option_id.clone(),
                display_name: option.display_name.clone(),
                connector_id: resolved.connector.connector_id.clone(),
                connector_revision: resolved.connector.revision,
                endpoint_profile_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
                endpoint_profile_revision: resolved.endpoint_profile.revision,
                origin: option.origin,
                billing_class: option.billing_class,
                authentication: resolved.connector.authentication,
                model_configuration_ids,
                registered_check_available,
            });
        }
        options.sort_by(|left, right| left.connection_option_id.cmp(&right.connection_option_id));
        Ok(options)
    }

    pub fn authorize_compute_discovery(
        &self,
        fact: RegisteredComputeDiscoveryFactV1,
    ) -> Result<RegisteredComputeCandidateV1, ComputeControlPlaneError> {
        self.compute_catalog_provenance()?;
        if !valid_id(&fact.agent_id)
            || !valid_id(&fact.scanner_id)
            || !valid_id(&fact.scanner_version)
            || !valid_id(&fact.discovered_source_ref)
            || fact.configuration_revision == 0
        {
            return Err(ComputeControlPlaneError::DiscoveryMismatch);
        }
        let resolved = self
            .resolve_connection_option(&fact.connection_option_id)
            .map_err(ComputeControlPlaneError::Release)?;
        let endpoint = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|endpoint| endpoint.protocol == UpstreamProtocol::Messages)
            .ok_or(ComputeControlPlaneError::DiscoveryMismatch)?;
        let exact_messages_url = format!("{}{}", endpoint.base_url, endpoint.request_path);
        let observed_messages_url = format!("{}/v1/messages", fact.registered_base_url);
        let capability = self
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .find(|capability| {
                capability.endpoint_profile_id == fact.endpoint_profile_id
                    && capability.model_configuration_id == fact.model_configuration_id
                    && capability.upstream_model_id == fact.observed_model_id
                    && capability.protocol_endpoint_id == endpoint.protocol_endpoint_id
            })
            .ok_or(ComputeControlPlaneError::DiscoveryMismatch)?;
        let inventory_eligible = exact_messages_url == observed_messages_url
            && resolved.endpoint_profile.endpoint_profile_id == fact.endpoint_profile_id
            && resolved.endpoint_profile.revision == fact.endpoint_profile_revision
            && capability.connector_id == resolved.connector.connector_id
            && capability.connector_revision == resolved.connector.revision;
        if !inventory_eligible {
            return Err(ComputeControlPlaneError::DiscoveryMismatch);
        }
        Ok(RegisteredComputeCandidateV1 {
            fact,
            billing_class: resolved.option.billing_class,
            authentication: resolved.connector.authentication,
            inventory_eligible,
        })
    }

    pub fn prepare_compute_projection(
        &self,
        candidate: RegisteredComputeCandidateV1,
        expected: ComputeProjectionExpectationV1,
        explicit_materialization: bool,
    ) -> Result<PreparedComputeProjectionV1, ComputeControlPlaneError> {
        let candidate = self.authorize_compute_discovery(candidate.fact)?;
        if candidate.billing_class.requires_explicit_materialization() && !explicit_materialization
        {
            return Err(ComputeControlPlaneError::ExplicitMaterializationRequired);
        }
        let fact = &candidate.fact;
        let resolved = self
            .resolve_connection_option(&fact.connection_option_id)
            .map_err(ComputeControlPlaneError::Release)?;
        let model_data = self.model_data();
        let messages_endpoint = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|endpoint| endpoint.protocol == UpstreamProtocol::Messages)
            .ok_or(ComputeControlPlaneError::DiscoveryMismatch)?;
        let capability = unique(model_data.model_endpoint_capabilities.iter().filter(
            |capability| {
                capability.endpoint_profile_id == fact.endpoint_profile_id
                    && capability.model_configuration_id == fact.model_configuration_id
                    && capability.upstream_model_id == fact.observed_model_id
                    && capability.protocol_endpoint_id == messages_endpoint.protocol_endpoint_id
            },
        ))?;
        let offer = unique(model_data.offers.iter().filter(|offer| {
            offer.endpoint_profile_id == fact.endpoint_profile_id
                && offer
                    .model_configuration_ids
                    .contains(&fact.model_configuration_id)
                && offer.billing_class == candidate.billing_class
        }))?;
        let scanner_evidence = scanner_evidence_digest(fact)?;
        let identity_revision = expected
            .source_revision
            .checked_add(1)
            .ok_or(ComputeControlPlaneError::RevisionOverflow)?;
        let identity =
            source_identity(fact, &resolved, identity_revision, scanner_evidence.clone());
        let identity_digest = identity
            .digest()
            .map_err(|_| ComputeControlPlaneError::Projection)?;
        let (ids, pool_id) = projection_ids(&identity_digest, fact, capability, offer)?;
        let source_id = ids.source_id;
        let source = ComputeSourceV1 {
            schema: COMPUTE_STATE_SCHEMA_V1.into(),
            source_id: source_id.clone(),
            revision: identity_revision,
            connection_option_id: fact.connection_option_id.clone(),
            connector_id: resolved.connector.connector_id.clone(),
            connector_revision: resolved.connector.revision,
            origin: match resolved.option.origin {
                ConnectionOrigin::NativeApi => SourceOrigin::NativeApi,
                ConnectionOrigin::AgentSubscription => SourceOrigin::Cpa,
                ConnectionOrigin::FreeCatalog => SourceOrigin::ReleaseFree,
            },
            identity_digest,
            identity,
            billing_class: candidate.billing_class,
            state: match candidate.authentication {
                AuthenticationKind::None => MaterializationState::Ready,
                AuthenticationKind::ProviderApiKey => MaterializationState::NeedsCredential,
                AuthenticationKind::ConnectorOwnedOpaque => {
                    MaterializationState::NeedsAuthorization
                }
            },
        };
        self.validate_source(&source, explicit_materialization)
            .map_err(|_| ComputeControlPlaneError::Projection)?;
        let credential_pool_id =
            (candidate.authentication != AuthenticationKind::None).then_some(pool_id);
        let binding = SourceBindingV1 {
            binding_id: ids.binding_id,
            revision: expected
                .binding_revision
                .checked_add(1)
                .ok_or(ComputeControlPlaneError::RevisionOverflow)?,
            source_id: source_id.clone(),
            source_revision: source.revision,
            source_identity_digest: source.identity_digest.clone(),
            model_data_bundle_version: model_data.bundle_version.clone(),
            capability_slice_version: model_data.capability_slice_version.clone(),
            offer_ref: offer.offer_id.clone(),
            offer_evidence_digest: offer.evidence_digest.clone(),
            billing_class: offer.billing_class,
            model_configuration_id: fact.model_configuration_id.clone(),
            upstream_model_id: fact.observed_model_id.clone(),
            capability_id: capability.capability_id.clone(),
            credential_pool_id,
        };
        binding
            .validate(&source, model_data)
            .map_err(|_| ComputeControlPlaneError::Projection)?;
        let credential_pool_identity = if candidate.authentication == AuthenticationKind::None {
            None
        } else {
            Some(
                CredentialPoolIdentityV1::for_resolved_binding(
                    &binding,
                    &source,
                    self.trusted_option(&source.connection_option_id)
                        .map_err(ComputeControlPlaneError::Release)?,
                    model_data,
                )
                .map_err(|_| ComputeControlPlaneError::Projection)?,
            )
        };
        let observed_models = vec![ObservedModelV1 {
            upstream_model_id: fact.observed_model_id.clone(),
            metadata: BTreeMap::new(),
        }];
        let inventory = ComputeInventorySnapshotV1 {
            source_id: source_id.clone(),
            endpoint_profile_id: fact.endpoint_profile_id.clone(),
            inventory_revision: expected
                .inventory_revision
                .checked_add(1)
                .ok_or(ComputeControlPlaneError::RevisionOverflow)?,
            inventory_digest: CanonicalDigest::of(&observed_models)
                .map_err(|_| ComputeControlPlaneError::Encoding)?,
            observed_models,
            // This inventory is a client-bundled catalog intersection, not a live Provider
            // listing. Its timestamp therefore comes from the current endpoint evidence; the
            // opaque scanner revision remains separately bound in `scanner` below.
            captured_at: resolved.endpoint_profile.last_verified_at,
        };
        let projection = ComputeControlProjectionV1 {
            schema: COMPUTE_CONTROL_PROJECTION_SCHEMA_V1.into(),
            source,
            binding,
            inventory,
            credential_pool_identity,
            catalog: self.compute_catalog_provenance()?,
            scanner: ComputeScannerEvidenceV1 {
                scanner_id: fact.scanner_id.clone(),
                scanner_version: fact.scanner_version.clone(),
                discovered_source_ref: fact.discovered_source_ref.clone(),
                configuration_revision: fact.configuration_revision,
                evidence_digest: scanner_evidence,
            },
        };
        let prepared = PreparedComputeProjectionV1 {
            expected,
            desired: projection,
        };
        prepared
            .validate()
            .map_err(|_| ComputeControlPlaneError::Projection)?;
        Ok(prepared)
    }

    pub fn compute_projection_ids(
        &self,
        candidate: &RegisteredComputeCandidateV1,
    ) -> Result<ComputeProjectionIdsV1, ComputeControlPlaneError> {
        let candidate = self.authorize_compute_discovery(candidate.fact.clone())?;
        let fact = &candidate.fact;
        let resolved = self
            .resolve_connection_option(&fact.connection_option_id)
            .map_err(ComputeControlPlaneError::Release)?;
        let model_data = self.model_data();
        let messages_endpoint = resolved
            .endpoint_profile
            .protocol_endpoints
            .iter()
            .find(|endpoint| endpoint.protocol == UpstreamProtocol::Messages)
            .ok_or(ComputeControlPlaneError::DiscoveryMismatch)?;
        let capability = unique(model_data.model_endpoint_capabilities.iter().filter(
            |capability| {
                capability.endpoint_profile_id == fact.endpoint_profile_id
                    && capability.model_configuration_id == fact.model_configuration_id
                    && capability.upstream_model_id == fact.observed_model_id
                    && capability.protocol_endpoint_id == messages_endpoint.protocol_endpoint_id
            },
        ))?;
        let offer = unique(model_data.offers.iter().filter(|offer| {
            offer.endpoint_profile_id == fact.endpoint_profile_id
                && offer
                    .model_configuration_ids
                    .contains(&fact.model_configuration_id)
                && offer.billing_class == candidate.billing_class
        }))?;
        let evidence = scanner_evidence_digest(fact)?;
        let identity = source_identity(fact, &resolved, 1, evidence);
        let identity_digest = identity
            .digest()
            .map_err(|_| ComputeControlPlaneError::Projection)?;
        projection_ids(&identity_digest, fact, capability, offer).map(|value| value.0)
    }

    /// Converts a live, connector-owned CPA account attestation into the same durable projection
    /// used by native connections. The OAuth material remains inside CPA; only its exact opaque
    /// credential reference participates in the transient execution pool below.
    pub fn prepare_cpa_compute_projection(
        &self,
        registered: &CpaRegisteredSourceV1,
        model_configuration_id: &str,
        expected: ComputeProjectionExpectationV1,
        explicit_materialization: bool,
    ) -> Result<PreparedComputeProjectionV1, ComputeControlPlaneError> {
        self.compute_catalog_provenance()?;
        let (resolved, capability, offer, upstream_model_id) =
            self.authorize_cpa_compute(registered, model_configuration_id)?;
        if registered
            .source
            .billing_class
            .requires_explicit_materialization()
            && !explicit_materialization
        {
            return Err(ComputeControlPlaneError::ExplicitMaterializationRequired);
        }
        let source_revision = expected
            .source_revision
            .checked_add(1)
            .ok_or(ComputeControlPlaneError::RevisionOverflow)?;
        let mut source = registered.source.clone();
        source.revision = source_revision;
        source.identity.identity_revision = source_revision;
        source.state = MaterializationState::Ready;
        self.validate_source(&source, explicit_materialization)
            .map_err(|_| ComputeControlPlaneError::Projection)?;

        let (ids, pool_id) = cpa_projection_ids(
            &source.source_id,
            model_configuration_id,
            upstream_model_id,
            capability,
            offer,
            &source.identity.endpoint_profile_id,
        )?;
        let binding = SourceBindingV1 {
            binding_id: ids.binding_id,
            revision: expected
                .binding_revision
                .checked_add(1)
                .ok_or(ComputeControlPlaneError::RevisionOverflow)?,
            source_id: source.source_id.clone(),
            source_revision: source.revision,
            source_identity_digest: source.identity_digest.clone(),
            model_data_bundle_version: self.model_data().bundle_version.clone(),
            capability_slice_version: self.model_data().capability_slice_version.clone(),
            offer_ref: offer.offer_id.clone(),
            offer_evidence_digest: offer.evidence_digest.clone(),
            billing_class: offer.billing_class,
            model_configuration_id: model_configuration_id.to_owned(),
            upstream_model_id: upstream_model_id.to_owned(),
            capability_id: capability.capability_id.clone(),
            credential_pool_id: Some(pool_id),
        };
        binding
            .validate(&source, self.model_data())
            .map_err(|_| ComputeControlPlaneError::Projection)?;
        let pool = CredentialPoolIdentityV1::for_resolved_binding(
            &binding,
            &source,
            self.trusted_option(&source.connection_option_id)
                .map_err(ComputeControlPlaneError::Release)?,
            self.model_data(),
        )
        .map_err(|_| ComputeControlPlaneError::Projection)?;
        let observed_models = registered
            .inventory
            .iter()
            .map(|model| ObservedModelV1 {
                upstream_model_id: model.upstream_model_id.clone(),
                metadata: model.metadata.clone(),
            })
            .collect::<Vec<_>>();
        let inventory = ComputeInventorySnapshotV1 {
            source_id: source.source_id.clone(),
            endpoint_profile_id: source.identity.endpoint_profile_id.clone(),
            inventory_revision: expected
                .inventory_revision
                .checked_add(1)
                .ok_or(ComputeControlPlaneError::RevisionOverflow)?,
            inventory_digest: CanonicalDigest::of(&observed_models)
                .map_err(|_| ComputeControlPlaneError::Encoding)?,
            observed_models,
            captured_at: resolved.endpoint_profile.last_verified_at,
        };
        let scanner_evidence = source
            .identity
            .evidence_refs
            .first()
            .cloned()
            .ok_or(ComputeControlPlaneError::DiscoveryMismatch)?;
        let desired = ComputeControlProjectionV1 {
            schema: COMPUTE_CONTROL_PROJECTION_SCHEMA_V1.into(),
            source,
            binding,
            inventory,
            credential_pool_identity: Some(pool),
            catalog: self.compute_catalog_provenance()?,
            scanner: ComputeScannerEvidenceV1 {
                scanner_id: "connector.cpa.account-discovery".into(),
                scanner_version: resolved.connector.revision.to_string(),
                discovered_source_ref: registered.source.source_id.clone(),
                configuration_revision: registered.credential_ref.generation(),
                evidence_digest: scanner_evidence,
            },
        };
        let prepared = PreparedComputeProjectionV1 { expected, desired };
        prepared
            .validate()
            .map_err(|_| ComputeControlPlaneError::Projection)?;
        Ok(prepared)
    }

    pub fn cpa_compute_projection_ids(
        &self,
        registered: &CpaRegisteredSourceV1,
        model_configuration_id: &str,
    ) -> Result<ComputeProjectionIdsV1, ComputeControlPlaneError> {
        let (_, capability, offer, upstream_model_id) =
            self.authorize_cpa_compute(registered, model_configuration_id)?;
        cpa_projection_ids(
            &registered.source.source_id,
            model_configuration_id,
            upstream_model_id,
            capability,
            offer,
            &registered.source.identity.endpoint_profile_id,
        )
        .map(|value| value.0)
    }

    /// Builds an in-memory pool from the current managed CPA generation. Connector-owned OAuth
    /// material is deliberately not persisted in the native Secret/pool database; every routing
    /// compilation revalidates it against the live CPA authority.
    pub fn transient_cpa_credential_pool(
        &self,
        registered: &CpaRegisteredSourceV1,
        source: &ComputeSourceV1,
        binding: &SourceBindingV1,
    ) -> Result<CredentialPoolV1, ComputeControlPlaneError> {
        let (_, _, _, _) =
            self.authorize_cpa_compute(registered, &binding.model_configuration_id)?;
        if registered.source.source_id != source.source_id
            || registered.credential_ref.owner_scope() != format!("source/{}", source.source_id)
        {
            return Err(ComputeControlPlaneError::DiscoveryMismatch);
        }
        let identity = CredentialPoolIdentityV1::for_resolved_binding(
            binding,
            source,
            self.trusted_option(&source.connection_option_id)
                .map_err(ComputeControlPlaneError::Release)?,
            self.model_data(),
        )
        .map_err(|_| ComputeControlPlaneError::Projection)?;
        let fingerprint = CanonicalDigest::of(&(
            "hiroute.connector-owned-credential/v1",
            &registered.credential_ref,
        ))
        .map_err(|_| ComputeControlPlaneError::Encoding)?;
        identity
            .materialize_first(registered.credential_ref.clone(), fingerprint)
            .map_err(|_| ComputeControlPlaneError::Projection)
    }

    fn authorize_cpa_compute<'a>(
        &'a self,
        registered: &'a CpaRegisteredSourceV1,
        model_configuration_id: &str,
    ) -> Result<
        (
            ResolvedConnectionOptionV1,
            &'a ModelEndpointCapabilityV1,
            &'a OfferV1,
            &'a str,
        ),
        ComputeControlPlaneError,
    > {
        let source = &registered.source;
        self.validate_source(source, true)
            .map_err(|_| ComputeControlPlaneError::DiscoveryMismatch)?;
        let resolved = self
            .resolve_connection_option(&source.connection_option_id)
            .map_err(ComputeControlPlaneError::Release)?;
        if source.origin != SourceOrigin::Cpa
            || source.state != MaterializationState::Ready
            || resolved.connector.runtime_kind != ConnectorRuntimeKind::CpaBridge
            || resolved.connector.authentication != AuthenticationKind::ConnectorOwnedOpaque
            || registered.credential_ref.owner_scope() != format!("source/{}", source.source_id)
            || registered.credential_ref.subject() != format!("connector/{}", source.connector_id)
            || registered.credential_ref.purpose() != "provider-auth"
            || registered.credential_ref.allowed_destinations()
                != &std::collections::BTreeSet::from([format!(
                    "connection-option/{}",
                    source.connection_option_id
                )])
            || registered.credential_ref.generation() == 0
        {
            return Err(ComputeControlPlaneError::DiscoveryMismatch);
        }
        let inventory = unique(registered.inventory.iter().filter(|model| {
            model.disposition == InventoryDisposition::CatalogMatched
                && model.model_configuration_id.as_deref() == Some(model_configuration_id)
        }))?;
        let capability = unique(self.model_data().model_endpoint_capabilities.iter().filter(
            |capability| {
                capability.endpoint_profile_id == source.identity.endpoint_profile_id
                    && capability.model_configuration_id == model_configuration_id
                    && capability.upstream_model_id == inventory.upstream_model_id
                    && capability.connector_id == source.connector_id
                    && capability.connector_revision == source.connector_revision
            },
        ))?;
        let offer = unique(self.model_data().offers.iter().filter(|offer| {
            offer.endpoint_profile_id == source.identity.endpoint_profile_id
                && offer.billing_class == source.billing_class
                && offer
                    .model_configuration_ids
                    .contains(&model_configuration_id.to_owned())
        }))?;
        Ok((resolved, capability, offer, &inventory.upstream_model_id))
    }
}

fn cpa_projection_ids(
    source_id: &str,
    model_configuration_id: &str,
    upstream_model_id: &str,
    capability: &ModelEndpointCapabilityV1,
    offer: &OfferV1,
    endpoint_profile_id: &str,
) -> Result<(ComputeProjectionIdsV1, String), ComputeControlPlaneError> {
    let binding_identity = CanonicalDigest::of(&(
        "hiroute.cpa-source-binding-identity/v1",
        source_id,
        model_configuration_id,
        upstream_model_id,
        &offer.offer_id,
        &capability.capability_id,
    ))
    .map_err(|_| ComputeControlPlaneError::Encoding)?;
    let suffix = digest_suffix(&binding_identity)?;
    Ok((
        ComputeProjectionIdsV1 {
            source_id: source_id.to_owned(),
            binding_id: format!("binding/cpa-{suffix}"),
            endpoint_profile_id: endpoint_profile_id.to_owned(),
            legacy_v7_source_id: source_id.to_owned(),
        },
        format!("pool/cpa-{suffix}"),
    ))
}

fn scanner_evidence_digest(
    fact: &RegisteredComputeDiscoveryFactV1,
) -> Result<CanonicalDigest, ComputeControlPlaneError> {
    CanonicalDigest::of(&(
        "hiroute.compute-scanner-evidence/v1",
        &fact.agent_id,
        &fact.scanner_id,
        &fact.scanner_version,
        &fact.discovered_source_ref,
        fact.configuration_revision,
        &fact.connection_option_id,
        &fact.endpoint_profile_id,
        fact.endpoint_profile_revision,
        &fact.registered_base_url,
        &fact.observed_model_id,
        &fact.model_configuration_id,
        fact.protected_credential_available,
    ))
    .map_err(|_| ComputeControlPlaneError::Encoding)
}

fn source_identity(
    fact: &RegisteredComputeDiscoveryFactV1,
    resolved: &ResolvedConnectionOptionV1,
    identity_revision: u64,
    evidence: CanonicalDigest,
) -> SourceIdentityV1 {
    SourceIdentityV1 {
        identity_revision,
        provider_platform_id: resolved.endpoint_profile.provider_platform_id.clone(),
        service_offering_id: resolved.endpoint_profile.service_offering_id.clone(),
        entitlement_id: resolved.endpoint_profile.entitlement_id.clone(),
        usage_scope: resolved.endpoint_profile.usage_scope.clone(),
        endpoint_profile_id: resolved.endpoint_profile.endpoint_profile_id.clone(),
        endpoint_profile_revision: resolved.endpoint_profile.revision,
        region_id: resolved.endpoint_profile.region_id.clone(),
        account_subject_ref: format!("account/agent/{}", fact.agent_id),
        evidence_refs: vec![evidence],
    }
}

fn projection_ids(
    source_identity_digest: &CanonicalDigest,
    fact: &RegisteredComputeDiscoveryFactV1,
    capability: &ModelEndpointCapabilityV1,
    offer: &OfferV1,
) -> Result<(ComputeProjectionIdsV1, String), ComputeControlPlaneError> {
    let source_suffix = digest_suffix(source_identity_digest)?;
    let source_id = format!("source/agent-{source_suffix}");
    let binding_identity = CanonicalDigest::of(&(
        "hiroute.compute-source-binding-identity/v1",
        &source_id,
        &fact.model_configuration_id,
        &fact.observed_model_id,
        &offer.offer_id,
        &capability.capability_id,
    ))
    .map_err(|_| ComputeControlPlaneError::Encoding)?;
    let binding_suffix = digest_suffix(&binding_identity)?;
    Ok((
        ComputeProjectionIdsV1 {
            source_id,
            binding_id: format!("binding/agent-{binding_suffix}"),
            endpoint_profile_id: fact.endpoint_profile_id.clone(),
            legacy_v7_source_id: legacy_v7_source_id(fact)?,
        },
        format!("pool/agent-{binding_suffix}"),
    ))
}

fn legacy_v7_source_id(
    fact: &RegisteredComputeDiscoveryFactV1,
) -> Result<String, ComputeControlPlaneError> {
    let digest = CanonicalDigest::of(&(
        "hiroute.compute-projection-identity/v1",
        &fact.agent_id,
        &fact.scanner_id,
        &fact.scanner_version,
        &fact.discovered_source_ref,
        &fact.connection_option_id,
        &fact.endpoint_profile_id,
        fact.endpoint_profile_revision,
        &fact.observed_model_id,
        &fact.model_configuration_id,
    ))
    .map_err(|_| ComputeControlPlaneError::Encoding)?;
    Ok(format!("source/agent-{}", digest_suffix(&digest)?))
}

fn unique<'a, T>(
    mut values: impl Iterator<Item = &'a T>,
) -> Result<&'a T, ComputeControlPlaneError> {
    let value = values
        .next()
        .ok_or(ComputeControlPlaneError::DiscoveryMismatch)?;
    if values.next().is_some() {
        return Err(ComputeControlPlaneError::DiscoveryMismatch);
    }
    Ok(value)
}

fn digest_suffix(digest: &CanonicalDigest) -> Result<&str, ComputeControlPlaneError> {
    digest
        .as_str()
        .strip_prefix("sha256:")
        .and_then(|value| value.get(..24))
        .ok_or(ComputeControlPlaneError::Encoding)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b':' | b'-')
        })
}

#[derive(Debug, Error)]
pub enum ComputeControlPlaneError {
    #[error("only a complete client-bundled ReleaseFacts catalog can authorize compute")]
    CatalogUnavailable,
    #[error("client-bundled catalog provenance is inconsistent")]
    CatalogProvenance,
    #[error("scanner facts do not exactly match the client-bundled catalog")]
    DiscoveryMismatch,
    #[error("explicit materialization consent is required")]
    ExplicitMaterializationRequired,
    #[error("compute projection revision overflow")]
    RevisionOverflow,
    #[error("compute projection is inconsistent")]
    Projection,
    #[error("compute projection encoding failed")]
    Encoding,
    #[error(transparent)]
    Release(#[from] ReleaseVerificationError),
}

#[cfg(test)]
mod legacy_lineage_tests {
    use super::*;

    fn discovery() -> RegisteredComputeDiscoveryFactV1 {
        RegisteredComputeDiscoveryFactV1 {
            agent_id: "agent.test".into(),
            scanner_id: "scanner.test".into(),
            scanner_version: "1".into(),
            discovered_source_ref: "agent/settings/test".into(),
            configuration_revision: 1,
            connection_option_id: "provider.test.v1".into(),
            endpoint_profile_id: "endpoint.test".into(),
            endpoint_profile_revision: 1,
            registered_base_url: "https://provider.test.invalid".into(),
            observed_model_id: "upstream-test".into(),
            model_configuration_id: "model.test".into(),
            protected_credential_available: true,
        }
    }

    #[test]
    fn legacy_v7_proof_binds_every_historical_identity_input() {
        let original = discovery();
        let proof = legacy_v7_source_id(&original).unwrap();
        for field in [
            "agent",
            "scanner",
            "scanner-version",
            "source-ref",
            "option",
            "endpoint",
            "endpoint-revision",
            "upstream-model",
            "model-configuration",
        ] {
            let mut changed = original.clone();
            match field {
                "agent" => changed.agent_id.push_str(".other"),
                "scanner" => changed.scanner_id.push_str(".other"),
                "scanner-version" => changed.scanner_version.push_str(".other"),
                "source-ref" => changed.discovered_source_ref.push_str(".other"),
                "option" => changed.connection_option_id.push_str(".other"),
                "endpoint" => changed.endpoint_profile_id.push_str(".other"),
                "endpoint-revision" => changed.endpoint_profile_revision += 1,
                "upstream-model" => changed.observed_model_id.push_str(".other"),
                "model-configuration" => changed.model_configuration_id.push_str(".other"),
                _ => unreachable!(),
            }
            assert_ne!(legacy_v7_source_id(&changed).unwrap(), proof, "{field}");
        }
        let mut non_identity_refresh = original;
        non_identity_refresh.configuration_revision += 1;
        non_identity_refresh
            .registered_base_url
            .push_str("/refreshed");
        non_identity_refresh.protected_credential_available = false;
        assert_eq!(legacy_v7_source_id(&non_identity_refresh).unwrap(), proof);
    }
}

#[cfg(test)]
#[path = "control_plane/tests.rs"]
mod tests;
