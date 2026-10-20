use hiroute_domain::CanonicalDigest;

use super::{FeedError, FeedFailureKind, FeedPayload, MAX_MODEL_FEED_BYTES};

#[derive(Clone, Debug)]
pub struct ValidatedFeed {
    pub payload: FeedPayload,
    pub payload_digest: CanonicalDigest,
}

/// Validate a publication fetched from the fixed HTTPS origin, or its private cache.
/// The digest binds exact bytes for cache integrity and sequence identity; HTTPS
/// and the production website pipeline are the content's trust boundary.
pub fn parse_feed(bytes: &[u8]) -> Result<ValidatedFeed, FeedError> {
    if bytes.len() > MAX_MODEL_FEED_BYTES {
        return Err(FeedError::new(FeedFailureKind::Size, "feed exceeds bound"));
    }
    let strict: StrictValue = serde_json::from_slice(bytes)
        .map_err(|_| FeedError::new(FeedFailureKind::Schema, "invalid or ambiguous JSON"))?;
    bounded(&strict.0, 0)?;
    let payload: FeedPayload = serde_json::from_value(strict.0)
        .map_err(|_| FeedError::new(FeedFailureKind::Schema, "invalid feed contract"))?;
    payload.validate()?;
    Ok(ValidatedFeed {
        payload,
        payload_digest: CanonicalDigest::of_bytes(bytes),
    })
}

struct StrictValue(serde_json::Value);
impl<'de> serde::Deserialize<'de> for StrictValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bounded unambiguous JSON")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut result = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if result.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                    result.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(serde_json::Value::Object(result)))
            }
            fn visit_seq<S: serde::de::SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> Result<Self::Value, S::Error> {
                let mut result = Vec::new();
                while let Some(value) = seq.next_element::<StrictValue>()? {
                    result.push(value.0);
                }
                Ok(StrictValue(serde_json::Value::Array(result)))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(n.into()))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(serde_json::Value::Null))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
fn bounded(value: &serde_json::Value, depth: usize) -> Result<(), FeedError> {
    use serde_json::Value;
    if depth > 64 {
        return Err(FeedError::validation("JSON depth exceeds bound"));
    }
    match value {
        Value::String(s) if s.len() > 16 * 1024 => {
            return Err(FeedError::validation("string exceeds bound"));
        }
        Value::Object(values) => {
            for (key, value) in values {
                if key.len() > 256 {
                    return Err(FeedError::validation("field key exceeds bound"));
                }
                bounded(value, depth + 1)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                if value.is_object()
                    && serde_json::to_vec(value).map_or(true, |b| b.len() > 256 * 1024)
                {
                    return Err(FeedError::validation("record exceeds bound"));
                }
                bounded(value, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
