use super::*;
use hiroute_application::compute_management::query_compute_management_v3;
use hiroute_application_api::{ComputeManagementChangeV3, ComputeManagementEditV1};
use hiroute_domain::{
    ComputeManagementMutation, ComputeManagementProvenanceV2, ComputeManagementRepositoryPort,
    ComputeManagementSourceV2, ComputeSubscriptionApiLinkIntentV1, ComputeSubscriptionApiLinkV1,
};

fn sources(stores: &LocalStorageSet) -> Vec<ComputeManagementSourceV2> {
    stores.control().compute_management_snapshot(&WorkspaceId::default()).unwrap().sources
}
fn links(stores: &LocalStorageSet) -> Vec<ComputeSubscriptionApiLinkV1> {
    stores.control().compute_subscription_api_links(&WorkspaceId::default(), None).unwrap()
}
fn apply(
    stores: &LocalStorageSet, registry: &TrustedComputeCandidateRegistry,
    change: ComputeManagementChangeV2, intent: ComputeSubscriptionApiLinkIntentV1, key: &str,
) -> (hiroute_application_api::ComputeSavePreviewV2, serde_json::Value) {
    let input = ProtectedInput;
    let planner = ComputeManagementPlanner::new(registry, stores.control(), stores.secrets(), &input);
    let preview = planner.preview_v3(ComputeManagementChangeV3 {
        schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
        change, default_preferences: Default::default(), api_link: Some(intent),
    }).unwrap();
    let runtime = TransactionRuntime::default(); let external = NoExternal;
    let coordinator = TransactionCoordinator::new(stores.control(), stores.secrets(), stores.runtime(), &external, &input, &runtime);
    coordinator.reconcile_startup_and_open().unwrap();
    let saved_preview = (preview.result.clone(), preview.plan().control().clone());
    assert!(preview.plan().secrets().is_empty());
    apply_preview(stores, &planner, &coordinator, &WorkspaceId::default(), preview, key);
    saved_preview
}
fn connect(stores: &LocalStorageSet, registry: &TrustedComputeCandidateRegistry, api: &ComputeManagementSourceV2) -> (ComputeManagementSourceV2, (hiroute_application_api::ComputeSavePreviewV2, serde_json::Value)) {
    let mut facts = subscription_selection::subscription("candidate/cpa/codex/managed/test-account");
    facts.provenance = ComputeCandidateProvenanceV2::ConnectorOwned {
        connector_id: "connector.cpa.codex".into(), account_ref: "account/subscription".into(),
    };
    subscription_selection::register_verified_subscription(stores, registry, &facts, "approval-api-link");
    let preview = apply(stores, registry, subscription_selection::change(stores, &facts, &["model/one"]), ComputeSubscriptionApiLinkIntentV1::Associate { api_source_id: api.source_id.clone() }, "link-initial");
    let subscription = sources(stores).into_iter().find(|s| !s.provenance.is_native()).unwrap();
    (subscription, preview)
}
fn metadata(stores: &LocalStorageSet, source: &ComputeManagementSourceV2) -> ComputeManagementChangeV2 {
    ComputeManagementChangeV2 {
        schema: "hiroute.compute-management-change/v2".into(),
        subject: ComputeManagementSubjectV2::SavedSource { source_id: source.source_id.clone() },
        expected_revisions: stores.control().compute_management_snapshot(&WorkspaceId::default()).unwrap().revisions,
        selected_model_refs: source.models.iter().map(|m| m.model_ref.clone()).collect(),
        intent: ComputeManagementIntentV2::SaveReady, key_edits: vec![], validation: None, edit: None,
    }
}

#[test]
fn subscription_api_v4_import_detach_reassociate_reopen_and_backup_preserve_both_sources() {
    let root = tempdir().unwrap(); populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    let api = lifecycle::saved(&stores); let registry = TrustedComputeCandidateRegistry::new();
    let (sub, preview) = connect(&stores, &registry, &api);
    assert_eq!(links(&stores), vec![ComputeSubscriptionApiLinkV1 { source_id: sub.source_id.clone(), api_source_id: api.source_id.clone() }]);
    assert_eq!(sources(&stores).into_iter().find(|s| s.source_id == api.source_id).unwrap(), api);
    assert_eq!(sub.models.len(), 1); // Association alone cannot enlarge either membership.
    let mutation: ComputeManagementMutation = serde_json::from_value(preview.1["compute_management_mutation"].clone()).unwrap();
    mutation.validate_for_spec(&preview.0.spec).unwrap();
    assert!(matches!(mutation, ComputeManagementMutation::V4(_)));
    let mut tampered = serde_json::to_value(&mutation).unwrap();
    tampered["api_link_after"]["api_source_id"] = serde_json::json!("source/unapproved");
    let tampered: ComputeManagementMutation = serde_json::from_value(tampered).unwrap();
    assert!(tampered.validate_for_spec(&preview.0.spec).is_err());

    apply(&stores, &registry, metadata(&stores, &sub), ComputeSubscriptionApiLinkIntentV1::Remove, "link-remove");
    assert!(links(&stores).is_empty());
    let after = sources(&stores).into_iter().find(|s| s.source_id == sub.source_id).unwrap();
    assert_eq!(after.validation, sub.validation); assert_eq!(after.models, sub.models); assert_eq!(after.credentials, sub.credentials);
    apply(&stores, &registry, metadata(&stores, &after), ComputeSubscriptionApiLinkIntentV1::Associate { api_source_id: api.source_id.clone() }, "link-again");
    let before = sources(&stores); let association = links(&stores);
    let backup_path = root.path().join("backup/control.db"); let restored_path = root.path().join("restored/control.db");
    let backup = stores.control().with_connection(|c| crate::backup::SqliteBackup::create(&crate::test_storage_authority(), c, &backup_path).unwrap());
    backup.restore_to(&restored_path).unwrap();
    let restored = rusqlite::Connection::open(restored_path).unwrap();
    assert_eq!(restored.query_row("SELECT api_source_id FROM compute_subscription_api_links", [], |r| r.get::<_, String>(0)).unwrap(), api.source_id);
    assert!(restored.query_row("SELECT count(*) FROM compute_management_effects WHERE api_link_after IS NOT NULL", [], |r| r.get::<_, i64>(0)).unwrap() >= 3);
    drop(stores);
    let reopened = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert_eq!(sources(&reopened), before); assert_eq!(links(&reopened), association);
}

