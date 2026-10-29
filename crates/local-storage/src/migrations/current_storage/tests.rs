use super::*;
use crate::LocalStorageSet;

fn history(stores: &LocalStorageSet, count: u64) {
    let mut publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../e2e/product/fixtures/routing/current-publication.v3.json"
    )))
    .unwrap();
    publication.grants.clear();
    publication.aliases.clear();
    for revision in 1..=count {
        publication.publication_revision = GatewayPublicationRevision::new(revision).unwrap();
        let record =
            PublicationRecordV1::from_publication(WorkspaceId::default(), &publication).unwrap();
        stores
            .control()
            .prepare_publication(
                &record,
                (revision > 1).then(|| GatewayPublicationRevision::new(revision - 1).unwrap()),
            )
            .unwrap();
        stores
            .control()
            .mark_publication_active(
                &WorkspaceId::default(),
                record.publication_revision,
                &record.digest,
            )
            .unwrap();
    }
}

#[test]
fn retained_publications_are_visited_one_at_a_time_including_history() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    history(&stores, 256);
    let mut visited = std::collections::BTreeSet::new();
    visit_publications(&stores.control.connection.borrow(), |publication| {
        // Only scalar identities survive the visitor; decoded publications are borrowed.
        assert!(visited.insert(publication.publication_revision.get()));
    })
    .unwrap();
    assert_eq!(visited, (1..=256).collect());
    validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).unwrap();
    stores.validate_recovered_grant_publications().unwrap();
}

#[test]
fn unrelated_historical_damage_is_isolated_but_explicit_audit_rejects_it() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    history(&stores, 3);
    // Keep active and LKG records intact; damage only retained history.
    stores.control.connection.borrow().execute(
        "UPDATE gateway_publications SET digest='corrupted-history' WHERE publication_revision=1",
        [],
    ).unwrap();
    validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).unwrap();
    stores.validate_recovered_grant_publications().unwrap();
    assert!(audit_current_storage(stores.control(), stores.runtime(), stores.secrets()).is_err());
    assert!(visit_publications(&stores.control.connection.borrow(), |_| {}).is_err());
    drop(stores);
    LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
}

#[test]
fn completed_upgrade_reopen_isolates_history_and_retired_backup_damage() {
    use crate::migrations::worker_dependencies_v24_tests::{cases, schema23_source};
    for damage in ["history", "backup"] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        schema23_source(&root, &cases()[0]);
        let stores = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
        let backup = crate::upgrade_backup_root(&root).unwrap();
        assert_eq!(
            crate::BackupSet::open(&crate::test_storage_authority(), &backup)
                .unwrap()
                .phase(),
            crate::backup::BackupSetPhase::Completed
        );
        history(&stores, 3);
        if damage == "history" {
            stores.control.connection.borrow().execute(
                "UPDATE gateway_publications SET digest='corrupted-history' WHERE publication_revision=1", [],
            ).unwrap();
        } else {
            std::fs::write(backup.join("control.db"), b"damaged retired backup").unwrap();
        }
        drop(stores);
        let reopened = LocalStorageSet::open_for_daemon_startup(&root)
            .unwrap_or_else(|e| panic!("{damage}: {e}"));
        if damage == "history" {
            assert!(
                audit_current_storage(reopened.control(), reopened.runtime(), reopened.secrets())
                    .is_err()
            );
        } else {
            assert!(crate::BackupSet::open(&crate::test_storage_authority(), &backup).is_err());
            assert_eq!(
                std::fs::read(backup.join("control.db")).unwrap(),
                b"damaged retired backup"
            );
        }
        reopened.validate_recovered_grant_publications().unwrap();
    }
}

