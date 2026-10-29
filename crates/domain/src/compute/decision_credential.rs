//! Exact, in-process authorization for a published decision consumer.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    CanonicalDigest, ComputeDecisionTargetV4, DecisionTargetAuthenticationV1,
    DecisionWireProtocolV2, NativeCredentialContractErrorV1,
};

/// Opaque identity of the exact protected header generation admitted by a diagnostic.
pub fn decision_header_state_revision(
    secret_id: &str,
    generation: u64,
) -> Result<CanonicalDigest, crate::CanonicalDigestError> {
    CanonicalDigest::of(&("hiroute.decision-header-state/v1", secret_id, generation))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCredentialBindingV1 {
    pub source_id: String,
    pub source_lineage_digest: CanonicalDigest,
    pub service_id: String,
    pub service_revision: u64,
    pub consumer_binding_id: String,
    pub model: String,
    pub endpoint: String,
    pub wire_protocol: DecisionWireProtocolV2,
    pub authentication: DecisionTargetAuthenticationV1,
    pub destination_id: String,
}

impl DecisionCredentialBindingV1 {
    pub fn for_service(
        service: &crate::DecisionServiceV2,
    ) -> Result<Option<Self>, NativeCredentialContractErrorV1> {
        use crate::{DecisionAuthenticationV2, DecisionConnectionV2};
        if !service.validate() {
            return Err(NativeCredentialContractErrorV1::InvalidRequest);
        }
        let DecisionConnectionV2::Native {
            model,
            endpoint,
            wire_protocol,
            authentication:
                DecisionAuthenticationV2::SharedSource {
                    source_id,
                    source_lineage_digest,
                },
            ..
        } = &service.connection
        else {
            return Ok(None);
        };
        let invalid = || NativeCredentialContractErrorV1::InvalidRequest;
        let target =
            ComputeDecisionTargetV4::new(endpoint, *wire_protocol).map_err(|_| invalid())?;
        let consumer = target
            .consumer(
                source_id,
                source_lineage_digest,
                &service.id,
                service.revision,
                model,
            )
            .map_err(|_| invalid())?;
        let binding = Self {
            source_id: source_id.clone(),
            source_lineage_digest: source_lineage_digest.clone(),
            service_id: service.id.clone(),
            service_revision: service.revision,
            consumer_binding_id: consumer.consumer_binding_id,
            model: model.clone(),
            endpoint: target.endpoint,
            wire_protocol: *wire_protocol,
            authentication: target.authentication,
            destination_id: target.destination_id,
        };
        binding.validate()?;
        Ok(Some(binding))
    }

    pub fn validate(&self) -> Result<(), NativeCredentialContractErrorV1> {
        let invalid = || NativeCredentialContractErrorV1::InvalidRequest;
        if !matches!(CanonicalDigest::parse(self.source_lineage_digest.as_str()), Ok(parsed) if parsed == self.source_lineage_digest)
        {
            return Err(invalid());
        }
        let target = ComputeDecisionTargetV4::new(&self.endpoint, self.wire_protocol)
            .map_err(|_| invalid())?;
        let consumer = target
            .consumer(
                &self.source_id,
                &self.source_lineage_digest,
                &self.service_id,
                self.service_revision,
                &self.model,
            )
            .map_err(|_| invalid())?;
        if target.endpoint != self.endpoint
            || target.authentication != self.authentication
            || target.destination_id != self.destination_id
            || consumer.consumer_binding_id != self.consumer_binding_id
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCredentialKeyV1 {
    pub key_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCredentialAdmissionV1 {
    pub state: crate::MaterializationState,
    pub keys: Vec<DecisionCredentialKeyV1>,
    pub credential_state_revision: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionCredentialLeaseRequestV1 {
    pub binding: DecisionCredentialBindingV1,
    pub admitted_keys: Vec<DecisionCredentialKeyV1>,
    pub excluded_key_ids: Vec<String>,
    pub key_id: String,
}

impl DecisionCredentialLeaseRequestV1 {
    pub fn validate(&self) -> Result<(), NativeCredentialContractErrorV1> {
        self.binding.validate()?;
        let invalid = || NativeCredentialContractErrorV1::InvalidRequest;
        let mut admitted = BTreeSet::new();
        for key in &self.admitted_keys {
            if super::common::validate_identifier(&key.key_id).is_err()
                || key.generation == 0
                || !admitted.insert(key.key_id.as_str())
            {
                return Err(invalid());
            }
        }
        let mut excluded = BTreeSet::new();
        if self
            .excluded_key_ids
            .iter()
            .any(|key| !admitted.contains(key.as_str()) || !excluded.insert(key.as_str()))
            || !admitted.contains(self.key_id.as_str())
            || excluded.contains(self.key_id.as_str())
        {
            return Err(invalid());
        }
        Ok(())
    }
}
