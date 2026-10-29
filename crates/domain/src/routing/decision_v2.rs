//! Versioned decision-service content. Existing V1 bytes remain a separate variant.
use serde::{Deserialize, Serialize};

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

/// Strict untagged V1 and explicitly tagged V2 records coexist without rewriting V1.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DecisionServiceRecord {
    V1(DecisionServiceV1),
    V2(DecisionServiceV2),
}

/// Authoring selector: the V2 discriminator cannot reinterpret a V1 service.
#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionClassifierSelectorV2 {
    LocalRules,
    DecisionService { service: Box<DecisionServiceV1> },
    DecisionServiceV2 { service: Box<DecisionServiceV2> },
}
