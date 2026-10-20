use hiroute_domain::{
    AuthenticationKind, BillingClass, CanonicalDigest, ComputeInventorySnapshotV1,
    ComputeSourceControlPort, ComputeSourceMutationV1, ComputeSourceV1, ConnectorRegistryBundleV1,
    ControlRepositoryPort, CredentialPoolControlPort, CredentialPoolMutationV1, CredentialPoolV1,
    EffectReconciliation, MaterializationState, ModelDataBundleV1, ObservedModelV1, OperationId,
    OwnedEffectV1, PortError, PortErrorCode, PortResult, PriceOverrideOperation, PriceOverrideV1,
    SourceBindingV1, WorkspaceId,
};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use super::ControlStore;

mod effects;
pub(super) mod management;
mod pools;
pub(super) mod projection;
mod source_effects;
mod subscriptions;

pub(super) fn begin_save_handoff_in(
    transaction: &rusqlite::Transaction<'_>,
    operation: &hiroute_domain::OperationV1,
) -> hiroute_domain::PortResult<()> {
    subscriptions::begin_save_handoff_in(transaction, operation)
}

pub(super) fn finish_save_handoff_in(
    transaction: &rusqlite::Transaction<'_>,
    operation: &hiroute_domain::OperationV1,
) -> hiroute_domain::PortResult<()> {
    subscriptions::finish_save_handoff_in(transaction, operation)
}

pub(super) use effects::write_credential_pool_in;
pub(super) use source_effects::write_compute_source_in;
pub use subscriptions::{
    ComputeSubscriptionValidationRecordV1, ComputeSubscriptionValidationStateV1,
};

pub type InventorySnapshotV1 = ComputeInventorySnapshotV1;

impl ControlStore {
    pub fn compute_projection_expectation(
        &self,
        source_id: &str,
        binding_id: &str,
        endpoint_profile_id: &str,
    ) -> PortResult<hiroute_domain::ComputeProjectionExpectationV1> {
        projection::expectation(
            &self.connection.borrow(),
            source_id,
            binding_id,
            endpoint_profile_id,
        )
    }