#[test]
fn recovered_grants_match_exact_material_and_generation_in_retained_history() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    history(&stores, 3);
    let mut publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../e2e/product/fixtures/routing/current-publication.v3.json"
    )))
    .unwrap();
    // Project the frozen facts with this test build's current compiler before issuing a grant.
    publication = publication.into_current().unwrap();
    let scope = AgentAccessGrantScopeV1::new(
        "agent-connection/history-match",
        publication.grants[0].model_grant.clone(),
    )
    .unwrap();
    let mutation = AgentAccessGrantMutationV1::ensure("principal/local-owner", scope, 0).unwrap();
    let operation = OperationId::parse("op_0123456789abcdef0123456789abcdef").unwrap();
    let effect = stores
        .secrets()
        .apply_agent_access_grant(&operation, &mutation, None)
        .unwrap();
    let reference = AgentAccessGrantRefV1::from_ensure_effect(&effect, &mutation).unwrap();
    stores
        .secrets()
        .activate_agent_access_grant(&effect)
        .unwrap();
    // Retain the other fixture grants so its executable alias projection stays closed.
    publication.grants[0].grant_id = reference.grant_id().into();
    publication.grants[0].generation = reference.generation();
    publication.grants[0].bearer_token_sha256 = reference.material_sha256().clone();
    publication
        .grants
        .sort_by(|a, b| a.grant_id.cmp(&b.grant_id));
    let grant_index = publication
        .grants
        .iter()
        .position(|grant| grant.grant_id == reference.grant_id())
        .unwrap();
    for revision in 1..=3 {
        publication.publication_revision = GatewayPublicationRevision::new(revision).unwrap();
        if revision == 2 {
            publication.grants[grant_index].bearer_token_sha256 =
                CanonicalDigest::of_bytes(b"different bearer");
        } else if revision == 3 {
            publication.grants[grant_index].bearer_token_sha256 =
                reference.material_sha256().clone();
            publication.grants[grant_index].generation += 1;
        }
        let record =
            PublicationRecordV1::from_publication(WorkspaceId::default(), &publication).unwrap();
        // Inject individually valid records with deliberately inconsistent cross-store grants.
        // Normal publication transitions correctly reject changing a generation's bearer.
        stores.control.connection.borrow().execute(
            "UPDATE gateway_publications SET digest=?1, publication_bytes=?2 WHERE publication_revision=?3",
            rusqlite::params![record.digest.as_str(), record.bytes.as_slice(), revision],
        ).unwrap();
        // Build the reference from the individually valid fixture record; lookup must
        // still authenticate its exact bytes, generation, material and scope later.
        crate::control::current_dependencies::record_publication_proofs(
            &stores.control.connection.borrow(),
            &record.verify().unwrap(),
        )
        .unwrap();
    }
    stores.validate_recovered_grant_publications().unwrap();
    stores
        .control
        .connection
        .borrow()
        .execute(
            "DELETE FROM gateway_publications WHERE publication_revision=1",
            [],
        )
        .unwrap();
    // Different material or generation never satisfies the active Secret's exact reference.
    assert!(stores.validate_recovered_grant_publications().is_err());
    // This state can be a saga checkpoint: opening admits recovery, not serving traffic.
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert!(stores.validate_recovered_grant_publications().is_err());
}

#[test]
fn ordinary_startup_visits_current_publications_independent_of_history_size() {
    for count in [10, 100, 1000] {
        let root = crate::test_tempdir().unwrap();
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        history(&stores, 3);
        {
            let mut db = stores.control.connection.borrow_mut();
            let tx = db.transaction().unwrap();
            for revision in 4..count + 4 {
                let publication = GatewayPublicationV1::new(
                    WorkspaceId::default(),
                    GatewayPublicationRevision::new(revision).unwrap(),
                    AliasRegistryV1::default(),
                    vec![],
                )
                .unwrap();
                let record =
                    PublicationRecordV1::from_publication(WorkspaceId::default(), &publication)
                        .unwrap();
                // Historical fixtures are valid immutable records, not current roots.
                tx.execute("INSERT INTO gateway_publications(workspace_id,publication_revision,digest,publication_bytes,state,created_at,updated_at) VALUES(?1,?2,?3,?4,'historical',1,1)",
                    params![record.workspace_id.as_str(),revision,record.digest.as_str(),record.bytes.as_slice()]).unwrap();
            }
            tx.commit().unwrap();
        }
        dependencies::PUBLICATION_READS.with(|v| v.set(0));
        validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).unwrap();
        assert_eq!(
            dependencies::PUBLICATION_READS.with(|v| v.get()),
            2,
            "history={count}"
        );
    }
}

