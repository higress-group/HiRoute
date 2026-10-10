use super::*;

fn reference(generation: u64) -> CredentialRefV1 {
    CredentialRefV1::new(
        "delete-key",
        "delete-owner",
        "hirouted",
        "provider-auth",
        ["provider-api".to_owned()],
        generation,
    )
    .unwrap()
}

fn operation(value: char) -> OperationId {
    OperationId::parse(format!("op_{}", value.to_string().repeat(32))).unwrap()
}

fn open(root: &Path) -> Result<LocalSecretStore, LocalStorageError> {
    LocalSecretStore::open(
        &crate::test_storage_authority(),
        root.join("secrets.db"),
        root.join("master-key"),
        root.join("backups"),
    )
}

#[test]
fn compute_delete_secret_journal_reopens_when_staged_activated_or_compensated() {
    for phase in 0..3 {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("data");
        let store = open(&root).unwrap();
        let secret = ProtectedSecret::new(b"delete-recovery-fixture".to_vec()).unwrap();
        let save = SecretMutationV1::upsert(
            reference(0),
            0,
            "input",
            Some(store.fingerprint(&secret).unwrap()),
        )
        .unwrap();
        let saved = store
            .apply_secret(&operation('1'), &save, Some(&secret))
            .unwrap();
        store.activate_secret(&saved).unwrap();
        let delete = SecretMutationV1::delete(reference(1), 1).unwrap();
        let effect = store.apply_secret(&operation('2'), &delete, None).unwrap();
        if phase > 0 {
            store.activate_secret(&effect).unwrap();
        }
        if phase == 2 {
            store.compensate_secret(&effect).unwrap();
        }
        drop(store);
        let reopened = open(&root).expect("metadata-only delete journal must reopen");
        let observed = reopened.observe_secret(&operation('2'), &delete).unwrap();
        match phase {
            0 => assert!(matches!(observed, EffectReconciliation::Staged(_))),
            1 => assert!(matches!(observed, EffectReconciliation::Applied(_))),
            _ => assert!(matches!(observed, EffectReconciliation::Missing)),
        }
        assert_eq!(
            read_entry(&reopened.connection.borrow(), "delete-key")
                .unwrap()
                .is_some(),
            phase != 1
        );
        if phase == 2 {
            let restored = reopened
                .compensated_secret_reference(&operation('2'), &delete)
                .unwrap()
                .unwrap();
            assert_eq!(restored, reference(3));
            let replacement = SecretMutationV1::upsert(
                restored,
                3,
                "input",
                Some(reopened.fingerprint(&secret).unwrap()),
            )
            .unwrap();
            let newer = reopened
                .apply_secret(&operation('4'), &replacement, Some(&secret))
                .unwrap();
            reopened.activate_secret(&newer).unwrap();
            assert!(
                reopened
                    .compensated_secret_reference(&operation('2'), &delete)
                    .is_err(),
                "same value under a newer owner must not prove old compensation"
            );
            assert_eq!(reopened.generation(&reference(4)).unwrap(), 4);
        }
    }
}

#[test]
fn compute_delete_secret_journal_with_unexpected_or_corrupt_ciphertext_stays_locked() {
    for column in [
        "staged_ciphertext",
        "staged_nonce",
        "before_ciphertext",
        "before_nonce",
    ] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("data");
        let store = open(&root).unwrap();
        let secret = ProtectedSecret::new(b"delete-authentication-fixture".to_vec()).unwrap();
        let save = SecretMutationV1::upsert(
            reference(0),
            0,
            "input",
            Some(store.fingerprint(&secret).unwrap()),
        )
        .unwrap();
        let saved = store
            .apply_secret(&operation('1'), &save, Some(&secret))
            .unwrap();
        store.activate_secret(&saved).unwrap();
        let delete = SecretMutationV1::delete(reference(1), 1).unwrap();
        store.apply_secret(&operation('3'), &delete, None).unwrap();
        store
            .connection
            .borrow()
            .execute(
                &format!("UPDATE secret_effects SET {column}=X'00' WHERE operation_id=?1"),
                [operation('3').as_str()],
            )
            .unwrap();
        drop(store);
        assert!(matches!(open(&root), Err(LocalStorageError::Locked)));
    }
}
