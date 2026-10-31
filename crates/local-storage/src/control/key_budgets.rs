//! Indexed spending authority. Request admission never scans observation/history tables.
mod current;
mod policy_undo;
use super::*;
use hiroute_domain::{
    KeyBudgetDefinitionV1, KeyBudgetGateErrorV1, KeyBudgetRenewalModeV1, KeyBudgetViewV1,
    KeySpendEstimateV1, KeySpendReservationV1, KeySpendSettlementV1,
};
pub(crate) use policy_undo::{PolicyUndo, capture_policy_undo, restore_policy_undo};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Wallet {
    definition: KeyBudgetDefinitionV1,
    balance: i64,
    reserved: u64,
    spent: u64,
    lifetime: u64,
    index: i64,
    epoch: u64,
}
fn failure() -> PortError {
    port(PortErrorCode::Unavailable, "compute.key_budget.authority")
}

fn encode<T: serde::Serialize>(value: &T) -> PortResult<String> {
    serde_json::to_string(value).map_err(|_| failure())
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> PortResult<T> {
    serde_json::from_str(value).map_err(|_| failure())
}
fn invalid(context: &'static str) -> PortError {
    port(PortErrorCode::InvalidData, context)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(i64::MAX as u128) as i64)
}
fn amount(value: u64) -> PortResult<i64> {
    i64::try_from(value).map_err(|_| failure())
}