#[test]
fn active_and_lkg_damage_still_blocks_ordinary_startup() {
    for revision in [2, 3] {
        let root = crate::test_tempdir().unwrap();
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        history(&stores, 3);
        stores
            .control
            .connection
            .borrow()
            .execute(
                "UPDATE gateway_publications SET digest='corrupt' WHERE publication_revision=?1",
                [revision],
            )
            .unwrap();
        assert!(
            validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).is_err()
        );
        drop(stores);
        assert!(LocalStorageSet::open_for_daemon_startup(root.path()).is_err());
    }
}

#[test]
fn historical_plan_bytes_become_strict_when_current_or_held() {
    for role in ["unreferenced", "head", "hold", "prepared-owner-missing"] {
        let root = crate::test_tempdir().unwrap();
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        let publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../e2e/product/fixtures/routing/current-publication.v3.json"
        )))
        .unwrap();
        let compiled = publication.into_current().unwrap().plans.remove(0);
        let version =
            PlanVersionV1::from_unversioned_compiled_recovery(WorkspaceId::default(), compiled)
                .unwrap();
        let r = &version.reference;
        let db = stores.control.connection.borrow();
        db.execute(
            "INSERT INTO plan_versions VALUES(?1,?2,?3,?4,?5,?6,'isolated-fixture')",
            params![
                r.workspace_id.as_str(),
                r.plan_id.as_str(),
                r.content_revision,
                r.content_digest.as_str(),
                serde_json::to_string(&version).unwrap(),
                if role == "prepared-owner-missing" {
                    "prepared"
                } else {
                    "published"
                }
            ],
        )
        .unwrap();
        if role == "head" {
            let head = PlanHeadV1 {
                head_revision: r.content_revision,
                reference: r.clone(),
                model_alias: version.compiled.model_alias().clone(),
                status: PlanLifecycleV1::Enabled,
            };
            db.execute(
                "INSERT INTO plan_heads VALUES(?1,?2,?3,?4)",
                params![
                    r.workspace_id.as_str(),
                    r.plan_id.as_str(),
                    head.head_revision,
                    serde_json::to_string(&head).unwrap()
                ],
            )
            .unwrap();
        } else if role == "hold" {
            db.execute(
                "INSERT INTO plan_version_holds VALUES(?1,'retained-worker',?2,?3,?4,0,1)",
                params![
                    r.workspace_id.as_str(),
                    r.plan_id.as_str(),
                    r.content_revision,
                    r.content_digest.as_str()
                ],
            )
            .unwrap();
        }
        drop(db);
        if role == "prepared-owner-missing" {
            assert!(
                validate_current_storage(stores.control(), stores.runtime(), stores.secrets())
                    .is_err()
            );
            continue;
        }
        validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).unwrap();
        stores
            .control
            .connection
            .borrow()
            .execute("UPDATE plan_versions SET version_json='{}'", [])
            .unwrap();
        let result = validate_current_storage(stores.control(), stores.runtime(), stores.secrets());
        assert_eq!(result.is_ok(), role == "unreferenced", "{role}");
        if role == "unreferenced" {
            assert!(
                audit_current_storage(stores.control(), stores.runtime(), stores.secrets())
                    .is_err()
            );
        } else {
            // Missing bytes are also a failure, rather than an empty JOIN result.
            let external = Connection::open(root.path().join("live/control.db")).unwrap();
            // Deliberately inject a broken reference in this disposable damage fixture.
            external.pragma_update(None, "foreign_keys", false).unwrap();
            external.execute("DELETE FROM plan_versions", []).unwrap();
            assert!(
                validate_current_storage(stores.control(), stores.runtime(), stores.secrets())
                    .is_err(),
                "{role}"
            );
        }
    }
}

