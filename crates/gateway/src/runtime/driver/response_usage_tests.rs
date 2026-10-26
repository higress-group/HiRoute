//! Preserve known nonstream usage before the decoder is consumed, without
//! depending on the independent canonical capture worker.
use super::*;
use std::time::{Duration, Instant};

use crate::ports::{ExecutionScope, InMemoryRuntimeStateStore, RuntimeStateKey};
use crate::runtime::state::{OwnedAttemptStatePermits, acquire_target_permit};
use crate::server::core_runtime::profiles::{ClientProtocolProfile, fixed_reasoning};
use hiroute_gateway_core::core::execution_plan::{
    CaPolicy, PoolEpoch, TransportReuseClassId, TransportScheme, TransportTarget,
};
use hiroute_gateway_core::runtime::attempt::{AttemptTransportFacts, CommitFence};
use hiroute_gateway_core::runtime::body::{BodyPlan, BudgetTree};
use hiroute_gateway_core::runtime::driver::{
    AttemptCleanupOutcome, AttemptCommitFacts, AttemptDownstreamOutcome, AttemptStreamOutcome,
    AttemptTerminationReason,
};
use serde_json::{Value, json};

fn chat_response(with_usage: bool) -> Value {
    let mut body = json!({
        "id": "nonstream-usage", "object": "chat.completion", "created": 1,
        "model": "physical",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"}]
    });
    if with_usage {
        body["usage"] = json!({
            "prompt_tokens": 32, "completion_tokens": 45, "total_tokens": 77,
            "prompt_tokens_details": {"cached_tokens": 0},
            "completion_tokens_details": {"reasoning_tokens": 36}
        });
    }
    body
}

async fn attempt_state(ingress: IngressProtocol) -> ProductionAttemptState {
    let profile = CandidateProtocolProfile::exact_portable_path(
        ingress,
        IngressProtocol::ChatCompletions,
        "physical",
        fixed_reasoning("fixed"),
    );
    let native_output = ingress == IngressProtocol::ChatCompletions;
    let budget = BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024)
        .unwrap()
        .stream(4 * 1024 * 1024)
        .unwrap();
    let store = InMemoryRuntimeStateStore::default();
    let key = RuntimeStateKey::binding("nonstream-usage");
    let scope = ExecutionScope::new(
        Instant::now() + Duration::from_secs(1),
        tokio_util::sync::CancellationToken::new(),
    );
    let permit = acquire_target_permit(&store, &key, &scope)
        .await
        .unwrap()
        .unwrap();
    let decoder = (!native_output).then(|| {
        adapters::NativeResponseDecoder::new_for_attempt(&profile, 200, false, None, budget.clone())
            .unwrap()
    });
    let projector = native_output.then(|| {
        Box::new(
            adapters::NativeResponseProjector::new_for_attempt(
                &profile,
                false,
                "alias".into(),
                None,
                budget.clone(),
            )
            .unwrap(),
        )
    });
    let plan = BodyPlan::PassThrough {
        max_chunk_bytes: 64 * 1024,
    };
    let prefix =
        ChargedBodyQueue::new(&budget, MemoryRole::ResponsePrefix, &plan, 256 * 1024, 64).unwrap();
    ProductionAttemptState {
        managed_cpa: false,
        request_reasoning_projection: None,
        request_projection_loss: Default::default(),
        #[cfg(all(unix, debug_assertions))]
        capture: None,
        response_status: Some(StatusCode::OK),
        resolved_target: TransportTarget {
            scheme: TransportScheme::Http,
            authority: "127.0.0.1:1".into(),
            addresses: Arc::from(["127.0.0.1:1".parse().unwrap()]),
            sni: None,
            ca: CaPolicy::System,
            alpn: Arc::from([]),
            connect_timeout: Duration::from_secs(1),
            transport_read_buffer_bytes: 16 * 1024,
            h2_stream_window_bytes: 65_535,
            h2_connection_window_bytes: 65_535,
            h2_max_concurrent_streams: 1,
            reuse_class: TransportReuseClassId(1),
            pool_epoch: PoolEpoch(1),
            connection_fingerprint: [0; 32],
        },
        permits: OwnedAttemptStatePermits::without_credential(key, permit),
        classified_failure: None,
        client_profile: ClientProtocolProfile::for_candidate(&profile).unwrap(),
        profile,
        served_model_alias: "alias".into(),
        streaming: false,
        native_output,
        omit_thinking: false,
        chat_tool_projection: None,
        function_encoding_projection: Default::default(),
        chat_tool_projection_budget: None,
        decoder,
        renderer: None,
        projector,
        prefix: Some(prefix),
        prefix_terminal_chunks: None,
        budget,
        semantic_seen: false,
        semantic_terminal: None,
        retry_after: None,
        runtime_state_authority: Default::default(),
    }
}

async fn completed_facts(
    ingress: IngressProtocol,
    with_usage: bool,
) -> ProviderClassificationFacts {
    completed_facts_with_write(ingress, with_usage, None).await
}

