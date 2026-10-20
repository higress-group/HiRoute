use super::*;

fn current() -> TrustedReleaseCatalog {
    load_catalog(
        |file| {
            std::fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../assets/release-facts/current/bundle")
                    .join(file),
            )
            .map_err(|e| e.to_string())
        },
        true,
    )
    .unwrap()
}
#[test]
fn model_feed_projection_includes_complete_existing_models() {
    let current = current();
    let clients = vec![SupportedFeedClient {
        version: "0.3.2".into(),
        catalog: current.clone(),
    }];
    let payload = project(
        &current,
        &clients,
        3,
        "2026-10-18T00:00:00Z",
        "0123456789abcdef0123456789abcdef01234567",
        vec![],
        None,
    )
    .unwrap();
    assert_eq!(payload.data.models, current.model_data().models);
    assert_eq!(payload.data.metadata_catalog, *current.model_metadata());
    assert!(!payload.data.pricing_facts.is_empty());
    assert_eq!(payload.sequence, 3);
    assert_eq!(
        project(
            &current,
            &clients,
            3,
            &payload.issued_at,
            &payload.source_revision,
            vec![],
            Some(&payload)
        )
        .unwrap(),
        payload
    );
}
#[test]
fn model_feed_bootstrap_checks_real_shared_metadata_and_is_repeatable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    build(&root, true).unwrap();
    build(&root, true).unwrap();
}
#[test]
fn model_feed_editorial_record_cannot_introduce_untrusted_links() {
    let current = current();
    let clients = vec![SupportedFeedClient {
        version: "0.3.2".into(),
        catalog: current.clone(),
    }];
    let featured = vec![FeaturedModel {
        id: "test-model-update".into(),
        kind: FeaturedKind::ModelUpdate,
        model_configuration_id: current.model_data().models[0]
            .model_configuration_id
            .clone(),
        headline: LocalizedFeedText {
            en: "Update".into(),
            zh: "更新".into(),
        },
        note: LocalizedFeedText {
            en: "Provider announcement".into(),
            zh: "供应商公告".into(),
        },
        links: vec!["https://elsewhere.example/redirect".into()],
        included_on: "2026-10-18".into(),
    }];
    assert!(
        project(
            &current,
            &clients,
            3,
            "2026-10-18T00:00:00Z",
            "0123456789abcdef0123456789abcdef01234567",
            featured,
            None
        )
        .is_err()
    );
}

fn without_model(id: &str) -> TrustedReleaseCatalog {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut input =
        crate::read_compiler_input(&root.join("assets/release-facts/current/compiler-input.json"))
            .unwrap();
    let data = &mut input.model_data.data;
    data.models.retain(|m| m.model_configuration_id != id);
    data.model_endpoint_capabilities
        .retain(|c| c.model_configuration_id != id);
    data.price_rates.retain(|p| p.model_configuration_id != id);
    for offer in &mut data.offers {
        offer.model_configuration_ids.retain(|member| member != id);
    }
    data.offers
        .retain(|offer| !offer.model_configuration_ids.is_empty());
    let ratings = &mut input.model_data.rating_snapshot;
    ratings.models.retain(|m| m.model_configuration_id != id);
    ratings.records.retain(|r| r.model_configuration_id != id);
    ratings.model_catalog_digest = CanonicalDigest::of(&data.models).unwrap();
    ratings.digest = ratings.computed_digest().unwrap();
    let output = crate::compile_release_facts(&input).unwrap();
    load_catalog(
        |file| {
            output
                .files
                .get(file)
                .cloned()
                .ok_or_else(|| "no complete snapshot in this released fixture".into())
        },
        false,
    )
    .unwrap()
}

#[test]
fn model_feed_projection_closes_a_real_model_graph_and_preserves_first_inclusion() {
    let current = current();
    let id = current
        .model_data()
        .offers
        .iter()
        .find(|offer| offer.billing_class == hiroute_domain::BillingClass::Paid)
        .expect("current catalog has a paid offer")
        .model_configuration_ids[0]
        .clone();
    let clients = vec![
        SupportedFeedClient {
            version: "0.3.2".into(),
            catalog: without_model(&id),
        },
        SupportedFeedClient {
            version: "0.3.3".into(),
            catalog: current.clone(),
        },
    ];
    let first = project(
        &current,
        &clients,
        1,
        "2026-10-18T00:00:00Z",
        &"1".repeat(40),
        vec![],
        None,
    )
    .unwrap();
    assert!(
        first
            .data
            .models
            .iter()
            .any(|m| m.model_configuration_id == id)
    );
    let admission = admit(&clients[0].catalog, &first, "0.3.2").unwrap();
    let effective = clients[0].catalog.with_feed(&admission).unwrap();
    assert_eq!(
        effective.model_data().model(&id),
        current.model_data().model(&id)
    );
    assert_eq!(
        admit(&clients[1].catalog, &first, "0.3.3")
            .unwrap()
            .counts
            .unresolved,
        0
    );
    let second = project(
        &current,
        &clients,
        2,
        "2026-10-19T00:00:00Z",
        &"2".repeat(40),
        vec![],
        Some(&first),
    )
    .unwrap();
    assert_eq!(first.data_digest, second.data_digest);
    validate_transition(&first, &second).unwrap();
    let mut changed = second.clone();
    changed.data.models[0].capabilities.context_tokens += 1;
    changed.data.rating_snapshot.model_catalog_digest =
        CanonicalDigest::of(&changed.data.models).unwrap();
    changed.data.rating_snapshot.digest = changed.data.rating_snapshot.computed_digest().unwrap();
    changed.data_digest = CanonicalDigest::of(&changed.data).unwrap();
    assert!(validate_transition(&first, &changed).is_err());
    changed = second;
    changed.sequence = first.sequence;
    assert!(validate_transition(&first, &changed).is_err());
}

