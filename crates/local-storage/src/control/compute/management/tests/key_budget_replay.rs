//! Deterministic accounting event times, using real reservation and settlement rows.
use super::key_budgets::{configure, estimate, new_budget, view};
use super::lifecycle::saved;
use super::*;
use hiroute_domain::*;
fn ms(value: &str) -> i64 {
    value.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}
#[test]
fn key_budget_policy_replay_prices_only_original_period_members_and_orders_capped_grants() {
    use crate::control::key_budgets::{apply_definition, capture_policy_undo, restore_policy_undo};
    for reset in [true, false] {
        let root = tempdir().unwrap();
        populate_saved_source(root.path(), &WorkspaceId::default());
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let key = configure(&stores, new_budget(1000), "key-budget-charge-replay");
        let source = saved(&stores);
        let quote = estimate(&source.source_id);
        let pending = stores
            .control()
            .reserve_key_spend(&key, 1, Some(&quote))
            .unwrap()
            .unwrap();
        let mut small = quote.clone();
        small.input_bound = 1;
        small.output_bound = 1;
        let one = stores
            .control()
            .reserve_key_spend(&key, 1, Some(&small))
            .unwrap()
            .unwrap();
        let two = stores
            .control()
            .reserve_key_spend(&key, 1, Some(&small))
            .unwrap()
            .unwrap();
        let mut old = view(&stores, &key).definition;
        old.policy.amount_micros = if reset { 1000 } else { 100 };
        old.policy.renewal = Some(KeyBudgetRenewalV1 {
            period: if reset {
                KeyBudgetPeriodV1::Daily
            } else {
                KeyBudgetPeriodV1::Monthly
            },
            mode: if reset {
                KeyBudgetRenewalModeV1::Reset
            } else {
                KeyBudgetRenewalModeV1::Accumulate
            },
            anchor_local: "2027-01-31T00:00:00".into(),
            timezone: "UTC".into(),
            balance_cap_micros: (!reset).then_some(500),
        });
        let at = ms("2027-01-30T00:00:00Z");
        let undo=stores.control().with_connection(|db| {
            apply_definition(db,&WorkspaceId::default(),&old,None,at).unwrap();
            db.execute("UPDATE compute_key_budgets SET balance_micros=450,spent_micros=200,lifetime_spent_micros=200",[]).unwrap();
            capture_policy_undo(db,&WorkspaceId::default(),&old.budget_id,at).unwrap().unwrap()
        });
        let mut temporary = old.clone();
        temporary.revision += 1;
        temporary.policy.renewal = None;
        stores.control().with_connection(|db| {
            apply_definition(db, &WorkspaceId::default(), &temporary, None, at).unwrap()
        });
        let events = if reset {
            vec![
                (
                    &one.reservation_id,
                    ms("2027-01-30T00:00:00Z"),
                    ms("2027-02-02T01:00:00Z"),
                    100,
                ),
                (
                    &two.reservation_id,
                    ms("2027-02-02T02:00:00Z"),
                    ms("2027-02-02T03:00:00Z"),
                    50,
                ),
            ]
        } else {
            vec![
                (
                    &one.reservation_id,
                    ms("2027-02-01T00:00:00Z"),
                    ms("2027-02-01T01:00:00Z"),
                    200,
                ),
                (
                    &two.reservation_id,
                    ms("2027-03-01T00:00:00Z"),
                    ms("2027-03-01T01:00:00Z"),
                    100,
                ),
            ]
        };
        stores.control().with_connection(|db| {
            for (id,created,settled,charge) in &events {
                db.execute("UPDATE compute_key_spend_reservations SET created_at_ms=?2,settled_at_ms=?3,charged_micros=?4,state='settled' WHERE reservation_id=?1",rusqlite::params![id,created,settled,charge]).unwrap();
                db.execute("INSERT INTO compute_key_spend_charges(workspace_id,budget_id,reservation_id,budget_epoch,created_at_ms,settled_at_ms,charged_micros) VALUES (?1,?2,?3,0,?4,?5,?6)",rusqlite::params![WorkspaceId::default().as_str(),old.budget_id,id,created,settled,charge]).unwrap();
            }
            db.execute("UPDATE compute_key_budgets SET lifetime_spent_micros=lifetime_spent_micros+?1",[if reset {150} else {300}]).unwrap();
        });
        old.revision = temporary.revision + 1;
        let now = if reset {
            ms("2027-02-02T04:00:00Z")
        } else {
            ms("2027-03-01T02:00:00Z")
        };
        stores.control().with_connection(|db| {
            restore_policy_undo(db, &WorkspaceId::default(), &old, &undo, now).unwrap()
        });
        let restored = stores
            .control()
            .key_budget_views(&[old.budget_id.clone()], now)
            .unwrap()
            .remove(0);
        if reset {
            assert_eq!(
                (
                    restored.balance_micros,
                    restored.reserved_micros,
                    restored.spent_micros,
                    restored.lifetime_spent_micros
                ),
                (950, 0, 50, 350)
            );
        } else {
            assert_eq!(
                (
                    restored.balance_micros,
                    restored.reserved_micros,
                    restored.spent_micros,
                    restored.lifetime_spent_micros
                ),
                (300, pending.amount_micros, 100, 500)
            );
        }
    }
}
