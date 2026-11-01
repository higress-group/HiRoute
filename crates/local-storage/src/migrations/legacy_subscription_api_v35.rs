//! Admit the retained pre-integration association layout without confusing it
//! with main's canonical source-network schema 35. Conversion stays inside the
//! normal backed-up three-store migration, preserving source and journal bytes.
use crate::LocalStorageError;
use rusqlite::{Connection, OptionalExtension};

pub(super) fn recognized_layout(db: &Connection) -> Result<bool, LocalStorageError> {
    let version: u32 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version != 35 {
        return Ok(false);
    }
    let sql: Option<String> = db.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='compute_subscription_api_links'",
        [], |row| row.get(0),
    ).optional()?;
    let network: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='native_source_network')",
        [],
        |row| row.get(0),
    )?;
    let Some(sql) = sql.filter(|_| !network) else {
        return Ok(false);
    };
    let expected = super::subscription_api_v36::CONTROL
        .split(';')
        .next()
        .unwrap();
    let normalize = |value: &str| value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalize(&sql) != normalize(expected) {
        return Err(LocalStorageError::InvalidData);
    }
    let columns: (u32, u32) = db.query_row(
        "SELECT sum(name IN ('api_link_before','api_link_after') AND type='TEXT' AND \"notnull\"=0), sum(name IN ('network_before','network_after')) FROM pragma_table_info('compute_management_effects')",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if columns != (2, 0) {
        return Err(LocalStorageError::InvalidData);
    }
    crate::control::agent_locator::validate_coverage(db)
        .map_err(|_| LocalStorageError::InvalidData)?;
    super::subscription_api_v36::validate_links(db)?;
    let unsupported: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM compute_subscription_api_links l JOIN compute_management_sources s USING(workspace_id,source_id) WHERE json_extract(s.source_json,'$.provenance.connector_id') IS NOT 'connector.cpa.codex')",
        [], |row| row.get(0),
    )?;
    if unsupported {
        return Err(LocalStorageError::InvalidData);
    }
    Ok(true)
}
