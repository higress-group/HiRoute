use super::lifecycle::{edit, saved};
use super::*;
use hiroute_domain::{
    ComputeManagementRepositoryPort, ModelDefaultFieldV1, ModelDefaultPreferenceV1,
};

#[test]
fn model_defaults_commit_with_source_cas_preserve_original_source_proofs_and_restart() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let source_id;
    let binding;
    {
        let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
        let before = saved(&stores);
        source_id = before.source_id.clone();
        binding = before.models[0].binding_id.clone();
        let registry = TrustedComputeCandidateRegistry::new();
        let input = ProtectedInput;
        let planner =
            ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
        let preferences = std::collections::BTreeMap::from([(
            binding.clone(),
            std::collections::BTreeMap::from([(
                ModelDefaultFieldV1::Vision,
                ModelDefaultPreferenceV1::PreserveSaved,
            )]),
        )]);
        let preview = planner
            .preview_v3(hiroute_application_api::ComputeManagementChangeV3 {
                schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
                change: edit(
                    &stores,
                    &before,
                    hiroute_application_api::ComputeManagementEditV1::Rename {
                        display_name: before.display_name.clone(),
                    },
                    &[],
                ),
                default_preferences: preferences.clone(),
            })
            .unwrap();
        // Original V2 proof stays byte-identical inside the authenticated V3 wrapper.
        let control = preview.plan().control();
        let mutation: hiroute_domain::ComputeManagementMutationV3 =
            serde_json::from_value(control["compute_management_mutation"].clone()).unwrap();
        mutation.validate(&preview.result.spec).unwrap();
        assert_eq!(mutation.source_mutation.expected(), Some(&before));
        let encoded = serde_json::to_value(preview.plan()).unwrap();
        let mut corrupt = encoded.clone();
        corrupt["control"]["compute_management_mutation"]["preferences_after"] =
            serde_json::json!({});
        let corrupt: hiroute_domain::ComputeManagementMutationV3 =
            serde_json::from_value(corrupt["control"]["compute_management_mutation"].clone())
                .unwrap();
        assert!(corrupt.validate(&preview.result.spec).is_err());
        hiroute_domain::TransactionPlanV1::from_registered_typed_planner(
            preview.result.spec.clone(),
            encoded["control"].clone(),
            None,
            preview.plan().secrets().to_vec(),
            vec![],
            vec![],
        )
        .unwrap();
        let stale_request = ComputeConnectionApplyRequestV1 {
            spec: preview.result.spec.clone(),
            accept_digest: preview.result.accept_digest.clone(),
            expected_revisions: preview.result.expected_revisions.clone(),
            idempotency_key: "stale-defaults".into(),
        };
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
            "save-defaults",
        );
        assert_eq!(
            stores
                .control()
                .source_default_preferences(&source_id)
                .unwrap(),
            preferences
        );
        let after = saved(&stores);
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(after.models, before.models);
        assert!(planner.prepare_apply(stale_request).is_err());
    }
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    assert_eq!(
        stores
            .control()
            .source_default_preferences(&source_id)
            .unwrap()[&binding][&ModelDefaultFieldV1::Vision],
        ModelDefaultPreferenceV1::PreserveSaved
    );
    let before = saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new();
    lifecycle::execute(
        &stores,
        &registry,
        edit(
            &stores,
            &before,
            hiroute_application_api::ComputeManagementEditV1::Rename {
                display_name: "Retained default intent".into(),
            },
            &[],
        ),
        "old-client-rename",
    );
    assert!(
        stores
            .control()
            .source_default_preferences(&source_id)
            .unwrap()
            .contains_key(&binding)
    );
}

/// Normal V3 save used by rollback and backup scenarios, with no direct fixture DB writes.
pub(super) fn save_preserved_defaults(
    stores: &LocalStorageSet,
) -> hiroute_domain::SourceDefaultPreferencesV1 {
    let before = saved(stores);
    let registry = TrustedComputeCandidateRegistry::new();
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    let preferences = std::collections::BTreeMap::from([(
        before.models[0].binding_id.clone(),
        std::collections::BTreeMap::from([(
            ModelDefaultFieldV1::Vision,
            ModelDefaultPreferenceV1::PreserveSaved,
        )]),
    )]);
    let preview = planner
        .preview_v3(hiroute_application_api::ComputeManagementChangeV3 {
            schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
            change: edit(
                stores,
                &before,
                hiroute_application_api::ComputeManagementEditV1::Rename {
                    display_name: before.display_name.clone(),
                },
                &[],
            ),
            default_preferences: preferences.clone(),
        })
        .unwrap();
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
        stores,
        &planner,
        &coordinator,
        &WorkspaceId::default(),
        preview,
        "seed-preserved-defaults",
    );
    preferences
}

#[test]
fn model_defaults_backup_restores_preferences_and_authenticated_management_effects() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let preferences = save_preserved_defaults(&stores);
    let source = saved(&stores);
    let backup_path = directory.path().join("backup/control.db");
    let restored_path = directory.path().join("restored/control.db");
    let backup = stores.control().with_connection(|connection| {
        crate::backup::SqliteBackup::create(
            &crate::test_storage_authority(),
            connection,
            &backup_path,
        )
        .unwrap()
    });
    backup.restore_to(&restored_path).unwrap();
    let restored = rusqlite::Connection::open(&restored_path).unwrap();
    let content: String = restored
        .query_row(
            "SELECT preferences_json FROM compute_source_defaults WHERE source_id=?1",
            [&source.source_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<hiroute_domain::SourceDefaultPreferencesV1>(&content).unwrap(),
        preferences
    );
    let receipts: i64 = restored
        .query_row(
            "SELECT COUNT(*) FROM compute_management_effects WHERE preferences_after IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(receipts > 0);
    let foreign_keys: i64 = restored
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(foreign_keys, 0);
}
