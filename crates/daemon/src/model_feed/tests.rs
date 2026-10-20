use super::*;
use hiroute_integrations::model_feed::{FeedPayload, MODEL_FEED_SCHEMA};

fn payload(sequence: u64) -> Vec<u8> {
    let mut snapshot = crate::release_catalog::current_fixture_catalog().official_snapshot();
    snapshot.data_revision = sequence.max(1);
    publication_bytes(&FeedPayload::from_snapshot(
        snapshot,
        sequence,
        "2026-10-18T00:00:00Z".into(),
        "0123456789abcdef0123456789abcdef01234567".into(),
        vec![],
    ))
}
fn publication_bytes(payload: &FeedPayload) -> Vec<u8> {
    serde_json::to_vec(payload).unwrap()
}

#[test]
fn model_feed_restart_and_removal_preserve_captured_catalog_without_authorizing_new_use() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    let bytes = publication_bytes(&super::fixtures::publication(17));
    service
        .refresh_using(
            |_| {
                Ok(FeedDownload::Content {
                    bytes: bytes.clone(),
                    etag: None,
                })
            },
            |_, _| Ok(()),
        )
        .unwrap();
    let captured = service.current();
    assert!(
        captured
            .model_data()
            .model(super::fixtures::MODEL)
            .is_some()
    );
    drop(service);
    let restarted = open(root.path(), captured.generation);
    assert_eq!(
        restarted.current().model_data_provenance(),
        captured.model_data_provenance()
    );
    restarted
        .remove(&restarted.removal_digest().unwrap(), true, |_, _| Ok(()))
        .unwrap();
    assert!(
        restarted
            .current()
            .model_data()
            .model(super::fixtures::MODEL)
            .is_none()
    );
    assert!(
        captured
            .model_data()
            .model(super::fixtures::MODEL)
            .is_some()
    );
    let generation = restarted.current().generation;
    drop(restarted);
    let restarted = open(root.path(), generation);
    assert!(
        restarted
            .current()
            .model_data()
            .model(super::fixtures::MODEL)
            .is_none()
    );
    assert_eq!(
        restarted.status().unwrap().highest_accepted_sequence,
        Some(17)
    );
    restarted
        .refresh_using(
            |_| {
                Ok(FeedDownload::Content {
                    bytes: bytes.clone(),
                    etag: None,
                })
            },
            |_, _| Ok(()),
        )
        .unwrap();
    assert!(
        restarted
            .current()
            .model_data()
            .model(super::fixtures::MODEL)
            .is_some()
    );
}
fn open(path: &Path, min: u64) -> ModelFeedService {
    ModelFeedService::open(
        path,
        crate::release_catalog::current_fixture_catalog(),
        min,
        0,
    )
    .unwrap()
}
fn fetch(service: &ModelFeedService, sequence: u64) -> Result<(), String> {
    service.refresh_using(
        |_| {
            Ok(FeedDownload::Content {
                bytes: payload(sequence),
                etag: Some("test-etag".into()),
            })
        },
        |_, _| Ok(()),
    )
}

#[test]
fn model_feed_delete_preserves_high_water_and_identical_publication_restores_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 10).unwrap();
    let old = service.current();
    service
        .remove(&service.removal_digest().unwrap(), true, |_, _| Ok(()))
        .unwrap();
    assert!(!service.status().unwrap().retrieval_enabled);
    assert!(service.status().unwrap().held.is_none());
    assert_eq!(old.generation, 1);
    assert_eq!(old.accepted_sequence, 10);
    assert_eq!(service.current().accepted_sequence, 10);
    let restarted = open(root.path(), 2);
    assert_eq!(
        restarted.status().unwrap().highest_accepted_sequence,
        Some(10)
    );
    assert!(fetch(&restarted, 9).is_err());
    assert_eq!(
        restarted.status().unwrap().last_attempt.unwrap().result,
        "older_sequence"
    );
    fetch(&restarted, 10).unwrap();
    assert_eq!(restarted.status().unwrap().held.unwrap().sequence, 10);
    assert_eq!(restarted.current().generation, 3);
}

