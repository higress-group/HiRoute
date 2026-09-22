//! Frozen P0 matrix requirements and their production-process receipt bindings.
//!
//! This registry is deliberately the authority for the checked-in coverage
//! manifest.  The manifest is an auditable projection, never an input that can
//! claim its own rows, executions, or assertion identities.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::canonical::{canonical_json_digest, canonical_json_value};

pub const SCHEMA_VERSION: &str = "hiroute.e2e.p0-scenario-coverage/v3";
pub const PROCESS_ID: &str = "PROCESS-22009";
pub const LISTENER_TRANSPORT: &str = "loopback_h1";

/// Execution modes that can satisfy a frozen P0 receipt. This remains an enum
/// rather than a free manifest label so a source-only golden cannot be passed
/// off as a production invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionMode {
    RealHiroutedNativeProvider,
}

impl ExecutionMode {
    const fn manifest_label(self) -> &'static str {
        match self {
            Self::RealHiroutedNativeProvider => "real_hirouted_native_provider",
        }
    }

    fn has_real_process_marker(self, test_source: &str) -> bool {
        match self {
            Self::RealHiroutedNativeProvider => {
                test_source.contains("Hirouted") || test_source.contains("RuntimeFixture")
            }
        }
    }
}

/// The frozen way a test must bind its assertions to an execution receipt.
///
/// Most production tests can declare their assertion list before launching the
/// process because their assertions are indivisible. A streaming receipt is
/// different: each listed semantic assertion must be marked only after the
/// corresponding real response assertion completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssertionBinding {
    Declaration,
    RuntimeAssertionMarks,
}

