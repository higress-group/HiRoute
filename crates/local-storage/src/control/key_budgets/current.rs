//! The current source owner authenticates limits even when a link row is absent.
use super::*;
pub(crate) fn validate(db: &Connection, workspace: &WorkspaceId, source: &str) -> PortResult<()> {
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<bool>,
        Option<bool>,
        String,
        Option<String>,
        Option<bool>,
    );
    let row:Option<Row>=db.query_row("SELECT m.key_budget_witness_digest,m.key_budget_accounting_digest,b.witness_json,b.applied,b.compensated,s.source_json,m.key_budget_compensation_root,e.compensated FROM compute_management_sources s LEFT JOIN compute_management_effects m ON m.operation_id=s.owner_operation_id AND m.workspace_id=s.workspace_id AND m.source_id=s.source_id LEFT JOIN compute_key_budget_effects b ON b.operation_id=s.owner_operation_id AND b.workspace_id=s.workspace_id AND b.source_id=s.source_id LEFT JOIN control_effects e ON e.operation_id=s.owner_operation_id AND e.workspace_id=s.workspace_id WHERE s.workspace_id=?1 AND s.source_id=?2",params![workspace.as_str(),source],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|_|failure())?;
    let Some((
        marker,
        accounting_marker,
        json,
        applied,
        compensated,
        source_json,
        compensation_root,
        control_compensated,
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
            || CanonicalDigest::of(&source)
                .map_err(|_| failure())?
                .as_str()
                != compensation_root.as_deref().ok_or_else(failure)?
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
