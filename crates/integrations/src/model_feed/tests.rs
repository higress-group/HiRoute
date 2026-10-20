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
    FeedPayload {
        schema: MODEL_FEED_SCHEMA.into(),
        sequence: 43,
        issued_at: "2026-10-18T00:00:00Z".into(),
        source_revision: "0123456789abcdef0123456789abcdef01234567".into(),
        records: vec![],
        featured: vec![],
    }
}

fn new_model() -> FeedPayload {
    let current = catalog();
    let capability = current
        .model_data()
        .model_endpoint_capabilities
        .iter()
        .find(|c| {
            current.model_data().offers.iter().any(|o| {
                o.endpoint_profile_id == c.endpoint_profile_id
                    && o.model_configuration_ids
                        .contains(&c.model_configuration_id)
            })
        })
        .unwrap()
        .clone();
    let mut definition = current
        .model_data()
        .model(&capability.model_configuration_id)
        .unwrap()
        .clone();
    definition.model_configuration_id = "model.test.new".into();
    definition.display_name = "New model".into();
    let mut endpoint = capability.clone();
    endpoint.model_configuration_id = definition.model_configuration_id.clone();
    endpoint.capability_id = "capability.test.new".into();
    endpoint.upstream_model_id = "new-upstream".into();
    let canonical = current.model_metadata().canonical_models[0].clone();
    let mut offer = current
        .model_data()
        .offers
        .iter()
        .find(|o| {
            o.endpoint_profile_id == capability.endpoint_profile_id
                && o.model_configuration_ids
                    .contains(&capability.model_configuration_id)
        })
        .unwrap()
        .clone();
    offer
        .model_configuration_ids
        .push(definition.model_configuration_id.clone());
    let mut payload = empty();
    payload.records = vec![
        FeedRecord::Model {
            min_client_version: "0.3.2".into(),
            included_on: "2026-10-18".into(),
            record: Box::new(FeedModel {
                definition,
                native_reasoning: None,
                canonical,
            }),
        },
        FeedRecord::ModelEndpointCapability {
            min_client_version: "0.3.2".into(),
            included_on: "2026-10-18".into(),
            record: endpoint,
        },
        FeedRecord::Offer {
            min_client_version: "0.3.2".into(),
            included_on: "2026-10-18".into(),
            record: offer,
        },
    ];
    payload
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
    payload.schema = "hiroute.model-feed/v2".into();
    assert!(payload.validate().is_err());
    payload = empty();
    payload.issued_at = "2026-02-30T00:00:00Z".into();
    assert!(payload.validate().is_err());
}

#[test]
fn model_feed_adds_model_and_offer_members_without_changing_registry_or_ratings() {
    let current = catalog();
    let payload = new_model();
    let admission = admit(&current, &payload, "0.3.2").unwrap();
    assert_eq!(admission.counts.applied, 3);
    let effective = current.with_feed(&admission).unwrap();
    assert!(effective.model_data().model("model.test.new").is_some());
    assert!(effective.model_data().bundle_version.ends_with("/feed:43"));
    assert_eq!(effective.registry(), current.registry());
    assert_eq!(effective.rating_snapshot(), current.rating_snapshot());
    assert_eq!(
        effective.current_release_model_data(),
        current.current_release_model_data()
    );
    assert_eq!(
        effective.model_data_provenance().2,
        current
            .with_feed(&admission)
            .unwrap()
            .model_data_provenance()
            .2
    );
    assert!(matches!(
        effective.model_fact_origin("model.test.new"),
        Some(ModelFactOrigin::Feed { sequence: 43, .. })
    ));
    assert!(effective.compute_catalog_provenance().is_ok());
}

#[test]
fn model_feed_embedded_identity_and_price_win() {
    let current = catalog();
    let mut payload = new_model();
    if let FeedRecord::Model { record, .. } = &mut payload.records[0] {
        record.definition = current.model_data().models[0].clone();
    }
    payload.records.truncate(1);
    let rate = current.model_data().price_rates[0].clone();
    payload.records.push(FeedRecord::PriceRate {
        min_client_version: "0.3.2".into(),
        included_on: "2026-10-18".into(),
        record: rate,
    });
    let admission = admit(&current, &payload, "0.3.2").unwrap();
    assert_eq!(admission.counts.ignored_embedded, 2);
    let effective = current.with_feed(&admission).unwrap();
    assert_eq!(
        effective.model_data_provenance(),
        current.model_data_provenance()
    );
}

#[test]
fn model_feed_offer_cannot_drop_or_change_embedded_authority() {
    let mut payload = new_model();
    if let FeedRecord::Offer { record, .. } = &mut payload.records[2] {
        record.model_configuration_ids.remove(0);
    }
    assert!(admit(&catalog(), &payload, "0.3.2").is_err());
    payload = new_model();
    if let FeedRecord::Offer { record, .. } = &mut payload.records[2] {
        record.revision += 1;
    }
    assert!(admit(&catalog(), &payload, "0.3.2").is_err());
}

#[test]
fn model_feed_newer_model_is_non_executable_but_can_be_announced() {
    let current = catalog();
    let mut payload = new_model();
    for entry in &mut payload.records {
        match entry {
            FeedRecord::Model {
                min_client_version, ..
            }
            | FeedRecord::ModelEndpointCapability {
                min_client_version, ..
            }
            | FeedRecord::Offer {
                min_client_version, ..
            }
            | FeedRecord::PriceRate {
                min_client_version, ..
            } => *min_client_version = "0.3.3".into(),
        }
    }
    payload.featured.push(FeaturedModel {
        id: "test.new".into(),
        kind: FeaturedKind::NewModel,
        model_configuration_id: "model.test.new".into(),
        headline: LocalizedFeedText {
            en: "New model".into(),
            zh: "新模型".into(),
        },
        note: LocalizedFeedText {
            en: "Requires a client upgrade".into(),
            zh: "需要升级".into(),
        },
        links: vec!["https://hiroute.ai/models/".into()],
        included_on: "2026-10-18".into(),
    });
    let admission = admit(&current, &payload, "0.3.2").unwrap();
    let effective = current.with_feed(&admission).unwrap();
    assert!(effective.model_data().model("model.test.new").is_none());
    assert_eq!(
        effective.required_feed_client("model.test.new"),
        Some("0.3.3")
    );
    assert_eq!(effective.featured_models().len(), 1);
    payload.featured[0].model_configuration_id = "model.nonexistent".into();
    assert!(
        current
            .with_feed(&admit(&current, &payload, "0.3.2").unwrap())
            .unwrap()
            .featured_models()
            .is_empty()
    );
}

#[test]
fn model_feed_rejects_invalid_dates_links_duplicates_and_versions() {
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
    let mut payload = new_model();
    payload.records.push(payload.records[0].clone());
    assert!(payload.validate().is_err());
    payload = new_model();
    if let FeedRecord::Model {
        min_client_version, ..
    } = &mut payload.records[0]
    {
        *min_client_version = "latest".into();
    }
    assert!(payload.validate().is_err());
}
