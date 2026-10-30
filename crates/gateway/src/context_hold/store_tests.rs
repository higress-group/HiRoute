use super::*;
use crate::server::core_runtime::model_ir::{
    CanonicalInstruction, CanonicalMessage, ContentPart, InstructionRole, MODEL_REQUEST_IR_SCHEMA,
    MessageRole, ModelRequestIRV1, RequestedReasoningControl, ToolChoice,
};
use crate::server::request_plan::IngressProtocol;

const HISTORY_KEY: [u8; 32] = [23; 32];

#[path = "store_request_measurement_tests.rs"]
mod request_measurement;

#[test]
fn source_uncertainty_keeps_one_authorized_scope_with_separate_branch_evidence() {
    let authorized = crate::server::publication::authorized_native_consumer_fixture(None);
    let request = history_request(1, &[1]);
    let mut headers = http::HeaderMap::new();
    headers.insert("session-id", "same-native-session".parse().unwrap());
    let mut identity = crate::context_hold::identity_facts(
        &authorized,
        IngressProtocol::Responses,
        &headers,
        &serde_json::json!({}),
        &request,
        &HISTORY_KEY,
        &[41; 32],
    );
    let verified = ContextHoldKey::routing_history(
        &authorized,
        IngressProtocol::Responses,
        &identity,
        &HISTORY_KEY,
    )
    .unwrap();
    identity.mark_routing_origin_unverified();
    let observed = ContextHoldKey::routing_history(
        &authorized,
        IngressProtocol::Responses,
        &identity,
        &HISTORY_KEY,
    )
    .unwrap();
    assert_eq!(observed, verified);
}

fn key(id: usize) -> ContextHoldKey {
    let mut digest = [0_u8; 32];
    let bytes = id.to_be_bytes();
    digest[..bytes.len()].copy_from_slice(&bytes);
    ContextHoldKey(digest)
}

fn history_request(instruction: u8, messages: &[u8]) -> ModelRequestIRV1 {
    ModelRequestIRV1 {
        native_body: None,
        native_only: false,
        native_history: None,
        schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
        ingress_protocol: IngressProtocol::Responses,
        served_model_id: "agent/test".into(),
        stream: false,
        instructions: vec![CanonicalInstruction {
            role: InstructionRole::Developer,
            content: vec![ContentPart::Text {
                text: format!("instruction-{instruction}"),
            }],
        }],
        messages: messages
            .iter()
            .map(|message| CanonicalMessage {
                role: MessageRole::User,
                content: vec![ContentPart::Text {
                    text: format!("message-{message}"),
                }],
                name: None,
            })
            .collect(),
        tools: Vec::new(),
        tool_namespaces: Vec::new(),
        responses_tool_order: Vec::new(),
        web_search: None,
        responses_search_history: Default::default(),
        responses_annotations: Default::default(),
        responses_message_phases: Default::default(),
        responses_internal_chat_message_metadata: Default::default(),
        responses_reasoning_history: Default::default(),
        tool_choice: ToolChoice::None,
        parallel_tool_calls: false,
        requested_reasoning: RequestedReasoningControl::absent(),
        requested_max_output_tokens: None,
        provider_state: Vec::new(),
        responses_options: None,
        responses_item_ids: Default::default(),
        responses_item_statuses: Default::default(),
    }
}

fn begin(
    store: &ContextHoldStore,
    scope: usize,
    instruction: u8,
    messages: &[u8],
    now: Instant,
) -> Option<HoldTicket> {
    let request = history_request(instruction, messages);
    let history = super::super::visible_history(&request, &HISTORY_KEY)?;
    store.begin(key(scope), &history, now)
}

fn preference(id: &str) -> HoldPreferenceV1 {
    HoldPreferenceV1 {
        stable_binding_id: format!("binding-{id}"),
        candidate_id: id.into(),
        profile_digest: format!("sha256:{id}"),
        reasoning_profile_id: "fixed".into(),
        origin_group_id: "group".into(),
    }
}

