//! Missing human-origin proof must not disable actual execution history.
use super::*;
use crate::context_hold::{NativeCheckpoint, NativeUserOrigin};

#[test]
fn cold_observed_input_recovers_a_b_history_and_an_actual_execution_target() {
    for origin in [NativeUserOrigin::Observed, NativeUserOrigin::NativeTurn] {
        let fixture = Fixture::new();
        let old = json!({"role":"user","content":"uncaptured compressed context"});
        let a = json!({"role":"user","content":"actual question A"});
        let input_a = Fixture::request(json!([old, a]));
        let mut hold = fixture.prepare(&input_a);
        hold.set_native(NativeCheckpoint {
            turn: Some([1; 32]),
            generation: Some(3),
            compacted: true,
            user_origin: origin,
            ..Default::default()
        });
        assert!(!hold.history_is_proven());
        assert!(!hold.native_user_unavailable());
        let mut admitted = fixture.begin(&input_a, hold);
        assert!(admitted.history.history_partial);
        assert!(admitted.history.visible_conversation.is_empty());
        fixture.finish(
            &mut admitted,
            &[text_event("actual accepted output A")],
            true,
        );
        drop(admitted);

        let input_b = Fixture::request(json!([old, a,
            {"role":"assistant","content":"actual accepted output A"},
            {"role":"user","content":"actual question B"}
        ]));
        let mut hold = fixture.prepare(&input_b);
        hold.set_native(NativeCheckpoint {
            turn: Some([2; 32]),
            generation: Some(3),
            compacted: true,
            user_origin: origin,
            ..Default::default()
        });
        let next = fixture.begin(&input_b, hold);
        let target = next.history.assessment_target.as_ref().unwrap();
        assert!(
            !target.target_partial,
            "source uncertainty is not missing output"
        );
        assert_eq!(target.through_ordinal, 1);
        let visible = serde_json::to_string(&next.history.visible_conversation).unwrap();
        assert!(visible.contains("actual question A"));
        assert!(visible.contains("actual accepted output A"));
        assert!(visible.contains("unverified_request_user_origin"));
        assert!(!visible.contains("uncaptured compressed context"));
        assert!(!visible.contains("actual question B"));
    }
}

#[test]
fn a_known_native_checkpoint_discloses_missing_prefix_without_invalidating_new_execution() {
    let fixture = Fixture::new();
    let a = json!({"role":"user","content":"actual new A after native compaction"});
    let input_a = Fixture::request(json!([a]));
    let mut hold = fixture.prepare(&input_a);
    hold.set_native(NativeCheckpoint {
        turn: Some([1; 32]),
        checkpoint: Some([7; 32]),
        requires_checkpoint: true,
        user_origin: NativeUserOrigin::CurrentUser,
        ..Default::default()
    });
    let mut admitted = fixture.begin(&input_a, hold);
    assert!(
        admitted.history.history_partial,
        "the known lost prefix is not an empty complete history"
    );
    fixture.finish(&mut admitted, &[text_event("actual A execution")], true);
    drop(admitted);
    let input_b = Fixture::request(json!([a,
        {"role":"assistant","content":"actual A execution"},
        {"role":"user","content":"actual new B"}
    ]));
    let mut hold = fixture.prepare(&input_b);
    hold.set_native(NativeCheckpoint {
        turn: Some([2; 32]),
        checkpoint: Some([7; 32]),
        requires_checkpoint: true,
        user_origin: NativeUserOrigin::CurrentUser,
        ..Default::default()
    });
    let next = fixture.begin(&input_b, hold);
    assert!(next.history.history_partial);
    assert!(
        !next
            .history
            .assessment_target
            .as_ref()
            .unwrap()
            .target_partial
    );
}
