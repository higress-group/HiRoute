//! Restore owned policy effects against the original calendar, retaining actual charges.
use super::*;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyUndo {
    wallet: Wallet,
    charge_sequence: u64,
}
impl PolicyUndo {
    pub(crate) fn matches(&self, definition: &KeyBudgetDefinitionV1) -> bool {
        &self.wallet.definition == definition
    }
}
pub(crate) fn capture_policy_undo(
    db: &Connection,
    workspace: &WorkspaceId,
    id: &str,
    at: i64,
) -> PortResult<Option<PolicyUndo>> {
    let Some(mut wallet) = load(db, workspace, id)? else {
        return Ok(None);
    };
    if renew(&mut wallet, at)? {
        save(db, workspace, &wallet)?;
    }
    let charge_sequence=db.query_row("SELECT COALESCE(MAX(sequence),0) FROM compute_key_spend_charges WHERE workspace_id=?1 AND budget_id=?2",params![workspace.as_str(),id],|r|r.get(0)).map_err(|_|failure())?;
    Ok(Some(PolicyUndo {
        wallet,
        charge_sequence,
    }))
}
pub(crate) fn restore_policy_undo(
    db: &Connection,
    workspace: &WorkspaceId,
    definition: &KeyBudgetDefinitionV1,
    undo: &PolicyUndo,
    at: i64,
) -> PortResult<()> {
    let id = &definition.budget_id;
    let current = load(db, workspace, id)?.ok_or_else(failure)?;
    if undo.wallet.definition.budget_id != *id || undo.wallet.definition.policy != definition.policy
    {
        return Err(failure());
    }
    let mut restored = undo.wallet.clone();
    let baseline_epoch = restored.epoch;
    let baseline_index = restored.index;
    let boundary = |wallet: &Wallet| -> PortResult<Option<i64>> {
        let Some(rule) = &wallet.definition.policy.renewal else {
            return Ok(None);
        };
        if rule.mode == KeyBudgetRenewalModeV1::Reset && wallet.index > baseline_index {
            Ok(Some(
                rule.event_at(wallet.index - 1).map_err(|_| failure())?,
            ))
        } else {
            Ok(None)
        }
    };
    // Only charges since this operation's accounting checkpoint are relevant.
    // Offline calendar gaps are advanced in O(1), without scanning prior usage.
    let mut query=db.prepare("SELECT created_at_ms,settled_at_ms,budget_epoch,charged_micros FROM compute_key_spend_charges WHERE workspace_id=?1 AND budget_id=?2 AND sequence>?3 ORDER BY sequence").map_err(|_|failure())?;
    let rows = query
        .query_map(params![workspace.as_str(), id, undo.charge_sequence], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, u64>(3)?,
            ))
        })
        .map_err(|_| failure())?;
    for row in rows {
        let (created, settled, epoch, charge) = row.map_err(|_| failure())?;
        renew(&mut restored, settled)?;
        if epoch >= baseline_epoch && boundary(&restored)?.is_none_or(|last| created >= last) {
            restored.balance = restored
                .balance
                .checked_sub(amount(charge)?)
                .ok_or_else(failure)?;
            restored.spent = restored.spent.checked_add(charge).ok_or_else(failure)?;
        }
    }
    renew(&mut restored, at)?;
    let last = boundary(&restored)?.unwrap_or(i64::MIN);
    // A fresh epoch cannot collide with any provisional reset. Rebind only holds
    // that would still belong to the restored calendar; other records stay late.
    restored.epoch = current
        .epoch
        .max(restored.epoch)
        .checked_add(1)
        .ok_or_else(failure)?;
    db.execute("UPDATE compute_key_spend_reservations SET budget_epoch=?4 WHERE workspace_id=?1 AND budget_id=?2 AND budget_epoch>=?3 AND created_at_ms>=?5 AND state IN ('reserved','sent','pending')",params![workspace.as_str(),id,baseline_epoch,restored.epoch,last]).map_err(|_|failure())?;
    restored.reserved=db.query_row("SELECT COALESCE(SUM(amount_micros),0) FROM compute_key_spend_reservations WHERE workspace_id=?1 AND budget_id=?2 AND budget_epoch=?3 AND state IN ('reserved','sent','pending')",params![workspace.as_str(),id,restored.epoch],|r|r.get(0)).map_err(|_|failure())?;
    restored.lifetime = current.lifetime;
    restored.definition = definition.clone();
    save(db, workspace, &restored)
}
