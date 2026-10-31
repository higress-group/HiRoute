//! The policy companion shares source CAS, transaction ownership and compensation witnesses.
use super::*;
use hiroute_domain::NativeSourceNetworkRecordV1;

fn stored(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source: &str,
) -> PortResult<Option<NativeSourceNetworkRecordV1>> {
    let json: Option<String> = connection
        .query_row(
            "SELECT record_json FROM native_source_network WHERE workspace_id=?1 AND source_id=?2",
            params![workspace.as_str(), source],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| port("native.network.read"))?;
    json.map(|v| decode(&v)).transpose()
}

fn validate(
    source: Option<&ComputeManagementSourceV2>,
    record: Option<&NativeSourceNetworkRecordV1>,
) -> PortResult<()> {
    match (source.filter(|s| s.provenance.is_native()), record) {
        (Some(source), Some(record)) => record
            .validate_for(source)
            .map_err(|_| corrupt("native.network.closure")),
        (None, None) => Ok(()),
        _ => Err(corrupt("native.network.missing_or_orphan")),
    }
}

pub(super) fn read(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source: &str,
) -> PortResult<Option<NativeSourceNetworkRecordV1>> {
    let record = stored(connection, workspace, source)?;
    validate(
        read_source(connection, workspace, source)?.as_ref(),
        record.as_ref(),
    )?;
    Ok(record)
}

pub(super) fn after(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    mutation: &ComputeManagementMutation,
) -> PortResult<Option<NativeSourceNetworkRecordV1>> {
    let before = read(connection, workspace, mutation.source_id())?;
    let after = if let Some(network) = mutation.network() {
        network
            .validate(mutation.expected(), mutation.desired())
            .map_err(|_| invalid("native.network.mutation"))?;
        if network.before != before {
            return Err(conflict("native.network.cas"));
        }
        network.after.clone()
    } else {
        match mutation.desired().filter(|s| s.provenance.is_native()) {
            Some(source) => Some(match before {
                Some(record) => record,
                None => NativeSourceNetworkRecordV1::initial(source)
                    .map_err(|_| invalid("native.network.initial"))?,
            }),
            None => None,
        }
    };
    validate(mutation.desired(), after.as_ref())?;
    Ok(after)
}

pub(super) fn stage(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    operation: &str,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    let before = read(connection, workspace, mutation.source_id())?;
    let after = after(connection, workspace, mutation)?;
    connection.execute("UPDATE compute_management_effects SET network_before=?2,network_after=?3 WHERE operation_id=?1",
        params![operation, encode(&before)?, encode(&after)?])
        .map_err(|_| port("native.network.stage"))?;
    Ok(())
}

pub(super) fn write(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source: &str,
    record: Option<&NativeSourceNetworkRecordV1>,
    operation: &str,
) -> PortResult<()> {
    validate(read_source(connection, workspace, source)?.as_ref(), record)?;
    match record {
        Some(record) => {
            connection.execute("INSERT INTO native_source_network(workspace_id,source_id,record_json,owner_operation_id) VALUES(?1,?2,?3,?4) ON CONFLICT(workspace_id,source_id) DO UPDATE SET record_json=excluded.record_json,owner_operation_id=excluded.owner_operation_id",
            params![workspace.as_str(), source, encode(record)?, operation])
            .map_err(|_| port("native.network.write"))?;
        }
        None => {
            connection
                .execute(
                    "DELETE FROM native_source_network WHERE workspace_id=?1 AND source_id=?2",
                    params![workspace.as_str(), source],
                )
                .map_err(|_| port("native.network.delete"))?;
        }
    }
    Ok(())
}

/// Outer None means an authentic retained pre-policy effect; inner None is an explicit deletion.
pub(super) fn effect(
    connection: &rusqlite::Connection,
    operation: &str,
    before: bool,
) -> PortResult<Option<Option<NativeSourceNetworkRecordV1>>> {
    let column = if before {
        "network_before"
    } else {
        "network_after"
    };
    let json: Option<String> = connection
        .query_row(
            &format!("SELECT {column} FROM compute_management_effects WHERE operation_id=?1"),
            params![operation],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| port("native.network.effect"))?
        .flatten();
    json.map(|v| decode(&v)).transpose()
}

pub(super) fn restore(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    operation: &str,
    before_owner: &str,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    let before = match effect(connection, operation, true)? {
        Some(record) => record,
        None if mutation.network().is_none() => {
            match mutation.expected().filter(|s| s.provenance.is_native()) {
                Some(source) => Some(
                    stored(connection, workspace, mutation.source_id())?.unwrap_or(
                        NativeSourceNetworkRecordV1::initial(source)
                            .map_err(|_| corrupt("native.network.legacy_restore"))?,
                    ),
                ),
                None => None,
            }
        }
        None => return Err(corrupt("native.network.effect_missing")),
    };
    write(
        connection,
        workspace,
        mutation.source_id(),
        before.as_ref(),
        before_owner,
    )
}
