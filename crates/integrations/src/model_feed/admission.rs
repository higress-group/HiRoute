use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{CanonicalDigest, MetadataCanonicalModelV1, ModelNativeReasoningV1};
use serde::{Deserialize, Serialize};

use super::{FeaturedModel, FeedError, FeedPayload};
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

/// Validate the complete body, then qualify only references implemented by this release.
/// Descriptive records and rating identity are never filtered by executable eligibility.
pub fn admit(
    embedded: &TrustedReleaseCatalog,
    payload: &FeedPayload,
    client_version: &str,
) -> Result<FeedAdmissionV1, FeedError> {
    payload.validate()?;
    semver::Version::parse(client_version)
        .map_err(|_| FeedError::validation("invalid client version"))?;
    let mut records = Vec::new();
    for model in &payload.data.models {
        records.push(AdmittedRecord {
            kind: "model".into(),
            id: model.model_configuration_id.clone(),
            outcome: RecordAdmission::Applied,
        });
    }
    for capability in &payload.data.model_endpoint_capabilities {
        let connector = embedded.registry().connector(&capability.connector_id);
        let profile = embedded
            .registry()
            .endpoint_profile(&capability.endpoint_profile_id);
        let qualified = connector.is_some_and(|c| {
            c.revision == capability.connector_revision
                && c.endpoint_profile_refs
                    .contains(&capability.endpoint_profile_id)
        }) && profile.is_some_and(|p| {
            p.revision == capability.endpoint_profile_revision
                && p.protocol_endpoints.iter().any(|e| {
                    e.protocol_endpoint_id == capability.protocol_endpoint_id
                        && e.protocol == capability.upstream_protocol
                        && e.adapter_ref == capability.required_adapter_ref
                        && e.adapter_revision == capability.required_adapter_revision
                })
        });
        records.push(AdmittedRecord {
            kind: "model_endpoint_capability".into(),
            id: capability.capability_id.clone(),
            outcome: if qualified {
                RecordAdmission::Applied
            } else {
                RecordAdmission::Unresolved
            },
        });
    }
    for offer in &payload.data.offers {
        // Membership/evidence are replaceable facts; entitlement/billing/region remain installed.
        let installed = embedded
            .current_release_model_data()
            .data
            .offers
            .iter()
            .find(|o| o.offer_id == offer.offer_id);
        let qualified = if let Some(old) = installed {
            let mut expected = old.clone();
            expected.model_configuration_ids = offer.model_configuration_ids.clone();
            expected.evidence_digest = offer.evidence_digest.clone();
            if expected != *offer {
                return Err(FeedError::validation(
                    "download changes installed offer authority",
                ));
            }
            true
        } else {
            false
        };
        records.push(AdmittedRecord {
            kind: "offer".into(),
            id: offer.offer_id.clone(),
            outcome: if qualified {
                RecordAdmission::Applied
            } else {
                RecordAdmission::Unresolved
            },
        });
    }
    let mut counts = FeedCounts::default();
    for record in &records {
        if record.outcome == RecordAdmission::Applied {
            counts.applied += 1;
        } else {
            counts.unresolved += 1;
        }
    }
    let result = FeedAdmissionV1 {
        records,
        counts,
        payload: payload.clone(),
        embedded_digest: embedded.model_data_provenance().2.clone(),
        registry_digest: embedded.registry_provenance().2.clone(),
    };
    embedded.with_feed(&result)?;
    Ok(result)
}

impl TrustedReleaseCatalog {
    pub fn official_snapshot(&self) -> hiroute_domain::OfficialModelSnapshotV1 {
        self.official_snapshot
            .as_deref()
            .cloned()
            .unwrap_or_else(|| {
                let data = hiroute_domain::OfficialModelDataV1::from_release(
                    &self.current_release_model_data,
                );
                let data_digest =
                    CanonicalDigest::of(&data).expect("serializable embedded model data");
                hiroute_domain::OfficialModelSnapshotV1 {
                    data_revision: 1,
                    data_digest,
                    data,
                }
            })
    }

