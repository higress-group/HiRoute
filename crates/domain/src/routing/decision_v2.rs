//! Versioned decision-service content. Existing V1 bytes remain a separate variant.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::{ClassifierAuthHeaderV1, DecisionServiceV1};
use crate::CanonicalDigest;

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DecisionServiceSchemaV2 {
    #[serde(rename = "hiroute.decision-service/v2")]
    V2,
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionWireProtocolV2 {
    SystemOne,
    OpenaiDecisions,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionAuthenticationV2 {
    Independent {
        auth_header: ClassifierAuthHeaderV1,
    },
    SharedSource {
        source_id: String,
        source_lineage_digest: CanonicalDigest,
    },
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionConnectionV2 {
    Native {
        provider: String,
        wire_protocol: DecisionWireProtocolV2,
        model: String,
        endpoint: String,
        timeout_ms: u64,
        authentication: DecisionAuthenticationV2,
    },
    Custom {
        endpoint: String,
        timeout_ms: u64,
        auth_header: Option<ClassifierAuthHeaderV1>,
    },
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceV2 {
    pub schema: DecisionServiceSchemaV2,
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub connection: DecisionConnectionV2,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionExpectedSourceRevisionV1 {
    pub source_id: String,
    pub revision: u64,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceChangeV2 {
    pub id: String,
    pub expected_revision: u64,
    pub service: Option<DecisionServiceV2>,
    pub expected_source_revisions: Vec<DecisionExpectedSourceRevisionV1>,
}

impl DecisionServiceChangeV2 {
    pub fn validate(&self) -> bool {
        if !super::strategy::valid_reference(&self.id) || self.id.len() > 128 {
            return false;
        }
        match &self.service {
            Some(service) => {
                if !service.validate()
                    || service.id != self.id
                    || self.expected_revision.checked_add(1) != Some(service.revision)
                {
                    return false;
                }
            }
            None if self.expected_revision == 0 => return false,
            None => {}
        }
        let mut sources = BTreeSet::new();
        self.expected_source_revisions.iter().all(|source| {
            super::strategy::valid_reference(&source.source_id)
                && source.source_id.len() <= 128
                && source.revision > 0
                && sources.insert(&source.source_id)
        })
    }
}

impl DecisionConnectionV2 {
    pub fn transport(&self) -> (&str, u64, Option<&ClassifierAuthHeaderV1>) {
        match self {
            Self::Native {
                endpoint,
                timeout_ms,
                authentication,
                ..
            } => (
                endpoint,
                *timeout_ms,
                match authentication {
                    DecisionAuthenticationV2::Independent { auth_header } => Some(auth_header),
                    DecisionAuthenticationV2::SharedSource { .. } => None,
                },
            ),
            Self::Custom {
                endpoint,
                timeout_ms,
                auth_header,
            } => (endpoint, *timeout_ms, auth_header.as_ref()),
        }
    }

    pub fn validate(&self) -> bool {
        let (endpoint, timeout, header) = self.transport();
        if !super::classifier::valid_classifier_transport(endpoint, timeout, header) {
            return false;
        }
        match self {
            Self::Native {
                provider,
                model,
                authentication,
                ..
            } => {
                valid_text(provider, 128)
                    && valid_text(model, 256)
                    && match authentication {
                        DecisionAuthenticationV2::Independent { auth_header } => {
                            auth_header.name.eq_ignore_ascii_case("authorization")
                        }
                        DecisionAuthenticationV2::SharedSource {
                            source_id,
                            source_lineage_digest,
                        } => {
                            super::strategy::valid_reference(source_id)
                                && source_id.len() <= 128
                                && matches!(
                                    CanonicalDigest::parse(source_lineage_digest.as_str()),
                                    Ok(parsed) if &parsed == source_lineage_digest
                                )
                        }
                    }
            }
            Self::Custom { .. } => true,
        }
    }
}

impl DecisionServiceV2 {
    pub fn validate(&self) -> bool {
        super::strategy::valid_reference(&self.id)
            && self.id.len() <= 128
            && self.revision > 0
            && valid_text(&self.name, 128)
            && self.connection.validate()
    }
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.trim() == value
        && value.chars().count() <= max
        && !value.chars().any(char::is_control)
}

/// Strict untagged V1 and explicitly tagged V2 records coexist without rewriting V1.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DecisionServiceRecord {
    V1(DecisionServiceV1),
    V2(DecisionServiceV2),
}

impl DecisionServiceRecord {
    pub fn id(&self) -> &str {
        match self {
            Self::V1(service) => &service.id,
            Self::V2(service) => &service.id,
        }
    }

    pub fn revision(&self) -> u64 {
        match self {
            Self::V1(service) => service.revision,
            Self::V2(service) => service.revision,
        }
    }

    pub fn validate(&self) -> bool {
        match self {
            Self::V1(service) => service.validate(),
            Self::V2(service) => service.validate(),
        }
    }

    pub fn auth_header(&self) -> Option<&ClassifierAuthHeaderV1> {
        match self {
            Self::V1(service) => service.connection.transport().2,
            Self::V2(service) => service.connection.transport().2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn service() -> DecisionServiceV2 {
        serde_json::from_value(json!({
            "schema": "hiroute.decision-service/v2",
            "id": "decision/example", "revision": 1, "name": "Example",
            "connection": {
                "kind": "native", "provider": "openai", "wire_protocol": "openai_decisions",
                "model": "gpt-6-luna", "endpoint": "https://example.test/v1/decisions",
                "timeout_ms": 1000,
                "authentication": { "kind": "shared_source", "source_id": "source/example",
                    "source_lineage_digest": format!("sha256:{}", "1".repeat(64)) }
            }
        }))
        .unwrap()
    }

    #[test]
    fn validates_exact_authentication_shape_and_source_identity() {
        let mut value = service();
        assert!(value.validate());
        if let DecisionConnectionV2::Native { authentication, .. } = &mut value.connection {
            *authentication = DecisionAuthenticationV2::SharedSource {
                source_id: "invalid source".into(),
                source_lineage_digest: CanonicalDigest::of_bytes(b"source"),
            };
        }
        assert!(!value.validate());
        if let DecisionConnectionV2::Native { authentication, .. } = &mut value.connection {
            *authentication = DecisionAuthenticationV2::Independent {
                auth_header: ClassifierAuthHeaderV1 {
                    name: "X-Api-Key".into(),
                    value_secret_ref: "decision/example/ref".into(),
                },
            };
        }
        assert!(!value.validate());
        if let DecisionConnectionV2::Native { authentication, .. } = &mut value.connection
            && let DecisionAuthenticationV2::Independent { auth_header } = authentication
        {
            auth_header.name = "Authorization".into();
        }
        assert!(value.validate());
    }

    #[test]
    fn unknown_fields_and_versions_do_not_become_v1_records() {
        let mut encoded = serde_json::to_value(service()).unwrap();
        encoded["schema"] = json!("hiroute.decision-service/v3");
        assert!(serde_json::from_value::<DecisionServiceRecord>(encoded).is_err());
        let mut encoded = serde_json::to_value(service()).unwrap();
        encoded["connection"]["wire_protocol"] = json!("unknown");
        assert!(serde_json::from_value::<DecisionServiceRecord>(encoded).is_err());
    }
}
