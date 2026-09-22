use std::fs;

use crate::test_tempdir as tempdir;
use hiroute_domain::{
    AgentAccessGrantMaterialActionV1, AgentAccessGrantMutationV1, AgentAccessGrantRefV1,
    AgentAccessGrantScopeV1, AgentIngressProtocolV1, AgentModelGrantV2, AgentModelRouteV2,
    AgentPlanId, CanonicalDigest, CompensationOutcome, EffectReconciliation, ModelAlias,
    OperationId, PortErrorCode, ProtectedSecret, SecretStorePort,
};

use super::LocalSecretStore;

const CONNECTION_ID: &str = "agent-connection/claude-code";
const OWNER_SCOPE: &str = "principal/local-owner";

fn operation_id(value: char) -> OperationId {
    OperationId::parse(format!("op_{}", value.to_string().repeat(32))).unwrap()
}

fn scope(marker: &[u8]) -> AgentAccessGrantScopeV1 {
    scope_for(CONNECTION_ID, marker)
}

fn scope_for(connection_id: &str, marker: &[u8]) -> AgentAccessGrantScopeV1 {
    let routes = ["hiroute/0123456789abcdef", "hiroute/fedcba9876543210"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            (
                name.to_owned(),
                AgentModelRouteV2::Plan {
                    plan_id: AgentPlanId::parse(format!("plan/secret-test-{index}")).unwrap(),
                    alias: ModelAlias::parse(name).unwrap(),
                    revision: 1,
                    semantic_digest: CanonicalDigest::of_bytes(marker),
                },
            )
        })
        .collect();
    AgentAccessGrantScopeV1::new(
        connection_id,
        AgentModelGrantV2::seal(AgentIngressProtocolV1::Messages, routes).unwrap(),
    )
    .unwrap()
}

fn ensure(scope: AgentAccessGrantScopeV1, expected_generation: u64) -> AgentAccessGrantMutationV1 {
    AgentAccessGrantMutationV1::ensure(OWNER_SCOPE, scope, expected_generation).unwrap()
}

fn store() -> (tempfile::TempDir, LocalSecretStore) {
    let directory = tempdir().unwrap();
    let root = directory.path().join("data");
    let store = LocalSecretStore::open(
        &crate::test_storage_authority(),
        root.join("secrets.db"),
        root.join("master-key"),
        root.join("backups"),
    )
    .unwrap();
    (directory, store)
}

fn staged_ref(
    effect: &hiroute_domain::OwnedEffectV1,
    mutation: &AgentAccessGrantMutationV1,
) -> AgentAccessGrantRefV1 {
    AgentAccessGrantRefV1::from_ensure_effect(effect, mutation).unwrap()
}

#[test]
fn apply_persists_only_aead_ciphertext_and_non_secret_effect_metadata() {
    let (_directory, store) = store();
    let mutation = ensure(scope(b"plan-grant-v1"), 0);
    let effect = store
        .apply_agent_access_grant(&operation_id('1'), &mutation, None)
        .unwrap();
    let staged = staged_ref(&effect, &mutation);

    assert!(matches!(
        store
            .observe_agent_access_grant(&operation_id('1'), &mutation)
            .unwrap(),
        EffectReconciliation::Staged(_)
    ));
    assert!(
        store
            .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
            .unwrap()
            .is_none()
    );
    assert_eq!(staged.generation(), 1);

    store.activate_agent_access_grant(&effect).unwrap();
    let active = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    assert_eq!(active, staged);
    let material = store.resolve_agent_access_grant(&active).unwrap();
    let plaintext = material.expose().to_vec();
    assert_eq!(plaintext.len(), 43);

    store.with_connection(|connection| {
        let (ciphertext, aad_schema): (Vec<u8>, String) = connection
            .query_row(
                "SELECT ciphertext, aad_schema FROM agent_access_grant_versions
                 WHERE connection_id = ?1 AND generation = 1",
                [CONNECTION_ID],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_ne!(ciphertext, plaintext);
        assert!(
            !ciphertext
                .windows(plaintext.len())
                .any(|part| part == plaintext)
        );
        assert_eq!(aad_schema, "hiroute.agent-access-grant-entry/v1");
        let journal_columns: Vec<String> = connection
            .prepare("PRAGMA table_info(agent_access_grant_effects)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(!journal_columns.iter().any(|column| {
            column.contains("ciphertext") || column.contains("plaintext") || column == "material"
        }));
    });

    let effect_json = serde_json::to_vec(&effect).unwrap();
    let reference_json = serde_json::to_vec(&active).unwrap();
    assert!(
        !effect_json
            .windows(plaintext.len())
            .any(|part| part == plaintext)
    );
    assert!(
        !reference_json
            .windows(plaintext.len())
            .any(|part| part == plaintext)
    );
    store.checkpoint().unwrap();
    let database = fs::read(store.database_path()).unwrap();
    assert!(
        !database
            .windows(plaintext.len())
            .any(|part| part == plaintext)
    );
}

