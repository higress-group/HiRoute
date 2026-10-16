use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::WorkspaceId;

macro_rules! observation_id {
    ($name:ident) => {
        #[derive(
            schemars::JsonSchema,
            Clone,
            Debug,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, ObservationIdentityError> {
                let value = value.into();
                if valid_portable_id(&value) {
                    Ok(Self(value))
                } else {
                    Err(ObservationIdentityError::InvalidIdentifier)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

observation_id!(ProducerId);
observation_id!(ProducerEpoch);
observation_id!(StreamId);
observation_id!(EventId);
observation_id!(SessionId);
observation_id!(TurnId);
observation_id!(LogicalRequestId);
observation_id!(AttemptId);
observation_id!(ReceiptId);
observation_id!(MessageInstanceId);
observation_id!(ContentId);

macro_rules! hmac_identity {
    ($name:ident) => {
        #[derive(
            schemars::JsonSchema,
            Clone,
            Debug,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, ObservationIdentityError> {
                let value = value.into();
                if valid_hmac_identity(&value) {
                    Ok(Self(value.to_ascii_lowercase()))
                } else {
                    Err(ObservationIdentityError::InvalidDigest)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

hmac_identity!(MessageDigest);
hmac_identity!(CacheAffinityKey);

macro_rules! gateway_digest_identity {
    ($name:ident, $prefix:literal) => {
        #[derive(
            schemars::JsonSchema,
            Clone,
            Debug,
            Deserialize,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, ObservationIdentityError> {
                let value = value.into();
                if value.len() == $prefix.len() + 64
                    && value.starts_with($prefix)
                    && value[$prefix.len()..]
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                {
                    Ok(Self(value))
                } else {
                    Err(ObservationIdentityError::InvalidGatewayDigest)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

gateway_digest_identity!(ContentBlobDigest, "blob-");
gateway_digest_identity!(TranscriptRoot, "transcript-");

#[derive(
    schemars::JsonSchema,
    Clone,
    Copy,
    Debug,
    Deserialize,
    Eq,
    Hash,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ObservationChannel {
    Fact,
    Content,
}

impl ObservationChannel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Content => "content",
        }
    }
}

#[derive(
    schemars::JsonSchema, Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(deny_unknown_fields)]
pub struct ObservationStreamV1 {
    pub producer_id: ProducerId,
    pub producer_epoch: ProducerEpoch,
    pub stream_id: StreamId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationScopeV1 {
    pub workspace_id: WorkspaceId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SequenceRangeV1 {
    pub first: u64,
    pub last: u64,
}

impl SequenceRangeV1 {
    pub fn new(first: u64, last: u64) -> Result<Self, ObservationIdentityError> {
        if first == 0 || first > last {
            Err(ObservationIdentityError::InvalidSequenceRange)
        } else {
            Ok(Self { first, last })
        }
    }

    pub const fn contains(self, sequence: u64) -> bool {
        self.first <= sequence && sequence <= self.last
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LossNoticeV1 {
    pub range: SequenceRangeV1,
    pub scope: ObservationScopeV1,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ObservationIdentityError {
    #[error("observation identifier is not a bounded portable identifier")]
    InvalidIdentifier,
    #[error("observation digest must be hmac-sha256:<64 hexadecimal characters>")]
    InvalidDigest,
    #[error("Gateway content identity must have its frozen prefix and 64 hexadecimal characters")]
    InvalidGatewayDigest,
    #[error("observation sequence ranges are one-based and ordered")]
    InvalidSequenceRange,
}

fn valid_portable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':' | b'/')
        })
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
        && !value.contains("..")
}

fn valid_hmac_identity(value: &str) -> bool {
    value.len() == 76
        && value.starts_with("hmac-sha256:")
        && value[12..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_reject_paths_and_unbounded_values() {
        assert!(SessionId::parse("session-1").is_ok());
        assert!(SessionId::parse("../session").is_err());
        assert!(SessionId::parse("/session").is_err());
        assert!(SessionId::parse("x".repeat(161)).is_err());
    }

    #[test]
    fn hmac_identities_are_canonicalized() {
        let value = format!("hmac-sha256:{}", "AB".repeat(32));
        let digest = MessageDigest::parse(value).unwrap();
        assert_eq!(digest.as_str(), format!("hmac-sha256:{}", "ab".repeat(32)));
    }

    #[test]
    fn gateway_content_identities_preserve_the_frozen_namespaces() {
        let blob = ContentBlobDigest::parse(format!("blob-{}", "ab".repeat(32))).unwrap();
        let root = TranscriptRoot::parse(format!("transcript-{}", "cd".repeat(32))).unwrap();
        assert_eq!(blob.as_str(), format!("blob-{}", "ab".repeat(32)));
        assert_eq!(root.as_str(), format!("transcript-{}", "cd".repeat(32)));
        assert!(ContentBlobDigest::parse(format!("blob-{}", "AB".repeat(32))).is_err());
        assert!(TranscriptRoot::parse(format!("transcript-{}", "CD".repeat(32))).is_err());
        assert!(ContentBlobDigest::parse(format!("transcript-{}", "ab".repeat(32))).is_err());
        assert!(TranscriptRoot::parse(format!("hmac-sha256:{}", "cd".repeat(32))).is_err());
    }
}
