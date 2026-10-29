//! Transactionally maintained, keyed projection of current decision membership.
use hiroute_domain::{
    CanonicalDigest, ComputeManagementSourceV2, DecisionAuthenticationV2, DecisionConnectionV2,
    DecisionCredentialAdmissionV1, DecisionCredentialBindingV1, DecisionCredentialKeyV1,
    DecisionCredentialLeaseRequestV1, DecisionServiceV2, MaterializationState,
    NativeCredentialAccessDeniedV1, NativeCredentialLeaseErrorV1, NativeCredentialLeaseV1,
    PortError, PortErrorCode, PortResult, WorkspaceId,
};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{encode, invalid, port};

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RuntimeKey {
    key_id: String,
    generation: u64,
    ordinal: u32,
    enabled: bool,
}

pub(super) fn write_source(
    transaction: &Transaction<'_>,
    workspace: &WorkspaceId,
    source: &ComputeManagementSourceV2,
    owner: &str,
) -> PortResult<()> {
    // Recovery may finish a retained older-target batch before opening v34.
    let schema: u32 = transaction
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| port("decision.runtime.schema"))?;
    if schema < 34 {
        return Ok(());
    }
    if source.decision_targets.is_empty() {
        transaction
            .execute(
                "DELETE FROM decision_source_runtime WHERE workspace_id=?1 AND source_id=?2",
                params![workspace.as_str(), source.source_id],
            )
            .map_err(|_| port("decision.runtime.delete"))?;
        return Ok(());
    }
    let targets_digest = CanonicalDigest::of(&source.decision_targets)
        .map_err(|_| invalid("decision.runtime.targets_digest"))?;
    let previous: Option<String> = transaction.query_row(
        "SELECT targets_digest FROM decision_source_runtime WHERE workspace_id=?1 AND source_id=?2",
        params![workspace.as_str(), source.source_id], |row| row.get(0),
    ).optional().map_err(|_| port("decision.runtime.previous"))?;
    let credentials = source
        .credentials
        .iter()
        .map(|key| RuntimeKey {
            key_id: key.key_id.clone(),
            generation: key.credential.generation(),
            ordinal: key.ordinal,
            enabled: key.enabled,
        })
        .collect::<Vec<_>>();
    let state =
        serde_json::to_value(source.state).map_err(|_| invalid("decision.runtime.state"))?;
    let state = state
        .as_str()
        .ok_or_else(|| invalid("decision.runtime.state"))?;
    transaction.execute(
        "INSERT INTO decision_source_runtime(workspace_id,source_id,lineage_digest,source_revision,source_owner_operation_id,state,targets_digest,credentials_json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
         ON CONFLICT(workspace_id,source_id) DO UPDATE SET lineage_digest=excluded.lineage_digest,source_revision=excluded.source_revision,source_owner_operation_id=excluded.source_owner_operation_id,state=excluded.state,targets_digest=excluded.targets_digest,credentials_json=excluded.credentials_json",
        params![workspace.as_str(), source.source_id, source.lineage_digest.as_str(), source.revision, owner, state, targets_digest.as_str(), encode(&credentials)?],
    ).map_err(|_| port("decision.runtime.source"))?;
    if previous.as_deref() == Some(targets_digest.as_str()) {
        return Ok(());
    }
    transaction
        .execute(
            "DELETE FROM decision_consumer_runtime WHERE workspace_id=?1 AND source_id=?2",
            params![workspace.as_str(), source.source_id],
        )
        .map_err(|_| port("decision.runtime.clear_consumers"))?;
    for target in &source.decision_targets {
        let codec = serde_json::to_value(target.wire_protocol)
            .map_err(|_| invalid("decision.runtime.codec"))?;
        let codec = codec
            .as_str()
            .ok_or_else(|| invalid("decision.runtime.codec"))?;
        for consumer in &target.consumers {
            transaction.execute(
                "INSERT INTO decision_consumer_runtime(workspace_id,source_id,consumer_binding_id,service_id,service_revision,model,endpoint,wire_protocol,destination_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![workspace.as_str(), source.source_id, consumer.consumer_binding_id, consumer.service_id, consumer.revision, consumer.model, target.endpoint, codec, target.destination_id],
            ).map_err(|_| port("decision.runtime.consumer"))?;
        }
    }
    Ok(())
}

fn denied(context: &'static str) -> NativeCredentialLeaseErrorV1 {
    PortError::new(PortErrorCode::PermissionDenied, context).into()
}

impl crate::LocalStorageSet {
    /// One indexed source and consumer lookup; no enumeration of unrelated sources or revisions.
    pub fn admit_decision_credentials(
        &self,
        binding: &DecisionCredentialBindingV1,
    ) -> Result<DecisionCredentialAdmissionV1, NativeCredentialLeaseErrorV1> {
        let (state, keys) = self.read_decision_binding(binding)?;
        let revision = CanonicalDigest::of(&(
            &binding.source_id,
            &binding.source_lineage_digest,
            state,
            &binding.destination_id,
            &binding.consumer_binding_id,
            &keys,
        ))
        .map_err(|_| denied("decision.runtime.state_digest"))?;
        Ok(DecisionCredentialAdmissionV1 {
            state,
            keys: keys
                .iter()
                .filter(|key| key.enabled && state == MaterializationState::Ready)
                .map(|key| DecisionCredentialKeyV1 {
                    key_id: key.key_id.clone(),
                    generation: key.generation,
                })
                .collect(),
            credential_state_revision: revision,
        })
    }