/// A synthetic retained source is produced by the real migration and writers at
/// its requested old target. This is not a claim of exact released-producer input.
fn schema32_source(root: &Path) -> LocalStorageSet {
    crate::prepare_owner_directory(root).unwrap();
    let live = root.join("live");
    crate::prepare_owner_directory(&live).unwrap();
    let backup = crate::upgrade_backup_root(root).unwrap();
    let authority = crate::test_storage_authority();
    let control = crate::ControlStore::open_from_migration_set(
        &authority,
        &live.join("control.db"),
        &backup,
        "00000000000000000000000000000001",
        32,
    )
    .unwrap();
    let runtime = crate::RuntimeStore::open_from_migration_set(
        &authority,
        &live.join("runtime.db"),
        &backup,
        "00000000000000000000000000000002",
        32,
    )
    .unwrap();
    let secrets = crate::LocalSecretStore::open_test_source_at_schema(
        &authority,
        &live.join("secrets.db"),
        &root.join("master-key"),
        &backup,
        32,
    )
    .unwrap();
    LocalStorageSet {
        control,
        runtime,
        secrets,
    }
}

#[test]
fn locator_upgrade_from_admitted_32_preserves_unrelated_publication_damage() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    let stores = schema32_source(&root);
    history(&stores, 3);
    stores.control.connection.borrow().execute(
        "UPDATE gateway_publications SET digest='corrupted-history' WHERE publication_revision=1", [],
    ).unwrap();
    validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).unwrap();
    assert!(audit_current_storage(stores.control(), stores.runtime(), stores.secrets()).is_err());
    let original: Vec<u8> = stores
        .control
        .connection
        .borrow()
        .query_row(
            "SELECT publication_bytes FROM gateway_publications WHERE publication_revision=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(stores);
    // The actual production three-store coordinator must own the upgrade and backup.
    let upgraded = LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    assert_eq!(
        database_version(&root.join("live/control.db")).unwrap(),
        Some(33)
    );
    assert_eq!(
        upgraded
            .control
            .connection
            .borrow()
            .query_row(
                "SELECT publication_bytes FROM gateway_publications WHERE publication_revision=1",
                [],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .unwrap(),
        original
    );
    assert!(
        audit_current_storage(upgraded.control(), upgraded.runtime(), upgraded.secrets()).is_err()
    );
    upgraded.validate_recovered_grant_publications().unwrap();
    drop(upgraded);
    LocalStorageSet::open_for_daemon_startup(&root).unwrap();
}

#[test]
fn locator_upgrade_resumes_each_store_commit_and_precompletion_from_verified_32() {
    use crate::backup::test_writer_barrier;

    // Zero means before Control; odd values mean a store committed before its
    // phase was recorded; even values mean the phase was durably recorded.
    // Six is after all three commits and immediately before final completion.
    for boundary in 0..=6 {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let stores = schema32_source(&root);
        history(&stores, 3);
        stores
            .control
            .connection
            .borrow()
            .execute(
                "UPDATE gateway_publications SET digest='corrupted-history' WHERE publication_revision=1",
                [],
            )
            .unwrap();
        validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).unwrap();
        assert!(
            audit_current_storage(stores.control(), stores.runtime(), stores.secrets()).is_err()
        );
        drop(stores);

        let paths = live_database_paths(&root.join("live"));
        let binding =
            crate::LocalSecretStore::migration_binding(&paths[2], &root.join("master-key"))
                .unwrap()
                .unwrap();
        let barrier = test_writer_barrier();
        let mut set = BackupSet::create_migration(
            &barrier,
            &crate::upgrade_backup_root(&root).unwrap(),
            &open_existing_for_snapshot(&paths[0]).unwrap(),
            &open_existing_for_snapshot(&paths[1]).unwrap(),
            &open_existing_for_snapshot(&paths[2]).unwrap(),
            &binding.key_id,
            33,
        )
        .unwrap();
        let set_id = set.set_id().to_owned();
        let backup_root = set.directory().to_owned();
        let original_backup = fs::read(backup_root.join("control.db")).unwrap();

        for (index, (kind, phase)) in [
            (DatabaseKind::Control, BackupSetPhase::ControlMigrated),
            (DatabaseKind::Runtime, BackupSetPhase::RuntimeMigrated),
            (DatabaseKind::Secrets, BackupSetPhase::SecretsMigrated),
        ]
        .into_iter()
        .enumerate()
        {
            if boundary < 2 * index + 1 {
                break;
            }
            let mut connection = Connection::open(&paths[index]).unwrap();
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            tx.execute_batch(migration_sql(kind, 33).unwrap()).unwrap();
            if kind == DatabaseKind::Control {
                agent_locator_v33::initialize(&tx).unwrap();
            }
            tx.pragma_update(None, "user_version", 33).unwrap();
            tx.execute(
                "INSERT INTO schema_migrations(version,applied_at,binary_version) VALUES(33,1,'interrupted-producer')",
                [],
            )
            .unwrap();
            tx.commit().unwrap();
            if boundary >= 2 * index + 2 {
                set.persist_phase(&barrier, phase).unwrap();
            }
        }
        drop(set);

        let upgraded = LocalStorageSet::open_for_daemon_startup(&root)
            .unwrap_or_else(|error| panic!("boundary {boundary}: {error:?}"));
        for path in &paths {
            assert_eq!(
                database_version(path).unwrap(),
                Some(33),
                "boundary {boundary}"
            );
        }
        assert!(
            audit_current_storage(upgraded.control(), upgraded.runtime(), upgraded.secrets())
                .is_err()
        );
        upgraded.validate_recovered_grant_publications().unwrap();
        drop(upgraded);
        let completed = BackupSet::open(&crate::test_storage_authority(), &backup_root).unwrap();
        assert_eq!(completed.set_id(), set_id, "boundary {boundary}");
        assert_eq!(
            completed.phase(),
            BackupSetPhase::Completed,
            "boundary {boundary}"
        );
        assert_eq!(
            fs::read(backup_root.join("control.db")).unwrap(),
            original_backup
        );
        LocalStorageSet::open_for_daemon_startup(&root).unwrap();
    }
}

