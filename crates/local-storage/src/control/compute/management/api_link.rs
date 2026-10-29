//! Association metadata is never credential or model-execution authority.
use super::*;
use hiroute_domain::{ComputeManagementProvenanceV2, ComputeSubscriptionApiLinkV1};

pub(super) fn list(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source_id: Option<&str>,
) -> PortResult<Vec<ComputeSubscriptionApiLinkV1>> {
    let mut statement = connection.prepare(
        "SELECT l.source_id,l.api_source_id,s.source_json,a.source_json
         FROM compute_subscription_api_links l
         LEFT JOIN compute_management_sources s ON s.workspace_id=l.workspace_id AND s.source_id=l.source_id
         LEFT JOIN compute_management_sources a ON a.workspace_id=l.workspace_id AND a.source_id=l.api_source_id
         WHERE l.workspace_id=?1 AND (?2 IS NULL OR l.source_id=?2) ORDER BY l.source_id",
    ).map_err(|_| port("compute.api_link.list"))?;
    let rows = statement
        .query_map(params![workspace.as_str(), source_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|_| port("compute.api_link.query"))?;
    let mut result = Vec::new();
    for row in rows {
        let (source_id, api_source_id, primary, api) =
            row.map_err(|_| port("compute.api_link.row"))?;
        let link = ComputeSubscriptionApiLinkV1 {
            source_id,
            api_source_id,
        };
        link.validate()
            .map_err(|_| corrupt("compute.api_link.identity"))?;
        let source = decode_source(primary, Some(&link.source_id))?
            .ok_or_else(|| corrupt("compute.api_link.source"))?;
        if !is_codex(&source) {
            return Err(corrupt("compute.api_link.provider"));
        }
        // Deleting the API leaves an explicitly unavailable association. It cannot grant access.
        if let Some(api) = decode_source(api, Some(&link.api_source_id))?
            && !api.provenance.is_native()
        {
            return Err(corrupt("compute.api_link.api"));
        }
        result.push(link);
    }
    Ok(result)
}
fn is_codex(source: &ComputeManagementSourceV2) -> bool {
    matches!(&source.provenance, ComputeManagementProvenanceV2::ConnectorOwned { connector_id, .. } if connector_id == "connector.cpa.codex")
}
fn read(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source: &str,
) -> PortResult<Option<ComputeSubscriptionApiLinkV1>> {
    Ok(list(connection, workspace, Some(source))?
        .into_iter()
        .next())
}
pub(super) fn validate_before(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    if let Some((before, after)) = mutation.api_link_change() {
        if &read(connection, workspace, mutation.source_id())? != before {
            return Err(conflict("compute.api_link.cas"));
        }
        if let Some(after) = after {
            let api = read_source(connection, workspace, &after.api_source_id)?
                .ok_or_else(|| conflict("compute.api_link.target_missing"))?;
            if !api.provenance.is_native() {
                return Err(conflict("compute.api_link.target_kind"));
            }
        }
    }
    Ok(())
}
pub(super) fn stage(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    operation: &str,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    validate_before(connection, workspace, mutation)?;
    let before = read(connection, workspace, mutation.source_id())?;
    let after = if mutation.desired().is_none() {
        None
    } else if let Some((_, after)) = mutation.api_link_change() {
        after.clone()
    } else {
        before.clone()
    };
    connection.execute("UPDATE compute_management_effects SET api_link_before=?2,api_link_after=?3 WHERE operation_id=?1", params![operation, encode(&before)?, encode(&after)?])
        .map_err(|_| port("compute.api_link.stage"))?;
    Ok(())
}
fn effect(
    connection: &rusqlite::Connection,
    operation: &str,
    before: bool,
) -> PortResult<Option<Option<ComputeSubscriptionApiLinkV1>>> {
    let column = if before {
        "api_link_before"
    } else {
        "api_link_after"
    };
    let value: Option<String> = connection
        .query_row(
            &format!("SELECT {column} FROM compute_management_effects WHERE operation_id=?1"),
            params![operation],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.api_link.effect"))?
        .flatten();
    value.map(|value| decode(&value)).transpose()
}
fn write(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source_id: &str,
    link: Option<&ComputeSubscriptionApiLinkV1>,
) -> PortResult<()> {
    if let Some(link) = link {
        link.validate()
            .map_err(|_| corrupt("compute.api_link.write_identity"))?;
        let source = read_source(connection, workspace, source_id)?
            .ok_or_else(|| corrupt("compute.api_link.write_source"))?;
        if link.source_id != source_id || !is_codex(&source) {
            return Err(corrupt("compute.api_link.write_kind"));
        }
        connection.execute("INSERT INTO compute_subscription_api_links(workspace_id,source_id,api_source_id) VALUES (?1,?2,?3) ON CONFLICT(workspace_id,source_id) DO UPDATE SET api_source_id=excluded.api_source_id", params![workspace.as_str(), source_id, link.api_source_id])
            .map_err(|_| port("compute.api_link.write"))?;
    } else {
        connection
            .execute(
                "DELETE FROM compute_subscription_api_links WHERE workspace_id=?1 AND source_id=?2",
                params![workspace.as_str(), source_id],
            )
            .map_err(|_| port("compute.api_link.delete"))?;
    }
    Ok(())
}
pub(super) fn apply_effect(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source: &str,
    operation: &str,
    before: bool,
) -> PortResult<()> {
    if let Some(link) = effect(connection, operation, before)? {
        write(connection, workspace, source, link.as_ref())?;
    }
    Ok(())
}
pub(super) fn current_matches(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source: &str,
    operation: &str,
) -> PortResult<bool> {
    match effect(connection, operation, false)? {
        Some(after) => Ok(read(connection, workspace, source)? == after),
        None => Ok(true),
    }
}
