//! Protected original auth → native field restore across an artifact-store restart.
use super::*;
use hiroute_domain::{AgentConfigChangeV1, AgentConfigDocumentV1};
use hiroute_integrations::{
    claude_native_configuration_is_applied, stage_claude_configuration, stage_claude_restoration,
};
use std::collections::BTreeMap;

#[path = "native_claude_test.rs"]
mod native_claude_test;

fn change(existing: bool) -> AgentConfigChangeV1 {
    let fields = if existing {
        BTreeMap::from([
            ("apiKeyHelper".into(), json!({"configured":true})),
            (
                "hiroute.auth_environment".into(),
                json!({"configured":true}),
            ),
            ("env.ANTHROPIC_MODEL".into(), json!("old-model")),
        ])
    } else {
        BTreeMap::new()
    };
    AgentConfigChangeV1::preview(&AgentConfigDocumentV1 { fields }, BTreeMap::from([
        ("apiKeyHelper".into(), Some(json!({"executable":"/trusted/hiroute", "argv":[
            hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1, "agent-connection/claude/native-test"]}))),
        ("hiroute.auth_environment".into(), None),
        ("env.ANTHROPIC_MODEL".into(), Some(json!("hiroute/0011223344556677"))),
        ("env.ANTHROPIC_BASE_URL".into(), Some(json!("http://127.0.0.1:5837"))),
    ])).unwrap()
}
fn claude_intent(
    kind: AgentConnectionTransactionKindV1,
    before: Option<CanonicalDigest>,
) -> ExternalEffectIntentV1 {
    intent_for_agent(
        kind,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        before,
        "agent.claude",
        "claude.profile.v1",
    )
}
const BEFORE: &[u8] = br#"{"apiKeyHelper":"old-helper --never-execute","env":{"ANTHROPIC_AUTH_TOKEN":"old-api-secret","ANTHROPIC_MODEL":"old-model","UNRELATED":"keep"},"theme":"before"}"#;

#[test]
fn claude_protected_restore_preserves_original_auth_and_unrelated_user_edits() {
    for existing in [false, true] {
        for user_edit in [false, true] {
            let root = crate::test_tempdir().unwrap();
            let path = root.path().join("settings.json");
            if existing {
                write(&path, BEFORE);
            }
            let proto = claude_intent(AgentConnectionTransactionKindV1::Apply, None);
            let first = store(root.path(), proto.target(), &path);
            let install = claude_intent(
                AgentConnectionTransactionKindV1::Apply,
                first.current_external_fingerprint(proto.target()).unwrap(),
            );
            let operation = OperationId::parse("op_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
            let before = if existing { BEFORE } else { &[] };
            let change = change(existing);
            let staged = stage_claude_configuration(
                &first,
                &operation,
                &install,
                &CanonicalDigest::of_bytes(before),
                &change,
            )
            .unwrap();
            first.activate_artifact(&staged).unwrap();
            let applied = fs::read_to_string(&path).unwrap();
            assert!(!applied.contains("old-api-secret"));
            assert!(!applied.contains("old-helper"));
            assert!(applied.contains(hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1));
            if user_edit {
                let mut value: serde_json::Value = serde_json::from_str(&applied).unwrap();
                value["theme"] = json!("user-theme");
                write(&path, &serde_json::to_vec(&value).unwrap());
            }
            assert!(
                claude_native_configuration_is_applied(&first, &operation, &install).unwrap(),
                "unrelated user edits must not revoke an otherwise-owned ordinary entry"
            );
            drop(first);
            let reopened = store(root.path(), install.target(), &path);
            let restore = claude_intent(
                AgentConnectionTransactionKindV1::Restore,
                reopened
                    .current_external_fingerprint(install.target())
                    .unwrap(),
            );
            let restore_operation =
                OperationId::parse("op_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
            let effect = stage_claude_restoration(
                &reopened,
                &restore_operation,
                &restore,
                &operation,
                &install,
            )
            .unwrap();
            reopened.activate_artifact(&effect).unwrap();
            assert_eq!(
                stage_claude_restoration(
                    &reopened,
                    &restore_operation,
                    &restore,
                    &operation,
                    &install
                )
                .unwrap(),
                effect
            );
            if !existing && !user_edit {
                assert!(!path.exists());
                continue;
            }
            let restored: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            let mut expected = if existing {
                serde_json::from_slice(BEFORE).unwrap()
            } else {
                json!({})
            };
            if user_edit {
                expected["theme"] = json!("user-theme");
            }
            assert_eq!(restored, expected);
        }
    }
}

#[test]
fn claude_native_restore_rejects_owned_field_edits_and_duplicate_unknown_json() {
    let root = crate::test_tempdir().unwrap();
    let path = root.path().join("settings.json");
    let install = claude_intent(AgentConnectionTransactionKindV1::Apply, None);
    let artifacts = store(root.path(), install.target(), &path);
    let operation = OperationId::parse("op_cccccccccccccccccccccccccccccccc").unwrap();
    let change = change(false);
    let staged = stage_claude_configuration(
        &artifacts,
        &operation,
        &install,
        &CanonicalDigest::of_bytes(b""),
        &change,
    )
    .unwrap();
    artifacts.activate_artifact(&staged).unwrap();
    let mut modified: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    modified["env"]["ANTHROPIC_MODEL"] = json!("user-model");
    let bytes = serde_json::to_vec(&modified).unwrap();
    write(&path, &bytes);
    assert!(!claude_native_configuration_is_applied(&artifacts, &operation, &install).unwrap());
    let restore = claude_intent(
        AgentConnectionTransactionKindV1::Restore,
        artifacts
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    assert!(
        stage_claude_restoration(
            &artifacts,
            &OperationId::parse("op_dddddddddddddddddddddddddddddddd").unwrap(),
            &restore,
            &operation,
            &install
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    let duplicate = br#"{"unowned":{"a":1,"a":2}}"#;
    write(&path, duplicate);
    let install = claude_intent(
        AgentConnectionTransactionKindV1::Apply,
        artifacts
            .current_external_fingerprint(install.target())
            .unwrap(),
    );
    assert!(
        stage_claude_configuration(
            &artifacts,
            &OperationId::parse("op_eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee").unwrap(),
            &install,
            &CanonicalDigest::of_bytes(duplicate),
            &change
        )
        .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), duplicate);
}
