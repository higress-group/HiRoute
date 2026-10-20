use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{CanonicalDigest, MetadataCanonicalModelV1, ModelNativeReasoningV1};
use serde::{Deserialize, Serialize};

use super::{FeaturedModel, FeedError, FeedPayload, FeedRecord};
use crate::TrustedReleaseCatalog;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecordAdmission {
    Applied,
    IgnoredEmbedded,
    NewerClient { min_client_version: String },
    Unresolved,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedRecord {
    pub kind: String,
    pub id: String,
    pub outcome: RecordAdmission,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeedCounts {
    pub applied: u64,
    pub ignored_embedded: u64,
    pub newer_client: u64,
    pub unresolved: u64,
}

#[derive(Clone, Debug)]
pub struct FeedAdmissionV1 {
    pub records: Vec<AdmittedRecord>,
    pub counts: FeedCounts,
    pub payload: FeedPayload,
    embedded_digest: CanonicalDigest,
    registry_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelFactOrigin {
    Embedded { client_version: String },
    Feed { sequence: u64, included_on: String },
}

#[derive(Clone, Debug)]
pub struct NewerFeedModel {
    pub model_configuration_id: String,
    pub display_name: String,
    pub publisher_id: String,
    pub capabilities: hiroute_domain::CapabilityFactsV1,
    pub minimum_client: String,
    pub included_on: String,
    pub identity_stability: String,
    pub sequence: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct FeedCatalogFacts {
    pub newer_models: Vec<NewerFeedModel>,
    pub origins: BTreeMap<String, ModelFactOrigin>,
    pub canonical: BTreeMap<String, MetadataCanonicalModelV1>,
    pub native_reasoning: Vec<ModelNativeReasoningV1>,
    pub required_clients: BTreeMap<String, String>,
    pub featured: Vec<FeaturedModel>,
}

/// Classification is shared by the runtime and publication generator. It never accepts registry
/// records and is bound to the embedded catalog used for that classification.
pub fn admit(
    embedded: &TrustedReleaseCatalog,
    payload: &FeedPayload,
    client_version: &str,
) -> Result<FeedAdmissionV1, FeedError> {
    payload.validate()?;
    let version = semver::Version::parse(client_version)
        .map_err(|_| FeedError::validation("invalid client version"))?;
    let mut models = BTreeMap::new();
    for item in &payload.records {
        if let FeedRecord::Model { record, .. } = item {
            let state = if embedded
                .model_data()
                .model(&record.definition.model_configuration_id)
                .is_some()
            {
                RecordAdmission::IgnoredEmbedded
            } else {
                version_admission(item, &version)
            };
            models.insert(record.definition.model_configuration_id.as_str(), state);
        }
    }
    let mut outcomes = Vec::new();
    for item in &payload.records {
        let initial = version_admission(item, &version);
        let outcome = match item {
            FeedRecord::Model { record, .. } => {
                models[record.definition.model_configuration_id.as_str()].clone()
            }
            FeedRecord::ModelEndpointCapability { record, .. } => {
                if embedded
                    .model_data()
                    .model_endpoint_capabilities
                    .iter()
                    .any(|c| {
                        c.capability_id == record.capability_id
                            || (c.endpoint_profile_id == record.endpoint_profile_id
                                && c.upstream_model_id == record.upstream_model_id)
                    })
                {
                    RecordAdmission::IgnoredEmbedded
                } else if let Some(dependent) =
                    dependent_state(&models, &record.model_configuration_id)
                {
                    dependent
                } else if initial != RecordAdmission::Applied {
                    initial
                } else {
                    let connector = embedded.registry().connector(&record.connector_id);
                    let profile = embedded
                        .registry()
                        .endpoint_profile(&record.endpoint_profile_id);
                    let resolved = connector.is_some_and(|c| {
                        c.revision == record.connector_revision
                            && c.endpoint_profile_refs
                                .contains(&record.endpoint_profile_id)
                    }) && profile.is_some_and(|p| {
                        p.revision == record.endpoint_profile_revision
                            && p.protocol_endpoints.iter().any(|e| {
                                e.protocol_endpoint_id == record.protocol_endpoint_id
                                    && e.protocol == record.upstream_protocol
                                    && e.adapter_ref == record.required_adapter_ref
                                    && e.adapter_revision == record.required_adapter_revision
                            })
                    });
                    if resolved {
                        RecordAdmission::Applied
                    } else {
                        RecordAdmission::Unresolved
                    }
                }
            }
            FeedRecord::PriceRate { record, .. } => {
                if embedded.model_data().price_rates.iter().any(|p| {
                    p.price_rate_id == record.price_rate_id
                        || (p.offer_ref == record.offer_ref
                            && p.model_configuration_id == record.model_configuration_id
                            && p.currency == record.currency)
                }) {
                    RecordAdmission::IgnoredEmbedded
                } else {
                    dependent_state(&models, &record.model_configuration_id).unwrap_or(initial)
                }
            }
            FeedRecord::Offer { record, .. } => {
                if let Some(old) = embedded.model_data().offer(&record.offer_id) {
                    let mut expected = old.clone();
                    expected.model_configuration_ids = record.model_configuration_ids.clone();
                    // Evidence may be refreshed but cannot change entitlement, revision or billing.
                    expected.evidence_digest = record.evidence_digest.clone();
                    if &expected != record
                        || old
                            .model_configuration_ids
                            .iter()
                            .any(|id| !record.model_configuration_ids.contains(id))
                    {
                        return Err(FeedError::validation(
                            "offer extension changes embedded authority or drops a member",
                        ));
                    }
                    if record
                        .model_configuration_ids
                        .iter()
                        .all(|id| old.model_configuration_ids.contains(id))
                    {
                        RecordAdmission::IgnoredEmbedded
                    } else {
                        initial
                    }
                } else {
                    let profile = embedded
                        .registry()
                        .endpoint_profile(&record.endpoint_profile_id);
                    if profile.is_none() {
                        if initial == RecordAdmission::Applied {
                            RecordAdmission::Unresolved
                        } else {
                            initial
                        }
                    } else if embedded.model_data().offers.iter().any(|o| {
                        o.endpoint_profile_id == record.endpoint_profile_id
                            && o.entitlement_id == record.entitlement_id
                            && o.usage_scope == record.usage_scope
                            && o.region_id == record.region_id
                    }) {
                        RecordAdmission::IgnoredEmbedded
                    } else {
                        initial
                    }
                }
            }
        };
        outcomes.push(AdmittedRecord {
            kind: item.kind().into(),
            id: item.id().into(),
            outcome,
        });
    }
    // Price records cannot become executable without their admitted offer. Unsupported members
    // are filtered from membership extensions, preserving every embedded member.
    let offers: BTreeSet<_> = outcomes
        .iter()
        .filter(|o| o.kind == "offer" && o.outcome == RecordAdmission::Applied)
        .map(|o| o.id.clone())
        .collect();
    for (item, outcome) in payload.records.iter().zip(&mut outcomes) {
        if outcome.outcome != RecordAdmission::Applied {
            continue;
        }
        if let FeedRecord::PriceRate { record, .. } = item
            && !offers.contains(&record.offer_ref)
            && embedded.model_data().offer(&record.offer_ref).is_none()
        {
            outcome.outcome = RecordAdmission::Unresolved;
        }
    }
    let mut counts = FeedCounts::default();
    for record in &outcomes {
        match record.outcome {
            RecordAdmission::Applied => counts.applied += 1,
            RecordAdmission::IgnoredEmbedded => counts.ignored_embedded += 1,
            RecordAdmission::NewerClient { .. } => counts.newer_client += 1,
            RecordAdmission::Unresolved => counts.unresolved += 1,
        }
    }
    let result = FeedAdmissionV1 {
        records: outcomes,
        counts,
        payload: payload.clone(),
        embedded_digest: embedded.model_data_provenance().2.clone(),
        registry_digest: embedded.registry_provenance().2.clone(),
    };
    embedded.with_feed(&result)?;
    Ok(result)
}

fn version_admission(record: &FeedRecord, version: &semver::Version) -> RecordAdmission {
    if semver::Version::parse(record.minimum_client()).is_ok_and(|minimum| minimum > *version) {
        RecordAdmission::NewerClient {
            min_client_version: record.minimum_client().into(),
        }
    } else {
        RecordAdmission::Applied
    }
}

fn dependent_state(models: &BTreeMap<&str, RecordAdmission>, id: &str) -> Option<RecordAdmission> {
    match models.get(id) {
        Some(RecordAdmission::Applied) => None,
        Some(other) => Some(other.clone()),
        None => Some(RecordAdmission::Unresolved),
    }
}

impl TrustedReleaseCatalog {
    pub fn with_feed(&self, admission: &FeedAdmissionV1) -> Result<Self, FeedError> {
        if self.model_data_provenance().2 != &admission.embedded_digest
            || self.registry_provenance().2 != &admission.registry_digest
            || self.feed_facts.is_some()
        {
            return Err(FeedError::validation(
                "admission does not belong to this embedded catalog",
            ));
        }
        let mut effective = self.clone();
        let mut facts = FeedCatalogFacts {
            newer_models: admission
                .payload
                .records
                .iter()
                .zip(&admission.records)
                .filter_map(|(record, outcome)| {
                    if let (
                        FeedRecord::Model {
                            record,
                            included_on,
                            ..
                        },
                        RecordAdmission::NewerClient { min_client_version },
                    ) = (record, &outcome.outcome)
                    {
                        Some(NewerFeedModel {
                            model_configuration_id: record
                                .definition
                                .model_configuration_id
                                .clone(),
                            display_name: record.definition.display_name.clone(),
                            publisher_id: record.definition.publisher_id.clone(),
                            capabilities: record.definition.capabilities.clone(),
                            minimum_client: min_client_version.clone(),
                            included_on: included_on.clone(),
                            identity_stability: record.canonical.identity_stability.clone(),
                            sequence: admission.payload.sequence,
                        })
                    } else {
                        None
                    }
                })
                .collect(),
            origins: BTreeMap::new(),
            canonical: BTreeMap::new(),
            native_reasoning: self.native_reasoning().to_vec(),
            required_clients: BTreeMap::new(),
            featured: Vec::new(),
        };
        let applied_models: BTreeSet<_> = admission
            .payload
            .records
            .iter()
            .zip(&admission.records)
            .filter_map(|(record, result)| match record {
                FeedRecord::Model { record, .. } if result.outcome == RecordAdmission::Applied => {
                    Some(record.definition.model_configuration_id.clone())
                }
                _ => None,
            })
            .collect();
        for (item, result) in admission.payload.records.iter().zip(&admission.records) {
            if let (
                FeedRecord::Model { record, .. },
                RecordAdmission::NewerClient { min_client_version },
            ) = (item, &result.outcome)
            {
                facts.required_clients.insert(
                    record.definition.model_configuration_id.clone(),
                    min_client_version.clone(),
                );
            }
            if result.outcome != RecordAdmission::Applied {
                continue;
            }
            match item {
                FeedRecord::Model {
                    record,
                    included_on,
                    ..
                } => {
                    let id = record.definition.model_configuration_id.clone();
                    effective.model_data.models.push(record.definition.clone());
                    facts.origins.insert(
                        id.clone(),
                        ModelFactOrigin::Feed {
                            sequence: admission.payload.sequence,
                            included_on: included_on.clone(),
                        },
                    );
                    facts.canonical.insert(id, record.canonical.clone());
                    if let Some(native) = &record.native_reasoning {
                        facts.native_reasoning.push(native.clone());
                    }
                }
                FeedRecord::ModelEndpointCapability { record, .. } => effective
                    .model_data
                    .model_endpoint_capabilities
                    .push(record.clone()),
                FeedRecord::Offer { record, .. } => {
                    if let Some(old) = effective
                        .model_data
                        .offers
                        .iter_mut()
                        .find(|o| o.offer_id == record.offer_id)
                    {
                        for id in &record.model_configuration_ids {
                            if applied_models.contains(id)
                                && !old.model_configuration_ids.contains(id)
                            {
                                old.model_configuration_ids.push(id.clone());
                            }
                        }
                    } else {
                        let mut offer = record.clone();
                        offer
                            .model_configuration_ids
                            .retain(|id| applied_models.contains(id));
                        if !offer.model_configuration_ids.is_empty() {
                            effective.model_data.offers.push(offer);
                        }
                    }
                }
                FeedRecord::PriceRate { record, .. } => {
                    effective.model_data.price_rates.push(record.clone())
                }
            }
        }
        if effective.model_data != self.model_data {
            let suffix = format!("/feed:{}", admission.payload.sequence);
            for version in [
                &mut effective.model_data.bundle_version,
                &mut effective.model_data.models_slice_version,
                &mut effective.model_data.capability_slice_version,
                &mut effective.model_data.prices_slice_version,
            ] {
                version.push_str(&suffix);
            }
            effective
                .model_data
                .validate_against(&effective.registry)
                .map_err(|_| {
                    FeedError::validation("effective model data violates embedded invariants")
                })?;
            effective.model_data_digest = CanonicalDigest::of(&effective.model_data)
                .map_err(|_| FeedError::validation("effective catalog digest"))?;
        }
        facts.featured = admission
            .payload
            .featured
            .iter()
            .filter(|entry| {
                effective
                    .model_data
                    .model(&entry.model_configuration_id)
                    .is_some()
                    || facts
                        .required_clients
                        .contains_key(&entry.model_configuration_id)
            })
            .cloned()
            .collect();
        effective.feed_facts = Some(Box::new(facts));
        Ok(effective)
    }

    pub fn model_fact_origin(&self, id: &str) -> Option<ModelFactOrigin> {
        if let Some(origin) = self
            .feed_facts
            .as_ref()
            .and_then(|facts| facts.origins.get(id))
        {
            return Some(origin.clone());
        }
        self.model_data
            .model(id)
            .map(|_| ModelFactOrigin::Embedded {
                client_version: self.release_facts.product_release.clone(),
            })
    }

    pub fn model_identity_stability(&self, id: &str) -> &str {
        if let Some(canonical) = self
            .feed_facts
            .as_ref()
            .and_then(|facts| facts.canonical.get(id))
        {
            return &canonical.identity_stability;
        }
        let upstream: BTreeSet<_> = self
            .model_data
            .model_endpoint_capabilities
            .iter()
            .filter(|c| c.model_configuration_id == id)
            .map(|c| c.upstream_model_id.as_str())
            .collect();
        self.model_metadata()
            .canonical_models
            .iter()
            .find(|m| m.upstream_ids.iter().any(|u| upstream.contains(u.as_str())))
            .map_or("unknown", |m| m.identity_stability.as_str())
    }

    pub fn required_feed_client(&self, id: &str) -> Option<&str> {
        self.feed_facts
            .as_ref()?
            .required_clients
            .get(id)
            .map(String::as_str)
    }

    pub fn newer_feed_models(&self) -> &[NewerFeedModel] {
        self.feed_facts
            .as_ref()
            .map_or(&[], |facts| facts.newer_models.as_slice())
    }
    pub fn featured_models(&self) -> &[FeaturedModel] {
        self.feed_facts
            .as_ref()
            .map_or(&[], |facts| facts.featured.as_slice())
    }
}