pub(crate) fn bindings(
    db: &Connection,
    workspace: &WorkspaceId,
    source: &str,
) -> PortResult<BTreeMap<String, String>> {
    let mut query=db.prepare("SELECT key_id,budget_id FROM compute_key_budget_bindings WHERE workspace_id=?1 AND source_id=?2 ORDER BY key_id").map_err(|_|failure())?;
    query
        .query_map(params![workspace.as_str(), source], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(|_| failure())?
        .collect::<Result<_, _>>()
        .map_err(|_| failure())
}
pub(crate) fn definition(
    db: &Connection,
    workspace: &WorkspaceId,
    id: &str,
) -> PortResult<Option<KeyBudgetDefinitionV1>> {
    let value:Option<String>=db.query_row("SELECT definition_json FROM compute_key_budgets WHERE workspace_id=?1 AND budget_id=?2",params![workspace.as_str(),id],|r|r.get(0)).optional().map_err(|_|failure())?;
    value.map(|v| decode(&v)).transpose()
}
fn load(db: &Connection, workspace: &WorkspaceId, id: &str) -> PortResult<Option<Wallet>> {
    type Raw = (String, i64, u64, u64, u64, i64, u64);
    let row:Option<Raw>=db.query_row("SELECT definition_json,balance_micros,reserved_micros,spent_micros,lifetime_spent_micros,renewal_index,budget_epoch FROM compute_key_budgets WHERE workspace_id=?1 AND budget_id=?2",params![workspace.as_str(),id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|_|failure())?;
    row.map(|(json, balance, reserved, spent, lifetime, index, epoch)| {
        Ok(Wallet {
            definition: decode(&json)?,
            balance,
            reserved,
            spent,
            lifetime,
            index,
            epoch,
        })
    })
    .transpose()
}
fn save(db: &Connection, workspace: &WorkspaceId, wallet: &Wallet) -> PortResult<()> {
    if db.execute("UPDATE compute_key_budgets SET revision=?3,definition_json=?4,balance_micros=?5,reserved_micros=?6,spent_micros=?7,lifetime_spent_micros=?8,renewal_index=?9,budget_epoch=?10 WHERE workspace_id=?1 AND budget_id=?2",params![workspace.as_str(),wallet.definition.budget_id,wallet.definition.revision,encode(&wallet.definition)?,wallet.balance,amount(wallet.reserved)?,amount(wallet.spent)?,amount(wallet.lifetime)?,wallet.index,wallet.epoch]).map_err(|_|failure())?!=1 {return Err(failure());}
    Ok(())
}
fn renew(wallet: &mut Wallet, at: i64) -> PortResult<bool> {
    let Some(renewal) = &wallet.definition.policy.renewal else {
        return Ok(false);
    };
    let count = renewal.elapsed(at).map_err(|_| failure())?;
    if count <= wallet.index {
        return Ok(false);
    }
    let due = count - wallet.index;
    match renewal.mode {
        KeyBudgetRenewalModeV1::Reset => {
            wallet.balance = amount(wallet.definition.policy.amount_micros)?;
            // A reset expires the old allowance, not its durable records. Late
            // settlement remains in the original epoch and cannot debit twice.
            wallet.epoch = wallet.epoch.checked_add(due as u64).ok_or_else(failure)?;
            wallet.reserved = 0;
        }
        KeyBudgetRenewalModeV1::Accumulate => {
            let cap = renewal.balance_cap_micros.ok_or_else(failure)?;
            let value = i128::from(wallet.balance)
                + i128::from(due) * i128::from(wallet.definition.policy.amount_micros);
            wallet.balance = i64::try_from(value.min(i128::from(cap))).map_err(|_| failure())?;
        }
    }
    wallet.spent = 0;
    wallet.index = count;
    Ok(true)
}

pub(crate) fn apply_definition(
    db: &Connection,
    workspace: &WorkspaceId,
    value: &KeyBudgetDefinitionV1,
    initial: Option<u64>,
    at: i64,
) -> PortResult<()> {
    value.validate().map_err(|_| failure())?;
    if let Some(mut wallet) = load(db, workspace, &value.budget_id)? {
        if wallet.definition.policy.currency != value.policy.currency {
            return Err(invalid("compute.key_budget.currency_immutable"));
        }
        renew(&mut wallet, at)?;
        // A cap edit changes remaining allowance by the cap delta, never by resetting usage.
        let delta = i128::from(value.policy.amount_micros)
            - i128::from(wallet.definition.policy.amount_micros);
        wallet.balance =
            i64::try_from(i128::from(wallet.balance) + delta).map_err(|_| failure())?;
        if wallet.definition.policy.renewal != value.policy.renewal {
            wallet.index = value
                .policy
                .renewal
                .as_ref()
                .map(|r| r.elapsed(at).map_err(|_| failure()))
                .transpose()?
                .unwrap_or(0);
        }
        wallet.definition = value.clone();
        save(db, workspace, &wallet)
    } else {
        let available = initial.unwrap_or(value.policy.amount_micros);
        if available > value.policy.amount_micros {
            return Err(invalid("compute.key_budget.initial"));
        }
        let index = value
            .policy
            .renewal
            .as_ref()
            .map(|r| r.elapsed(at).map_err(|_| failure()))
            .transpose()?
            .unwrap_or(0);
        db.execute("INSERT INTO compute_key_budgets(workspace_id,budget_id,revision,definition_json,balance_micros,renewal_index) VALUES (?1,?2,?3,?4,?5,?6)",params![workspace.as_str(),value.budget_id,value.revision,encode(value)?,amount(available)?,index]).map_err(|_|failure())?;
        Ok(())
    }
}

impl ControlStore {
    pub fn validate_current_key_budgets(&self, source: &str) -> PortResult<()> {
        current::validate(&self.connection.borrow(), &WorkspaceId::default(), source)
    }

    pub fn key_spend_currency(&self, key_id: &str) -> PortResult<Option<String>> {
        let db = self.connection.borrow();
        let json:Option<String>=db.query_row("SELECT b.definition_json FROM compute_key_budget_bindings k JOIN compute_key_budgets b ON b.workspace_id=k.workspace_id AND b.budget_id=k.budget_id WHERE k.workspace_id=?1 AND k.key_id=?2",params![WorkspaceId::default().as_str(),key_id],|r|r.get(0)).optional().map_err(|_|failure())?;
        json.map(|v| decode::<KeyBudgetDefinitionV1>(&v).map(|d| d.policy.currency))
            .transpose()
    }

    pub fn key_budget_bindings(&self, source_id: &str) -> PortResult<BTreeMap<String, String>> {
        bindings(
            &self.connection.borrow(),
            &WorkspaceId::default(),
            source_id,
        )
    }
    pub fn key_budget_definition(
        &self,
        budget_id: &str,
    ) -> PortResult<Option<KeyBudgetDefinitionV1>> {
        definition(
            &self.connection.borrow(),
            &WorkspaceId::default(),
            budget_id,
        )
    }
    pub fn key_budget_views(&self, ids: &[String], at_ms: i64) -> PortResult<Vec<KeyBudgetViewV1>> {
        if ids.len() > 256 {
            return Err(invalid("compute.key_budget.read_limit"));
        }
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let workspace = WorkspaceId::default();
        let mut views = Vec::new();
        for id in ids {
            let Some(mut wallet) = load(&tx, &workspace, id)? else {
                continue;
            };
            if renew(&mut wallet, at_ms)? {
                save(&tx, &workspace, &wallet)?;
            }
            let key_count=tx.query_row("SELECT COUNT(*) FROM compute_key_budget_bindings WHERE workspace_id=?1 AND budget_id=?2",params![workspace.as_str(),id],|r|r.get(0)).map_err(|_|failure())?;
            views.push(KeyBudgetViewV1 {
                next_renewal_at_ms: wallet
                    .definition
                    .policy
                    .renewal
                    .as_ref()
                    .map(|r| r.event_at(wallet.index).map_err(|_| failure()))
                    .transpose()?,
                available_micros: wallet
                    .balance
                    .saturating_sub(amount(wallet.reserved)?)
                    .max(0) as u64,
                balance_micros: wallet.balance,
                reserved_micros: wallet.reserved,
                spent_micros: wallet.spent,
                lifetime_spent_micros: wallet.lifetime,
                definition: wallet.definition,
                key_count,
            });
        }
        tx.commit().map_err(|_| failure())?;
        Ok(views)
    }
    pub fn key_budget_available_ids(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> PortResult<Vec<String>> {
        let db = self.connection.borrow();
        let mut query=db.prepare("SELECT budget_id FROM compute_key_budgets WHERE workspace_id=?1 AND budget_id>?2 ORDER BY budget_id LIMIT ?3").map_err(|_|failure())?;
        query
            .query_map(
                params![
                    WorkspaceId::default().as_str(),
                    after.unwrap_or(""),
                    limit.min(100) as u64
                ],
                |r| r.get(0),
            )
            .map_err(|_| failure())?
            .collect::<Result<_, _>>()
            .map_err(|_| failure())
    }

    pub fn reserve_key_spend(
        &self,
        key_id: &str,
        generation: u64,
        estimate: Option<&KeySpendEstimateV1>,
    ) -> Result<Option<KeySpendReservationV1>, KeyBudgetGateErrorV1> {
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let workspace = WorkspaceId::default();
        let link:Option<(String,String)>=tx.query_row("SELECT source_id,budget_id FROM compute_key_budget_bindings WHERE workspace_id=?1 AND key_id=?2",params![workspace.as_str(),key_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|failure())?;
        let Some((source, id)) = link else {
            return Ok(None);
        };
        let mut wallet = load(&tx, &workspace, &id)?.ok_or_else(failure)?;
        let at = now_ms();
        renew(&mut wallet, at)?;
        let estimate = estimate.ok_or(KeyBudgetGateErrorV1::PriceUnavailable)?;
        let quote = estimate
            .pricing
            .quote
            .as_ref()
            .ok_or(KeyBudgetGateErrorV1::PriceUnavailable)?;
        if quote.actual_source_ref != source || quote.exact_target.source_id != source {
            return Err(KeyBudgetGateErrorV1::PriceUnavailable);
        }
        if quote.currency != wallet.definition.policy.currency {
            return Err(KeyBudgetGateErrorV1::CurrencyMismatch);
        }
        let cost = estimate
            .upper_bound_micros()
            .ok_or(KeyBudgetGateErrorV1::PriceUnavailable)?;
        let available = wallet
            .balance
            .saturating_sub(amount(wallet.reserved)?)
            .max(0) as u64;
        if cost > available {
            return Err(KeyBudgetGateErrorV1::Exhausted);
        }
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| failure())?;
        let reservation_id = format!(
            "spend/{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
        wallet.reserved = wallet.reserved.checked_add(cost).ok_or_else(failure)?;
        save(&tx, &workspace, &wallet)?;
        tx.execute("INSERT INTO compute_key_spend_reservations(workspace_id,reservation_id,budget_id,key_id,key_generation,budget_epoch,amount_micros,pricing_json,state,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'reserved',?9)",params![workspace.as_str(),reservation_id,id,key_id,generation,wallet.epoch,amount(cost)?,encode(estimate)?,at]).map_err(|_|failure())?;
        tx.commit().map_err(|_| failure())?;
        Ok(Some(KeySpendReservationV1 {
            reservation_id,
            budget_id: id,
            amount_micros: cost,
            budget_epoch: wallet.epoch,
        }))
    }

    pub fn mark_key_spend_sent(&self, id: &str) -> PortResult<()> {
        self.connection.borrow().execute("UPDATE compute_key_spend_reservations SET state='sent' WHERE workspace_id=?1 AND reservation_id=?2 AND state='reserved'",params![WorkspaceId::default().as_str(),id]).map_err(|_|failure())?;
        Ok(())
    }
    pub fn settle_key_spend(&self, id: &str, settlement: &KeySpendSettlementV1) -> PortResult<()> {
        let mut connection = self.connection.borrow_mut();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| failure())?;
        let workspace = WorkspaceId::default();
        let row:Option<(String,u64,u64,String,String)>=tx.query_row("SELECT budget_id,budget_epoch,amount_micros,pricing_json,state FROM compute_key_spend_reservations WHERE workspace_id=?1 AND reservation_id=?2",params![workspace.as_str(),id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|failure())?;
        let Some((budget, epoch, reserved, json, state)) = row else {
            return Err(invalid("compute.key_budget.reservation"));
        };
        if matches!(state.as_str(), "settled" | "released") {
            return Ok(());
        }
        let estimate: KeySpendEstimateV1 = decode(&json)?;
        let charge = match settlement {
            KeySpendSettlementV1::Unsent => Some(0),
            KeySpendSettlementV1::Usage { usage } => estimate.usage_micros(usage),
            KeySpendSettlementV1::Unknown => None,
        };
        let Some(charge) = charge else {
            tx.execute("UPDATE compute_key_spend_reservations SET state='pending' WHERE workspace_id=?1 AND reservation_id=?2",params![workspace.as_str(),id]).map_err(|_|failure())?;
            tx.commit().map_err(|_| failure())?;
            return Ok(());
        };
        let mut wallet = load(&tx, &workspace, &budget)?.ok_or_else(failure)?;
        let at = now_ms();
        renew(&mut wallet, at)?;
        if epoch == wallet.epoch {
            wallet.reserved = wallet.reserved.checked_sub(reserved).ok_or_else(failure)?;
            wallet.balance = wallet
                .balance
                .checked_sub(amount(charge)?)
                .ok_or_else(failure)?;
            wallet.spent = wallet.spent.checked_add(charge).ok_or_else(failure)?;
        }
        wallet.lifetime = wallet.lifetime.checked_add(charge).ok_or_else(failure)?;
        save(&tx, &workspace, &wallet)?;
        tx.execute("INSERT INTO compute_key_spend_charges(workspace_id,budget_id,reservation_id,budget_epoch,created_at_ms,settled_at_ms,charged_micros) SELECT workspace_id,budget_id,reservation_id,budget_epoch,created_at_ms,?3,?4 FROM compute_key_spend_reservations WHERE workspace_id=?1 AND reservation_id=?2",params![workspace.as_str(),id,at,amount(charge)?]).map_err(|_|failure())?;
        tx.execute("UPDATE compute_key_spend_reservations SET state=?3,settled_at_ms=?4,charged_micros=?5 WHERE workspace_id=?1 AND reservation_id=?2",params![workspace.as_str(),id,if matches!(settlement,KeySpendSettlementV1::Unsent){"released"}else{"settled"},at,amount(charge)?]).map_err(|_|failure())?;
        tx.commit().map_err(|_| failure())?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "key_budget_calendar_tests.rs"]
mod calendar_tests;
