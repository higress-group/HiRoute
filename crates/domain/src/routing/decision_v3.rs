//! Current immutable connection content. Old records never acquire a context snapshot.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum DecisionServiceSchemaV3 {
    #[serde(rename = "hiroute.decision-service/v3")]
    V3,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionContextConfigurationV1 {
    pub policy: DecisionContextPolicyV1,
    pub resolved: ResolvedDecisionContextV1,
}

#[derive(schemars::JsonSchema, Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceV3 {
    pub schema: DecisionServiceSchemaV3,
    pub id: String,
    pub revision: u64,
    pub name: String,
    pub connection: DecisionConnectionV2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<DecisionContextConfigurationV1>,
}

impl<'de> Deserialize<'de> for DecisionServiceV3 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            schema: DecisionServiceSchemaV3,
            id: String,
            revision: u64,
            name: String,
            connection: DecisionConnectionV2,
            #[serde(default)]
            context: Option<DecisionContextConfigurationV1>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let service = Self {
            schema: wire.schema,
            id: wire.id,
            revision: wire.revision,
            name: wire.name,
            connection: wire.connection,
            context: wire.context,
        };
        if !service.valid_context() {
            return Err(serde::de::Error::custom(
                "V3 native connections require a valid context snapshot; custom extensions must omit it",
            ));
        }
        Ok(service)
    }
}

impl DecisionServiceV3 {
    pub fn validate(&self) -> bool {
        if !super::strategy::valid_reference(&self.id)
            || self.id.len() > 128
            || self.revision == 0
            || self.name.trim().is_empty()
            || self.name.trim() != self.name
            || self.name.chars().count() > 128
            || self.name.chars().any(char::is_control)
            || !self.connection.validate()
        {
            return false;
        }
        self.valid_context()
    }

    fn valid_context(&self) -> bool {
        match (&self.connection, &self.context) {
            (DecisionConnectionV2::Custom { .. }, None) => true,
            (
                DecisionConnectionV2::Native {
                    endpoint,
                    wire_protocol,
                    ..
                },
                Some(context),
            ) => {
                let policy = &context.policy;
                let resolved = &context.resolved;
                policy.validate()
                    && resolved.validate()
                    && resolved.typesafe_long_context_header
                        == (policy.typesafe_long_context == TypeSafeLongContextV1::Automatic
                            && official_typesafe_long_context_destination(endpoint, *wire_protocol))
                    && policy.window_tokens.is_none_or(|window| {
                        window == resolved.request_window_tokens
                            && resolved.request_source == DecisionContextSourceV1::User
                    })
                    && (policy.mode != DecisionContextModeV1::Automatic
                        || resolved.request_source != DecisionContextSourceV1::User)
                    && policy.per_question_tokens.is_none_or(|window| {
                        window == resolved.per_question_window_tokens
                            && resolved.per_question_source == DecisionContextSourceV1::User
                    })
            }
            _ => false,
        }
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionServiceChangeV3 {
    pub id: String,
    pub expected_revision: u64,
    pub service: Option<DecisionServiceV3>,
    pub expected_source_revisions: Vec<DecisionExpectedSourceRevisionV1>,
}

impl DecisionServiceChangeV3 {
    pub fn validate(&self) -> bool {
        let mut sources = std::collections::BTreeSet::new();
        super::strategy::valid_reference(&self.id)
            && self.id.len() <= 128
            && match &self.service {
                Some(service) => {
                    service.validate()
                        && service.id == self.id
                        && self.expected_revision.checked_add(1) == Some(service.revision)
                }
                None => self.expected_revision > 0,
            }
            && self.expected_source_revisions.iter().all(|source| {
                super::strategy::valid_reference(&source.source_id)
                    && source.source_id.len() <= 128
                    && source.revision > 0
                    && sources.insert(&source.source_id)
            })
    }
}

impl DecisionServiceRecord {
    /// Shared authentication/transport content; this never projects a V3 service as V2.
    pub fn versioned_connection(&self) -> Option<&DecisionConnectionV2> {
        match self {
            Self::V1(_) => None,
            Self::V2(service) => Some(&service.connection),
            Self::V3(service) => Some(&service.connection),
        }
    }

    pub fn resolved_context(&self) -> Option<&ResolvedDecisionContextV1> {
        match self {
            Self::V3(service) => service.context.as_ref().map(|c| &c.resolved),
            _ => None,
        }
    }

    pub fn credential_binding(
        &self,
    ) -> Result<Option<crate::DecisionCredentialBindingV1>, crate::NativeCredentialContractErrorV1>
    {
        if !self.validate() {
            return Err(crate::NativeCredentialContractErrorV1::InvalidRequest);
        }
        match self.versioned_connection() {
            Some(connection) => crate::DecisionCredentialBindingV1::for_connection(
                self.id(),
                self.revision(),
                connection,
            ),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests;