#[test]
fn preparing_an_interleaved_history_cannot_invalidate_the_live_completion() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1], now).unwrap();
    let background = history_request(2, &[8]);
    let history = super::super::visible_history(&background, &HISTORY_KEY).unwrap();
    let prepared = store.prepare(key(1), &history, now).unwrap();
    assert_eq!(prepared.history_continuity(), HistoryContinuity::Rewritten);
    assert_eq!(
        store.complete(&first, preference("main"), now),
        HoldCompleteOutcome::Applied
    );
    let continued = begin(&store, 1, 9, &[1, 2], now).unwrap();
    assert_eq!(continued.hint.unwrap().candidate_id, "main");
}

#[test]
fn a_stale_prepared_history_cannot_overwrite_the_winning_checkpoint() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let request = history_request(9, &[1]);
    let history = super::super::visible_history(&request, &HISTORY_KEY).unwrap();
    let stale = store.prepare(key(1), &history, now).unwrap();
    let winner = begin(&store, 1, 9, &[2], now).unwrap();
    assert!(store.commit(stale, now).is_none());
    assert_eq!(
        store.complete(&winner, preference("winner"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        begin(&store, 1, 9, &[2, 3], now)
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "winner"
    );
}

#[test]
fn dropping_a_reserved_rewrite_preserves_the_live_checkpoint_and_completion() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let live = begin(&store, 1, 9, &[1], now).unwrap();
    let rewritten = history_request(2, &[8]);
    let evidence = super::super::visible_history(&rewritten, &HISTORY_KEY).unwrap();
    let reservation = store
        .reserve_commit(store.prepare(key(1), &evidence, now).unwrap(), now)
        .unwrap();
    drop(reservation);
    assert_eq!(
        store.complete(&live, preference("main"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        begin(&store, 1, 9, &[1, 2], now)
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "main"
    );
}

#[test]
fn a_busy_hold_store_skips_optional_reads_and_writes() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1], now).unwrap();
    let request = history_request(9, &[1, 2]);
    let history = super::super::visible_history(&request, &HISTORY_KEY).unwrap();
    let prepared = store.prepare(key(1), &history, now).unwrap();
    let lock = store.inner.lock();
    assert!(store.prepare(key(1), &history, now).is_none());
    assert!(store.commit(prepared, now).is_none());
    assert_eq!(
        store.complete(&first, preference("skipped"), now),
        HoldCompleteOutcome::CapacityMiss
    );
    drop(lock);
    assert_eq!(
        store.complete(&first, preference("main"), now),
        HoldCompleteOutcome::Applied
    );
}

#[test]
fn append_and_repeat_keep_while_rebuild_clears() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1], now).unwrap();
    assert_eq!(first.history_continuity, HistoryContinuity::NoCheckpoint);
    assert!(first.previous_success.is_none());
    assert_eq!(
        store.complete(&first, preference("a"), now),
        HoldCompleteOutcome::Applied
    );

    let appended = begin(&store, 1, 9, &[1, 2], now + Duration::from_secs(1)).unwrap();
    assert_eq!(appended.history_continuity, HistoryContinuity::Continuing);
    assert_eq!(appended.hint.as_ref().unwrap().candidate_id, "a");
    assert_eq!(
        appended.previous_success.as_ref().unwrap().candidate_id,
        "a"
    );
    let repeated = begin(&store, 1, 9, &[1, 2], now + Duration::from_secs(2)).unwrap();
    assert_eq!(repeated.history_continuity, HistoryContinuity::Continuing);
    assert_eq!(repeated.hint.as_ref().unwrap().candidate_id, "a");
    let changed_instructions = begin(&store, 1, 10, &[1, 2], now + Duration::from_secs(3)).unwrap();
    assert_eq!(
        changed_instructions.history_continuity,
        HistoryContinuity::Continuing
    );
    assert!(changed_instructions.hint.is_none());
    assert_eq!(
        changed_instructions.previous_success.unwrap().candidate_id,
        "a"
    );
    let rebuilt = begin(&store, 1, 10, &[1, 3], now + Duration::from_secs(4)).unwrap();
    assert_eq!(rebuilt.history_continuity, HistoryContinuity::Rewritten);
    assert!(rebuilt.hint.is_none());
    assert_eq!(rebuilt.previous_success.unwrap().candidate_id, "a");
}