#[test]
fn model_feed_cacheless_304_retries_without_conditional_header() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 10).unwrap();
    service
        .remove(&service.removal_digest().unwrap(), false, |_, _| Ok(()))
        .unwrap();
    let mut calls = 0;
    service
        .refresh_using(
            |etag| {
                assert!(etag.is_none());
                calls += 1;
                if calls == 1 {
                    Ok(FeedDownload::NotModified)
                } else {
                    Ok(FeedDownload::Content {
                        bytes: payload(10),
                        etag: None,
                    })
                }
            },
            |_, _| Ok(()),
        )
        .unwrap();
    assert_eq!(calls, 2);
    assert!(service.status().unwrap().held.is_some());
}

#[test]
fn model_feed_corrupt_state_is_preserved_and_mutations_are_refused() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 5).unwrap();
    drop(service);
    let path = root.path().join("model-feed/state.json");
    std::fs::write(&path, b"future schema").unwrap();
    let reopened = open(root.path(), 1);
    assert!(reopened.status().unwrap().held.is_none());
    assert!(reopened.settings(Some(true), None).is_err());
    assert!(fetch(&reopened, 6).is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"future schema");
}

#[test]
fn model_feed_crash_before_state_commit_does_not_adopt_unreferenced_blob() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 5).unwrap();
    let bytes = payload(6);
    service
        .store
        .persist_blob(&bytes, &CanonicalDigest::of_bytes(&bytes))
        .unwrap();
    drop(service);
    let reopened = open(root.path(), 1);
    assert_eq!(reopened.status().unwrap().held.unwrap().sequence, 5);
}

#[test]
fn model_feed_crash_after_commit_recovers_catalog_and_generation() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 5).unwrap();
    assert!(
        service
            .refresh_using(
                |_| Ok(FeedDownload::Content {
                    bytes: payload(6),
                    etag: None
                }),
                |_, _| Err("injected activation interruption".into())
            )
            .is_err()
    );
    assert_eq!(service.available_catalog().unwrap().generation, 1);
    assert!(service.settings(Some(true), None).is_err());
    assert!(fetch(&service, 7).is_err());
    drop(service);
    let reopened = open(root.path(), 1);
    assert_eq!(reopened.current().generation, 2);
    assert_eq!(reopened.status().unwrap().held.unwrap().sequence, 6);
}

#[test]
fn model_feed_generation_is_captured_with_catalog_before_dependency_activation() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    let before = service.current();
    service
        .refresh_using(
            |_| {
                Ok(FeedDownload::Content {
                    bytes: payload(2),
                    etag: None,
                })
            },
            |generation, sequence| {
                assert_eq!(sequence, 2);
                assert_eq!(generation, 1);
                assert_eq!(service.current().generation, 0);
                assert!(service.available_catalog().is_none());
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(before.generation, 0);
    assert_eq!(service.current().generation, 1);
}

#[test]
fn model_feed_disabling_then_reenabling_invalidates_in_flight_retrieval() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    let result = service.refresh_using(
        |_| {
            service.settings(Some(false), None).unwrap();
            service.settings(Some(true), None).unwrap();
            Ok(FeedDownload::Content {
                bytes: payload(1),
                etag: None,
            })
        },
        |_, _| panic!("superseded retrieval cannot activate"),
    );
    assert!(result.is_err());
    assert!(service.status().unwrap().held.is_none());
}

#[test]
fn model_feed_invalid_cache_retains_high_water_on_reopen() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 5).unwrap();
    let held = service.status().unwrap().held.unwrap();
    std::fs::remove_file(
        root.path()
            .join("model-feed/publications")
            .join(format!("{}.json", &held.payload_digest.as_str()[7..])),
    )
    .unwrap();
    drop(service);
    let reopened = open(root.path(), 1);
    assert!(reopened.status().unwrap().held.is_none());
    assert_eq!(
        reopened.status().unwrap().highest_accepted_sequence,
        Some(5)
    );
    assert!(fetch(&reopened, 4).is_err());
    fetch(&reopened, 5).unwrap();
}