    pub fn compute_projection_rows(
        &self,
    ) -> PortResult<Vec<(ComputeSourceV1, SourceBindingV1, InventorySnapshotV1)>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT s.source_json, b.binding_json, i.inventory_revision,
                        i.inventory_digest, i.observed_models_json, i.captured_at,
                        i.endpoint_profile_id
                 FROM compute_sources s
                 JOIN source_bindings b ON b.source_id=s.source_id AND b.active=1
                 JOIN source_inventory_snapshots i ON i.source_id=s.source_id
                 ORDER BY b.binding_id",
            )
            .map_err(|_| port("compute.projection.list"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|_| port("compute.projection.rows"))?;
        let mut result = Vec::new();
        for row in rows {
            let (source, binding, revision, digest, observed, captured_at, endpoint) =
                row.map_err(|_| port("compute.projection.row"))?;
            let source: ComputeSourceV1 = decode(&source)?;
            let binding: SourceBindingV1 = decode(&binding)?;
            let inventory = InventorySnapshotV1 {
                source_id: source.source_id.clone(),
                endpoint_profile_id: endpoint,
                inventory_revision: revision,
                inventory_digest: CanonicalDigest::parse(digest)
                    .map_err(|_| invalid("compute.projection.inventory_digest"))?,
                observed_models: decode(&observed)?,
                captured_at,
            };
            source
                .validate_shape()
                .map_err(|_| invalid("compute.projection.source"))?;
            binding
                .validate_shape()
                .map_err(|_| invalid("compute.projection.binding"))?;
            inventory
                .validate()
                .map_err(|_| invalid("compute.projection.inventory"))?;
            if binding.source_id != source.source_id
                || binding.source_revision != source.revision
                || binding.source_identity_digest != source.identity_digest
                || inventory.endpoint_profile_id != source.identity.endpoint_profile_id
            {
                return Err(invalid("compute.projection.cross_reference"));
            }
            result.push((source, binding, inventory));
        }
        Ok(result)
    }

    /// Publishes only the optimistic-concurrency revision for the single catalog embedded in the
    /// running daemon. Catalog bytes and catalog history are deliberately not installed here.
    pub fn activate_embedded_release_catalog(
        &self,
        workspace: &WorkspaceId,
        sequence: u64,
    ) -> PortResult<()> {
        if sequence == 0 {
            return Err(invalid("compute.release.sequence"));
        }
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port("compute.release.begin"))?;
        install_dependency_revision_in(&transaction, workspace, "release.registry", sequence)?;
        install_dependency_revision_in(&transaction, workspace, "release.model_data", sequence)?;
        transaction
            .commit()
            .map_err(|_| port("compute.release.commit"))
    }

    /// The daemon binds this revision to its captured immutable feed catalog snapshot.
    pub fn activate_model_feed_generation(
        &self,
        workspace: &WorkspaceId,
        generation: u64,
        accepted_sequence: u64,
    ) -> PortResult<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port("compute.feed.begin"))?;
        install_dependency_revision_in(&transaction, workspace, "feed.model_data", generation)?;
        install_dependency_revision_in(
            &transaction,
            workspace,
            "feed.sequence",
            accepted_sequence,
        )?;
        transaction
            .commit()
            .map_err(|_| port("compute.feed.commit"))
    }

    pub fn put_compute_source(
        &self,
        expected_revision: u64,
        source: &ComputeSourceV1,
        registry: &ConnectorRegistryBundleV1,
        explicit_materialization: bool,
    ) -> PortResult<()> {
        registry
            .validate()
            .map_err(|_| invalid("compute.registry.validate"))?;
        source
            .validate(registry, explicit_materialization)
            .map_err(|_| invalid("compute.source.validate"))?;
        put_revisioned(
            &mut self.connection.borrow_mut(),
            "compute_sources",
            "source_id",
            &source.source_id,
            "source_json",
            source.revision,
            expected_revision,
            source,
            Some(("identity_digest", source.identity_digest.as_str())),
        )
    }

    pub fn compute_source(&self, source_id: &str) -> PortResult<Option<ComputeSourceV1>> {
        let physical_source_id =
            projection::physical_source_id(&self.connection.borrow(), source_id)?;
        let source: Option<ComputeSourceV1> = get_json(
            &self.connection.borrow(),
            "compute_sources",
            "source_id",
            &physical_source_id,
            "source_json",
        )?;
        source
            .map(|source| {
                if source.source_id != physical_source_id {
                    return Err(invalid("compute.source.identity"));
                }
                source
                    .validate_shape()
                    .map_err(|_| invalid("compute.source.validate"))?;
                Ok(source)
            })
            .transpose()
    }

    pub fn put_inventory_snapshot(&self, snapshot: &InventorySnapshotV1) -> PortResult<()> {
        validate_storage_id(&snapshot.source_id)?;
        validate_storage_id(&snapshot.endpoint_profile_id)?;
        if snapshot.inventory_revision == 0
            || snapshot.captured_at <= 0
            || snapshot.observed_models.len() > 10_000
        {
            return Err(invalid("compute.inventory.shape"));
        }
        let source = self
            .compute_source(&snapshot.source_id)?
            .ok_or_else(|| invalid("compute.inventory.source"))?;
        if source.identity.endpoint_profile_id != snapshot.endpoint_profile_id {
            return Err(invalid("compute.inventory.endpoint_profile"));
        }
        for model in &snapshot.observed_models {
            model
                .validate()
                .map_err(|_| invalid("compute.inventory.model"))?;
        }
        if CanonicalDigest::of(&snapshot.observed_models)
            .map_err(|_| invalid("compute.inventory.digest"))?
            != snapshot.inventory_digest
        {
            return Err(invalid("compute.inventory.digest"));
        }
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| port("compute.inventory.begin"))?;
        let current: Option<u64> = transaction
            .query_row(
                "SELECT inventory_revision FROM source_inventory_snapshots
                 WHERE source_id=?1 AND endpoint_profile_id=?2",
                params![snapshot.source_id, snapshot.endpoint_profile_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| port("compute.inventory.lookup"))?;
        if current.unwrap_or(0).checked_add(1) != Some(snapshot.inventory_revision) {
            return Err(conflict("compute.inventory.revision"));
        }
        let encoded = encode(&snapshot.observed_models)?;
        transaction
            .execute(
                "INSERT INTO source_inventory_snapshots(
                source_id, endpoint_profile_id, inventory_revision, inventory_digest,
                observed_models_json, captured_at
             ) VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(source_id,endpoint_profile_id) DO UPDATE SET
                inventory_revision=excluded.inventory_revision,
                inventory_digest=excluded.inventory_digest,
                observed_models_json=excluded.observed_models_json,
                captured_at=excluded.captured_at",
                params![
                    snapshot.source_id,
                    snapshot.endpoint_profile_id,
                    snapshot.inventory_revision,
                    snapshot.inventory_digest.as_str(),
                    encoded,
                    snapshot.captured_at
                ],
            )
            .map_err(|_| port("compute.inventory.write"))?;
        transaction
            .commit()
            .map_err(|_| port("compute.inventory.commit"))
    }

    pub fn inventory_snapshot(
        &self,
        source_id: &str,
        endpoint_profile_id: &str,
    ) -> PortResult<Option<InventorySnapshotV1>> {
        validate_storage_id(source_id)?;
        validate_storage_id(endpoint_profile_id)?;
        let row: Option<(u64, String, String, i64)> = self
            .connection
            .borrow()
            .query_row(
                "SELECT inventory_revision, inventory_digest, observed_models_json, captured_at
                 FROM source_inventory_snapshots WHERE source_id=?1 AND endpoint_profile_id=?2",
                params![source_id, endpoint_profile_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|_| port("compute.inventory.read"))?;
        row.map(|(inventory_revision, digest, encoded, captured_at)| {
            let observed_models: Vec<ObservedModelV1> = decode(&encoded)?;
            for model in &observed_models {
                model
                    .validate()
                    .map_err(|_| invalid("compute.inventory.model"))?;
            }
            let inventory_digest =
                CanonicalDigest::parse(digest).map_err(|_| invalid("compute.inventory.digest"))?;
            if CanonicalDigest::of(&observed_models)
                .map_err(|_| invalid("compute.inventory.digest"))?
                != inventory_digest
            {
                return Err(invalid("compute.inventory.digest"));
            }
            Ok(InventorySnapshotV1 {
                source_id: source_id.to_owned(),
                endpoint_profile_id: endpoint_profile_id.to_owned(),
                inventory_revision,
                inventory_digest,
                observed_models,
                captured_at,
            })
        })
        .transpose()
    }

    pub fn put_price_override(
        &self,
        expected_revision: u64,
        value: &PriceOverrideV1,
        registry: &ConnectorRegistryBundleV1,
        model_data: &ModelDataBundleV1,
    ) -> PortResult<()> {
        registry
            .validate()
            .map_err(|_| invalid("compute.registry.validate"))?;
        model_data
            .validate_against(registry)
            .map_err(|_| invalid("compute.model_data.validate"))?;
        value
            .validate()
            .map_err(|_| invalid("compute.override.validate"))?;
        let offer = model_data
            .offer(&value.offer_ref)
            .filter(|offer| {
                offer
                    .model_configuration_ids
                    .contains(&value.model_configuration_id)
            })
            .ok_or_else(|| invalid("compute.override.offer"))?;
        if value.operation == PriceOverrideOperation::DisableCatalogRule
            && value.target_rule_id.as_deref().is_none_or(|target| {
                !model_data.price_rates.iter().any(|rate| {
                    rate.price_rate_id == target
                        && rate.offer_ref == offer.offer_id
                        && rate.model_configuration_id == value.model_configuration_id
                        && rate.currency == value.currency
                })
            })
        {
            return Err(invalid("compute.override.rule"));
        }
        put_revisioned(
            &mut self.connection.borrow_mut(),
            "price_overrides",
            "override_id",
            &value.override_id,
            "override_json",
            value.revision,
            expected_revision,
            value,
            None,
        )
    }

    pub fn price_overrides(&self) -> PortResult<Vec<PriceOverrideV1>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare("SELECT override_json FROM price_overrides ORDER BY override_id")
            .map_err(|_| port("compute.override.query"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| port("compute.override.rows"))?;
        rows.map(|row| {
            let value: PriceOverrideV1 = decode(&row.map_err(|_| port("compute.override.row"))?)?;
            value
                .validate()
                .map_err(|_| invalid("compute.override.validate"))?;
            Ok(value)
        })
        .collect()
    }
}

