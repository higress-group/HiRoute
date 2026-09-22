use super::*;
use crate::server::core_runtime::model_ir::{
    CanonicalInstruction, CanonicalMessage, ContentPart, InstructionRole, MODEL_REQUEST_IR_SCHEMA,
    MessageRole, ModelRequestIRV1, RequestedReasoningControl, ToolChoice,
};
use crate::server::request_plan::IngressProtocol;

const HISTORY_KEY: [u8; 32] = [23; 32];

fn key(id: usize) -> ContextHoldKey {
    let mut digest = [0_u8; 32];
    let bytes = id.to_be_bytes();
    digest[..bytes.len()].copy_from_slice(&bytes);
    ContextHoldKey(digest)
}

fn history_request(instruction: u8, messages: &[u8]) -> ModelRequestIRV1 {
    ModelRequestIRV1 {
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
fn append_and_repeat_keep_while_rebuild_clears() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1], now).unwrap();
    assert!(!first.history_continues);
    assert_eq!(
        store.complete(&first, preference("a"), now),
        HoldCompleteOutcome::Applied
    );

    let appended = begin(&store, 1, 9, &[1, 2], now + Duration::from_secs(1)).unwrap();
    assert!(appended.history_continues);
    assert_eq!(appended.hint.as_ref().unwrap().candidate_id, "a");
    let repeated = begin(&store, 1, 9, &[1, 2], now + Duration::from_secs(2)).unwrap();
    assert!(repeated.history_continues);
    assert_eq!(repeated.hint.as_ref().unwrap().candidate_id, "a");
    let rebuilt = begin(&store, 1, 9, &[1, 3], now + Duration::from_secs(3)).unwrap();
    assert!(!rebuilt.history_continues);
    assert!(rebuilt.hint.is_none());
}

#[test]
fn first_compatible_success_wins_across_repeat_and_append() {
    let store = ContextHoldStore::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL);
    let now = Instant::now();
    let first = begin(&store, 1, 9, &[1], now).unwrap();
    let repeated = begin(&store, 1, 9, &[1], now + Duration::from_millis(1)).unwrap();
    assert_eq!(
        store.complete(&first, preference("a"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        store.complete(&repeated, preference("b"), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        begin(&store, 1, 9, &[1, 2], now + Duration::from_secs(1))
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "a"
    );

    let earlier = begin(&store, 2, 9, &[1], now).unwrap();
    let appended = begin(&store, 2, 9, &[1, 2], now + Duration::from_millis(1)).unwrap();
    assert_eq!(
        store.complete(&earlier, preference("earlier"), now),
        HoldCompleteOutcome::Applied
    );
    assert_eq!(
        store.complete(&appended, preference("appended"), now),
        HoldCompleteOutcome::Stale
    );
    assert_eq!(
        begin(&store, 2, 9, &[1, 2, 3], now + Duration::from_secs(1))
            .unwrap()
            .hint
            .unwrap()
            .candidate_id,
        "earlier"
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
