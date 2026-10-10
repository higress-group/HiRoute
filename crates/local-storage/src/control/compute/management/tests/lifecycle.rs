use super::*;
use hiroute_application_api::ComputeManagementEditV1;
use hiroute_domain::{ComputeManagementRepositoryPort, ComputeManagementSourceV2};

pub(super) fn saved(stores: &LocalStorageSet) -> ComputeManagementSourceV2 {
    stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap()
        .sources
        .remove(0)
}

pub(super) fn edit(
    stores: &LocalStorageSet,
    source: &ComputeManagementSourceV2,
    action: ComputeManagementEditV1,
    selected: &[&str],
) -> ComputeManagementChangeV2 {
    ComputeManagementChangeV2 {
        schema: "hiroute.compute-management-change/v2".into(),
        subject: ComputeManagementSubjectV2::SavedSource {
            source_id: source.source_id.clone(),
        },
        expected_revisions: stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
            .revisions,
        selected_model_refs: selected.iter().map(|value| (*value).into()).collect(),
        intent: ComputeManagementIntentV2::SaveReady,
        key_edits: Vec::new(),
        validation: None,
        edit: Some(action),
    }
}

pub(super) fn execute(
    stores: &LocalStorageSet,
    registry: &TrustedComputeCandidateRegistry,
    change: ComputeManagementChangeV2,
    key: &str,
) {
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(registry, stores.control(), stores.secrets(), &input);
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
    let preview = planner
        .preview(change)
        .unwrap_or_else(|error| panic!("{key} preview failed: {error:?}"));
    apply_preview(
        stores,
        &planner,
        &coordinator,
        &WorkspaceId::default(),
        preview,
        key,
    );
}