#[test]
fn rebuilt_history_exposes_only_prior_complete_delivery_for_fallback() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1], now).unwrap();
    let without_success = begin(&store, 1, 9, &[2], now + Duration::from_secs(1)).unwrap();
    assert!(without_success.previous_success.is_none());
    assert_eq!(
        store.complete(&without_success, preference("luna"), now),
        HoldCompleteOutcome::Applied
    );
    let compacted = begin(&store, 1, 9, &[3], now + Duration::from_secs(2)).unwrap();
    assert_eq!(compacted.history_continuity, HistoryContinuity::Rewritten);
    assert!(compacted.hint.is_none());
    assert_eq!(compacted.previous_success.unwrap().candidate_id, "luna");
    assert_eq!(
        store.complete(&first, preference("stale"), now),
        HoldCompleteOutcome::Stale
    );
}

#[test]
fn cleaned_prefix_stays_fixed_and_does_not_restore_invalidated_hint() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1, 2], now).unwrap();
    assert_eq!(
        store.complete_with_cleanup(&first, preference("b"), Some(2), now),
        HoldCompleteOutcome::Applied
    );
    let appended = begin(&store, 1, 9, &[1, 2, 3], now).unwrap();
    assert_eq!(appended.cleaned_prefix_len, Some(2));
    assert_eq!(appended.hint.as_ref().unwrap().candidate_id, "b");
    assert_eq!(
        store.complete_with_cleanup(&appended, preference("b"), appended.cleaned_prefix_len, now),
        HoldCompleteOutcome::Applied
    );
    let changed_tools = begin(&store, 1, 10, &[1, 2, 3, 4], now).unwrap();
    assert_eq!(changed_tools.cleaned_prefix_len, Some(2));
    assert!(changed_tools.hint.is_none());
    let repeated = begin(&store, 1, 10, &[1, 2, 3, 4], now).unwrap();
    assert_eq!(repeated.cleaned_prefix_len, Some(2));
    assert!(repeated.hint.is_none());
    assert_eq!(
        store.complete_with_cleanup(&changed_tools, preference("a"), Some(4), now),
        HoldCompleteOutcome::Stale
    );
    let compacted = begin(&store, 1, 10, &[8], now).unwrap();
    assert_eq!(compacted.cleaned_prefix_len, None);
    assert_eq!(
        store.complete_with_cleanup(&repeated, preference("b"), Some(2), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        begin(&store, 2, 10, &[1, 2, 3, 4], now)
            .unwrap()
            .cleaned_prefix_len,
        None
    );
}