fn install_dependency_revision_in(
    transaction: &rusqlite::Transaction<'_>,
    workspace: &WorkspaceId,
    key: &str,
    revision: u64,
) -> PortResult<()> {
    validate_storage_id(key)?;
    let current: Option<u64> = transaction
        .query_row(
            "SELECT revision FROM dependency_revisions
             WHERE workspace_id=?1 AND dependency_key=?2",
            params![workspace.as_str(), key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.release.dependency_read"))?;
    if current.is_some_and(|current| current > revision) {
        return Err(conflict("compute.release.dependency_rollback"));
    }
    transaction
        .execute(
            "INSERT INTO dependency_revisions(workspace_id, dependency_key, revision)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(workspace_id, dependency_key) DO UPDATE SET revision=excluded.revision",
            params![workspace.as_str(), key, revision],
        )
        .map_err(|_| port("compute.release.dependency_write"))?;
    Ok(())
}

fn authentication_kind(value: AuthenticationKind) -> &'static str {
    match value {
        AuthenticationKind::None => "none",
        AuthenticationKind::ProviderApiKey => "provider_api_key",
        AuthenticationKind::ConnectorOwnedOpaque => "connector_owned_opaque",
    }
}

fn billing_class(value: BillingClass) -> &'static str {
    match value {
        BillingClass::Free => "free",
        BillingClass::Paid => "paid",
        BillingClass::Subscription => "subscription",
        BillingClass::Unknown => "unknown",
    }
}

