//! An optional import association; it conveys no credentials or routing authority.
use super::*;

pub const COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4: &str = "hiroute.compute-management-change/v4";
pub const COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V4: &str = "hiroute.compute-management-mutation/v4";

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeSubscriptionApiLinkV1 {
    pub source_id: String,
    pub api_source_id: String,
}
impl ComputeSubscriptionApiLinkV1 {
    pub fn validate(&self) -> Result<(), ComputeManagementErrorV2> {
        validate_identifier(&self.source_id)?;
        validate_identifier(&self.api_source_id)?;
        if self.source_id == self.api_source_id {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        Ok(())
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "intent", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputeSubscriptionApiLinkIntentV1 {
    Associate { api_source_id: String },
    Remove,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementMutationV4 {
    pub schema: String,
    pub source_mutation: Box<ComputeManagementMutation>,
    pub api_link_before: Option<ComputeSubscriptionApiLinkV1>,
    pub api_link_after: Option<ComputeSubscriptionApiLinkV1>,
    pub change_spec_digest: CanonicalDigest,
}
impl ComputeManagementMutationV4 {
    pub fn validate(&self, spec: &ChangeSpecV1) -> Result<(), ComputeManagementErrorV2> {
        if self.schema != COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V4
            || matches!(&*self.source_mutation, ComputeManagementMutation::V4(_))
            || spec
                .desired_state
                .get("schema")
                .and_then(serde_json::Value::as_str)
                != Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
            || CanonicalDigest::of(spec).map_err(|_| ComputeManagementErrorV2::InvalidDigest)?
                != self.change_spec_digest
        {
            return Err(ComputeManagementErrorV2::InvalidMutation);
        }
        let source = self
            .source_mutation
            .desired()
            .ok_or(ComputeManagementErrorV2::InvalidMutation)?;
        if !matches!(&source.provenance, ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } if connector_id == "connector.cpa.codex")
        {
            return Err(ComputeManagementErrorV2::InvalidProvenance);
        }
        for link in self
            .api_link_before
            .iter()
            .chain(self.api_link_after.iter())
        {
            link.validate()?;
            if link.source_id != source.source_id {
                return Err(ComputeManagementErrorV2::InvalidMutation);
            }
        }
        let intent: ComputeSubscriptionApiLinkIntentV1 = serde_json::from_value(
            spec.desired_state
                .get("api_link")
                .cloned()
                .ok_or(ComputeManagementErrorV2::InvalidMutation)?,
        )
        .map_err(|_| ComputeManagementErrorV2::InvalidMutation)?;
        match (&intent, &self.api_link_after) {
            (ComputeSubscriptionApiLinkIntentV1::Associate { api_source_id }, Some(after))
                if api_source_id == &after.api_source_id => {}
            (ComputeSubscriptionApiLinkIntentV1::Remove, None) => {}
            _ => return Err(ComputeManagementErrorV2::InvalidMutation),
        }
        let mut original = spec.clone();
        original.desired_state = spec
            .desired_state
            .get("change")
            .cloned()
            .ok_or(ComputeManagementErrorV2::InvalidMutation)?;
        self.source_mutation.validate_for_spec(&original)
    }
}