#[test]
fn latest_history_checkpoint_fences_repeat_and_append_completions() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1], now).unwrap();
    let repeated = begin(&store, 1, 9, &[1], now + Duration::from_millis(1)).unwrap();
    assert_eq!(
        store.complete(&first, preference("a"), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        store.complete(&repeated, preference("b"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        begin(&store, 1, 9, &[1, 2], now + Duration::from_secs(1))
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "b"
    );

    let earlier = begin(&store, 2, 9, &[1], now).unwrap();
    let appended = begin(&store, 2, 9, &[1, 2], now + Duration::from_millis(1)).unwrap();
    assert_eq!(
        store.complete(&earlier, preference("earlier"), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        store.complete(&appended, preference("appended"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        begin(&store, 2, 9, &[1, 2, 3], now + Duration::from_secs(1))
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "appended"
    );

    let slow = begin(&store, 3, 9, &[1], now).unwrap();
    let fast = begin(&store, 3, 9, &[1, 2], now + Duration::from_millis(1)).unwrap();
    assert_eq!(
        store.complete(&fast, preference("fast"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        store.complete(&slow, preference("slow"), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        begin(&store, 3, 9, &[1, 2, 3], now + Duration::from_secs(1))
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "fast"
    );
}

#[test]
fn completion_value_version_is_independent_from_history_checkpoint_revision() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let ticket = begin(&store, 1, 9, &[1], now).unwrap();
    let snapshot = {
        let inner = store.inner.lock();
        BeginSnapshot::from(inner.entries.get(&ticket.key).unwrap())
    };

    assert_eq!(
        store.complete(&ticket, preference("winner"), now),
        HoldCompleteOutcome::Applied
    );
    let inner = store.inner.lock();
    let completed = inner.entries.get(&ticket.key).unwrap();
    assert!(snapshot.matches_checkpoint(completed));
    assert_ne!(completed.value_version, ticket.value_version);
}

#[test]
fn rebuilt_cycle_rejects_prior_completion_without_rewinding_checkpoint() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let old = begin(&store, 1, 9, &[1], now).unwrap();
    let rebuilt = begin(&store, 1, 9, &[2], now + Duration::from_millis(1)).unwrap();
    assert_eq!(
        store.complete(&old, preference("old"), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        store.complete(&rebuilt, preference("new"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        begin(&store, 1, 9, &[2, 3], now + Duration::from_secs(1))
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "new"
    );
}

#[test]
fn idle_expiry_and_capacity_never_reinsert_old_completion() {
    let now = Instant::now();
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, Duration::from_secs(60));
    let expired = begin(&store, 1, 9, &[1], now).unwrap();
    let after_expiry = now + Duration::from_secs(60);
    let replacement = begin(&store, 1, 9, &[1], after_expiry).unwrap();
    assert_eq!(
        replacement.history_continuity,
        HistoryContinuity::NoCheckpoint
    );
    assert_eq!(
        store.complete(&expired, preference("old"), after_expiry),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        store.complete(&replacement, preference("new"), after_expiry),
        HoldCompleteOutcome::Applied
    );

    let tiny_budget = store_fixed_bytes().saturating_add(1);
    let tiny = ContextHoldStore::new(tiny_budget, DEFAULT_IDLE_TTL);
    assert!(begin(&tiny, 1, 9, &[1], now).is_none());
    assert!(tiny.accounted_bytes() <= tiny_budget);

    let eviction_budget = retained_bytes(3, 0);
    let evicting = ContextHoldStore::new(eviction_budget, DEFAULT_IDLE_TTL);
    let evicted = begin(&evicting, 10, 9, &[1], now).unwrap();
    assert!(begin(&evicting, 11, 9, &[1], now).is_some());
    assert!(begin(&evicting, 12, 9, &[1], now).is_some());
    assert!(begin(&evicting, 13, 9, &[1], now).is_some());
    assert_eq!(
        evicting.complete(&evicted, preference("evicted"), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        begin(&evicting, 10, 9, &[9], now)
            .unwrap()
            .history_continuity,
        HistoryContinuity::NoCheckpoint
    );
    assert!(evicting.peak_accounted_bytes() <= eviction_budget);
}

#[test]
fn byte_budget_has_no_4096_entry_ceiling() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    for id in 0..4_200 {
        assert!(begin(&store, id, 9, &[1], now).is_some());
    }
    assert!(store.entry_count() > 4_096);
    assert!(store.accounted_bytes() <= DEFAULT_MAX_MEMORY_BYTES);
    assert!(store.peak_accounted_bytes() <= DEFAULT_MAX_MEMORY_BYTES);
}

#[test]
fn active_entries_have_no_absolute_ttl_and_maintenance_is_bounded() {
    let now = Instant::now();
    let active = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let first = begin(&active, 1, 9, &[1], now).unwrap();
    assert_eq!(
        active.complete(&first, preference("kept"), now),
        HoldCompleteOutcome::Applied
    );
    for hour in 1..=3 {
        let ticket = begin(
            &active,
            1,
            9,
            &[1],
            now + Duration::from_secs(hour * 59 * 60),
        )
        .unwrap();
        assert_eq!(ticket.hint.unwrap().candidate_id, "kept");
    }

    let expired = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, Duration::from_secs(1));
    for id in 0..100 {
        assert!(begin(&expired, id, 9, &[1], now).is_some());
    }
    let before = expired.maintenance_visits();
    assert!(begin(&expired, 101, 9, &[1], now + Duration::from_secs(1)).is_some());
    let visits = expired.maintenance_visits().saturating_sub(before);
    assert!(visits <= MAINTENANCE_BATCH);
    assert!(expired.entry_count() >= 100 - MAINTENANCE_BATCH);
}

#[test]
fn ordered_indexes_charge_each_entry_before_insertion() {
    assert!(retained_bytes(4, 0) > retained_bytes(3, 0));
    assert_eq!(
        retained_bytes(4, 0).saturating_sub(retained_bytes(3, 0)),
        TREE_INDEX_ENTRY_BYTES
    );
}

#[test]
fn history_rewrites_and_unavailable_checkpoints_have_distinct_causes() {
    let now = Instant::now();
    for (replacement, expected) in [
        (&[9][..], HistoryContinuity::Shortened),
        (&[1, 2][..], HistoryContinuity::Shortened),
        (&[1, 9, 3][..], HistoryContinuity::Rewritten),
        (&[1, 9, 3, 4][..], HistoryContinuity::Rewritten),
    ] {
        let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
        assert_eq!(
            begin(&store, 1, 1, &[1, 2, 3], now)
                .unwrap()
                .history_continuity,
            HistoryContinuity::NoCheckpoint
        );
        // Only a decrease is the compaction heuristic, including a shorter rewind.
        assert_eq!(
            begin(&store, 1, 1, replacement, now)
                .unwrap()
                .history_continuity,
            expected
        );
    }
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, Duration::from_secs(1));
    begin(&store, 1, 1, &[1, 2, 3], now).unwrap();
    assert_eq!(
        begin(&store, 1, 1, &[9], now + Duration::from_secs(1))
            .unwrap()
            .history_continuity,
        HistoryContinuity::NoCheckpoint
    );
    let disabled = ContextHoldStore {
        key: None,
        ..ContextHoldStore::default()
    };
    assert!(begin(&disabled, 1, 1, &[9], now).is_none());
}

#[test]
fn checkpoint_cas_contention_does_not_claim_a_rewrite() {
    let store = ContextHoldStore::default();
    let now = Instant::now();
    begin(&store, 1, 1, &[1], now).unwrap();
    let request = history_request(1, &[9]);
    let history = super::super::visible_history(&request, &HISTORY_KEY).unwrap();
    let contested = store.begin_with_measure(key(1), &history, now, |prefix| {
        // Deterministically interleave another entry while the original hashes outside the lock.
        assert_eq!(
            begin(&store, 1, 1, &[1, 2], now)
                .unwrap()
                .history_continuity,
            HistoryContinuity::Continuing
        );
        history.measure(prefix)
    });
    assert!(contested.is_none());
    assert_eq!(
        begin(&store, 1, 1, &[1, 2, 3], now)
            .unwrap()
            .history_continuity,
        HistoryContinuity::Continuing
    );
}

#[test]
fn native_instruction_text_changes_keep_semantic_history_without_kv_or_cleanup_reuse() {
    for role in [MessageRole::System, MessageRole::Developer] {
        let store = ContextHoldStore::default();
        let now = Instant::now();
        let mut original = history_request(1, &[1, 2]);
        original.messages[0].role = role;
        let evidence = super::super::visible_history(&original, &HISTORY_KEY).unwrap();
        let mut prepared = store.prepare(key(1), &evidence, now).unwrap();
        prepared.set_native(super::super::routing::NativeCheckpoint {
            turn: Some([1; 32]),
            requires_checkpoint: true,
            ..Default::default()
        });
        let first = store.commit(prepared, now).unwrap();
        assert_eq!(
            store.complete_with_cleanup(&first, preference("accepted"), Some(2), now),
            HoldCompleteOutcome::Applied
        );
        let mut changed = original.clone();
        changed.messages[0].content[0] = ContentPart::Text {
            text: "new native skills".into(),
        };
        changed
            .messages
            .push(history_request(1, &[3]).messages.remove(0));
        let evidence = super::super::visible_history(&changed, &HISTORY_KEY).unwrap();
        let mut prepared = store.prepare(key(1), &evidence, now).unwrap();
        prepared.set_native(super::super::routing::NativeCheckpoint {
            turn: Some([2; 32]),
            requires_checkpoint: true,
            ..Default::default()
        });
        assert_eq!(
            prepared.history_continuity(),
            HistoryContinuity::NativeNewTurn
        );
        assert!(prepared.history_is_proven());
        let next = store.commit(prepared, now).unwrap();
        assert!(
            next.hint.is_none(),
            "semantic history cannot restore KV affinity"
        );
        assert_eq!(
            next.cleaned_prefix_len, None,
            "changed bytes cannot reuse cleanup"
        );
        assert_ne!(first.cycle, next.cycle);
        assert_eq!(
            store.complete(&first, preference("stale"), now),
            HoldCompleteOutcome::Stale
        );
    }
}

#[test]
fn a_new_native_turn_does_not_authorize_changed_user_assistant_or_tool_prefixes() {
    let mut original = crate::server::core_runtime::adapters::decode_ingress_request(
        IngressProtocol::Responses,
        &serde_json::json!({"model":"route","input":[
            {"role":"system","content":"native instructions"},
            {"role":"user","content":"original user"},
            {"role":"assistant","content":"original assistant"},
            {"type":"function_call","call_id":"call","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","call_id":"call","output":"original receipt"}
        ]}),
    )
    .unwrap();
    // Exercise canonical prefix changes; the variants below deliberately edit
    // and append IR messages rather than re-decoding a native request.
    original.native_body = None;
    original.native_history = None;
    for index in 1..5 {
        let store = ContextHoldStore::default();
        let now = Instant::now();
        let evidence = super::super::visible_history(&original, &HISTORY_KEY).unwrap();
        let mut prepared = store.prepare(key(1), &evidence, now).unwrap();
        prepared.set_native(super::super::routing::NativeCheckpoint {
            turn: Some([1; 32]),
            requires_checkpoint: true,
            ..Default::default()
        });
        store.commit(prepared, now).unwrap();
        let mut changed = original.clone();
        // Replacing any of these parts changes the prior conversational prefix,
        // even when only the native instruction text also changes.
        changed.messages[0].content[0] = ContentPart::Text {
            text: "updated instructions".into(),
        };
        changed.messages[index].content[0] = ContentPart::Text {
            text: "rewritten conversation".into(),
        };
        changed
            .messages
            .push(history_request(1, &[9]).messages.remove(0));
        let evidence = super::super::visible_history(&changed, &HISTORY_KEY).unwrap();
        let mut prepared = store.prepare(key(1), &evidence, now).unwrap();
        prepared.set_native(super::super::routing::NativeCheckpoint {
            turn: Some([2; 32]),
            requires_checkpoint: true,
            ..Default::default()
        });
        assert_eq!(
            prepared.history_continuity(),
            HistoryContinuity::Rewritten,
            "index={index}"
        );
        assert!(
            !prepared.history_is_proven(),
            "new turn accepted a rewritten prefix at {index}"
        );
    }
}

#[test]
fn borrowed_native_user_batch_view_cannot_reuse_repartitioned_wire_kv_or_cleanup() {
    let mut first_request = history_request(1, &[1, 2]);
    let text = |value: &str| ContentPart::Text { text: value.into() };
    first_request.messages[0].content = vec![text("A")];
    first_request.messages[1].content = vec![text("B"), text("C")];
    let mut next_request = first_request.clone();
    next_request.messages[0].content = vec![text("A"), text("B")];
    next_request.messages[1].content = vec![text("C")];
    let mut semantic = first_request.clone();
    semantic.messages[0].role = MessageRole::System;
    semantic.messages[0].content.clear();
    semantic.messages[1].content = vec![text("A"), text("B"), text("C")];
    let store = ContextHoldStore::default();
    let now = Instant::now();
    let first = super::super::visible_history(&first_request, &HISTORY_KEY)
        .unwrap()
        .with_semantic_request(Some(&semantic))
        .unwrap();
    let first = store.begin(key(1), &first, now).unwrap();
    assert_eq!(
        store.complete_with_cleanup(&first, preference("accepted"), Some(2), now),
        HoldCompleteOutcome::Applied
    );
    let next = super::super::visible_history(&next_request, &HISTORY_KEY)
        .unwrap()
        .with_semantic_request(Some(&semantic))
        .unwrap();
    let next = store.begin(key(1), &next, now).unwrap();
    assert_eq!(next.history_continuity, HistoryContinuity::Continuing);
    assert!(
        next.hint.is_none(),
        "normalized user batches are not byte-prefix proof"
    );
    assert_eq!(next.cleaned_prefix_len, None);
    assert_ne!(first.cycle, next.cycle);
}