#[test]
fn model_feed_bootstrap_rejects_invalid_editorial_inputs_before_release_lookup() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("assets/model-feed");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("policy.json"),
        r#"{"minimum_feed_client":"0.3.2"}"#,
    )
    .unwrap();
    std::fs::write(
        directory.join("publication.json"),
        r#"{"sequence":1,"issued_at":"2026-02-30T00:00:00Z"}"#,
    )
    .unwrap();
    std::fs::write(directory.join("featured.json"), "[]").unwrap();
    let error = build(root.path(), true).unwrap_err();
    assert!(
        !error.contains("cannot read"),
        "schema validation must run in bootstrap: {error}"
    );
}

#[test]
fn feed_payload_check_survives_a_bundle_changing_squash_merge() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for relative in [
        "assets/release-facts/current/bundle/official-model-data.json",
        "crates/integrations/src/model_feed/mod.rs",
        "assets/release-facts/current/bundle/manifest.json",
        "assets/release-facts/current/bundle/connector-registry.json",
        "assets/release-facts/current/bundle/model-data.json",
        "assets/model-feed/publication.json",
        "assets/model-feed/featured.json",
        "assets/model-feed/payload.json",
        "assets/model-data/current/inputs/metadata-catalog.json",
        "contracts/releases/v0.3.1.json",
    ] {
        let target = root.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(source.join(relative), target).unwrap();
    }
    std::fs::write(
        root.join("assets/model-feed/policy.json"),
        br#"{"minimum_feed_client":"0.3.1"}"#,
    )
    .unwrap();
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Feed test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Feed test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git operation failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    run(&["init", "-b", "main"]);
    run(&["add", "."]);
    run(&["commit", "-m", "Supported catalog"]);
    let revision = run(&["rev-parse", "HEAD"]);
    run(&["tag", "v0.3.1"]);
    let mut record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("contracts/releases/v0.3.1.json")).unwrap(),
    )
    .unwrap();
    record["release"]["revision"] = serde_json::json!(revision);
    record["release"]["tag"] = serde_json::json!("v0.3.1");
    let snapshot = serde_json::to_vec(&record).unwrap();
    std::fs::write(root.join("contracts/releases/v0.3.1.json"), &snapshot).unwrap();
    std::fs::write(root.join("contracts/releases/index.v1.json"), serde_json::to_vec(&serde_json::json!({
        "schema":"hiroute.release-contract-index/v1", "releases":[{"channel":"stable","file":"v0.3.1.json","tag":"v0.3.1","revision":revision,"sha256":&CanonicalDigest::of_bytes(&snapshot).as_str()[7..]}]
    })).unwrap()).unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "Record supported source"]);
    run(&["checkout", "-b", "trial"]);
    std::fs::write(
        root.join("assets/release-facts/current/bundle/source-note.txt"),
        "A committed bundle update.\n",
    )
    .unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "Update bundle"]);
    build(root, false).unwrap();
    run(&["add", "."]);
    run(&["commit", "-m", "Project publication"]);
    run(&["checkout", "main"]);
    run(&["merge", "--squash", "trial"]);
    run(&["commit", "-m", "Land catalog and publication"]);
    build(root, true).unwrap();
    let payload: FeedPayload = serde_json::from_slice(
        &std::fs::read(root.join("assets/model-feed/payload.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        git_bytes(
            root,
            &[
                "show",
                &format!("{}:model-data.json", payload.source_revision)
            ]
        )
        .unwrap(),
        std::fs::read(root.join("assets/release-facts/current/bundle/model-data.json")).unwrap()
    );
}

#[test]
fn model_feed_current_and_recorded_v2_sources_require_the_complete_snapshot() {
    let bundle =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/release-facts/current/bundle");
    for require_complete in [false, true] {
        let result = load_catalog(
            |file| {
                if file == "official-model-data.json" {
                    return Err("complete snapshot missing".into());
                }
                std::fs::read(bundle.join(file)).map_err(|e| e.to_string())
            },
            require_complete,
        );
        assert_eq!(result.is_err(), require_complete);
    }
}