#[test]
fn subscription_api_legacy_query_shape_is_unchanged_and_opt_in_is_source_scoped() {
    let root = tempdir().unwrap(); populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap(); let api = lifecycle::saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new(); let (sub, _) = connect(&stores, &registry, &api);
    let query = ComputeManagementQueryV2::default();
    assert!(serde_json::to_value(&query).unwrap().get("include_subscription_api_links").is_none());
    let old = query_compute_management_v3(stores.control(), stores.runtime(), &WorkspaceId::default(), &query, None).unwrap();
    assert!(serde_json::to_value(&old).unwrap().get("subscription_api_links").is_none());
    for id in [&sub.source_id, &api.source_id] {
        let view = query_compute_management_v3(stores.control(), stores.runtime(), &WorkspaceId::default(), &ComputeManagementQueryV2 { source_id: Some(id.clone()), include_subscription_api_links: true }, None).unwrap();
        assert_eq!(view.subscription_api_links.as_ref().unwrap().len(), usize::from(id == &sub.source_id));
        assert!(serde_json::to_value(view.into_v2()).unwrap().get("subscription_api_links").is_none());
    }
}

#[test]
fn subscription_api_ordinary_lifecycle_keeps_link_missing_api_remains_visible_parent_delete_clears_it() {
    let root = tempdir().unwrap(); populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap(); let api = lifecycle::saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new(); let (sub, _) = connect(&stores, &registry, &api);
    lifecycle::execute(&stores, &registry, lifecycle::edit(&stores, &sub, ComputeManagementEditV1::Rename { display_name: "Renamed subscription".into() }, &[]), "sub-rename");
    assert_eq!(links(&stores).len(), 1);
    lifecycle::execute(&stores, &registry, lifecycle::edit(&stores, &api, ComputeManagementEditV1::Delete, &[]), "api-delete");
    assert_eq!(links(&stores)[0].api_source_id, api.source_id);
    drop(stores);
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    assert_eq!(links(&stores).len(), 1);
    let sub = sources(&stores).remove(0);
    lifecycle::execute(&stores, &registry, lifecycle::edit(&stores, &sub, ComputeManagementEditV1::Delete, &[]), "sub-delete");
    assert!(links(&stores).is_empty()); assert!(sources(&stores).is_empty());
}

#[test]
fn subscription_api_link_cannot_change_models_keys_defaults_or_authorization() {
    let root = tempdir().unwrap(); populate_saved_source(root.path(), &WorkspaceId::default());
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap(); let api = lifecycle::saved(&stores);
    let registry = TrustedComputeCandidateRegistry::new(); let (sub, _) = connect(&stores, &registry, &api);
    let input = ProtectedInput; let planner = ComputeManagementPlanner::new(&registry, stores.control(), stores.secrets(), &input);
    for kind in ["models", "intent", "validation", "native", "missing", "self"] {
        let mut change = metadata(&stores, &sub); let mut target = api.source_id.clone();
        match kind {
            "models" => change.selected_model_refs.clear(),
            "intent" => change.intent = ComputeManagementIntentV2::SaveDisabled,
            "validation" => change.validation = Some(hiroute_application_api::ComputeValidationRefV2 { approval_operation: hiroute_application_api::OperationReferenceV1 { operation_id: sub.validation.as_ref().unwrap().approval_operation_id.clone(), state: "succeeded".into(), sequence: 1, cancellable: false }, validation_ref: sub.validation.as_ref().unwrap().validation_ref.clone(), validation_revision: sub.validation.as_ref().unwrap().validation_revision }),
            "native" => change = metadata(&stores, &api),
            "missing" => target = "source/missing".into(),
            "self" => target = sub.source_id.clone(),
            _ => unreachable!(),
        }
        assert!(planner.preview_v3(ComputeManagementChangeV3 { schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(), change, default_preferences: Default::default(), api_link: Some(ComputeSubscriptionApiLinkIntentV1::Associate { api_source_id: target }) }).is_err(), "{kind}");
    }
    assert!(matches!(sub.provenance, ComputeManagementProvenanceV2::ConnectorOwned { .. }));
}

#[test]
fn subscription_api_current_schema_missing_table_fails_closed() {
    let root = tempdir().unwrap(); drop(LocalStorageSet::open_for_daemon_startup(root.path()).unwrap());
    let path = root.path().join("live/control.db");
    let c = rusqlite::Connection::open(&path).unwrap(); c.execute_batch("DROP TABLE compute_subscription_api_links").unwrap(); drop(c);
    assert!(LocalStorageSet::open_for_daemon_startup(root.path()).is_err());
    let c = rusqlite::Connection::open(path).unwrap();
    assert_eq!(c.query_row("SELECT count(*) FROM sqlite_master WHERE name='compute_subscription_api_links'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}