#[test]
fn model_feed_missing_state_recovers_floor_from_independent_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 17).unwrap();
    drop(service);
    std::fs::remove_dir_all(root.path().join("model-feed")).unwrap();
    let reopened = open(root.path(), 0);
    assert_eq!(
        reopened.status().unwrap().highest_accepted_sequence,
        Some(17)
    );
    assert!(reopened.status().unwrap().held.is_none());
    assert!(!reopened.status().unwrap().retrieval_enabled);
    assert_eq!(fetch(&reopened, 16).unwrap_err(), "older_sequence");
    fetch(&reopened, 17).unwrap();
}

#[test]
fn model_feed_corrupt_state_can_be_explicitly_quarantined_without_resetting_acceptance() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 18).unwrap();
    drop(service);
    let path = root.path().join("model-feed/state.json");
    std::fs::write(&path, b"broken owned state").unwrap();
    let reopened = open(root.path(), 1);
    assert!(reopened.status().unwrap().recovery_available);
    assert_eq!(
        reopened.status().unwrap().highest_accepted_sequence,
        Some(18)
    );
    reopened
        .remove(&reopened.removal_digest().unwrap(), true, |_, sequence| {
            assert_eq!(sequence, 18);
            Ok(())
        })
        .unwrap();
    assert!(!reopened.status().unwrap().recovery_available);
    let saved = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("state.quarantined-")
        })
        .unwrap();
    assert_eq!(std::fs::read(saved.path()).unwrap(), b"broken owned state");
    assert_eq!(fetch(&reopened, 17).unwrap_err(), "older_sequence");
    fetch(&reopened, 18).unwrap();
}

#[test]
fn model_feed_newer_schema_and_corruption_without_checkpoint_cannot_be_reset() {
    for (contents, checkpoint) in [
        (
            br#"{"schema":"hiroute.model-feed.state/v99"}"#.as_slice(),
            true,
        ),
        (b"broken".as_slice(), false),
    ] {
        let root = tempfile::tempdir().unwrap();
        let service = open(root.path(), 0);
        if checkpoint {
            fetch(&service, 19).unwrap();
        }
        drop(service);
        let path = root.path().join("model-feed/state.json");
        std::fs::write(&path, contents).unwrap();
        let reopened = open(root.path(), 0);
        assert!(!reopened.status().unwrap().recovery_available);
        assert!(
            reopened
                .remove(&reopened.removal_digest().unwrap(), true, |_, _| Ok(()))
                .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), contents);
    }
}

#[test]
fn model_feed_acceptance_commit_precedes_state_and_never_adopts_orphan_blob() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 5).unwrap();
    let bytes = payload(20);
    let validated = parse_feed(&bytes).unwrap();
    service
        .store
        .persist_blob(&bytes, &validated.payload_digest)
        .unwrap();
    // Crash after high-water commit but before the state/activation commit.
    service
        .store
        .retain_acceptance(&HighestAccepted {
            sequence: 20,
            payload_digest: validated.payload_digest,
        })
        .unwrap();
    drop(service);
    let reopened = open(root.path(), 1);
    assert_eq!(
        reopened.status().unwrap().highest_accepted_sequence,
        Some(20)
    );
    assert!(reopened.status().unwrap().held.is_none());
    assert_eq!(fetch(&reopened, 19).unwrap_err(), "older_sequence");
    fetch(&reopened, 20).unwrap();
}