#[test]
fn locator_upgrade_refuses_missing_original_recovery_coverage() {
    let directory = crate::test_tempdir().unwrap();
    let root = directory.path().join("storage");
    let stores = schema32_source(&root);
    stores
        .control
        .connection
        .borrow()
        .execute("DELETE FROM operation_recovery_coverage", [])
        .unwrap();
    drop(stores);
    assert!(LocalStorageSet::open_for_daemon_startup(&root).is_err());
    assert_eq!(
        database_version(&root.join("live/control.db")).unwrap(),
        Some(32)
    );
    let restored = Connection::open(root.join("live/control.db")).unwrap();
    assert_eq!(
        restored
            .query_row(
                "SELECT count(*) FROM operation_recovery_coverage",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn locator_upgrade_refuses_damaged_current_publication() {
    for role in ["active", "last-known-good", "prepared"] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let stores = schema32_source(&root);
        history(&stores, 3);
        if role == "prepared" {
            let mut publication: GatewayPublicationV1 =
                serde_json::from_slice(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../e2e/product/fixtures/routing/current-publication.v3.json"
                )))
                .unwrap();
            publication.grants.clear();
            publication.aliases.clear();
            publication.publication_revision = GatewayPublicationRevision::new(4).unwrap();
            let record =
                PublicationRecordV1::from_publication(WorkspaceId::default(), &publication)
                    .unwrap();
            stores
                .control()
                .prepare_publication(&record, Some(GatewayPublicationRevision::new(3).unwrap()))
                .unwrap();
        }
        let revision = match role {
            "active" => 3,
            "last-known-good" => 2,
            _ => 4,
        };
        stores.control.connection.borrow().execute(
            "UPDATE gateway_publications SET digest='corrupted-current' WHERE publication_revision=?1", [revision],
        ).unwrap();
        drop(stores);
        assert!(
            LocalStorageSet::open_for_daemon_startup(&root).is_err(),
            "{role}"
        );
        assert_eq!(
            database_version(&root.join("live/control.db")).unwrap(),
            Some(32)
        );
        let restored = Connection::open(root.join("live/control.db")).unwrap();
        assert_eq!(
            restored
                .query_row(
                    "SELECT digest FROM gateway_publications WHERE publication_revision=?1",
                    [revision],
                    |r| r.get::<_, String>(0),
                )
                .unwrap(),
            "corrupted-current",
            "{role}"
        );
    }
}