#[test]
fn same_desired_scope_reuses_active_generation_and_material() {
    let (_directory, store) = store();
    let desired = scope(b"same-plan-grant");
    let first_mutation = ensure(desired.clone(), 0);
    let first_effect = store
        .apply_agent_access_grant(&operation_id('2'), &first_mutation, None)
        .unwrap();
    store.activate_agent_access_grant(&first_effect).unwrap();
    let first_ref = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    let first_material = store.resolve_agent_access_grant(&first_ref).unwrap();

    let second_mutation = ensure(desired, 1);
    let second_effect = store
        .apply_agent_access_grant(&operation_id('3'), &second_mutation, None)
        .unwrap();
    let second_staged = staged_ref(&second_effect, &second_mutation);
    assert_eq!(second_staged, first_ref);
    store.activate_agent_access_grant(&second_effect).unwrap();
    let second_ref = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    let second_material = store.resolve_agent_access_grant(&second_ref).unwrap();
    assert_eq!(second_ref, first_ref);
    assert_eq!(second_material.expose(), first_material.expose());
    store.with_connection(|connection| {
        let count: u64 = connection
            .query_row(
                "SELECT count(*) FROM agent_access_grant_versions WHERE connection_id = ?1",
                [CONNECTION_ID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    });
}

#[test]
fn scope_change_preserves_token_and_compensation_restores_previous_scope() {
    let (_directory, store) = store();
    let first_mutation = ensure(scope(b"plan-grant-before"), 0);
    let first_effect = store
        .apply_agent_access_grant(&operation_id('4'), &first_mutation, None)
        .unwrap();
    store.activate_agent_access_grant(&first_effect).unwrap();
    let before = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    let before_material = store.resolve_agent_access_grant(&before).unwrap();
    let before_plaintext = before_material.expose().to_vec();

    let rotate = ensure(scope(b"plan-grant-after"), 1);
    let rotate_effect = store
        .apply_agent_access_grant(&operation_id('5'), &rotate, None)
        .unwrap();
    let staged = staged_ref(&rotate_effect, &rotate);
    assert_eq!(staged.generation(), 2);
    assert_eq!(staged.material_sha256(), before.material_sha256());
    assert_eq!(
        store
            .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        store
            .resolve_agent_access_grant(&staged)
            .err()
            .unwrap()
            .code,
        PortErrorCode::PermissionDenied
    );

    store.activate_agent_access_grant(&rotate_effect).unwrap();
    assert_eq!(
        store.resolve_agent_access_grant(&staged).unwrap().expose(),
        before_plaintext
    );
    assert_eq!(
        store
            .resolve_agent_access_grant(&before)
            .err()
            .unwrap()
            .code,
        PortErrorCode::PermissionDenied
    );
    assert_eq!(
        store.compensate_agent_access_grant(&rotate_effect).unwrap(),
        CompensationOutcome::Compensated
    );
    let restored = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    assert_eq!(restored, before);
    assert_eq!(
        store
            .resolve_agent_access_grant(&restored)
            .unwrap()
            .expose(),
        before_plaintext
    );
    assert!(matches!(
        store
            .observe_agent_access_grant(&operation_id('5'), &rotate)
            .unwrap(),
        EffectReconciliation::Missing
    ));
}

#[test]
fn custom_save_duplicate_regeneration_and_invalid_input_preserve_atomic_grant() {
    let (_directory, store) = store();
    let initial = ensure(scope(b"initial"), 0);
    let initial_effect = store
        .apply_agent_access_grant(&operation_id('a'), &initial, None)
        .unwrap();
    store.activate_agent_access_grant(&initial_effect).unwrap();
    let first = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    let initial_token = store.resolve_agent_access_grant(&first).unwrap();

    let invalid = ProtectedSecret::new(b"short".to_vec()).unwrap();
    let invalid_mutation = ensure(scope(b"initial"), 1)
        .with_material_action(AgentAccessGrantMaterialActionV1::Set {
            input_slot: "candidate/native/agent-token-invalid".into(),
            fingerprint: store.fingerprint(&invalid).unwrap(),
        })
        .unwrap();
    assert_eq!(
        store
            .apply_agent_access_grant(&operation_id('b'), &invalid_mutation, Some(&invalid))
            .unwrap_err()
            .code,
        PortErrorCode::InvalidData,
    );
    assert_eq!(
        store
            .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
            .unwrap(),
        Some(first.clone())
    );
    assert_eq!(
        store.resolve_agent_access_grant(&first).unwrap().expose(),
        initial_token.expose()
    );

    let custom = ProtectedSecret::new(b"custom-unique-token-0001".to_vec()).unwrap();
    let set = |generation| {
        ensure(scope(b"initial"), generation)
            .with_material_action(AgentAccessGrantMaterialActionV1::Set {
                input_slot: "candidate/native/agent-token-custom".into(),
                fingerprint: store.fingerprint(&custom).unwrap(),
            })
            .unwrap()
    };
    let changed = set(1);
    let effect = store
        .apply_agent_access_grant(&operation_id('c'), &changed, Some(&custom))
        .unwrap();
    store.activate_agent_access_grant(&effect).unwrap();
    let second = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    assert_eq!(second.generation(), 2);
    assert_eq!(
        store.resolve_agent_access_grant(&second).unwrap().expose(),
        custom.expose()
    );
    assert_ne!(second.material_sha256(), first.material_sha256());
    assert_eq!(
        store.resolve_agent_access_grant(&first).err().unwrap().code,
        PortErrorCode::PermissionDenied
    );

    let duplicate = set(2);
    let duplicate_effect = store
        .apply_agent_access_grant(&operation_id('d'), &duplicate, Some(&custom))
        .unwrap();
    store
        .activate_agent_access_grant(&duplicate_effect)
        .unwrap();
    let unchanged = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    assert_eq!(unchanged, second);

    let regenerate = ensure(scope(b"initial"), 2)
        .with_material_action(AgentAccessGrantMaterialActionV1::Regenerate)
        .unwrap();
    let generated_effect = store
        .apply_agent_access_grant(&operation_id('e'), &regenerate, None)
        .unwrap();
    store
        .activate_agent_access_grant(&generated_effect)
        .unwrap();
    let third = store
        .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
        .unwrap()
        .unwrap();
    assert_eq!(third.generation(), 3);
    assert_ne!(third.material_sha256(), second.material_sha256());
    assert_eq!(
        store
            .resolve_agent_access_grant(&second)
            .err()
            .unwrap()
            .code,
        PortErrorCode::PermissionDenied
    );
}

#[test]
fn separate_agent_connections_keep_independent_tokens() {
    let (_directory, store) = store();
    let codex_id = "agent-connection/codex-shared-scope";
    let claude = ensure(scope(b"claude"), 0);
    let codex = ensure(scope_for(codex_id, b"codex"), 0);
    let claude_effect = store
        .apply_agent_access_grant(&operation_id('a'), &claude, None)
        .unwrap();
    store.activate_agent_access_grant(&claude_effect).unwrap();
    let codex_effect = store
        .apply_agent_access_grant(&operation_id('b'), &codex, None)
        .unwrap();
    store.activate_agent_access_grant(&codex_effect).unwrap();
    let codex_before = store
        .inspect_agent_access_grant(OWNER_SCOPE, codex_id)
        .unwrap()
        .unwrap();
    let codex_token = store.resolve_agent_access_grant(&codex_before).unwrap();
    let claude_new = ensure(scope(b"claude"), 1)
        .with_material_action(AgentAccessGrantMaterialActionV1::Regenerate)
        .unwrap();
    let claude_effect = store
        .apply_agent_access_grant(&operation_id('c'), &claude_new, None)
        .unwrap();
    store.activate_agent_access_grant(&claude_effect).unwrap();
    assert_eq!(
        store
            .inspect_agent_access_grant(OWNER_SCOPE, codex_id)
            .unwrap(),
        Some(codex_before.clone())
    );
    assert_eq!(
        store
            .resolve_agent_access_grant(&codex_before)
            .unwrap()
            .expose(),
        codex_token.expose()
    );
}

#[test]
fn staged_grant_is_recovered_after_reopen_without_becoming_active_early() {
    let (directory, store) = store();
    let root = directory.path().join("data");
    let mutation = ensure(scope(b"recovery-plan-grant"), 0);
    let operation = operation_id('6');
    let effect = store
        .apply_agent_access_grant(&operation, &mutation, None)
        .unwrap();
    let expected = staged_ref(&effect, &mutation);
    drop(store);

    let reopened = LocalSecretStore::open(
        &crate::test_storage_authority(),
        root.join("secrets.db"),
        root.join("master-key"),
        root.join("backups"),
    )
    .unwrap();
    let recovered_effect = match reopened
        .observe_agent_access_grant(&operation, &mutation)
        .unwrap()
    {
        EffectReconciliation::Staged(effect) => effect,
        other => panic!("expected staged grant recovery, got {other:?}"),
    };
    assert_eq!(recovered_effect, effect);
    assert!(
        reopened
            .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
            .unwrap()
            .is_none()
    );
    reopened
        .activate_agent_access_grant(&recovered_effect)
        .unwrap();
    assert_eq!(
        reopened
            .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
            .unwrap()
            .unwrap(),
        expected
    );
}

#[test]
fn staged_grant_compensation_removes_the_unpublished_version() {
    let (_directory, store) = store();
    let mutation = ensure(scope(b"staged-rollback"), 0);
    let operation = operation_id('7');
    let effect = store
        .apply_agent_access_grant(&operation, &mutation, None)
        .unwrap();

    assert_eq!(
        store.compensate_agent_access_grant(&effect).unwrap(),
        CompensationOutcome::Compensated
    );
    assert!(matches!(
        store
            .observe_agent_access_grant(&operation, &mutation)
            .unwrap(),
        EffectReconciliation::Missing
    ));
    assert!(
        store
            .inspect_agent_access_grant(OWNER_SCOPE, CONNECTION_ID)
            .unwrap()
            .is_none()
    );
    store.with_connection(|connection| {
        let count: u64 = connection
            .query_row(
                "SELECT count(*) FROM agent_access_grant_versions WHERE connection_id = ?1",
                [CONNECTION_ID],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    });
}

#[test]
fn prepared_material_is_bound_to_the_original_uncompensated_operation_and_scope() {
    let (_directory, store) = store();
    let mutation = ensure(scope(b"confirmed"), 0);
    let op = operation_id('d');
    assert!(
        store
            .resolve_prepared_agent_access_grant(&op, &mutation)
            .is_err()
    );
    let effect = store
        .apply_agent_access_grant(&op, &mutation, None)
        .unwrap();
    let material = store
        .resolve_prepared_agent_access_grant(&op, &mutation)
        .unwrap();
    assert!(
        store
            .resolve_agent_access_grant(&staged_ref(&effect, &mutation))
            .is_err()
    );
    assert!(
        store
            .resolve_prepared_agent_access_grant(&operation_id('e'), &mutation)
            .is_err()
    );
    assert!(
        store
            .resolve_prepared_agent_access_grant(&op, &ensure(scope(b"other"), 0))
            .is_err()
    );
    assert!(
        store
            .resolve_prepared_agent_access_grant(&op, &ensure(scope(b"confirmed"), 1))
            .is_err()
    );
    store.activate_agent_access_grant(&effect).unwrap();
    assert_eq!(
        store
            .resolve_prepared_agent_access_grant(&op, &mutation)
            .unwrap()
            .expose(),
        material.expose()
    );
    store.compensate_agent_access_grant(&effect).unwrap();
    assert!(
        store
            .resolve_prepared_agent_access_grant(&op, &mutation)
            .is_err()
    );
}