#[test]
fn model_feed_unknown_digest_floor_requires_a_strictly_higher_sequence() {
    let root = tempfile::tempdir().unwrap();
    let service = ModelFeedService::open(
        root.path(),
        crate::release_catalog::current_fixture_catalog(),
        4,
        11,
    )
    .unwrap();
    assert_eq!(
        service.status().unwrap().highest_accepted_sequence,
        Some(11)
    );
    assert!(!service.status().unwrap().retrieval_enabled);
    assert_eq!(fetch(&service, 11).unwrap_err(), "older_sequence");
    fetch(&service, 12).unwrap();
}

#[test]
fn model_feed_invalid_refresh_keeps_the_last_good_catalog_and_private_cache() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    let bytes = publication_bytes(&super::fixtures::publication(17));
    service
        .refresh_using(
            |_| {
                Ok(FeedDownload::Content {
                    bytes: bytes.clone(),
                    etag: None,
                })
            },
            |_, _| Ok(()),
        )
        .unwrap();
    let captured = service.current();
    let held = service.status().unwrap().held.unwrap();
    for invalid in [b"invalid JSON".to_vec(), b"null".to_vec(), payload(16)] {
        assert!(
            service
                .refresh_using(
                    |_| Ok(FeedDownload::Content {
                        bytes: invalid.clone(),
                        etag: None
                    }),
                    |_, _| panic!("invalid or older publication cannot activate"),
                )
                .is_err()
        );
        assert_eq!(service.current().generation, captured.generation);
        assert!(
            service
                .current()
                .model_data()
                .model(super::fixtures::MODEL)
                .is_some()
        );
        assert_eq!(service.store.blob(&held).unwrap(), bytes);
    }
    drop(service);
    let reopened = open(root.path(), captured.generation);
    assert!(
        reopened
            .current()
            .model_data()
            .model(super::fixtures::MODEL)
            .is_some()
    );
    assert_eq!(reopened.status().unwrap().verification, "validated");
}

#[test]
fn model_feed_valid_json_with_changed_cached_bytes_is_discarded_without_lowering_acceptance() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 17).unwrap();
    let held = service.status().unwrap().held.unwrap();
    let file = root
        .path()
        .join("model-feed/publications")
        .join(format!("{}.json", &held.payload_digest.as_str()[7..]));
    let mut changed = payload(17);
    changed.push(b'\n');
    std::fs::write(&file, changed).unwrap();
    drop(service);
    let reopened = open(root.path(), 1);
    assert!(reopened.status().unwrap().held.is_none());
    assert_eq!(
        reopened.status().unwrap().highest_accepted_sequence,
        Some(17)
    );
    assert_eq!(fetch(&reopened, 16).unwrap_err(), "older_sequence");
    fetch(&reopened, 17).unwrap();
}

#[test]
fn announcement_only_publication_preserves_semantic_generation_and_confirmation_dependencies() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 5).unwrap();
    let before = service.current();
    let mut publication: FeedPayload = serde_json::from_slice(&payload(5)).unwrap();
    publication.sequence = 6;
    publication.issued_at = "2026-10-19T00:00:00Z".into();
    service
        .refresh_using(
            |_| {
                Ok(FeedDownload::Content {
                    bytes: publication_bytes(&publication),
                    etag: None,
                })
            },
            |_, _| panic!("editorial-only updates cannot activate a semantic/price generation"),
        )
        .unwrap();
    assert_eq!(service.current().generation, before.generation);
    assert_eq!(
        service.current().official_snapshot(),
        before.official_snapshot()
    );
    assert_eq!(service.status().unwrap().highest_accepted_sequence, Some(6));
}

#[test]
fn higher_publication_sequence_cannot_lower_semantic_data_even_after_removal() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    fetch(&service, 5).unwrap();
    service
        .remove(&service.removal_digest().unwrap(), false, |_, _| Ok(()))
        .unwrap();
    let mut publication: FeedPayload = serde_json::from_slice(&payload(4)).unwrap();
    publication.sequence = 100;
    let bytes = publication_bytes(&publication);
    assert_eq!(
        service
            .refresh_using(
                |_| Ok(FeedDownload::Content {
                    bytes: bytes.clone(),
                    etag: None
                }),
                |_, _| Ok(())
            )
            .unwrap_err(),
        "older_data_revision"
    );
    drop(service);
    let restarted = open(root.path(), 0);
    assert_eq!(
        restarted.status().unwrap().highest_accepted_sequence,
        Some(5)
    );
    assert_eq!(
        restarted.store.read_data_floor().unwrap().unwrap().revision,
        5
    );
}

