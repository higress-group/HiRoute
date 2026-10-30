//! Plan exact source membership changes without reading or copying Key material.
use crate::{
    CredentialRefV1, DecisionAuthenticationV2, DecisionConnectionV2, DecisionServiceV2,
    SecretMutationV1,
};

use super::{
    COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V4, ComputeDecisionTargetV4, ComputeManagementErrorV2,
    ComputeManagementSourceV2,
};

/// The Secret effects and both source versions are sealed into one journaled Operation.
#[derive(Clone, Debug)]
pub struct DecisionSourceTransitionV1 {
    pub before: ComputeManagementSourceV2,
    pub after: ComputeManagementSourceV2,
    pub secrets: Vec<SecretMutationV1>,
}

pub fn register_decision_consumer(
    source: &ComputeManagementSourceV2,
    service: &DecisionServiceV2,
) -> Result<DecisionSourceTransitionV1, ComputeManagementErrorV2> {
    source.validate()?;
    if !service.validate() || !source.provenance.is_native() {
        return Err(ComputeManagementErrorV2::InvalidTarget);
    }
    let DecisionConnectionV2::Native {
        endpoint,
        wire_protocol,
        model,
        authentication:
            DecisionAuthenticationV2::SharedSource {
                source_id,
                source_lineage_digest,
            },
        ..
    } = &service.connection
    else {
        return Err(ComputeManagementErrorV2::InvalidTarget);
    };
    if source_id != &source.source_id || source_lineage_digest != &source.lineage_digest {
        return Err(ComputeManagementErrorV2::RevisionConflict);
    }
    let mut next = source.clone();
    next.schema = COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V4.into();
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or(ComputeManagementErrorV2::InvalidMutation)?;
    let target = ComputeDecisionTargetV4::new(endpoint, *wire_protocol)?;
    let consumer = target.consumer(
        source_id,
        source_lineage_digest,
        &service.id,
        service.revision,
        model,
    )?;
    if next.decision_targets.iter().any(|target| {
        target
            .consumers
            .iter()
            .any(|old| old.service_id == service.id && old.revision == service.revision)
    }) {
        return Err(ComputeManagementErrorV2::DuplicateIdentity);
    }
    if let Some(existing) = next
        .decision_targets
        .iter_mut()
        .find(|item| item.destination_id == target.destination_id)
    {
        existing.consumers.push(consumer);
        existing
            .consumers
            .sort_by(|a, b| (&a.service_id, a.revision).cmp(&(&b.service_id, b.revision)));
    } else {
        let mut target = target;
        target.consumers.push(consumer);
        next.decision_targets.push(target);
        next.decision_targets
            .sort_by(|a, b| a.destination_id.cmp(&b.destination_id));
    }
    finalize(source, next)
}

/// Remove exactly one service's retained consumers, leaving every other owner intact.
pub fn remove_decision_consumers(
    source: &ComputeManagementSourceV2,
    service_id: &str,
) -> Result<Option<DecisionSourceTransitionV1>, ComputeManagementErrorV2> {
    source.validate()?;
    super::validate_identifier(service_id)?;
    let mut next = source.clone();
    let before_count: usize = next
        .decision_targets
        .iter()
        .map(|target| target.consumers.len())
        .sum();
    for target in &mut next.decision_targets {
        target
            .consumers
            .retain(|consumer| consumer.service_id != service_id);
    }
    let after_count: usize = next
        .decision_targets
        .iter()
        .map(|target| target.consumers.len())
        .sum();
    if before_count == after_count {
        return Ok(None);
    }
    next.decision_targets
        .retain(|target| !target.consumers.is_empty());
    next.schema = COMPUTE_MANAGEMENT_SOURCE_SCHEMA_V4.into();
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or(ComputeManagementErrorV2::InvalidMutation)?;
    finalize(source, next).map(Some)
}

fn finalize(
    before: &ComputeManagementSourceV2,
    mut after: ComputeManagementSourceV2,
) -> Result<DecisionSourceTransitionV1, ComputeManagementErrorV2> {
    let old_destinations = before.native_destinations()?;
    let new_destinations = after.native_destinations()?;
    let mut secrets = Vec::new();
    if old_destinations != new_destinations {
        for key in &mut after.credentials {
            let prior = &key.credential;
            let generation = prior
                .generation()
                .checked_add(1)
                .ok_or(ComputeManagementErrorV2::InvalidCredential)?;
            secrets.push(
                SecretMutationV1::rebind(
                    prior.clone(),
                    new_destinations.clone(),
                    key.fingerprint.clone(),
                )
                .map_err(|_| ComputeManagementErrorV2::InvalidCredential)?,
            );
            key.credential = CredentialRefV1::new(
                prior.credential_id(),
                prior.owner_scope(),
                prior.subject(),
                prior.purpose(),
                new_destinations.iter().cloned(),
                generation,
            )
            .map_err(|_| ComputeManagementErrorV2::InvalidCredential)?;
        }
    }
    secrets.sort_by(|a, b| {
        a.credential()
            .credential_id()
            .cmp(b.credential().credential_id())
    });
    after.validate()?;
    Ok(DecisionSourceTransitionV1 {
        before: before.clone(),
        after,
        secrets,
    })
}
