//! Defaults share the source CAS, Control transaction and compensation witness.
use super::*;
use hiroute_domain::{
    ComputeManagementMutation, SourceDefaultPreferencesV1, validate_source_default_preferences,
};

pub(super) fn read(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source_id: &str,
) -> PortResult<SourceDefaultPreferencesV1> {
    let json: Option<String> = connection.query_row(
        "SELECT preferences_json FROM compute_source_defaults WHERE workspace_id=?1 AND source_id=?2",
        params![workspace.as_str(), source_id], |row| row.get(0),
    ).optional().map_err(|_| port("compute.defaults.read"))?;
    let preferences = json.map(|v| decode(&v)).transpose()?.unwrap_or_default();
    validate_source_default_preferences(
        read_source(connection, workspace, source_id)?.as_ref(),
        &preferences,
    )
    .map_err(|_| corrupt("compute.defaults.closure"))?;
    Ok(preferences)
}
pub(super) fn validate_before(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    if let Some((before, _)) = mutation.preferences()
        && &read(connection, workspace, mutation.source_id())? != before
    {
        return Err(conflict("compute.defaults.cas"));
    }
    Ok(())
}
pub(super) fn write(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    source_id: &str,
    preferences: &SourceDefaultPreferencesV1,
) -> PortResult<()> {
    let source = read_source(connection, workspace, source_id)?;
    validate_source_default_preferences(source.as_ref(), preferences)
        .map_err(|_| invalid("compute.defaults.closure"))?;
    if preferences.is_empty() {
        connection
            .execute(
                "DELETE FROM compute_source_defaults WHERE workspace_id=?1 AND source_id=?2",
                params![workspace.as_str(), source_id],
            )
            .map_err(|_| port("compute.defaults.delete"))?;
    } else {
        connection.execute("INSERT INTO compute_source_defaults(workspace_id,source_id,preferences_json) VALUES (?1,?2,?3) ON CONFLICT(workspace_id,source_id) DO UPDATE SET preferences_json=excluded.preferences_json",
            params![workspace.as_str(), source_id, encode(preferences)?]).map_err(|_| port("compute.defaults.write"))?;
    }
    Ok(())
}
pub(super) fn after(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    mutation: &ComputeManagementMutation,
) -> PortResult<SourceDefaultPreferencesV1> {
    if let Some((_, after)) = mutation.preferences() {
        return Ok(after.clone());
    }
    let mut values = read(connection, workspace, mutation.source_id())?;
    values.retain(|binding, _| {
        mutation
            .desired()
            .is_some_and(|s| s.models.iter().any(|m| &m.binding_id == binding))
    });
    // V2 did not express a reset. Explicitly changed declarations cannot inherit a V3 default.
    for model in mutation.desired().into_iter().flat_map(|s| &s.models) {
        let old = mutation
            .expected()
            .and_then(|s| s.models.iter().find(|m| m.binding_id == model.binding_id));
        if let Some(fields) = values.get_mut(&model.binding_id) {
            use hiroute_domain::{ModelDefaultFieldV1::*, ModelDefaultPreferenceV1::PreserveSaved};
            macro_rules! changed {
                ($field:ident, $key:ident) => {
                    if old.is_none_or(|old| old.capabilities.$field != model.capabilities.$field) {
                        fields.insert($key, PreserveSaved);
                    }
                };
            }
            changed!(tool, Tool);
            changed!(vision, Vision);
            changed!(streaming, Streaming);
            changed!(context_tokens, ContextTokens);
            changed!(max_output_tokens, MaxOutputTokens);
            changed!(native_reasoning, NativeReasoning);
        }
    }
    Ok(values)
}

pub(super) fn stage(
    connection: &rusqlite::Connection,
    workspace: &WorkspaceId,
    operation: &str,
    mutation: &ComputeManagementMutation,
) -> PortResult<()> {
    let before = read(connection, workspace, mutation.source_id())?;
    let after = after(connection, workspace, mutation)?;
    connection.execute("UPDATE compute_management_effects SET preferences_before=?2,preferences_after=?3 WHERE operation_id=?1", params![operation, encode(&before)?, encode(&after)?])
        .map_err(|_| port("compute.defaults.stage"))?;
    Ok(())
}
pub(super) fn effect(
    connection: &rusqlite::Connection,
    operation: &str,
    before: bool,
) -> PortResult<Option<SourceDefaultPreferencesV1>> {
    let column = if before {
        "preferences_before"
    } else {
        "preferences_after"
    };
    let value: Option<String> = connection
        .query_row(
            &format!("SELECT {column} FROM compute_management_effects WHERE operation_id=?1"),
            params![operation],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| port("compute.defaults.effect"))?
        .flatten();
    value.map(|value| decode(&value)).transpose()
}
