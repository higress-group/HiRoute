use super::*;
use crate::{CanonicalDigest, ChangeSpecV1, ComputeManagementErrorV2, ComputeManagementMutation};

pub const COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4: &str = "hiroute.compute-management-change/v4";
pub const COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V4: &str = "hiroute.compute-management-mutation/v4";

/// An explicit wrapper retains the original V2/V3 source/default proof byte-for-byte.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeManagementMutationV4 {
    pub schema: String,
    pub source_mutation: Box<ComputeManagementMutation>,
    pub network: NativeNetworkMutationV1,
    pub change_spec_digest: CanonicalDigest,
}

impl ComputeManagementMutationV4 {
    pub fn validate(&self, spec: &ChangeSpecV1) -> Result<(), ComputeManagementErrorV2> {
        let invalid = ComputeManagementErrorV2::InvalidMutation;
        if self.schema != COMPUTE_MANAGEMENT_MUTATION_SCHEMA_V4
            || spec.command_id != "compute.connection.apply"
            || spec
                .desired_state
                .get("schema")
                .and_then(serde_json::Value::as_str)
                != Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
            || CanonicalDigest::of(spec).map_err(|_| invalid)? != self.change_spec_digest
            || spec.desired_state.get("network")
                != Some(&serde_json::to_value(&self.network).map_err(|_| invalid)?)
        {
            return Err(invalid);
        }
        let mut original = spec.clone();
        original.desired_state = spec.desired_state.get("change").cloned().ok_or(invalid)?;
        match self.source_mutation.as_ref() {
            ComputeManagementMutation::V2(v) => v.validate_shape(&original)?,
            ComputeManagementMutation::V3(v) => v.validate(&original)?,
            ComputeManagementMutation::V4(_) => return Err(invalid),
        }
        self.network
            .validate(
                self.source_mutation.expected(),
                self.source_mutation.desired(),
            )
            .map_err(|_| invalid)
    }
}

/// Exempt only admitted proxy URL fields from the generic effect-URL feature gate.
/// All other fields, including the complete inner V2/V3 intent, retain normal admission.
/// Used both when admitting an operation and when reconstructing its sealed journal.
pub fn native_network_feature_gate_state(
    spec: &ChangeSpecV1,
) -> Result<std::borrow::Cow<'_, serde_json::Value>, NativeNetworkErrorV1> {
    if spec.command_id != "compute.connection.apply"
        || spec
            .desired_state
            .get("schema")
            .and_then(serde_json::Value::as_str)
            != Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V4)
    {
        return Ok(std::borrow::Cow::Borrowed(&spec.desired_state));
    }
    let network: NativeNetworkMutationV1 = serde_json::from_value(
        spec.desired_state
            .get("network")
            .cloned()
            .ok_or(NativeNetworkErrorV1::InvalidState)?,
    )
    .map_err(|_| NativeNetworkErrorV1::InvalidState)?;
    for record in [&network.before, &network.after].into_iter().flatten() {
        if record.policy.normalized()? != record.policy {
            return Err(NativeNetworkErrorV1::InvalidState);
        }
    }
    let mut guarded = spec.desired_state.clone();
    for side in ["before", "after"] {
        if let Some(policy) = guarded
            .get_mut("network")
            .and_then(|n| n.get_mut(side))
            .and_then(|r| r.get_mut("policy"))
            .and_then(serde_json::Value::as_object_mut)
        {
            policy.remove("proxy_url");
        }
    }
    Ok(std::borrow::Cow::Owned(guarded))
}
