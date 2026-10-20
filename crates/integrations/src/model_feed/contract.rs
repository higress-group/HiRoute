use super::{FeedError, MODEL_FEED_SCHEMA};
use hiroute_domain::{CanonicalDigest, OfficialModelDataV1, OfficialModelSnapshotV1};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeedPayload {
    pub schema: String,
    pub sequence: u64,
    pub issued_at: String,
    pub source_revision: String,
    pub data_revision: u64,
    pub data_digest: CanonicalDigest,
    pub data: OfficialModelDataV1,
    pub featured: Vec<FeaturedModel>,
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

impl FeedPayload {
    pub fn snapshot(&self) -> OfficialModelSnapshotV1 {
        OfficialModelSnapshotV1 {
            data_revision: self.data_revision,
            data_digest: self.data_digest.clone(),
            data: self.data.clone(),
        }
    }
    pub fn from_snapshot(
        snapshot: OfficialModelSnapshotV1,
        sequence: u64,
        issued_at: String,
        source_revision: String,
        featured: Vec<FeaturedModel>,
    ) -> Self {
        Self {
            schema: MODEL_FEED_SCHEMA.into(),
            sequence,
            issued_at,
            source_revision,
            data_revision: snapshot.data_revision,
            data_digest: snapshot.data_digest,
            data: snapshot.data,
            featured,
        }
    }
    pub fn validate(&self) -> Result<(), FeedError> {
        if self.schema != MODEL_FEED_SCHEMA {
            return Err(FeedError::new(
                super::FeedFailureKind::Schema,
                "unsupported payload schema",
            ));
        }
        validate_publication_fields(
            self.sequence,
            &self.issued_at,
            &self.source_revision,
            &self.featured,
        )?;
        self.snapshot()
            .validate()
            .map_err(|_| FeedError::validation("invalid complete model data"))?;
        Ok(())
    }
}

pub fn validate_publication_fields(
    sequence: u64,
    issued_at: &str,
    source_revision: &str,
    featured: &[FeaturedModel],
) -> Result<(), FeedError> {
    if sequence == 0
        || !valid_utc_timestamp(issued_at)
        || source_revision.len() != 40
        || !source_revision.bytes().all(|b| b.is_ascii_hexdigit())
        || featured.len() > 128
    {
        return Err(FeedError::validation(
            "invalid publication identity or bounds",
        ));
    }
    let mut ids = BTreeSet::new();
    for item in featured {
        if !ids.insert(item.id.as_str())
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
