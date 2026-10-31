use super::key_budget::*;

fn renewal(anchor: &str, timezone: &str, period: KeyBudgetPeriodV1) -> KeyBudgetRenewalV1 {
    KeyBudgetRenewalV1 {
        period,
        mode: KeyBudgetRenewalModeV1::Reset,
        anchor_local: anchor.into(),
        timezone: timezone.into(),
        balance_cap_micros: None,
    }
}
fn ms(value: &str) -> i64 {
    value.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

#[test]
fn budget_month_end_restores_original_day_after_clamping() {
    let value = renewal("2027-01-31T00:00:00", "UTC", KeyBudgetPeriodV1::Monthly);
    assert_eq!(value.event_at(1).unwrap(), ms("2027-02-28T00:00:00Z"));
    assert_eq!(value.event_at(2).unwrap(), ms("2027-03-31T00:00:00Z"));
    assert_eq!(value.elapsed(ms("2027-02-27T23:59:59Z")).unwrap(), 1);
    assert_eq!(value.elapsed(ms("2027-02-28T00:00:00Z")).unwrap(), 2);
    assert_eq!(value.elapsed(ms("2026-12-31T00:00:00Z")).unwrap(), 0);
}
#[test]
fn budget_calendar_days_follow_timezone_and_do_not_drift_after_dst() {
    let value = renewal(
        "2026-03-07T02:30:00",
        "America/New_York",
        KeyBudgetPeriodV1::Daily,
    );
    assert_eq!(value.event_at(1).unwrap(), ms("2026-03-08T07:30:00Z"));
    assert_eq!(value.event_at(2).unwrap(), ms("2026-03-09T06:30:00Z"));
    assert_eq!(value.elapsed(ms("2026-03-08T07:29:59Z")).unwrap(), 1);
    assert_eq!(value.elapsed(ms("2026-03-08T07:30:00Z")).unwrap(), 2);
}
#[test]
fn budget_yearly_leap_day_and_long_offline_periods_remain_bounded() {
    let value = renewal("2024-02-29T00:00:00", "UTC", KeyBudgetPeriodV1::Yearly);
    assert_eq!(value.event_at(1).unwrap(), ms("2025-02-28T00:00:00Z"));
    assert_eq!(value.event_at(4).unwrap(), ms("2028-02-29T00:00:00Z"));
    assert_eq!(value.elapsed(ms("2040-02-29T00:00:00Z")).unwrap(), 17);
    let weekly = renewal(
        "2026-11-02T00:00:00",
        "Asia/Shanghai",
        KeyBudgetPeriodV1::Weekly,
    );
    assert_eq!(weekly.elapsed(ms("2026-11-08T16:00:00Z")).unwrap(), 2);
}
#[test]
fn budget_policy_rejects_unknown_timezone_invalid_amounts_and_unbounded_accumulation() {
    let mut policy = KeyBudgetPolicyV1 {
        currency: "USD".into(),
        amount_micros: 100_000_000,
        renewal: None,
    };
    assert!(policy.validate().is_ok());
    policy.amount_micros = 0;
    assert!(policy.validate().is_err());
    policy.amount_micros = 100_000_000;
    let mut value = renewal(
        "2026-11-01T00:00:00",
        "Imaginary/Zone",
        KeyBudgetPeriodV1::Monthly,
    );
    policy.renewal = Some(value.clone());
    assert!(policy.validate().is_err());
    value.timezone = "UTC".into();
    value.mode = KeyBudgetRenewalModeV1::Accumulate;
    policy.renewal = Some(value.clone());
    assert!(policy.validate().is_err());
    value.balance_cap_micros = Some(200_000_000);
    policy.renewal = Some(value);
    assert!(policy.validate().is_ok());
}
