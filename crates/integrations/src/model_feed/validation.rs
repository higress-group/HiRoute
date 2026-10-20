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
    let payload: FeedPayload = serde_json::from_slice(bytes)
        .map_err(|_| FeedError::new(FeedFailureKind::Schema, "invalid feed contract"))?;
    payload.validate()?;
    Ok(ValidatedFeed {
        payload,
        payload_digest: CanonicalDigest::of_bytes(bytes),
    })
}