async fn completed_facts_with_write(
    ingress: IngressProtocol,
    with_usage: bool,
    write: Option<u64>,
) -> ProviderClassificationFacts {
    let mut state = attempt_state(ingress).await;
    let mut response = chat_response(with_usage);
    if let Some(write) = write {
        response["usage"]["prompt_tokens_details"]["cache_write_tokens"] = json!(write);
    }
    let bytes = serde_json::to_vec(&response).unwrap();
    let body_bytes = bytes.len() as u64;
    let input =
        ChargedBytes::copy_from_opaque(&state.budget, MemoryRole::ResponsePrefix, &bytes).unwrap();
    assert!(
        classify_precommit(&mut state, PrecommitEvent::Body(input))
            .unwrap()
            .classified
            .is_none()
    );
    let classified = classify_precommit(&mut state, PrecommitEvent::EndStream)
        .unwrap()
        .classified
        .unwrap();
    assert_eq!(
        state.semantic_terminal,
        Some(SemanticTerminalOutcome::Complete)
    );
    if !state.native_output {
        assert!(state.decoder.is_none());
        assert!(classified.readiness.decoder.is_none());
        assert_eq!(
            classified.facts.usage.is_some(),
            with_usage,
            "converted nonstream usage must survive consuming the decoder"
        );
    }
    let now = Instant::now();
    let completion = ProviderAttemptCompletion {
        disposition: Disposition::Accept,
        commits: AttemptCommitFacts {
            upstream_request: CommitFence::WriteConfirmed,
            downstream_headers: CommitFence::WriteConfirmed,
            downstream_semantic: CommitFence::WriteConfirmed,
        },
        stream: AttemptStreamOutcome::CompletedEos,
        downstream: AttemptDownstreamOutcome::Completed,
        cleanup: AttemptCleanupOutcome::Completed,
        transport: AttemptTransportFacts {
            started_at: now,
            upstream_protocol: None,
            connect_elapsed: None,
            request_write_elapsed: None,
            upstream_ttfb: None,
            last_upstream_progress_at: None,
            local_read_suppressed: Duration::ZERO,
            upstream_body_bytes: body_bytes,
            timeout: None,
        },
        ended_at: now,
        termination_reason: AttemptTerminationReason::AcceptedEos,
    };
    let facts = finalize_attempt_facts(
        &state,
        Some(&classified.readiness),
        Some(&classified.facts),
        &completion,
    );
    assert_eq!(
        facts.model_event.as_ref().unwrap().as_str(),
        "response_complete"
    );
    facts
}

#[tokio::test]
async fn nonstream_cache_write_evidence_reaches_gateway_completion_facts() {
    for ingress in [
        IngressProtocol::Responses,
        IngressProtocol::Messages,
        IngressProtocol::ChatCompletions,
    ] {
        for write in [0, 7] {
            let facts = completed_facts_with_write(ingress, true, Some(write)).await;
            assert!(facts.error_class.is_none(), "{ingress:?}: {facts:?}");
            assert_eq!(
                facts.usage.unwrap().cache_write,
                UsageDimension::reported(write)
            );
        }
    }
}

#[tokio::test]
async fn converted_nonstream_success_preserves_usage_after_decoder_consumption() {
    for ingress in [IngressProtocol::Responses, IngressProtocol::Messages] {
        let facts = completed_facts(ingress, true).await;
        assert!(facts.error_class.is_none(), "{ingress:?}: {facts:?}");
        let usage = facts.usage.unwrap();
        assert_eq!(usage.input, UsageDimension::reported(32));
        assert_eq!(usage.output, UsageDimension::reported(45));
        assert_eq!(usage.cache_read, UsageDimension::reported(0));
        assert_eq!(usage.reasoning, UsageDimension::reported(36));
        assert_eq!(usage.cache_write, UsageDimension::unknown());
        assert_eq!(usage.billable, UsageDimension::unknown());
    }
}

#[tokio::test]
async fn native_nonstream_success_keeps_the_same_reported_usage() {
    let facts = completed_facts(IngressProtocol::ChatCompletions, true).await;
    assert!(facts.error_class.is_none(), "{facts:?}");
    let usage = facts.usage.unwrap();
    assert_eq!(usage.input, UsageDimension::reported(32));
    assert_eq!(usage.output, UsageDimension::reported(45));
    assert_eq!(usage.cache_read, UsageDimension::reported(0));
    assert_eq!(usage.reasoning, UsageDimension::reported(36));
    assert_eq!(usage.cache_write, UsageDimension::unknown());
    assert_eq!(usage.billable, UsageDimension::unknown());
}

#[tokio::test]
async fn native_nonstream_success_without_reported_usage_does_not_invent_zeroes() {
    let facts = completed_facts(IngressProtocol::ChatCompletions, false).await;
    assert!(facts.error_class.is_none(), "{facts:?}");
    assert!(facts.usage.is_none());
}

#[tokio::test]
async fn converted_nonstream_without_required_usage_remains_a_protocol_failure() {
    for ingress in [IngressProtocol::Responses, IngressProtocol::Messages] {
        let facts = completed_facts(ingress, false).await;
        assert!(facts.error_class.is_some(), "{ingress:?}: {facts:?}");
        assert!(facts.usage.is_none());
    }
}