#[test]
fn locator_upgrade_distinguishes_unheld_plan_history_from_current_and_held_versions() {
    for role in ["unreferenced", "head", "hold"] {
        let directory = crate::test_tempdir().unwrap();
        let root = directory.path().join("storage");
        let stores = schema32_source(&root);
        let publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../e2e/product/fixtures/routing/current-publication.v3.json"
        )))
        .unwrap();
        let compiled = publication.into_current().unwrap().plans.remove(0);
        let version =
            PlanVersionV1::from_unversioned_compiled_recovery(WorkspaceId::default(), compiled)
                .unwrap();
        let r = &version.reference;
        let db = stores.control.connection.borrow();
        db.execute(
            "INSERT INTO plan_versions VALUES(?1,?2,?3,?4,?5,?6,'isolated-fixture')",
            params![
                r.workspace_id.as_str(),
                r.plan_id.as_str(),
                r.content_revision,
                r.content_digest.as_str(),
                serde_json::to_string(&version).unwrap(),
                "published"
            ],
        )
        .unwrap();
        if role == "head" {
            let head = PlanHeadV1 {
                head_revision: r.content_revision,
                reference: r.clone(),
                model_alias: version.compiled.model_alias().clone(),
                status: PlanLifecycleV1::Enabled,
            };
            db.execute(
                "INSERT INTO plan_heads VALUES(?1,?2,?3,?4)",
                params![
                    r.workspace_id.as_str(),
                    r.plan_id.as_str(),
                    head.head_revision,
                    serde_json::to_string(&head).unwrap()
                ],
            )
            .unwrap();
        } else if role == "hold" {
            db.execute(
                "INSERT INTO plan_version_holds VALUES(?1,'retained-worker',?2,?3,?4,0,1)",
                params![
                    r.workspace_id.as_str(),
                    r.plan_id.as_str(),
                    r.content_revision,
                    r.content_digest.as_str()
                ],
            )
            .unwrap();
        }
        drop(db);
        validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).unwrap();
        stores
            .control
            .connection
            .borrow()
            .execute("UPDATE plan_versions SET version_json='{}'", [])
            .unwrap();
        drop(stores);
        let result = LocalStorageSet::open_for_daemon_startup(&root);
        assert_eq!(result.is_ok(), role == "unreferenced", "{role}");
        if let Ok(upgraded) = result {
            assert!(
                audit_current_storage(upgraded.control(), upgraded.runtime(), upgraded.secrets())
                    .is_err()
            );
        } else {
            assert_eq!(
                database_version(&root.join("live/control.db")).unwrap(),
                Some(32)
            );
        }
    }
}
