use super::*;

#[test]
fn selector_stops_before_later_secret_and_ignores_nested_model() {
    let mut selector = ModelSelector::new(64);
    let body = br#"{"metadata":{"model":"wrong"},"model":"plan-fast","secret":"later"}"#;
    assert_eq!(selector.feed(body).unwrap(), Some("plan-fast".into()));
}

#[test]
fn selector_is_incremental_and_bounded() {
    let mut selector = ModelSelector::new(32);
    assert_eq!(selector.feed(br#"{"model":"plan-"#).unwrap(), None);
    assert_eq!(
        selector.feed(br#"fast","input":"#).unwrap(),
        Some("plan-fast".into())
    );
    assert_eq!(
        ModelSelector::new(8).feed(br#"{"input":"too late"}"#),
        Err(SelectorError::LimitExceeded(8))
    );
}

#[test]
fn selector_waits_when_a_model_value_has_not_arrived() {
    let mut selector = ModelSelector::new(64);
    for chunk in [
        br#"{"m"#.as_slice(),
        b"ode",
        br#"l":"#.as_slice(),
        br#""wi"#.as_slice(),
    ] {
        assert_eq!(selector.feed(chunk).unwrap(), None);
    }
    assert_eq!(
        selector.feed(br#"re-protocol"}"#).unwrap(),
        Some("wire-protocol".into())
    );

    assert_eq!(
        ModelSelector::new(32).feed(br#"{"model":   "#).unwrap(),
        None
    );
    assert_eq!(
        ModelSelector::new(32).feed(br#"{"model":1}"#),
        Err(SelectorError::Invalid)
    );
}

#[test]
fn every_split_preserves_escaped_keys_values_and_nested_delimiters() {
    let body = br#"{"metadata":{"model":"wrong","quoted":"[\"\\{}]"},"long unrelated key outside the recognition buffer":null,"\u006d\u006f\u0064\u0065\u006c":"plan-\u8def\u7531-\ud83d\ude80","later":"ignored"}"#;
    for split in 0..=body.len() {
        let mut selector = ModelSelector::new(usize::MAX);
        let selected = selector
            .feed(&body[..split])
            .unwrap()
            .or_else(|| selector.feed(&body[split..]).unwrap());
        assert_eq!(selected.as_deref(), Some("plan-路由-🚀"), "split={split}");
    }
    let mut selector = ModelSelector::new(usize::MAX);
    let selected = body
        .iter()
        .find_map(|byte| selector.feed(&[*byte]).unwrap());
    assert_eq!(selected.as_deref(), Some("plan-路由-🚀"));
}

#[test]
fn large_prefix_is_processed_once_without_retained_body_capacity() {
    let text = "long prefix { [ \\\" unicode 路由 ] } ".repeat(100_000);
    let body = format!(
        r#"{{"input":{},"model":"plan-fast","later":"ignored"}}"#,
        serde_json::to_string(&text).unwrap()
    );
    for chunk_bytes in [1, 17, 4096, 16 * 1024] {
        let mut selector = ModelSelector::new(usize::MAX);
        let selected = body
            .as_bytes()
            .chunks(chunk_bytes)
            .find_map(|chunk| selector.feed(chunk).unwrap());
        assert_eq!(selected.as_deref(), Some("plan-fast"));
        let prefix = body.find(",\"later\"").unwrap();
        assert_eq!(
            selector.observed, prefix,
            "each prefix byte is consumed once"
        );
        assert!(
            std::mem::size_of_val(&selector) + MAX_MODEL_TOKEN_BYTES * 2
                < ModelSelector::MEMORY_BYTES
        );
    }
}

#[test]
fn alias_validation_limits_and_incomplete_input_remain_bounded() {
    for body in [
        r#"{"model":""}"#,
        r#"{"model":false}"#,
        r#"{"model":"\uZZZZ"}"#,
    ] {
        assert!(
            ModelSelector::new(usize::MAX)
                .feed(body.as_bytes())
                .is_err()
        );
    }
    for body in [r#"{"input":"incomplete"#, r#"{"input":[{"x":1}]"#] {
        let mut selector = ModelSelector::new(usize::MAX);
        assert_eq!(selector.feed(body.as_bytes()), Ok(None));
        assert_eq!(selector.finish(), Err(SelectorError::Invalid));
    }
    assert_eq!(
        ModelSelector::new(100).feed(br#"{"input":1}"#),
        Err(SelectorError::Missing)
    );
    let model = r#"\u0061"#.repeat(MAX_ALIAS_BYTES);
    let body = format!(r#"{{"model":"{model}"}}"#);
    assert_eq!(
        ModelSelector::new(usize::MAX)
            .feed(body.as_bytes())
            .unwrap(),
        Some("a".repeat(MAX_ALIAS_BYTES))
    );
    let body = format!(r#"{{"model":"{}"}}"#, "a".repeat(100_000));
    assert_eq!(
        ModelSelector::new(usize::MAX).feed(body.as_bytes()),
        Err(SelectorError::InvalidModel)
    );
    let duplicate = br#"{"model":"first","model":"last"}"#;
    assert_eq!(
        ModelSelector::new(usize::MAX).feed(duplicate).unwrap(),
        Some("first".into())
    );
}