    pub fn lease_decision_credential_exact(
        &self,
        request: &DecisionCredentialLeaseRequestV1,
    ) -> Result<Option<NativeCredentialLeaseV1>, NativeCredentialLeaseErrorV1> {
        request
            .validate()
            .map_err(|_| denied("decision.runtime.request"))?;
        let (state, keys) = self.read_decision_binding(&request.binding)?;
        if state == MaterializationState::Disabled {
            return Err(NativeCredentialLeaseErrorV1::Denied(
                NativeCredentialAccessDeniedV1::SourceDisabled,
            ));
        }
        if state != MaterializationState::Ready {
            return Ok(None);
        }
        let admitted = request
            .admitted_keys
            .iter()
            .find(|key| key.key_id == request.key_id)
            .ok_or_else(|| denied("decision.runtime.admission"))?;
        let Some(current) = keys.iter().find(|key| key.key_id == request.key_id) else {
            return Ok(None);
        };
        if !current.enabled {
            return Err(NativeCredentialLeaseErrorV1::Denied(
                NativeCredentialAccessDeniedV1::CredentialDisabled,
            ));
        }
        if current.generation != admitted.generation {
            return Ok(None);
        }
        self.secrets().lease_decision_credential_exact(
            &request.binding.source_id,
            &request.binding.destination_id,
            &request.key_id,
            admitted.generation,
        )
    }

    fn read_decision_binding(
        &self,
        binding: &DecisionCredentialBindingV1,
    ) -> Result<(MaterializationState, Vec<RuntimeKey>), NativeCredentialLeaseErrorV1> {
        binding
            .validate()
            .map_err(|_| denied("decision.runtime.binding"))?;
        let workspace = WorkspaceId::default();
        let control = self.control();
        let connection = control.connection.borrow();
        let row: Option<(String, String, String)> = connection.query_row(
            "SELECT r.lineage_digest,r.state,r.credentials_json FROM decision_source_runtime r JOIN compute_management_sources s USING(workspace_id,source_id) WHERE r.workspace_id=?1 AND r.source_id=?2 AND r.source_revision=s.revision AND r.source_owner_operation_id=s.owner_operation_id AND r.lineage_digest=s.lineage_digest",
            params![workspace.as_str(), binding.source_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional().map_err(|_| port("decision.runtime.read_source"))?;
        let Some((lineage, state, keys)) = row else {
            return Err(denied("decision.runtime.source_or_projection_missing"));
        };
        if lineage != binding.source_lineage_digest.as_str() {
            return Err(denied("decision.runtime.lineage"));
        }
        let codec = serde_json::to_value(binding.wire_protocol)
            .map_err(|_| denied("decision.runtime.codec"))?;
        let valid: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM decision_consumer_runtime WHERE workspace_id=?1 AND source_id=?2 AND consumer_binding_id=?3 AND service_id=?4 AND service_revision=?5 AND model=?6 AND endpoint=?7 AND wire_protocol=?8 AND destination_id=?9)",
            params![workspace.as_str(), binding.source_id, binding.consumer_binding_id, binding.service_id, binding.service_revision, binding.model, binding.endpoint, codec.as_str(), binding.destination_id], |row| row.get(0),
        ).map_err(|_| port("decision.runtime.read_consumer"))?;
        if !valid {
            return Err(denied("decision.runtime.consumer"));
        }
        let service: Option<String> = connection.query_row(
            "SELECT service_json FROM decision_services WHERE workspace_id=?1 AND service_id=?2 AND revision=?3",
            params![workspace.as_str(), binding.service_id, binding.service_revision], |row| row.get(0),
        ).optional().map_err(|_| port("decision.runtime.read_service"))?;
        let service: DecisionServiceV2 = serde_json::from_str(
            &service.ok_or_else(|| denied("decision.runtime.service_missing"))?,
        )
        .map_err(|_| denied("decision.runtime.service_invalid"))?;
        let matches = matches!(&service.connection,
            DecisionConnectionV2::Native { model, endpoint, wire_protocol,
                authentication: DecisionAuthenticationV2::SharedSource { source_id, source_lineage_digest }, .. }
                if model == &binding.model && *wire_protocol == binding.wire_protocol && source_id == &binding.source_id
                    && source_lineage_digest == &binding.source_lineage_digest
                    && hiroute_domain::canonical_decision_endpoint(endpoint).is_ok_and(|url| url == binding.endpoint)
        );
        if !service.validate()
            || service.id != binding.service_id
            || service.revision != binding.service_revision
            || !matches
        {
            return Err(denied("decision.runtime.service_binding"));
        }
        let state: MaterializationState = serde_json::from_value(serde_json::Value::String(state))
            .map_err(|_| denied("decision.runtime.state"))?;
        let keys: Vec<RuntimeKey> =
            serde_json::from_str(&keys).map_err(|_| denied("decision.runtime.keys"))?;
        if keys
            .iter()
            .enumerate()
            .any(|(index, key)| key.ordinal as usize != index || key.generation == 0)
        {
            return Err(denied("decision.runtime.key_order"));
        }
        Ok((state, keys))
    }
}