    /// Attach a generated, compiled fallback only after verifying its shared release slices.
    pub fn with_embedded_official_data(
        mut self,
        snapshot: hiroute_domain::OfficialModelSnapshotV1,
    ) -> Result<Self, FeedError> {
        snapshot
            .validate()
            .map_err(|_| FeedError::validation("invalid embedded official data"))?;
        let expected =
            hiroute_domain::OfficialModelDataV1::from_release(&self.current_release_model_data);
        let actual = &snapshot.data;
        if actual.models != expected.models
            || actual.model_endpoint_capabilities != expected.model_endpoint_capabilities
            || actual.offers != expected.offers
            || actual.price_rates != expected.price_rates
            || actual.metadata_catalog != expected.metadata_catalog
            || actual.rating_snapshot != expected.rating_snapshot
        {
            return Err(FeedError::validation(
                "embedded official and release slices differ",
            ));
        }
        self.official_snapshot = Some(Box::new(snapshot));
        Ok(self)
    }

    pub fn with_feed(&self, admission: &FeedAdmissionV1) -> Result<Self, FeedError> {
        if self.model_data_provenance().2 != &admission.embedded_digest
            || self.registry_provenance().2 != &admission.registry_digest
            || self.feed_facts.is_some()
        {
            return Err(FeedError::validation(
                "admission does not belong to this installed catalog",
            ));
        }
        let mut effective = self.clone();
        let selected = &admission.payload.data;
        let applied = |kind: &str, id: &str| {
            admission
                .records
                .iter()
                .any(|r| r.kind == kind && r.id == id && r.outcome == RecordAdmission::Applied)
        };
        effective.model_data.models = selected.models.clone();
        effective.model_data.model_endpoint_capabilities = selected
            .model_endpoint_capabilities
            .iter()
            .filter(|c| applied("model_endpoint_capability", &c.capability_id))
            .cloned()
            .collect();
        effective.model_data.offers = selected
            .offers
            .iter()
            .filter(|o| applied("offer", &o.offer_id))
            .cloned()
            .collect();
        effective.model_data.price_rates = selected
            .price_rates
            .iter()
            .filter(|p| applied("offer", &p.offer_ref))
            .cloned()
            .collect();
        let ids: BTreeSet<_> = selected
            .models
            .iter()
            .map(|m| m.model_configuration_id.as_str())
            .collect();
        for free in &mut effective.model_data.free_offers {
            free.model_configuration_ids.retain(|id| {
                ids.contains(id.as_str())
                    && effective.model_data.offers.iter().any(|o| {
                        o.offer_id == free.offer_ref && o.model_configuration_ids.contains(id)
                    })
            });
        }
        effective
            .model_data
            .free_offers
            .retain(|free| !free.model_configuration_ids.is_empty());
        effective.model_data.ratings_slice_version = selected.rating_snapshot.version.clone();
        effective
            .model_data
            .validate_against(&effective.registry)
            .map_err(|_| {
                FeedError::validation("qualified model data violates installed invariants")
            })?;
        effective.model_data_digest = CanonicalDigest::of(&effective.model_data)
            .map_err(|_| FeedError::validation("effective catalog digest"))?;
        let mut facts = FeedCatalogFacts {
            newer_models: vec![],
            origins: BTreeMap::new(),
            canonical: BTreeMap::new(),
            native_reasoning: selected.rating_snapshot.models.clone(),
            required_clients: BTreeMap::new(),
            featured: admission.payload.featured.clone(),
        };
        for model in &selected.models {
            let id = &model.model_configuration_id;
            facts.origins.insert(
                id.clone(),
                ModelFactOrigin::Feed {
                    sequence: admission.payload.sequence,
                    included_on: selected.metadata_catalog.as_of.clone(),
                },
            );
        }
        effective.official_snapshot = Some(Box::new(admission.payload.snapshot()));
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
