use super::*;
use hiroute_integrations::restage_codex_catalog;

fn catalog() -> Vec<u8> {
    include_bytes!("../../../integrations/src/agents/codex_bundled_catalog.json").to_vec()
}

fn catalog_intent(bytes: &[u8]) -> ExternalEffectIntentV1 {
    catalog_intent_with_before(bytes, None)
}

fn catalog_intent_with_before(
    bytes: &[u8],
    before: Option<CanonicalDigest>,
) -> ExternalEffectIntentV1 {
    let spec = ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "agents.settings.apply".into(),
        resource_id: Some("agent-context/catalog".into()),
        desired_state: json!({
            "schema_version":{"major":2,"minor":0},
            "context_id":"agent-context/catalog",
            "model":{"intent":"configure","settings":{
                "mode":"codex_default",
                "native_model_mode":"preserve_available",
                "fixed_models":[{"client_model_id":"native","candidate":{"binding_id":"binding/native"}}],
                "allowed_plan_ids":[],"default_selection":{"kind":"preserve_native"}
            }},
            "collaboration":{"intent":"keep"}
        }),
    };
    let subject = AgentConnectionTransactionSubjectV1::from_registered_profile(
        "agent_codex_default",
        "codex-responses-v1",
        "builtin/codex-responses/v1",
    )
    .unwrap();
    let control = AgentConnectionControlIntentV1::from_settings_planner(
        subject,
        &spec,
        false,
        &json!({"revision":1}),
    )
    .unwrap();
    ExternalEffectIntentV1::from_agent_connection_planner(
        &control,
        AgentConnectionEffectRoleV1::ModelCatalog,
        before,
        &json!({
            "schema":"hiroute.codex-catalog-artifact/v1",
            "context_id":"agent-context/catalog",
            "source_revision":hiroute_integrations::CODEX_CATALOG_SOURCE_REVISION,
            "content_digest":CanonicalDigest::of_bytes(bytes),
            "producer_kind":"target_cache",
            "producer_path":"/target/.codex/models_cache.json",
            "producer_content_digest":CanonicalDigest::of_bytes(b"native-catalog"),
            "producer_context_digest":CanonicalDigest::of_bytes(b"codex-context"),
            "producer_dependency_digest":CanonicalDigest::of_bytes(b"catalog-dependencies")
        }),
        0o600,
    )
    .unwrap()
}

#[test]
fn existing_identical_immutable_catalog_is_a_protected_no_change_reuse() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("catalog.json");
    let bytes = catalog();
    write(&path, &bytes);
    let prototype = catalog_intent(&bytes);
    let artifacts = store(temp.path(), prototype.target(), &path);
    let before = artifacts
        .current_external_fingerprint(prototype.target())
        .unwrap();
    assert!(before.is_some());
    let intent = catalog_intent_with_before(&bytes, before);
    let op = operation();
    artifacts.save_native_restore(&op, &intent, &bytes).unwrap();
    let effect = restage_codex_catalog(&artifacts, &op, &intent).unwrap();
    artifacts.activate_artifact(&effect).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

fn operation() -> OperationId {
    OperationId::parse(format!("op_{}", "a".repeat(32))).unwrap()
}

#[test]
fn codex_catalog_protected_recovery_stages_then_activates_private_exact_bytes() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("catalog.json");
    let bytes = catalog();
    let intent = catalog_intent(&bytes);
    let op = operation();
    let artifacts = store(temp.path(), intent.target(), &path);
    artifacts.save_native_restore(&op, &intent, &bytes).unwrap();
    drop(artifacts);
    let artifacts = store(temp.path(), intent.target(), &path);
    let effect = restage_codex_catalog(&artifacts, &op, &intent).unwrap();
    assert!(!path.exists());
    artifacts.activate_artifact(&effect).unwrap();
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        restage_codex_catalog(&artifacts, &op, &intent).unwrap(),
        effect
    );
}

#[test]
fn codex_catalog_conflicting_existing_bytes_are_never_overwritten() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("catalog.json");
    write(&path, b"user content");
    let bytes = catalog();
    let intent = catalog_intent(&bytes);
    let artifacts = store(temp.path(), intent.target(), &path);
    artifacts
        .save_native_restore(&operation(), &intent, &bytes)
        .unwrap();
    let error = restage_codex_catalog(&artifacts, &operation(), &intent).unwrap_err();
    assert_eq!(error.code, PortErrorCode::Conflict);
    assert_eq!(fs::read(path).unwrap(), b"user content");
}

#[test]
fn codex_catalog_protected_bytes_must_match_journal_digest() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("catalog.json");
    let intent = catalog_intent(&catalog());
    let artifacts = store(temp.path(), intent.target(), &path);
    artifacts
        .save_native_restore(&operation(), &intent, b"different")
        .unwrap();
    assert_eq!(
        restage_codex_catalog(&artifacts, &operation(), &intent)
            .unwrap_err()
            .code,
        PortErrorCode::InvalidData
    );
    assert!(!path.exists());
}

#[test]
fn codex_catalog_matching_digest_does_not_replace_schema_validation() {
    let temp = crate::test_tempdir().unwrap();
    let path = temp.path().join("catalog.json");
    let bytes = br#"{"models":[{"slug":"native","priority":0,"visibility":"list","supported_in_api":true}]}"#;
    let intent = catalog_intent(bytes);
    let artifacts = store(temp.path(), intent.target(), &path);
    artifacts
        .save_native_restore(&operation(), &intent, bytes)
        .unwrap();
    assert_eq!(
        restage_codex_catalog(&artifacts, &operation(), &intent)
            .unwrap_err()
            .code,
        PortErrorCode::InvalidData
    );
    assert!(!path.exists());
}