#[allow(clippy::too_many_arguments)]
fn put_revisioned<T: Serialize>(
    connection: &mut rusqlite::Connection,
    table: &str,
    id_column: &str,
    id: &str,
    json_column: &str,
    next_revision: u64,
    expected_revision: u64,
    value: &T,
    extra: Option<(&str, &str)>,
) -> PortResult<()> {
    validate_storage_id(id)?;
    // Table and column names are fixed call-site constants, never user input.
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| port("compute.record.begin"))?;
    let current: Option<u64> = transaction
        .query_row(
            &format!("SELECT revision FROM {table} WHERE {id_column}=?1"),
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.record.lookup"))?;
    if current.unwrap_or(0) != expected_revision
        || expected_revision.checked_add(1) != Some(next_revision)
    {
        return Err(conflict("compute.record.revision"));
    }
    let encoded = encode(value)?;
    if let Some((extra_column, extra_value)) = extra {
        transaction
            .execute(
                &format!(
                    "INSERT INTO {table}({id_column},revision,{extra_column},{json_column},updated_at)
                     VALUES (?1,?2,?3,?4,unixepoch())
                     ON CONFLICT({id_column}) DO UPDATE SET revision=excluded.revision,
                     {extra_column}=excluded.{extra_column},{json_column}=excluded.{json_column},
                     updated_at=excluded.updated_at"
                ),
                params![id, next_revision, extra_value, encoded],
            )
            .map_err(|_| port("compute.record.write"))?;
    } else {
        transaction
            .execute(
                &format!(
                    "INSERT INTO {table}({id_column},revision,{json_column},updated_at)
                     VALUES (?1,?2,?3,unixepoch())
                     ON CONFLICT({id_column}) DO UPDATE SET revision=excluded.revision,
                     {json_column}=excluded.{json_column},updated_at=excluded.updated_at"
                ),
                params![id, next_revision, encoded],
            )
            .map_err(|_| port("compute.record.write"))?;
    }
    transaction
        .commit()
        .map_err(|_| port("compute.record.commit"))
}

fn get_json<T: DeserializeOwned>(
    connection: &rusqlite::Connection,
    table: &str,
    id_column: &str,
    id: &str,
    json_column: &str,
) -> PortResult<Option<T>> {
    validate_storage_id(id)?;
    let value: Option<String> = connection
        .query_row(
            &format!("SELECT {json_column} FROM {table} WHERE {id_column}=?1"),
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.record.read"))?;
    value.map(|value| decode(&value)).transpose()
}

fn encode<T: Serialize>(value: &T) -> PortResult<String> {
    serde_json::to_string(value).map_err(|_| invalid("compute.record.encode"))
}

fn decode<T: DeserializeOwned>(value: &str) -> PortResult<T> {
    serde_json::from_str(value).map_err(|_| invalid("compute.record.decode"))
}

fn validate_storage_id(value: &str) -> PortResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && !value.contains("//")
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
        });
    if valid {
        Ok(())
    } else {
        Err(invalid("compute.identifier"))
    }
}

fn port(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Unavailable, context)
}

fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}

fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}

#[cfg(test)]
mod tests;
