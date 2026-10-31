use super::*;
use crate::ComputeManagementSourceV2;

pub const NATIVE_SOURCE_NETWORK_SCHEMA_V1: &str = "hiroute.native-network-policy/v1";
pub const NATIVE_NETWORK_MUTATION_SCHEMA_V1: &str = "hiroute.native-network-mutation/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSourceNetworkRecordV1 {
    pub schema: String,
    pub source_id: String,
    pub lineage_digest: CanonicalDigest,
    pub network_revision: u64,
    pub policy: NativeNetworkPolicyV1,
}

impl NativeSourceNetworkRecordV1 {
    pub fn initial(source: &ComputeManagementSourceV2) -> Result<Self, NativeNetworkErrorV1> {
        let record = Self {
            schema: NATIVE_SOURCE_NETWORK_SCHEMA_V1.into(),
            source_id: source.source_id.clone(),
            lineage_digest: source.lineage_digest.clone(),
            network_revision: 1,
            policy: NativeNetworkPolicyV1::Direct,
        };
        record.validate_for(source)?;
        Ok(record)
    }

    pub fn validate_for(
        &self,
        source: &ComputeManagementSourceV2,
    ) -> Result<(), NativeNetworkErrorV1> {
        if self.schema != NATIVE_SOURCE_NETWORK_SCHEMA_V1
            || self.network_revision == 0
            || !source.provenance.is_native()
            || self.source_id != source.source_id
            || self.lineage_digest != source.lineage_digest
            || self.policy.normalized()? != self.policy
        {
            return Err(NativeNetworkErrorV1::InvalidState);
        }
        Ok(())
    }
}

/// Companion state is bound to the source mutation in the same Operation. Historical source
/// JSON remains unchanged; an absent new-format row is never interpreted as Direct.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeNetworkMutationV1 {
    pub schema: String,
    pub before: Option<NativeSourceNetworkRecordV1>,
    pub after: Option<NativeSourceNetworkRecordV1>,
}

impl NativeNetworkMutationV1 {
    pub fn validate(
        &self,
        expected: Option<&ComputeManagementSourceV2>,
        desired: Option<&ComputeManagementSourceV2>,
    ) -> Result<(), NativeNetworkErrorV1> {
        if self.schema != NATIVE_NETWORK_MUTATION_SCHEMA_V1 {
            return Err(NativeNetworkErrorV1::InvalidState);
        }
        for (source, record) in [
            (expected, self.before.as_ref()),
            (desired, self.after.as_ref()),
        ] {
            match (source.filter(|s| s.provenance.is_native()), record) {
                (None, None) => {}
                (Some(source), Some(record)) => record.validate_for(source)?,
                _ => return Err(NativeNetworkErrorV1::InvalidState),
            }
        }
        if let Some(after) = &self.after {
            let revision = match &self.before {
                Some(before) if before.policy == after.policy => before.network_revision,
                Some(before) => before
                    .network_revision
                    .checked_add(1)
                    .ok_or(NativeNetworkErrorV1::InvalidState)?,
                None => 1,
            };
            if after.network_revision != revision {
                return Err(NativeNetworkErrorV1::InvalidState);
            }
        }
        Ok(())
    }
}
