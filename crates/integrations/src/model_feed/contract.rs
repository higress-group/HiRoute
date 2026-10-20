use std::collections::BTreeSet;

use hiroute_domain::{
    MetadataCanonicalModelV1, ModelDefinitionV1, ModelEndpointCapabilityV1, ModelNativeReasoningV1,
    OfferV1, PriceRateV1,
};
use serde::{Deserialize, Serialize};

use super::{FeedError, MODEL_FEED_SCHEMA};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeedPayload {
    pub schema: String,
    pub sequence: u64,
    pub issued_at: String,
    pub source_revision: String,
    pub records: Vec<FeedRecord>,
    pub featured: Vec<FeaturedModel>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FeedRecord {
    Model {
        min_client_version: String,
        included_on: String,
        record: Box<FeedModel>,
    },
    ModelEndpointCapability {
        min_client_version: String,
        included_on: String,
        record: ModelEndpointCapabilityV1,
    },
    Offer {
        min_client_version: String,
        included_on: String,
        record: OfferV1,
    },
    PriceRate {
        min_client_version: String,
        included_on: String,
        record: PriceRateV1,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeedModel {
    pub definition: ModelDefinitionV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_reasoning: Option<ModelNativeReasoningV1>,
    pub canonical: MetadataCanonicalModelV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeaturedKind {
    NewModel,
    ModelUpdate,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LocalizedFeedText {
    pub en: String,
    pub zh: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeaturedModel {
    pub id: String,
    pub kind: FeaturedKind,
    pub model_configuration_id: String,
    pub headline: LocalizedFeedText,
    pub note: LocalizedFeedText,
    pub links: Vec<String>,
    pub included_on: String,
}

impl FeedRecord {
    pub fn minimum_client(&self) -> &str {
        match self {
            Self::Model {
                min_client_version, ..
            }
            | Self::ModelEndpointCapability {
                min_client_version, ..
            }
            | Self::Offer {
                min_client_version, ..
            }
            | Self::PriceRate {
                min_client_version, ..
            } => min_client_version,
        }
    }
    pub fn included_on(&self) -> &str {
        match self {
            Self::Model { included_on, .. }
            | Self::ModelEndpointCapability { included_on, .. }
            | Self::Offer { included_on, .. }
            | Self::PriceRate { included_on, .. } => included_on,
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::Model { record, .. } => &record.definition.model_configuration_id,
            Self::ModelEndpointCapability { record, .. } => &record.capability_id,
            Self::Offer { record, .. } => &record.offer_id,
            Self::PriceRate { record, .. } => &record.price_rate_id,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Model { .. } => "model",
            Self::ModelEndpointCapability { .. } => "model_endpoint_capability",
            Self::Offer { .. } => "offer",
            Self::PriceRate { .. } => "price_rate",
        }
    }
    pub fn model_id(&self) -> Option<&str> {
        match self {
            Self::Model { record, .. } => Some(&record.definition.model_configuration_id),
            Self::ModelEndpointCapability { record, .. } => Some(&record.model_configuration_id),
            Self::PriceRate { record, .. } => Some(&record.model_configuration_id),
            Self::Offer { .. } => None,
        }
    }
}

impl FeedPayload {
    pub fn validate(&self) -> Result<(), FeedError> {
        if self.schema != MODEL_FEED_SCHEMA {
            return Err(FeedError::new(
                super::FeedFailureKind::Schema,
                "unsupported payload schema",
            ));
        }
        if self.sequence == 0
            || !valid_utc_timestamp(&self.issued_at)
            || self.source_revision.len() != 40
            || !self.source_revision.bytes().all(|b| b.is_ascii_hexdigit())
            || self.records.len() > 8192
            || self.featured.len() > 128
        {
            return Err(FeedError::validation(
                "invalid publication identity or bounds",
            ));
        }
        let mut ids = BTreeSet::new();
        for entry in &self.records {
            if !ids.insert((entry.kind(), entry.id()))
                || !valid_id(entry.id())
                || semver::Version::parse(entry.minimum_client()).is_err()
                || !valid_date(entry.included_on())
            {
                return Err(FeedError::validation(
                    "invalid or duplicate record identity, version or date",
                ));
            }
            if let FeedRecord::Model { record, .. } = entry {
                if !matches!(
                    record.canonical.identity_stability.as_str(),
                    "immutable-or-provider-stable" | "rolling-alias" | "dynamic-route" | "unknown"
                ) || record.canonical.model_key.is_empty()
                    || record.canonical.upstream_ids.is_empty()
                    || record.canonical.evidence_refs.is_empty()
                {
                    return Err(FeedError::validation(
                        "model identity/evidence is not closed",
                    ));
                }
                if let Some(reasoning) = &record.native_reasoning
                    && (reasoning.model_configuration_id
                        != record.definition.model_configuration_id
                        || reasoning.validate().is_err())
                {
                    return Err(FeedError::validation("invalid native reasoning"));
                }
            }
        }
        ids.clear();
        for item in &self.featured {
            if !ids.insert(("featured", item.id.as_str()))
                || !valid_id(&item.id)
                || !valid_id(&item.model_configuration_id)
                || !valid_date(&item.included_on)
                || !valid_text(&item.headline.en, 160)
                || !valid_text(&item.headline.zh, 160)
                || !valid_text(&item.note.en, 1000)
                || !valid_text(&item.note.zh, 1000)
                || item.links.len() > 4
                || item.links.iter().any(|url| !valid_featured_url(url))
            {
                return Err(FeedError::validation("invalid featured entry"));
            }
        }
        Ok(())
    }
}

pub fn valid_featured_url(url: &str) -> bool {
    url.len() <= 2048
        && url.starts_with("https://hiroute.ai/")
        && !url
            .bytes()
            .any(|c| c.is_ascii_control() || c == b'\\' || c == b' ')
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.chars().count() <= max && !value.chars().any(char::is_control)
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.contains("..")
        && !value.contains("//")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'/'))
}

pub fn valid_date(date: &str) -> bool {
    if date.len() != 10 || date.as_bytes()[4] != b'-' || date.as_bytes()[7] != b'-' {
        return false;
    }
    let Some((year, month, day)) = date
        .get(..4)
        .and_then(|y| y.parse::<u32>().ok())
        .zip(date.get(5..7).and_then(|m| m.parse::<u32>().ok()))
        .zip(date.get(8..10).and_then(|d| d.parse::<u32>().ok()))
        .map(|((y, m), d)| (y, m, d))
    else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let limit = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    year > 0 && day > 0 && day <= limit
}

fn valid_utc_timestamp(value: &str) -> bool {
    if value.len() < 20 || !value.is_ascii() || !valid_date(&value[..10]) {
        return false;
    }
    let Some(clock) = value
        .strip_suffix('Z')
        .or_else(|| value.strip_suffix("+00:00"))
    else {
        return false;
    };
    if clock.len() < 19 {
        return false;
    }
    let seconds = &clock[17..];
    let (seconds, fraction) = seconds
        .split_once('.')
        .map_or((seconds, None), |(s, f)| (s, Some(f)));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    clock.as_bytes()[10] == b'T'
        && clock.as_bytes()[13] == b':'
        && clock.as_bytes()[16] == b':'
        && digits(&clock[11..13])
        && clock[11..13].parse::<u8>().is_ok_and(|n| n < 24)
        && digits(&clock[14..16])
        && clock[14..16].parse::<u8>().is_ok_and(|n| n < 60)
        && seconds.len() == 2
        && digits(seconds)
        && seconds.parse::<u8>().is_ok_and(|n| n < 60)
        && fraction.is_none_or(|s| !s.is_empty() && digits(s))
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;
    #[test]
    fn utc_rfc3339_supports_fractional_seconds_and_explicit_zero_offset() {
        for value in [
            "2026-10-18T08:09:10Z",
            "2026-10-18T08:09:10.123456Z",
            "2026-10-18T08:09:10+00:00",
        ] {
            assert!(valid_utc_timestamp(value), "{value}");
        }
        for value in [
            "2026-10-18T08:09:10+01:00",
            "2026-10-18T08:09:10.Z",
            "2026-02-30T08:09:10Z",
            "2026-10-18T+8:09:10Z",
            "2026-10-18T08:09:60Z",
            "2026-10-18T+00:00",
        ] {
            assert!(!valid_utc_timestamp(value), "{value}");
        }
    }
}
