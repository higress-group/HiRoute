use super::lifecycle::{edit, saved};
use super::*;
use hiroute_application_api::{ComputeManagementEditV1, NativeNetworkEditV1};
use hiroute_domain::{ComputeManagementRepositoryPort, NativeNetworkPolicyV1};
use rusqlite::params;

fn network(stores: &LocalStorageSet) -> hiroute_domain::NativeSourceNetworkRecordV1 {
    stores
        .control()
        .native_source_network(&saved(stores).source_id)
        .unwrap()
        .unwrap()
}

#[test]
fn network_policy_commits_with_rename_preserves_keys_and_survives_restart_and_old_edits() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let manual = NativeNetworkPolicyV1::Manual {
        proxy_url: "http://proxy.example:8080/".into(),
        no_proxy: "internal.example".into(),
    };
    {
        let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
        let before = saved(&stores);
        assert_eq!(network(&stores).policy, NativeNetworkPolicyV1::Direct);
        let registry = TrustedComputeCandidateRegistry::new();
        let input = ProtectedInput;
        let planner =
            ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
        let change = edit(
            &stores,
            &before,
            ComputeManagementEditV1::Rename {
                display_name: "Same source through proxy".into(),
            },
            &[],
        );
        let preview = planner
            .preview_network(
                serde_json::to_value(change).unwrap(),
                NativeNetworkEditV1 {
                    expected_network_revision: 1,
                    policy: manual.clone(),
                },
            )
            .unwrap();
        let frozen = hiroute_domain::StoredOperationInputV1::freeze(preview.plan()).unwrap();
        assert_eq!(frozen.build().unwrap().spec(), preview.plan().spec());
        assert!(
            preview
                .result
                .changes
                .iter()
                .any(|c| c.resource_kind == "native_network_policy")
        );
        let mut corrupt = preview.plan().control().clone();
        corrupt["compute_management_mutation"]["network"]["after"]["policy"] =
            serde_json::json!({"mode":"direct"});
        assert!(
            hiroute_domain::TransactionPlanV1::from_registered_typed_planner(
                preview.result.spec.clone(),
                corrupt,
                None,
                vec![],
                vec![],
                vec![]
            )
            .is_err()
        );
        let runtime = TransactionRuntime::default();
        let external = NoExternal;
        let coordinator = TransactionCoordinator::new(
            stores.control(),
            stores.secrets(),
            stores.runtime(),
            &external,
            &input,
            &runtime,
        );
        coordinator.reconcile_startup_and_open().unwrap();
        apply_preview(
            &stores,
            &planner,
            &coordinator,
            &WorkspaceId::default(),
            preview,
            "network-edit",
        );
        let after = saved(&stores);
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(after.source_id, before.source_id);
        assert_eq!(after.models, before.models);
        assert_eq!(after.credentials, before.credentials);
        assert_eq!(network(&stores).network_revision, 2);
        assert_eq!(network(&stores).policy, manual);
        let stale = planner.preview_network(
            serde_json::to_value(edit(
                &stores,
                &after,
                ComputeManagementEditV1::Rename {
                    display_name: after.display_name.clone(),
                },
                &[],
            ))
            .unwrap(),
            NativeNetworkEditV1 {
                expected_network_revision: 1,
                policy: NativeNetworkPolicyV1::Direct,
            },
        );
        assert!(stale.is_err());
        lifecycle::execute(
            &stores,
            &registry,
            edit(
                &stores,
                &after,
                ComputeManagementEditV1::Rename {
                    display_name: "Old caller rename".into(),
                },
                &[],
            ),
            "old-rename",
        );
        assert_eq!(network(&stores).policy, manual);
        assert_eq!(network(&stores).network_revision, 2);
    }
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    assert_eq!(network(&stores).policy, manual);
    assert_eq!(network(&stores).network_revision, 2);
}