impl AssertionBinding {
    const fn manifest_label(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::RuntimeAssertionMarks => "runtime_assertion_marks",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenReceipt {
    pub id: &'static str,
    pub execution: ExecutionMode,
    pub assertion_binding: AssertionBinding,
    pub test_target: &'static str,
    pub symbol: &'static str,
    pub source: &'static str,
    pub assertions: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrozenScenarioRow {
    pub id: &'static str,
    pub spec_id: &'static str,
    pub scenario: &'static str,
    pub receipt_ids: &'static [&'static str],
}

macro_rules! receipt {
    ($id:literal, $target:literal, $symbol:literal, $source:literal, [$($assertion:literal),+ $(,)?]) => {
        FrozenReceipt {
            id: $id,
            execution: ExecutionMode::RealHiroutedNativeProvider,
            assertion_binding: AssertionBinding::Declaration,
            test_target: $target,
            symbol: $symbol,
            source: $source,
            assertions: &[$($assertion),+],
        }
    };
}

macro_rules! runtime_receipt {
    ($id:literal, $target:literal, $symbol:literal, $source:literal, [$($assertion:literal),+ $(,)?]) => {
        FrozenReceipt {
            id: $id,
            execution: ExecutionMode::RealHiroutedNativeProvider,
            assertion_binding: AssertionBinding::RuntimeAssertionMarks,
            test_target: $target,
            symbol: $symbol,
            source: $source,
            assertions: &[$($assertion),+],
        }
    };
}

macro_rules! row {
    ($id:literal, $spec:literal, $scenario:literal, [$($receipt:literal),+ $(,)?]) => {
        FrozenScenarioRow {
            id: $id,
            spec_id: $spec,
            scenario: $scenario,
            receipt_ids: &[$($receipt),+],
        }
    };
}

pub const RECEIPTS: &[FrozenReceipt] = &[
    receipt!(
        "authority.live_publication",
        "p0_gateway_request_authority",
        "production_hirouted_live_publication_control_pins_cutover_and_preserves_lkg_on_nack",
        "tools/e2e-harness/tests/p0_gateway_request_authority.rs",
        [
            "publication.incompatible_or_gap",
            "publication.inflight_old_revision",
            "publication.lkg_restart",
        ]
    ),
    receipt!(
        "authority.wire_listener",
        "p0_gateway_request_authority",
        "production_hirouted_wire_authority_catalog_deadline_and_actual_restart",
        "tools/e2e-harness/tests/p0_gateway_request_authority.rs",
        [
            "gateway.isolated_native_listener",
            "authority.shared_entry_selects_alias_plan",
            "authority.unknown_or_unauthorized_alias",
            "authority.catalog_and_etag",
            "authority.alias_budget",
            "gateway.loopback_h1",
        ]
    ),
    receipt!(
        "protocol.nine_native_pairs",
        "p0_gateway_protocol",
        "production_matrix::protocol_real_hirouted_executes_every_native_protocol_pair_and_rejects_before_provider",
        "tools/e2e-harness/tests/p0_gateway_protocol/production_matrix.rs",
        [
            "protocol.responses_to_responses",
            "protocol.responses_to_chat_completions",
            "protocol.responses_to_messages",
            "protocol.chat_completions_to_responses",
            "protocol.chat_completions_to_chat_completions",
            "protocol.chat_completions_to_messages",
            "protocol.messages_to_responses",
            "protocol.messages_to_chat_completions",
            "protocol.messages_to_messages",
            "protocol.exact_native_request",
            "protocol.exact_client_projection",
            "protocol.fragmented_ingress",
            "protocol.preconnect_rejection",
        ]
    ),
    runtime_receipt!(
        "protocol.tool_and_sse",
        "p0_gateway_protocol",
        "continuation::production_native_tool_history_survives_restart_with_request_authentication",
        "tools/e2e-harness/tests/p0_gateway_protocol/continuation.rs",
        [
            "protocol.tool_round_trip",
            "protocol.sse_fragmented_semantics",
            "protocol.native_tool_identity",
            "protocol.stream_usage_terminal",
            "protocol.namespace_native_acceptance",
            "protocol.namespace_upstream_rejection",
            "protocol.continuation_request_authentication",
            "protocol.continuation_restart_native_history",
        ]
    ),
    receipt!(
        "planner.runtime_consumption",
        "p0_gateway_runtime",
        "real_listener_consumes_planner_order_and_binds_reason_ledger_identity",
        "tools/e2e-harness/tests/p0_gateway_runtime.rs",
        [
            "planner.frozen_order_consumed",
            "planner.reason_ledger_identity",
        ]
    ),
    receipt!(
        "runtime.fallback",
        "p0_gateway_runtime",
        "real_hirouted_reaches_native_provider_and_falls_back_before_commit",
        "tools/e2e-harness/tests/p0_gateway_runtime.rs",
        [
            "runtime.binding_fault",
            "runtime.frozen_budget",
            "runtime.precommit_fallback"
        ]
    ),
    receipt!(
        "runtime.context_hold.classified",
        "p0_gateway_context_hold",
        "real_hirouted_holds_only_inside_each_classified_branch",
        "tools/e2e-harness/tests/p0_gateway_context_hold.rs",
        [
            "runtime.context_hold.simple_to_complex",
            "runtime.context_hold.complex_to_simple",
            "runtime.context_hold.rebuild",
            "runtime.context_hold.responses_search_append"
        ]
    ),
    receipt!(
        "runtime.context_hold.fallback",
        "p0_gateway_context_hold",
        "real_hirouted_holds_the_successful_fallback_and_releases_it_on_rebuild",
        "tools/e2e-harness/tests/p0_gateway_context_hold.rs",
        [
            "runtime.context_hold.success_only",
            "runtime.context_hold.fallback_origin"
        ]
    ),
    receipt!(
        "protocol.previous_response_id",
        "p0_gateway_context_hold",
        "real_hirouted_rejects_opaque_previous_response_id_before_provider_io",
        "tools/e2e-harness/tests/p0_gateway_context_hold.rs",
        ["protocol.previous_response_id_preconnect_rejection"]
    ),
    receipt!(
        "runtime.key_handoff",
        "p0_gateway_runtime",
        "real_listener_distinguishes_quota_key_scope_from_overload_binding_scope",
        "tools/e2e-harness/tests/p0_gateway_runtime.rs",
        [
            "runtime.multiple_keys",
            "runtime.quota_key_handoff",
            "runtime.binding_overload"
        ]
    ),
    receipt!(
        "runtime.cas_write_fault",
        "p0_gateway_runtime",
        "real_listener_runtime_state_cas_write_failure_is_fail_closed_and_one_shot",
        "tools/e2e-harness/tests/p0_gateway_runtime.rs",
        [
            "runtime.cas_write_fault",
            "runtime.fail_closed_503",
            "runtime.zero_fallback_after_fault",
            "runtime.zero_bad_state_persist",
        ]
    ),
    receipt!(
        "runtime.probe_write_fault",
        "p0_gateway_runtime",
        "real_listener_probe_write_failure_creates_no_attempt_and_recovers_once",
        "tools/e2e-harness/tests/p0_gateway_runtime.rs",
        [
            "runtime.probe_write_fault",
            "runtime.probe_zero_attempt",
            "runtime.probe_one_shot_recovery",
        ]
    ),
    receipt!(
        "runtime.postcommit",
        "p0_gateway_commit_boundary",
        "real_listener_postcommit_disconnect_never_creates_another_attempt",
        "tools/e2e-harness/tests/p0_gateway_commit_boundary.rs",
        [
            "runtime.postcommit_no_retry",
            "runtime.postcommit_provider_count"
        ]
    ),
    receipt!(
        "replay.threshold",
        "p0_gateway_replay",
        "real_hirouted_replays_threshold_below_and_above_for_two_fallbacks",
        "tools/e2e-harness/tests/p0_gateway_replay.rs",
        [
            "replay.threshold",
            "replay.two_attempts",
            "replay.bounded_memory"
        ]
    ),
    receipt!(
        "replay.owner_only_permission",
        "p0_gateway_replay",
        "real_hirouted_rejects_non_owner_only_replay_root",
        "tools/e2e-harness/tests/p0_gateway_replay.rs",
        [
            "replay.owner_only_permission",
            "replay.zero_provider_on_permission"
        ]
    ),
    receipt!(
        "replay.request_capacity",
        "p0_gateway_replay",
        "real_hirouted_rejects_request_body_plan_capacity_before_provider_call",
        "tools/e2e-harness/tests/p0_gateway_replay.rs",
        [
            "replay.request_capacity_limit",
            "replay.zero_provider_on_capacity"
        ]
    ),
    receipt!(
        "replay.local_plaintext",
        "p0_gateway_replay",
        "real_hirouted_uses_one_owner_only_plaintext_replay_backing",
        "tools/e2e-harness/tests/p0_gateway_replay.rs",
        ["replay.owner_only_plaintext", "replay.single_backing"]
    ),
    receipt!(
        "replay.cancel_orphan",
        "p0_gateway_replay",
        "real_hirouted_cleans_exhaustion_deadline_disconnect_and_restart_orphan",
        "tools/e2e-harness/tests/p0_gateway_replay.rs",
        ["replay.cancel_cleanup", "replay.orphan_cleanup"]
    ),
    receipt!(
        "replay.long_sse",
        "p0_gateway_replay",
        "real_hirouted_releases_large_namespace_replay_before_long_sse_completion",
        "tools/e2e-harness/tests/p0_gateway_replay.rs",
        [
            "replay.long_prompt",
            "replay.long_sse",
            "replay.rss_bound",
            "replay.release"
        ]
    ),
    receipt!(
        "observation.accepted_content",
        "p0_gateway_observation",
        "real_hirouted_emits_request_route_attempt_commit_usage_and_accepted_only_content",
        "tools/e2e-harness/tests/p0_gateway_observation.rs",
        [
            "observation.execution_sequence",
            "observation.accepted_only_content",
            "observation.content_commit_boundary",
            "observation.otel_zero_body",
        ]
    ),
    receipt!(
        "observation.slow_sink",
        "p0_gateway_observation",
        "real_hirouted_isolates_slow_fail_and_panic_sinks_and_reports_each_gap",
        "tools/e2e-harness/tests/p0_gateway_observation.rs",
        [
            "observation.slow_sink",
            "observation.fact_gap",
            "observation.content_gap"
        ]
    ),
    receipt!(
        "privacy.secret_scan",
        "p0_gateway_privacy",
        "lifecycle_facts_and_default_otel_are_zero_content_and_rejected_auth_has_no_content",
        "tools/e2e-harness/tests/p0_gateway_privacy.rs",
        [
            "privacy.otel_zero_content",
            "privacy.rejected_auth_zero_content"
        ]
    ),
];

pub const REQUIRED_ROWS: &[FrozenScenarioRow] = &[
    row!(
        "publication.incompatible_or_gap",
        "SPEC-20001",
        "incompatible or gap publication is rejected before replacement",
        ["authority.live_publication"]
    ),
    row!(
        "publication.inflight_old_revision",
        "SPEC-20001",
        "in-flight request remains pinned to its old revision",
        ["authority.live_publication"]
    ),
    row!(
        "gateway.isolated_native_listener",
        "SPEC-20001",
        "isolated hirouted completes a native request",
        ["authority.wire_listener"]
    ),
    row!(
        "authority.shared_entry_selects_alias_plan",
        "SPEC-20002",
        "shared entry selects the granted alias-owned plan",
        ["authority.wire_listener"]
    ),
    row!(
        "authority.unknown_or_unauthorized_alias",
        "SPEC-20002",
        "unknown and unauthorized aliases fail uniformly",
        ["authority.wire_listener"]
    ),
    row!(
        "authority.catalog_and_etag",
        "SPEC-20002",
        "catalog and ETag are grant-scoped",
        ["authority.wire_listener"]
    ),
    row!(
        "authority.alias_budget_and_effort_override",
        "SPEC-20002",
        "alias budget remains request-owned",
        ["authority.wire_listener"]
    ),
    row!(
        "protocol.tool_id_and_continuation",
        "SPEC-20003",
        "tool identity and continuation are native-provider exact",
        ["protocol.tool_and_sse"]
    ),
    row!(
        "protocol.responses_to_responses",
        "SPEC-20003",
        "Responses ingress to Responses provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.responses_to_chat_completions",
        "SPEC-20003",
        "Responses ingress to Chat Completions provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.responses_to_messages",
        "SPEC-20003",
        "Responses ingress to Messages provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.chat_completions_to_responses",
        "SPEC-20003",
        "Chat Completions ingress to Responses provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.chat_completions_to_chat_completions",
        "SPEC-20003",
        "Chat Completions ingress to Chat Completions provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.chat_completions_to_messages",
        "SPEC-20003",
        "Chat Completions ingress to Messages provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.messages_to_responses",
        "SPEC-20003",
        "Messages ingress to Responses provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.messages_to_chat_completions",
        "SPEC-20003",
        "Messages ingress to Chat Completions provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.messages_to_messages",
        "SPEC-20003",
        "Messages ingress to Messages provider is exact",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.capability_fail_closed",
        "SPEC-20003",
        "unsupported semantics reject before native connect",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "protocol.sse_arbitrary_fragmentation",
        "SPEC-20003",
        "fragmented native SSE preserves semantic events",
        ["protocol.tool_and_sse"]
    ),
    row!(
        "planner.bilingual_complexity_and_keyword",
        "SPEC-20004",
        "frozen planner choice is consumed by the real listener",
        ["planner.runtime_consumption"]
    ),
    row!(
        "planner.free_first_no_classification",
        "SPEC-20004",
        "materialized planner order is consumed unchanged",
        ["planner.runtime_consumption"]
    ),
    row!(
        "planner.custom_order_and_determinism",
        "SPEC-20004",
        "custom planner order reaches native provider unchanged",
        ["planner.runtime_consumption"]
    ),
    row!(
        "planner.capability_n_and_n_plus_one",
        "SPEC-20004",
        "eligible planner candidate is the only native attempt",
        ["planner.runtime_consumption"]
    ),
    row!(
        "planner.context_hold_classified_continuation",
        "SPEC-20004",
        "same-turn continuation freezes its branch while a new turn selects independently",
        ["runtime.context_hold.classified"]
    ),
    row!(
        "runtime.binding_fault_and_total_budget",
        "SPEC-20005",
        "binding fault falls back inside frozen budget",
        ["runtime.fallback"]
    ),
    row!(
        "runtime.multiple_keys_quota_handoff",
        "SPEC-20005",
        "quota handoff remains scoped to the exact key",
        ["runtime.key_handoff"]
    ),
    row!(
        "runtime.cas_write_failure",
        "SPEC-20005",
        "CAS state write failure is fail-closed with no persisted error",
        ["runtime.cas_write_fault"]
    ),
    row!(
        "runtime.probe_write_failure",
        "SPEC-20005",
        "probe state write failure creates no native attempt",
        ["runtime.probe_write_fault"]
    ),
    row!(
        "runtime.postcommit_no_retry",
        "SPEC-20005",
        "postcommit disconnect does not create another attempt",
        ["runtime.postcommit"]
    ),
    row!(
        "runtime.context_hold_successful_fallback",
        "SPEC-20005",
        "only the successful fallback is held until visible history rebuilds",
        ["runtime.context_hold.fallback"]
    ),
    row!(
        "protocol.previous_response_id_preconnect_rejection",
        "SPEC-20003",
        "opaque Responses continuation is rejected before provider I/O",
        ["protocol.previous_response_id"]
    ),
    row!(
        "replay.storage_and_capacity",
        "SPEC-20006",
        "replay uses one owner-only plaintext backing and preserves request capacity gates",
        [
            "replay.owner_only_permission",
            "replay.request_capacity",
            "replay.local_plaintext"
        ]
    ),
    row!(
        "replay.cancel_and_restart_orphan",
        "SPEC-20006",
        "cancellation and restart remove replay backing",
        ["replay.cancel_orphan"]
    ),
    row!(
        "replay.long_prompt_multiple_attempts",
        "SPEC-20006",
        "large prompt bytes are replayed across attempts",
        ["replay.threshold"]
    ),
    row!(
        "replay.long_sse_bounded",
        "SPEC-20006",
        "long SSE keeps replay bounded until cleanup",
        ["replay.long_sse"]
    ),
    row!(
        "observation.otel_default_zero_content",
        "SPEC-20007",
        "default OTel has no request or response content",
        ["privacy.secret_scan", "observation.accepted_content"]
    ),
    row!(
        "observation.fact_sink_gap",
        "SPEC-20007",
        "fact sink failure reports only its own gap",
        ["observation.slow_sink"]
    ),
    row!(
        "observation.accepted_only_content",
        "SPEC-20007",
        "content contains only transport-accepted response",
        ["observation.accepted_content"]
    ),
    row!(
        "observation.content_sink_gap",
        "SPEC-20007",
        "content sink failure reports only its own gap",
        ["observation.slow_sink"]
    ),
    row!(
        "gateway.builtin_agent_emulator",
        "SPEC-20008",
        "native provider emulator is reached through hirouted",
        ["protocol.nine_native_pairs"]
    ),
    row!(
        "gateway.contract_green_only",
        "SPEC-20008",
        "all registered rows require a real execution receipt",
        ["authority.wire_listener"]
    ),
    row!(
        "gateway.four_automatic_checkpoints",
        "SPEC-20008",
        "production listener evidence remains ordered",
        ["observation.accepted_content"]
    ),
    row!(
        "gateway.resource_privacy_secret_scan",
        "SPEC-20008",
        "privacy and secret scan evidence is zero-leak",
        ["privacy.secret_scan"]
    ),
    row!(
        "gateway.loopback_h1",
        "SPEC-20008",
        "personal P0 listener is loopback HTTP/1",
        ["authority.wire_listener", "protocol.nine_native_pairs"]
    ),
    row!(
        "gateway.slow_sink",
        "SPEC-20008",
        "slow sink cannot backpressure a production response",
        ["observation.slow_sink"]
    ),
];

/// Binds an executed real-process test to a declaration-bound frozen receipt.
/// A mismatch is a test failure, not an annotation that the manifest can
/// self-report.
pub fn execution_receipt(id: &'static str, assertion_ids: &'static [&'static str]) {
    let receipt = RECEIPTS
        .iter()
        .find(|receipt| receipt.id == id)
        .unwrap_or_else(|| panic!("unknown P0 execution receipt {id}"));
    assert_eq!(
        receipt.assertions, assertion_ids,
        "P0 execution receipt {id} has drifted from its frozen assertions"
    );
    assert_eq!(
        receipt.execution,
        ExecutionMode::RealHiroutedNativeProvider,
        "P0 execution receipt {id} is not a real hirouted/native-provider invocation"
    );
    assert_eq!(
        receipt.assertion_binding,
        AssertionBinding::Declaration,
        "P0 execution receipt {id} must mark its assertions after execution"
    );
}

/// Runtime receipt state for semantics whose assertions must follow a real
/// streaming response. The receipt cannot finish unless every frozen
/// assertion is marked exactly once after its concrete assertion succeeds.
#[must_use = "a runtime execution receipt must finish after its assertions"]
pub struct RuntimeExecutionReceipt {
    id: &'static str,
    expected: BTreeSet<&'static str>,
    marked: BTreeSet<&'static str>,
}

pub fn runtime_execution_receipt(id: &'static str) -> RuntimeExecutionReceipt {
    let receipt = RECEIPTS
        .iter()
        .find(|receipt| receipt.id == id)
        .unwrap_or_else(|| panic!("unknown P0 execution receipt {id}"));
    assert_eq!(
        receipt.execution,
        ExecutionMode::RealHiroutedNativeProvider,
        "P0 execution receipt {id} is not a real hirouted/native-provider invocation"
    );
    assert_eq!(
        receipt.assertion_binding,
        AssertionBinding::RuntimeAssertionMarks,
        "P0 execution receipt {id} must declare its assertions"
    );
    RuntimeExecutionReceipt {
        id,
        expected: receipt.assertions.iter().copied().collect(),
        marked: BTreeSet::new(),
    }
}

impl RuntimeExecutionReceipt {
    /// Marks one assertion only after its matching production assertion has
    /// completed. Unknown or duplicate marks fail the test immediately.
    pub fn mark_assertion(&mut self, assertion_id: &'static str) {
        assert!(
            self.expected.contains(assertion_id),
            "P0 execution receipt {} cannot mark unknown assertion {assertion_id}",
            self.id
        );
        assert!(
            self.marked.insert(assertion_id),
            "P0 execution receipt {} marked assertion {assertion_id} twice",
            self.id
        );
    }

    pub fn finish(self) {
        assert_eq!(
            self.marked, self.expected,
            "P0 execution receipt {} did not mark its exact frozen assertions",
            self.id
        );
    }
}

#[macro_export]
macro_rules! p0_execution_receipt {
    ($id:literal, [$($assertion:literal),+ $(,)?]) => {
        $crate::p0::coverage::execution_receipt($id, &[$($assertion),+]);
    };
}

#[macro_export]
macro_rules! p0_runtime_execution_receipt {
    ($id:literal) => {
        $crate::p0::coverage::runtime_execution_receipt($id)
    };
}

pub fn registry_digest() -> String {
    let shape = json!({
        "schema_version": SCHEMA_VERSION,
        "process": PROCESS_ID,
        "listener_transport": LISTENER_TRANSPORT,
        "receipts": RECEIPTS.iter().map(receipt_value).collect::<Vec<_>>(),
        "rows": REQUIRED_ROWS.iter().map(row_value).collect::<Vec<_>>(),
    });
    digest_json(&shape)
}

/// Projects the typed registry without coupling the contract to source-file bytes.
/// `validate_manifest` separately verifies each registered source and test symbol.
pub fn manifest_value() -> Value {
    canonical_json_value(&json!({
        "schema_version": SCHEMA_VERSION,
        "process": PROCESS_ID,
        "listener_transport": LISTENER_TRANSPORT,
        "registry_digest": registry_digest(),
        "receipts": RECEIPTS.iter().map(receipt_value).collect::<Vec<_>>(),
        "rows": REQUIRED_ROWS.iter().map(row_value).collect::<Vec<_>>(),
    }))
}

pub fn write_manifest(e2e_root: &Path) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&manifest_value()).expect("coverage manifest serializes");
    fs::write(
        e2e_root.join("matrix/p0-gateway-coverage.json"),
        [bytes, b"\n".to_vec()].concat(),
    )
}

pub fn validate_manifest(root: &Path, value: &Value) -> Result<(), String> {
    let expected = manifest_value();
    if value != &expected {
        return Err("coverage manifest must be regenerated from the frozen typed registry".into());
    }
    let receipt_ids = RECEIPTS
        .iter()
        .map(|receipt| receipt.id)
        .collect::<BTreeSet<_>>();
    if receipt_ids.len() != RECEIPTS.len() {
        return Err("frozen receipt ids are not unique".into());
    }
    let row_ids = REQUIRED_ROWS
        .iter()
        .map(|row| row.id)
        .collect::<BTreeSet<_>>();
    if row_ids.len() != REQUIRED_ROWS.len() || row_ids.is_empty() {
        return Err("frozen required rows are empty or non-unique".into());
    }
    for row in REQUIRED_ROWS {
        if ![
            "SPEC-20001",
            "SPEC-20002",
            "SPEC-20003",
            "SPEC-20004",
            "SPEC-20005",
            "SPEC-20006",
            "SPEC-20007",
            "SPEC-20008",
        ]
        .contains(&row.spec_id)
            || row.receipt_ids.is_empty()
        {
            return Err(format!("invalid frozen row {}", row.id));
        }
        for receipt_id in row.receipt_ids {
            if !receipt_ids.contains(receipt_id) {
                return Err(format!(
                    "row {} references unknown receipt {receipt_id}",
                    row.id
                ));
            }
        }
    }
    for receipt in RECEIPTS {
        let source = fs::read_to_string(root.join(receipt.source))
            .map_err(|error| format!("cannot read {}: {error}", receipt.source))?;
        verify_test_symbol(&source, receipt)?;
    }
    Ok(())
}

fn receipt_value(receipt: &FrozenReceipt) -> Value {
    json!({
        "id": receipt.id,
        "execution": receipt.execution.manifest_label(),
        "assertion_binding": receipt.assertion_binding.manifest_label(),
        "test_target": receipt.test_target,
        "symbol": receipt.symbol,
        "source": receipt.source,
        "assertion_ids": receipt.assertions,
    })
}

fn row_value(row: &FrozenScenarioRow) -> Value {
    json!({
        "id": row.id,
        "spec_id": row.spec_id,
        "scenario": row.scenario,
        "receipt_ids": row.receipt_ids,
    })
}

fn verify_test_symbol(source: &str, receipt: &FrozenReceipt) -> Result<(), String> {
    let name = receipt.symbol.rsplit("::").next().unwrap_or(receipt.symbol);
    let needle = format!("fn {name}(");
    let position = source.find(&needle).ok_or_else(|| {
        format!(
            "receipt {} has no exact test symbol {}",
            receipt.id, receipt.symbol
        )
    })?;
    let prefix = &source[..position];
    let test_attribute = prefix.rfind("#[test]").unwrap_or(0);
    if position.saturating_sub(test_attribute) > 512 {
        return Err(format!("receipt {} symbol is not a test", receipt.id));
    }
    let test_source = source[position..]
        .split("#[test]")
        .next()
        .expect("test source slice");
    match receipt.assertion_binding {
        AssertionBinding::Declaration => {
            if !test_source.contains("p0_execution_receipt!") || !test_source.contains(receipt.id) {
                return Err(format!(
                    "receipt {} has no typed execution receipt",
                    receipt.id
                ));
            }
            for assertion in receipt.assertions {
                if !test_source.contains(assertion) {
                    return Err(format!("receipt {} does not bind {assertion}", receipt.id));
                }
            }
        }
        AssertionBinding::RuntimeAssertionMarks => {
            if !test_source.contains("p0_runtime_execution_receipt!")
                || !test_source.contains(receipt.id)
                || !test_source.contains("receipt.finish()")
            {
                return Err(format!(
                    "receipt {} has no completed runtime execution receipt",
                    receipt.id
                ));
            }
            for assertion in receipt.assertions {
                let mark = format!("receipt.mark_assertion(\"{assertion}\")");
                if !test_source.contains(&mark) {
                    return Err(format!(
                        "receipt {} does not mark runtime assertion {assertion}",
                        receipt.id
                    ));
                }
            }
        }
    }
    if !receipt.execution.has_real_process_marker(test_source) {
        return Err(format!(
            "receipt {} has no real hirouted process marker",
            receipt.id
        ));
    }
    Ok(())
}

fn digest_json(value: &Value) -> String {
    canonical_json_digest(value)
}
