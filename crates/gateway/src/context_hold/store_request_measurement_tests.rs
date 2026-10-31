use super::*;
use crate::context_hold::history::{measurement_count, visible_history};
use crate::replay::{ReplayConfig, ReplayManager, ReplayStore};
use hiroute_gateway_core::runtime::body::BudgetTree;

struct Root(std::path::PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn replay() -> (Root, ReplayStore) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = Root(std::env::temp_dir().join(format!(
        "hiroute-request-measurement-{}-{id}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    let manager = ReplayManager::open(ReplayConfig {
        root: root.0.clone(),
        ..ReplayConfig::production_default()
    })
    .unwrap();
    let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
        .unwrap()
        .stream(1024 * 1024)
        .unwrap();
    let replay = manager.begin_request(budget).unwrap();
    (root, replay)
}

#[test]
fn request_borrow_reuses_one_scan_and_preserves_the_uncached_continuity_oracle() {
    for rewrite in [false, true] {
        let (_root, replay) = replay();
        let store = ContextHoldStore::default();
        let digest_key = store.digest_key().unwrap();
        let oracle = ContextHoldStore {
            key: Some(digest_key),
            ..ContextHoldStore::default()
        };
        let now = Instant::now();
        let first = history_request(1, &[1]);
        let first_history = visible_history(&first, &digest_key).unwrap();
        let measured = measurement_count();
        let prepared = store
            .prepare_request(key(1), &first_history, &first, now)
            .unwrap();
        store
            .commit_on_branch(prepared, &[7; 32], &first, &replay, now)
            .unwrap();
        assert_eq!(measurement_count() - measured, 1);
        let prepared = oracle.prepare(key(1), &first_history, now).unwrap();
        oracle
            .commit_on_branch(prepared, &[7; 32], &first, &replay, now)
            .unwrap();

        let mut next = history_request(1, &[1, 2, 3]);
        next.messages[1].role = MessageRole::Assistant;
        if rewrite {
            next.messages[0].content[0] = ContentPart::Text {
                text: "changed first history message".into(),
            };
        }
        let history = visible_history(&next, &digest_key).unwrap();
        let measured = measurement_count();
        let prepared = store.prepare_request(key(1), &history, &next, now).unwrap();
        let actual = store
            .commit_on_branch(prepared, &[7; 32], &next, &replay, now)
            .unwrap();
        assert_eq!(measurement_count() - measured, 1);
        let prepared = oracle.prepare(key(1), &history, now).unwrap();
        let expected = oracle
            .commit_on_branch(prepared, &[7; 32], &next, &replay, now)
            .unwrap();
        assert_eq!(actual.history_continuity, expected.history_continuity);
        assert_eq!(actual.hint, expected.hint);
        assert_eq!(actual.previous_success, expected.previous_success);
        assert_eq!(actual.cleaned_prefix_len, expected.cleaned_prefix_len);
        assert_eq!(
            actual.history_continuity,
            if rewrite {
                HistoryContinuity::Rewritten
            } else {
                HistoryContinuity::Continuing
            }
        );
    }
}

#[test]
fn a_different_source_boundary_semantic_view_or_key_keeps_the_original_rescan() {
    for mismatch in 0..4 {
        let (_root, replay) = replay();
        let store = ContextHoldStore::default();
        let digest_key = store.digest_key().unwrap();
        let now = Instant::now();
        let first = history_request(1, &[1, 2]);
        let history = visible_history(&first, &digest_key).unwrap();
        let prepared = store
            .prepare_request(key(1), &history, &first, now)
            .unwrap();
        store
            .commit_on_branch(prepared, &[7; 32], &first, &replay, now)
            .unwrap();

        let mut next = history_request(1, &[1, 2, 3, 4, 5]);
        next.messages[if mismatch == 0 { 3 } else { 2 }].role = MessageRole::Assistant;
        let other_source = next.clone();
        let mut semantic = next.clone();
        semantic.messages[0].content[0] = ContentPart::Text {
            text: "routing-only alternate semantic view".into(),
        };
        let other_key = [61; 32];
        let history = visible_history(
            &next,
            if mismatch == 3 {
                &other_key
            } else {
                &digest_key
            },
        )
        .unwrap()
        .with_semantic_request((mismatch == 2).then_some(&semantic))
        .unwrap();
        let measured = measurement_count();
        let prepared = store.prepare_request(key(1), &history, &next, now).unwrap();
        let actual = store
            .commit_on_branch(
                prepared,
                &[7; 32],
                if mismatch == 1 { &other_source } else { &next },
                &replay,
                now,
            )
            .unwrap();
        assert_eq!(measurement_count() - measured, 2, "mismatch={mismatch}");
        assert_eq!(actual.history_continuity, HistoryContinuity::Continuing);
    }
}
