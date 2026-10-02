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
fn corrupt_historical_publication_is_rejected_even_without_active_grants() {
    let root = crate::test_tempdir().unwrap();
    let stores = LocalStorageSet::open_for_daemon_startup(root.path()).unwrap();
    history(&stores, 3);
    // Keep active and LKG records intact; damage only retained history.
    stores.control.connection.borrow().execute(
        "UPDATE gateway_publications SET digest='corrupted-history' WHERE publication_revision=1",
        [],
    ).unwrap();
    assert!(
        validate_current_storage(stores.control(), stores.runtime(), stores.secrets()).is_err()
    );
    assert!(stores.validate_recovered_grant_publications().is_err());
    drop(stores);
    assert!(LocalStorageSet::open_for_daemon_startup(root.path()).is_err());
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