#[test]
fn retained_v1_choices_and_floor_do_not_poison_v2_acceptance() {
    let root = tempfile::tempdir().unwrap();
    let _store = FeedStore::open(root.path()).unwrap();
    let legacy = serde_json::json!({
        "schema":"hiroute.model-feed.state/v1", "retrieval_enabled":false,
        "disclosure_acknowledged":true, "generation":7, "sequence_floor":99,
        "highest_accepted":{"sequence":99,"payload_digest":CanonicalDigest::of_bytes(b"legacy")},
        "held":null, "last_attempt":null,"last_success_at_ms":null,"etag":null
    });
    let bytes = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(root.path().join("model-feed/state.json"), &bytes).unwrap();
    let service = open(root.path(), 7);
    assert!(!service.status().unwrap().retrieval_enabled);
    assert!(service.status().unwrap().disclosure_acknowledged);
    assert_eq!(service.status().unwrap().highest_accepted_sequence, None);
    assert_eq!(
        std::fs::read(root.path().join("model-feed/retained-v1-state.json")).unwrap(),
        bytes
    );
    fetch(&service, 2).unwrap();
    let generation = service.current().generation;
    drop(service);
    let restarted = open(root.path(), generation);
    assert_eq!(
        restarted.status().unwrap().highest_accepted_sequence,
        Some(2)
    );
    restarted
        .remove(&restarted.removal_digest().unwrap(), true, |_, _| Ok(()))
        .unwrap();
    drop(restarted);
    let restarted = open(root.path(), generation + 1);
    assert_eq!(fetch(&restarted, 1).unwrap_err(), "older_sequence");
    assert!(restarted.status().unwrap().disclosure_acknowledged);
}

#[test]
fn model_feed_failed_preparation_keeps_previous_catalog_and_durable_selection() {
    let root = tempfile::tempdir().unwrap();
    let service = open(root.path(), 0);
    let old = service.current();
    assert!(
        service
            .refresh_using_prepared(
                |_| Ok(FeedDownload::Content {
                    bytes: payload(2),
                    etag: None
                }),
                |_| Err::<(), _>("prices.candidate.unavailable".into()),
                |_, ()| panic!("failed candidate must never activate"),
            )
            .is_err()
    );
    assert_eq!(service.current().generation, old.generation);
    assert!(service.status().unwrap().held.is_none());
    drop(service);
    assert_eq!(open(root.path(), 0).current().generation, old.generation);
}

#[test]
fn model_feed_slow_preparation_does_not_hold_feed_lock_and_superseded_candidate_never_installs() {
    let root = tempfile::tempdir().unwrap();
    let service = Arc::new(open(root.path(), 0));
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let worker = service.clone();
    let join = std::thread::spawn(move || {
        worker.refresh_using_prepared(
            |_| {
                Ok(FeedDownload::Content {
                    bytes: payload(2),
                    etag: None,
                })
            },
            |_| {
                entered_tx.send(()).unwrap();
                resume_rx.recv().unwrap();
                Ok(())
            },
            |_, ()| panic!("superseded candidate must never activate"),
        )
    });
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(service.available_catalog().is_some());
    service.settings(Some(false), None).unwrap();
    assert!(!service.status().unwrap().retrieval_enabled);
    resume_tx.send(()).unwrap();
    assert_eq!(
        join.join().unwrap().unwrap_err(),
        "feed_retrieval_superseded"
    );
    assert_eq!(service.current().generation, 0);
}