#[test]
fn rename_remove_delete_preserve_binding_and_clean_only_owned_credentials_after_restart() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let before = saved(&stores);
    execute(
        &stores,
        &registry,
        edit(
            &stores,
            &before,
            ComputeManagementEditV1::Rename {
                display_name: "Bailian · team".into(),
            },
            &[],
        ),
        "rename",
    );
    let named = saved(&stores);
    assert_eq!(named.display_name, "Bailian · team");
    assert_eq!(named.models, before.models);
    assert_eq!(named.credentials, before.credentials);
    assert_eq!(named.source_id, before.source_id);
    execute(
        &stores,
        &registry,
        edit(
            &stores,
            &named,
            ComputeManagementEditV1::RemoveModels,
            &[&named.models[0].model_ref],
        ),
        "remove",
    );
    let kept = saved(&stores);
    assert_eq!(kept.models, vec![named.models[1].clone()]);
    assert_eq!(kept.credentials, before.credentials);
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    assert!(
        planner
            .preview(edit(
                &stores,
                &kept,
                ComputeManagementEditV1::RemoveModels,
                &[&kept.models[0].model_ref]
            ))
            .is_err(),
        "last model requires explicit connection deletion"
    );
    execute(
        &stores,
        &registry,
        edit(&stores, &kept, ComputeManagementEditV1::Delete, &[]),
        "delete",
    );
    assert!(
        stores
            .control()
            .compute_management_snapshot(&WorkspaceId::default())
            .unwrap()
            .sources
            .is_empty()
    );
    for key in &kept.credentials {
        assert_owned_secret_deleted(&stores, &key.credential);
    }
    drop(stores);
    let reopened = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    assert!(
        reopened
            .control()
            .compute_management_source(&before.source_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn explicit_append_preserves_disabled_source_old_model_facts_and_key() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let original = saved(&stores);
    let mut disable = edit(&stores, &original, ComputeManagementEditV1::Delete, &[]);
    disable.edit = None;
    disable.intent = ComputeManagementIntentV2::SaveDisabled;
    disable.selected_model_refs = original
        .models
        .iter()
        .map(|model| model.model_ref.clone())
        .collect();
    execute(&stores, &registry, disable, "disable");
    let current = saved(&stores);
    let mut facts = candidate("candidate/append", "slot/primary");
    facts.existing_source_id = Some(current.source_id.clone());
    let mut extra = facts.models[0].clone();
    extra.model_ref = "model/third".into();
    extra.upstream_model_id = "third-upstream".into();
    extra.display_name = "Third model".into();
    facts.models = vec![extra];
    registry.register_compute_candidate(facts.clone()).unwrap();
    let mut change = edit(
        &stores,
        &current,
        ComputeManagementEditV1::AppendModels,
        &["model/third"],
    );
    change.subject = ComputeManagementSubjectV2::Candidate {
        candidate: facts.candidate.clone(),
    };
    execute(&stores, &registry, change, "append");
    let added = saved(&stores);
    assert_eq!(added.state, MaterializationState::Disabled);
    assert_eq!(&added.models[..current.models.len()], &current.models);
    assert_eq!(added.models.len(), current.models.len() + 1);
    assert_eq!(added.credentials, current.credentials);
    assert_eq!(added.display_name, current.display_name);
    assert_eq!(added.source_id, current.source_id);
}

#[test]
fn stale_saved_edit_cannot_recreate_deleted_connection() {
    let directory = tempdir().unwrap();
    populate_saved_source(directory.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(directory.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    let source = saved(&stores);
    let old = edit(
        &stores,
        &source,
        ComputeManagementEditV1::Rename {
            display_name: "Old page".into(),
        },
        &[],
    );
    execute(
        &stores,
        &registry,
        edit(&stores, &source, ComputeManagementEditV1::Delete, &[]),
        "delete",
    );
    let input = ProtectedInput;
    let planner =
        ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    assert!(planner.preview(old.clone()).is_err());
    let mut refreshed = old;
    refreshed.expected_revisions = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap()
        .revisions;
    assert!(planner.preview(refreshed).is_err());
}

#[test]
fn two_independent_connections_with_same_key_keep_distinct_owners_after_one_is_deleted() {
    let root = tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let registry = TrustedComputeCandidateRegistry::new();
    for number in 1..=2 {
        let mut facts = candidate(&format!("candidate/independent-{number}"), "slot/primary");
        facts.lineage_ref = format!("lineage/independent-{number}");
        registry.register_compute_candidate(facts.clone()).unwrap();
        execute(
            &stores,
            &registry,
            ComputeManagementChangeV2 {
                schema: "hiroute.compute-management-change/v2".into(),
                subject: ComputeManagementSubjectV2::Candidate {
                    candidate: facts.candidate,
                },
                expected_revisions: stores
                    .control()
                    .compute_management_snapshot(&WorkspaceId::default())
                    .unwrap()
                    .revisions,
                selected_model_refs: vec!["model/one".into()],
                intent: ComputeManagementIntentV2::SaveReady,
                key_edits: Vec::new(),
                validation: None,
                edit: Some(ComputeManagementEditV1::Rename {
                    display_name: format!("API {number}"),
                }),
            },
            &format!("create-{number}"),
        );
    }
    let mut sources = stores
        .control()
        .compute_management_snapshot(&WorkspaceId::default())
        .unwrap()
        .sources;
    sources.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    let (first, second) = (&sources[0], &sources[1]);
    assert_ne!(first.source_id, second.source_id);
    assert_ne!(first.models[0].binding_id, second.models[0].binding_id);
    assert_ne!(
        first.credentials[0].credential.owner_scope(),
        second.credentials[0].credential.owner_scope()
    );
    assert_ne!(first.credentials[0].key_id, second.credentials[0].key_id);
    execute(
        &stores,
        &registry,
        edit(&stores, first, ComputeManagementEditV1::Delete, &[]),
        "delete-one",
    );
    assert_eq!(saved(&stores), *second);
    assert_owned_secret_deleted(&stores, &first.credentials[0].credential);
    assert_eq!(
        stores
            .secrets()
            .generation(&second.credentials[0].credential)
            .unwrap(),
        second.credentials[0].credential.generation()
    );
}

fn assert_owned_secret_deleted(
    stores: &LocalStorageSet,
    credential: &hiroute_domain::CredentialRefV1,
) {
    // A delete advances the durable generation/absence witness; resetting it to zero
    // would permit stale credential references to regain authority after recreation.
    assert_eq!(
        stores.secrets().generation(credential).unwrap(),
        credential.generation() + 1
    );
    let present: bool = stores.secrets().with_connection(|connection| {
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM secret_entries WHERE credential_id=?1)",
                [credential.credential_id()],
                |row| row.get(0),
            )
            .unwrap()
    });
    assert!(
        !present,
        "deleted connection must no longer retain a live secret entry"
    );
}
