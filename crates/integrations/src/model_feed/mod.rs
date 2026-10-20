//! HTTPS-delivered, non-executable model facts. Registry authority always stays in the client.

mod admission;
mod contract;
mod transport;
mod validation;

pub use admission::*;
pub use contract::*;
pub use transport::{FeedDownload, retrieve_feed};
pub use validation::{ValidatedFeed, parse_feed};

use serde::{Deserialize, Serialize};

pub const MODEL_FEED_URL: &str = "https://hiroute.ai/model-feed/v2.json";
pub const MAX_MODEL_FEED_BYTES: usize = 16 * 1024 * 1024;
pub const MODEL_FEED_SCHEMA: &str = "hiroute.model-feed/v2";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedFailureKind {
    Network,
    Size,
    Schema,
    Validation,
    OlderSequence,
}

#[derive(Debug, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct FeedError {
    pub kind: FeedFailureKind,
    pub message: &'static str,
}

impl FeedError {
    pub fn new(kind: FeedFailureKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    pub(crate) fn validation(message: &'static str) -> Self {
        Self::new(FeedFailureKind::Validation, message)
    }
}

#[cfg(test)]
mod tests;
