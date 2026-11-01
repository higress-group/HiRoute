use super::*;
use hiroute_domain::{KeyBudgetPeriodV1, KeyBudgetPolicyV1, KeyBudgetRenewalV1};
fn ms(value: &str) -> i64 {
    value.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}
fn wallet(mode: KeyBudgetRenewalModeV1) -> Wallet {
    Wallet {
        definition: KeyBudgetDefinitionV1 {
            budget_id: "budget/calendar".into(),
            revision: 1,
            display_name: "Credits".into(),
            policy: KeyBudgetPolicyV1 {
                currency: "USD".into(),
                amount_micros: 100,
                renewal: Some(KeyBudgetRenewalV1 {
                    period: KeyBudgetPeriodV1::Monthly,
                    mode,
                    anchor_local: "2027-01-31T00:00:00".into(),
                    timezone: "UTC".into(),
                    balance_cap_micros: (mode == KeyBudgetRenewalModeV1::Accumulate).then_some(250),
                }),
            },
        },
        balance: 30,
        reserved: 20,
        spent: 70,
        lifetime: 70,
        index: 0,
        epoch: 0,
    }
}
#[test]
fn key_budget_reset_expires_old_allowances_and_holds_without_replaying_offline_grants() {
    let mut w = wallet(KeyBudgetRenewalModeV1::Reset);
    assert!(renew(&mut w, ms("2027-03-31T00:00:00Z")).unwrap());
    assert_eq!(
        (w.balance, w.reserved, w.spent, w.lifetime, w.index, w.epoch),
        (100, 0, 0, 70, 3, 3)
    );
    assert!(!renew(&mut w, ms("2027-03-31T00:00:01Z")).unwrap());
    assert_eq!(
        w.definition
            .policy
            .renewal
            .as_ref()
            .unwrap()
            .event_at(w.index)
            .unwrap(),
        ms("2027-04-30T00:00:00Z")
    );
}
#[test]
fn key_budget_accumulation_preserves_pending_holds_and_stops_at_the_ceiling() {
    let mut w = wallet(KeyBudgetRenewalModeV1::Accumulate);
    renew(&mut w, ms("2030-03-31T00:00:00Z")).unwrap();
    assert_eq!(
        (w.balance, w.reserved, w.spent, w.lifetime, w.index, w.epoch),
        (250, 20, 0, 70, 39, 0)
    );
    assert!(!renew(&mut w, ms("2030-03-31T00:00:00Z")).unwrap());
}
