//! The current source owner authenticates limits even when a link row is absent.
use super::*;

/// Preserve the last budget authority with one keyed reference, rather than
/// walking every past decision save during request admission.
pub(crate) fn owner(db: &Connection, workspace: &WorkspaceId, source: &str) -> PortResult<String> {
    let version: u32 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| failure())?;
    if version < 39 {
        return db.query_row("SELECT owner_operation_id FROM compute_management_sources WHERE workspace_id=?1 AND source_id=?2", params![workspace.as_str(),source], |r| r.get(0)).map_err(|_| failure());
    }
    type Owner = (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let (source_owner, decision, budget_owner, captured_digest, current_digest): Owner = db.query_row(
        "SELECT s.owner_operation_id,d.operation_id,d.key_budget_owner_operation_id,d.key_budget_witness_digest,m.key_budget_witness_digest FROM compute_management_sources s LEFT JOIN decision_source_effects d ON d.operation_id=s.owner_operation_id AND d.workspace_id=s.workspace_id AND d.source_id=s.source_id LEFT JOIN compute_management_effects m ON m.operation_id=d.key_budget_owner_operation_id AND m.workspace_id=s.workspace_id AND m.source_id=s.source_id WHERE s.workspace_id=?1 AND s.source_id=?2",
        params![workspace.as_str(),source], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_| failure())?;
    if decision.is_some() {
        if captured_digest != current_digest {
            return Err(failure());
        }
        budget_owner.ok_or_else(failure)
    } else {
        Ok(source_owner)
    }
}

pub(crate) fn bind_decision(
    db: &Connection,
    workspace: &WorkspaceId,
    operation: &str,
    source: &str,
) -> PortResult<()> {
    let version: u32 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|_| failure())?;
    if version < 39 {
        return Ok(());
    }
    validate(db, workspace, source)?;
    let budget_owner = owner(db, workspace, source)?;
    let digest: Option<String> = db.query_row("SELECT key_budget_witness_digest FROM compute_management_effects WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3", params![budget_owner,workspace.as_str(),source], |r| r.get(0)).optional().map_err(|_| failure())?.flatten();
    let changed = db.execute("UPDATE decision_source_effects SET key_budget_owner_operation_id=?4,key_budget_witness_digest=?5 WHERE operation_id=?1 AND workspace_id=?2 AND source_id=?3 AND before_owner_operation_id=(SELECT owner_operation_id FROM compute_management_sources WHERE workspace_id=?2 AND source_id=?3)", params![operation,workspace.as_str(),source,budget_owner,digest]).map_err(|_| failure())?;
    if changed != 1 {
        return Err(failure());
    }
    Ok(())
}

pub(crate) fn validate(db: &Connection, workspace: &WorkspaceId, source: &str) -> PortResult<()> {
    let authority_owner = owner(db, workspace, source)?;
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<bool>,
        Option<bool>,
        String,
        Option<String>,
        Option<bool>,
        String,
    );
    let row:Option<Row>=db.query_row("SELECT m.key_budget_witness_digest,m.key_budget_accounting_digest,b.witness_json,b.applied,b.compensated,s.source_json,m.key_budget_compensation_root,e.compensated,s.owner_operation_id FROM compute_management_sources s LEFT JOIN compute_management_effects m ON m.operation_id=?3 AND m.workspace_id=s.workspace_id AND m.source_id=s.source_id LEFT JOIN compute_key_budget_effects b ON b.operation_id=?3 AND b.workspace_id=s.workspace_id AND b.source_id=s.source_id LEFT JOIN control_effects e ON e.operation_id=?3 AND e.workspace_id=s.workspace_id WHERE s.workspace_id=?1 AND s.source_id=?2",params![workspace.as_str(),source,authority_owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional().map_err(|_|failure())?;
    let Some((
        marker,
        accounting_marker,
        json,
        applied,
        compensated,
        source_json,
        compensation_root,
        control_compensated,
        source_owner,
    )) = row
    else {
        return Err(failure());
    };
    let actual = bindings(db, workspace, source)?;
    let Some(marker) = marker else {
        return if json.is_none() && actual.is_empty() {
            Ok(())
        } else {
            Err(failure())
        };
    };
    let witness: hiroute_domain::KeyBudgetMutationV1 = decode(&json.ok_or_else(failure)?)?;
    if CanonicalDigest::of(&witness)
        .map_err(|_| failure())?
        .as_str()
        != marker
        || (compensated != Some(true) && (applied != Some(true) || accounting_marker.is_none()))
    {
        return Err(failure());
    }
    if compensated == Some(true) {
        let source: serde_json::Value =
            serde_json::from_str(&source_json).map_err(|_| failure())?;
        if control_compensated != Some(true)
            || (source_owner == authority_owner
                && CanonicalDigest::of(&source)
                    .map_err(|_| failure())?
                    .as_str()
                    != compensation_root.as_deref().ok_or_else(failure)?)
        {
            return Err(failure());
        }
    }
    let expected = if compensated == Some(true) {
        &witness.bindings_before
    } else {
        &witness.bindings_after
    };
    if &actual != expected {
        return Err(failure());
    }
    Ok(())
}
