use serde_json::json;

use super::*;
use crate::TrustedReleaseCatalog;

fn catalog() -> TrustedReleaseCatalog {
    TrustedReleaseCatalog::load_release_facts(
        include_bytes!("../../../../assets/release-facts/current/bundle/manifest.json"),
        include_bytes!("../../../../assets/release-facts/current/bundle/connector-registry.json"),
        include_bytes!("../../../../assets/release-facts/current/bundle/model-data.json"),
    )
    .unwrap()
}

fn empty() -> FeedPayload {
    FeedPayload::from_snapshot(
        catalog().official_snapshot(),
        43,
        "2026-10-18T00:00:00Z".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        vec![],
    )
}
fn seal(payload: &mut FeedPayload) {
    payload.data.rating_snapshot.model_catalog_digest =
        hiroute_domain::CanonicalDigest::of(&payload.data.models).unwrap();
    payload.data.rating_snapshot.digest = payload.data.rating_snapshot.computed_digest().unwrap();
    payload.data_digest = hiroute_domain::CanonicalDigest::of(&payload.data).unwrap();
}

#[test]
fn model_feed_direct_payload_validation_and_exact_byte_identity() {
    let bytes = serde_json::to_vec(&empty()).unwrap();
    let parsed = parse_feed(&bytes).unwrap();
    assert_eq!(parsed.payload, empty());
    assert_eq!(
        parsed.payload_digest,
        hiroute_domain::CanonicalDigest::of_bytes(&bytes)
    );
    let mut reformatted = bytes.clone();
    reformatted.push(b'\n');
    assert_eq!(parse_feed(&reformatted).unwrap().payload, parsed.payload);
    assert_ne!(
        parse_feed(&reformatted).unwrap().payload_digest,
        parsed.payload_digest
    );
    for invalid in [
        b"not JSON".as_slice(),
        b"null",
        b"{}",
        br#"{"schema":"hiroute.model-feed.envelope/v1","payload":"e30=","signatures":[]}"#,
    ] {
        assert_eq!(
            parse_feed(invalid).unwrap_err().kind,
            FeedFailureKind::Schema
        );
    }
}

#[test]
fn model_feed_size_schema_and_duplicate_fields_are_checked() {
    assert_eq!(
        parse_feed(&vec![0; MAX_MODEL_FEED_BYTES + 1])
            .unwrap_err()
            .kind,
        FeedFailureKind::Size
    );
    let mut boundary = serde_json::to_vec(&empty()).unwrap();
    boundary.resize(MAX_MODEL_FEED_BYTES, b' ');
    assert!(parse_feed(&boundary).is_ok());
    let duplicate = String::from_utf8(serde_json::to_vec(&empty()).unwrap())
        .unwrap()
        .replace("\"sequence\":43", "\"sequence\":42,\"sequence\":43");
    assert_eq!(
        parse_feed(duplicate.as_bytes()).unwrap_err().kind,
        FeedFailureKind::Schema
    );
    let mut unknown = serde_json::to_value(empty()).unwrap();
    unknown["unknown"] = json!(true);
    assert_eq!(
        parse_feed(&serde_json::to_vec(&unknown).unwrap())
            .unwrap_err()
            .kind,
        FeedFailureKind::Schema
    );
}

#[test]
fn model_feed_forbidden_or_unknown_payload_content_rejected() {
    for kind in [
        "rating",
        "free_offer",
        "connector",
        "endpoint_profile",
        "connection_option",
    ] {
        let mut value = serde_json::to_value(empty()).unwrap();
        value["records"] = json!([{"kind":kind,"min_client_version":"0.3.2","included_on":"2026-10-18","record":{}}]);
        assert_eq!(
            parse_feed(&serde_json::to_vec(&value).unwrap())
                .unwrap_err()
                .kind,
            FeedFailureKind::Schema
        );
    }
    let mut payload = empty();
    payload.schema = "hiroute.model-feed/v99".into();
    assert!(payload.validate().is_err());
    payload = empty();
    payload.issued_at = "2026-02-30T00:00:00Z".into();
    assert!(payload.validate().is_err());
}

#[test]
fn complete_snapshot_corrects_existing_model_and_prices_without_changing_authority() {
    let current = catalog();
    let mut payload = empty();
    payload.data.models[0].capabilities.context_tokens += 100;
    payload.data.price_rates[0].input_micros_per_million += 1;
    seal(&mut payload);
    let admission = admit(&current, &payload, "0.3.2").unwrap();
    assert_eq!(admission.counts.ignored_embedded, 0);
    let effective = current.with_feed(&admission).unwrap();
    assert_eq!(effective.model_data().models, payload.data.models);
    assert_eq!(effective.model_data().price_rates, payload.data.price_rates);
    assert_eq!(effective.rating_snapshot(), &payload.data.rating_snapshot);
    assert_eq!(effective.registry(), current.registry());
    assert_eq!(
        effective.current_release_model_data(),
        current.current_release_model_data()
    );
}

#[test]
fn unsupported_external_adapter_keeps_complete_descriptions_and_rating_identity() {
    let current = catalog();
    let mut payload = empty();
    payload.data.model_endpoint_capabilities[0].required_adapter_ref = "adapter.future".into();
    seal(&mut payload);
    let admission = admit(&current, &payload, "0.3.2").unwrap();
    assert!(admission.counts.unresolved > 0);
    let effective = current.with_feed(&admission).unwrap();
    assert_eq!(effective.model_metadata(), &payload.data.metadata_catalog);
    assert_eq!(effective.rating_snapshot(), &payload.data.rating_snapshot);
    assert!(
        !effective
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .any(|c| c.required_adapter_ref == "adapter.future")
    );
    payload.data.model_endpoint_capabilities[0].model_configuration_id =
        "missing-internal-model".into();
    seal(&mut payload);
    assert!(admit(&current, &payload, "0.3.2").is_err());
}

#[test]
fn complete_snapshot_allows_withdrawal_but_cannot_change_entitlement() {
    let current = catalog();
    let mut payload = empty();
    payload.data.offers[0].model_configuration_ids.pop();
    seal(&mut payload);
    assert!(admit(&current, &payload, "0.3.2").is_ok());
    payload.data.offers[0].usage_scope = "ungranted-scope".into();
    seal(&mut payload);
    assert!(admit(&current, &payload, "0.3.2").is_err());
}

#[test]
fn complete_snapshot_rejects_same_body_digest_mismatch_duplicates_and_nested_escaped_keys() {
    let mut payload = empty();
    payload.data.models[0].capabilities.tool = !payload.data.models[0].capabilities.tool;
    assert!(payload.validate().is_err());
    payload = empty();
    payload.data.models.push(payload.data.models[0].clone());
    seal(&mut payload);
    assert!(payload.validate().is_err());
    let bytes = serde_json::to_string(&empty()).unwrap();
    let duplicate = bytes.replacen("\"tool\":", "\"to\\u006fl\":false,\"tool\":", 1);
    assert!(parse_feed(duplicate.as_bytes()).is_err());
}

#[test]
fn model_feed_rejects_invalid_dates_and_links() {
    for date in ["2026-02-30", "2026-13-01", "2026-00-01", "not-a-date"] {
        assert!(!valid_date(date));
    }
    assert!(valid_date("2024-02-29"));
    assert!(!valid_date("2025-02-29"));
    for url in [
        "https://hiroute.ai.evil/",
        "https://hiroute.ai@evil/",
        "http://hiroute.ai/",
        "https://hiroute.ai/\\evil",
        "https://hiroute.ai/\n",
    ] {
        assert!(!valid_featured_url(url));
    }
}
