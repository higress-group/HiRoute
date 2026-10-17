use crate::session::*;
use hiroute_application_api::*;
use hiroute_domain::TokenRateV1;
use serde_json::Value;
#[tokio::test]
async fn native_price_confirmation_uses_real_cli_daemon_and_restart() {
    let root = tempfile::Builder::new()
        .prefix("hr10-")
        .tempdir_in(std::fs::canonicalize("/tmp").unwrap())
        .unwrap();
    let home = tempfile::tempdir().unwrap();
    // The registered external settings target requires an existing owned parent.
    std::fs::create_dir(home.path().join(".claude")).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/prepare_plan.py");
    let preparation = std::process::Command::new("python3")
        .arg(script)
        .arg(root.path())
        .env("HOME", std::fs::canonicalize(home.path()).unwrap())
        .output()
        .unwrap();
    assert!(
        preparation.status.success(),
        "CLI preparation: {}",
        String::from_utf8_lossy(&preparation.stderr)
    );
    let projection: Value = serde_json::from_slice(
        &std::fs::read(root.path().join("compute-projection.json")).unwrap(),
    )
    .unwrap();
    let binding = &projection["binding"];
    let binary = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("hirouted");
    let open = || {
        crate::session_from_resident(
            crate::bootstrap::Resident::open(root.path(), &binary).unwrap(),
        )
    };
    let input = |revision| PriceEditInput {
        target_locator: PriceTargetLocatorV1::Binding {
            binding_id: binding["binding_id"].as_str().unwrap().into(),
        },
        currency: "USD".into(),
        valuation_kind: hiroute_domain::PriceValuationKindV1::UsageEstimate,
        expected_source_revision: binding["source_revision"].as_u64().unwrap(),
        expected_binding_revision: Some(binding["revision"].as_u64().unwrap()),
        expected_override_revision: revision,
        language: "en".into(),
        action: DecimalPriceAction::Set {
            input_uncached: "1.200001".into(),
            output: "4.8".into(),
            cache_read: None,
            cache_write: None,
        },
    };
    let mut session = open();
    let context = session.preview_price(input(0)).await.unwrap();
    assert!(context.message().contains("1.200001"));
    let cancelled = session
        .finish_price_confirmation(context, false)
        .await
        .unwrap();
    assert_eq!(cancelled.state, "cancelled_before_apply");
    assert!(session.snapshot().await.unwrap().pending.is_none());
    let context = session.preview_price(input(0)).await.unwrap();
    let outcome = session
        .finish_price_confirmation(context, true)
        .await
        .unwrap();
    assert_eq!(outcome.operation.unwrap().state, "succeeded");
    assert!(session.preview_price(input(0)).await.is_err());
    drop(session);
    let mut session = open();
    let prices: EffectivePricesResultV2 = query(
        &session.client,
        "GetEffectivePrices",
        &GetEffectivePricesV2 {
            targets: vec![EffectivePriceTargetQueryV2 {
                query_id: "selected".into(),
                target_locator: input(1).target_locator,
                currency: "USD".into(),
                valuation_kind: hiroute_domain::PriceValuationKindV1::UsageEstimate,
            }],
        },
    )
    .await
    .unwrap();
    assert_eq!(
        prices.items[0].quote.origin,
        hiroute_domain::PriceOriginV1::Manual
    );
    assert_eq!(
        prices.items[0].quote.rates.input_uncached,
        TokenRateV1::known(1_200_001)
    );
    let mut restore = input(1);
    restore.action = DecimalPriceAction::FollowCatalog;
    let context = session.preview_price(restore).await.unwrap();
    assert_eq!(
        session
            .finish_price_confirmation(context, true)
            .await
            .unwrap()
            .operation
            .unwrap()
            .state,
        "succeeded"
    );
}