#[test]
fn missing_current_policy_is_refused_and_source_deletion_cascades_its_policy() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    {
        let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
        let source = saved(&stores);
        stores
            .control()
            .connection
            .borrow()
            .execute(
                "DELETE FROM native_source_network WHERE source_id=?1",
                params![source.source_id],
            )
            .unwrap();
        assert!(
            stores
                .control()
                .native_source_network(&source.source_id)
                .is_err()
        );
    }
    assert!(LocalStorageSet::open_for_daemon_startup(directory.path()).is_err());
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let source = saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new();
    lifecycle::execute(
        &stores,
        &registry,
        edit(&stores, &source, ComputeManagementEditV1::Delete, &[]),
        "delete-network-owner",
    );
    assert!(
        stores
            .control()
            .native_source_network(&source.source_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn schema34_source_initializes_direct_once_through_production_startup() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let (identity, keys, models) = {
        let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
        let source = saved(&stores);
        (source.source_id, source.credentials, source.models)
    };
    // This models the immediately preceding production SQL format; retained source payloads
    // themselves are not rewritten or regenerated by the migration.
    for database in ["control", "runtime", "secrets"] {
        let mut connection =
            rusqlite::Connection::open(directory.path().join(format!("live/{database}.db")))
                .unwrap();
        if database == "control" {
            let transaction = connection.transaction().unwrap();
            transaction.execute_batch("DROP TABLE native_source_network; ALTER TABLE compute_management_effects DROP COLUMN network_before; ALTER TABLE compute_management_effects DROP COLUMN network_after;").unwrap();
            // Model the trusted preceding schema, including its verified locator cookie.
            let schema: i64 = transaction
                .pragma_query_value(None, "schema_version", |row| row.get(0))
                .unwrap();
            transaction.execute("UPDATE agent_locator_coverage SET schema_version=?1 WHERE singleton=1 AND tampered=0", [schema]).unwrap();
            crate::control::agent_locator::validate_coverage(&transaction).unwrap();
            transaction.commit().unwrap();
        }
        connection.pragma_update(None, "user_version", 34).unwrap();
    }
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let source = saved(&stores);
    assert_eq!(source.source_id, identity);
    assert_eq!(source.credentials, keys);
    assert_eq!(source.models, models);
    assert_eq!(network(&stores).policy, NativeNetworkPolicyV1::Direct);
    assert_eq!(network(&stores).network_revision, 1);
}

#[test]
fn network_policy_activation_failure_rolls_back_the_source_and_survives_restart() {
    let root = tempdir().unwrap();
    populate_saved_source(root.path(), &WorkspaceId::default());
    let before;
    let before_network;
    {
        let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
        before = saved(&stores);
        before_network = network(&stores);
        let registry = TrustedComputeCandidateRegistry::new();
        let inputs = ProtectedInput;
        let planner =
            ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &inputs);
        let runtime = TransactionRuntime::default();
        let external = NoExternal;
        let coordinator = TransactionCoordinator::new(
            stores.control(),
            stores.secrets(),
            stores.runtime(),
            &external,
            &inputs,
            &runtime,
        );
        coordinator.reconcile_startup_and_open().unwrap();
        let preview = planner
            .preview_network(
                serde_json::to_value(edit(
                    &stores,
                    &before,
                    ComputeManagementEditV1::Rename {
                        display_name: "Atomic network rename".into(),
                    },
                    &[],
                ))
                .unwrap(),
                NativeNetworkEditV1 {
                    expected_network_revision: before_network.network_revision,
                    policy: NativeNetworkPolicyV1::Manual {
                        proxy_url: "http://proxy.invalid:8080".into(),
                        no_proxy: String::new(),
                    },
                },
            )
            .unwrap();
        let prepared = planner
            .prepare_apply(ComputeConnectionApplyRequestV1 {
                spec: preview.result.spec,
                accept_digest: preview.result.accept_digest,
                expected_revisions: preview.result.expected_revisions,
                idempotency_key: "network-activation-failure".into(),
            })
            .unwrap();
        let accepted = coordinator
            .accept_prepared(
                &WorkspaceId::default(),
                &VerifiedPrincipal::for_local_control(),
                prepared,
            )
            .unwrap();
        stores.control().with_connection(|db| db.execute_batch(
            "CREATE TEMP TRIGGER reject_network_activation BEFORE UPDATE ON main.native_source_network BEGIN SELECT RAISE(ABORT, 'fixture_network_activation'); END;"
        ).unwrap());
        let result = coordinator.run(&accepted.operation().operation_id).unwrap();
        assert_eq!(result.state, OperationState::RolledBack);
        assert_eq!(saved(&stores), before);
        assert_eq!(network(&stores), before_network);
    }
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert_eq!(saved(&stores), before);
    assert_eq!(network(&stores), before_network);
}
