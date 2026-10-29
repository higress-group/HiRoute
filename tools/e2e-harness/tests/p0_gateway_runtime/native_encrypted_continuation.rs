//! Native-shaped DSH retry history through the real listener and Replay path.
//! The tool result is a controlled receipt, not a claim that DSH ran an edit.
use super::*;
use hiroute_gateway::server::core_runtime::profiles::{
    CandidateProtocolProfile, Fidelity, NativeProviderStateEmission,
};
use hiroute_gateway::server::publication::GatewayPublicationSnapshotV3;
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SIMPLE: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
const ASSESSED_COMPLEX: &[u8] = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.1,"1":0.9}},"q1":{"type":"score","score":1.5}}}"#;
const A_USER: &str = "ACTUAL_A: edit the fixture and report completion";
const B_USER: &str = "ACTUAL_B: design the next independent task";
const COMMENTARY: &str = "I will edit the requested fixture.";
const TOOL_ERROR: &str = "Error: invalid justification: expected a non-empty sentence";
const SESSION: &str = "native-encrypted-retry";

#[test]
fn real_listener_encrypted_tool_retry_preserves_complete_stage_with_native_window() {
    assert_encrypted_tool_retry(true, false);
}

#[test]
fn real_listener_encrypted_tool_retry_preserves_complete_stage_without_origin_proof() {
    assert_encrypted_tool_retry(false, false);
}

#[test]
fn real_listener_final_phase_correction_preserves_tool_stage_and_assessment_members() {
    assert_encrypted_tool_retry(true, true);
}

fn reply(body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    }
}

pub(super) fn message(id: &str, phase: &str, text: &str) -> Value {
    json!({"type":"message","id":id,"role":"assistant","status":"completed",
        "phase":phase,"content":[{"type":"output_text","text":text,"annotations":[]}]})
}

fn edit(id: &str, justification: &str) -> Value {
    json!({"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":"edit",
        "status":"completed","arguments":json!({"file_path":"fixture.py",
            "old_string":"old","new_string":"new","justification":justification}).to_string()})
}

pub(super) fn streamed(id: &str, output: Vec<Value>) -> ProviderReply {
    streamed_phases(id, output, false)
}

fn streamed_phases(id: &str, output: Vec<Value>, correct_phase: bool) -> ProviderReply {
    let mut events = vec![json!({"type":"response.created","response":{
        "id":id,"model":"runtime-native","status":"in_progress","output":[]}})];
    for (index, item) in output.iter().enumerate() {
        let mut start = item.clone();
        start["status"] = json!("in_progress");
        match item["type"].as_str().unwrap() {
            "reasoning" => {
                start.as_object_mut().unwrap().remove("encrypted_content");
                start["summary"] = json!([]);
            }
            "message" => {
                start["content"] = json!([]);
                if correct_phase && item["phase"] == "commentary" {
                    // Derived provider-shaped regression: the done item remains
                    // authoritative and every item ID/body/tool byte is unchanged.
                    start["phase"] = json!("final_answer");
                }
            }
            "function_call" => {
                start["arguments"] = json!("");
            }
            other => panic!("unexpected fixture item {other}"),
        }
        events.push(json!({"type":"response.output_item.added","output_index":index,"item":start}));
        if item["type"] == "reasoning" {
            for (part_index, part) in item["summary"].as_array().unwrap().iter().enumerate() {
                let text = &part["text"];
                events.extend([
                    json!({"type":"response.reasoning_summary_part.added","output_index":index,
                        "item_id":item["id"],"summary_index":part_index,
                        "part":{"type":"summary_text","text":""}}),
                    json!({"type":"response.reasoning_summary_text.delta","output_index":index,
                        "item_id":item["id"],"summary_index":part_index,"delta":text}),
                    json!({"type":"response.reasoning_summary_text.done","output_index":index,
                        "item_id":item["id"],"summary_index":part_index,"text":text}),
                    json!({"type":"response.reasoning_summary_part.done","output_index":index,
                        "item_id":item["id"],"summary_index":part_index,"part":part}),
                ]);
            }
        } else if item["type"] == "message" {
            let text = &item["content"][0]["text"];
            events.extend([
                json!({"type":"response.output_text.delta","output_index":index,
                    "item_id":item["id"],"content_index":0,"delta":text}),
                json!({"type":"response.output_text.done","output_index":index,
                    "item_id":item["id"],"content_index":0,"text":text}),
            ]);
        } else if item["type"] == "function_call" {
            events.extend([
                json!({"type":"response.function_call_arguments.delta","output_index":index,
                    "item_id":item["id"],"delta":item["arguments"]}),
                json!({"type":"response.function_call_arguments.done","output_index":index,
                    "item_id":item["id"],"arguments":item["arguments"]}),
            ]);
        }
        events.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
    }
    events.push(json!({"type":"response.completed","response":{
        "id":id,"model":"runtime-native","status":"completed","output":output,
        "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}));
    let wire = events
        .into_iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect::<String>();
    ProviderReply::StreamDrip {
        status: 200,
        chunks: wire.as_bytes().chunks(257).map(<[u8]>::to_vec).collect(),
        interval: Duration::ZERO,
    }
}

fn enable_native_provider_state(fixture: &RuntimeFixture) {
    let mut publication = fixture.publication();
    configure_native_provider_state(&mut publication);
    publication.publication_revision += 1;
    fixture.publish(&mut publication);
}

pub(super) fn configure_native_provider_state(publication: &mut GatewayPublicationSnapshotV3) {
    for candidate in &mut publication.aliases[0].candidates {
        for profile in &mut candidate.protocol_profiles {
            let mut native: CandidateProtocolProfile =
                serde_json::from_value(serde_json::to_value(&*profile).unwrap()).unwrap();
            if native.ingress_protocol == IngressProtocol::Responses {
                native.capability.native_provider_state = NativeProviderStateEmission::Native;
                native.capability.request.provider_state = Fidelity::Exact;
                native.capability.response.provider_state = Fidelity::Exact;
                *profile = serde_json::from_value(serde_json::to_value(native).unwrap()).unwrap();
            }
        }
        candidate.protocol_profile_digest =
            hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    }
}

fn send(fixture: &RuntimeFixture, input: &[Value], turn: &str, proved: bool) -> Vec<Value> {
    let mut context = json!({"version":2,"agent":"dsh","purpose":"main",
        "session_id":SESSION,"turn_id":turn});
    if proved {
        let hashes = input
            .iter()
            .filter(|item| item["role"] == "user")
            .map(|item| {
                format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(&[item["content"].as_str().unwrap()]).unwrap()
                    )
                )
            })
            .collect::<Vec<_>>();
        let current = hashes.len() - 1;
        context["input"] = json!({"kind":"native_messages","event_id":turn,"coverage":"window",
            "messages":[{"index":current,"sha256":hashes[current]}],
            "scope_digest":format!("{:x}",Sha256::digest(serde_json::to_vec(&hashes).unwrap()))});
    }
    let context = context.to_string();
    // Larger than the fixture's Replay memory threshold and the aggregate IR
    // inline allowance. The small opaque leaf must survive production backing.
    let body = serde_json::to_vec(&json!({"model":MODEL,"stream":true,
        "instructions":"stable native instructions ".repeat(3072),"input":input,
        "tools":[{"type":"function","name":"edit","parameters":{"type":"object",
            "properties":{"file_path":{"type":"string"},"old_string":{"type":"string"},
                "new_string":{"type":"string"},"justification":{"type":"string"}}}}]}))
    .unwrap();
    assert!(body.len() > 64 * 1024);
    let response = request(
        fixture.address,
        "POST",
        "/v1/responses",
        &[
            ("X-HiRoute-Token", TOKEN),
            ("x-hiroute-agent", "dsh"),
            ("x-deepseek-harness-session-id", SESSION),
            ("x-hiroute-routing-context", &context),
        ],
        &body,
    );
    assert_eq!(
        response.status,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let events = std::str::from_utf8(&response.body)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|data| *data != "[DONE]")
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .collect::<Vec<_>>();
    assert!(
        !events.iter().any(|event| matches!(
            event["type"].as_str(),
            Some("error" | "response.failed" | "response.incomplete")
        )),
        "{events:?}"
    );
    let completed = events
        .iter()
        .filter(|event| event["type"] == "response.completed")
        .collect::<Vec<_>>();
    assert_eq!(completed.len(), 1, "{events:?}");
    assert_eq!(completed[0]["response"]["status"], "completed");
    completed[0]["response"]["output"]
        .as_array()
        .unwrap()
        .clone()
}

fn upstream_body(provider: &NativeProvider, index: usize) -> Value {
    let requests = provider.requests();
    let raw = &requests[index];
    let start = raw
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap()
        + 4;
    serde_json::from_slice(&raw[start..]).unwrap()
}

fn append_receipt(input: &mut Vec<Value>, delivered: Vec<Value>, receipt: &str) {
    let calls = delivered
        .iter()
        .filter(|item| item["type"] == "function_call")
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    let call_id = calls[0]["call_id"].clone();
    input.extend(delivered);
    input.push(json!({"type":"function_call_output","call_id":call_id,"output":receipt}));
}

fn assert_not_closed(fixture: &RuntimeFixture, count: usize) {
    let facts = wait_execution_facts(fixture, count);
    assert!(
        !facts.iter().any(|event| matches!(
            event["fact"]["kind"].as_str(),
            Some("agent_turn_finished" | "branch_assessment_recorded")
        )),
        "a completed HTTP/tool attempt is not a closed logical stage: {facts:?}"
    );
    let page = super::observed_history::timeline(&facts);
    assert_eq!(
        page.turns.len(),
        1,
        "no affinity hint must not split the admitted stage"
    );
    assert_eq!(page.turns[0].requests.len(), count);
    assert!(page.turns[0].agent_turn_id.is_some());
    assert!(page.turns[0].segment_id.is_none());
    assert_eq!(
        page.turns[0].status, "unknown",
        "completed tools await client continuation; they are not an active HTTP request or a completed task"
    );
}

fn assert_observed_retention(fixture: &RuntimeFixture, decisions: &[&Value], opaque: &str) {
    use base64::Engine as _;

    let path = fixture
        .observation_root
        .as_ref()
        .unwrap()
        .join("conversation-content.jsonl");
    // Execution and content persistence have independent workers. Observe every
    // response terminal before asserting absence, rather than an early snapshot.
    let deadline = Instant::now() + Duration::from_secs(5);
    let records = loop {
        let records = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        if decisions.iter().all(|decision| {
            records.iter().any(|row| {
                row["correlation"]["request_id"] == decision["correlation"]["request_id"]
                    && row["direction"] == "response_delivered"
                    && row["phase"] == "finish"
            })
        }) {
            break records;
        }
        assert!(Instant::now() < deadline, "response content did not finish");
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut response_parts = BTreeMap::<String, Vec<(u64, Vec<u8>)>>::new();
    let mut a2_opaque_parts = BTreeMap::<String, Vec<(u64, Vec<u8>)>>::new();
    for row in &records {
        assert_ne!(row["phase"], "abort", "content capture aborted: {row}");
        assert!(
            row["loss_watermark"].is_null(),
            "content capture lost records: {row}"
        );
        if row["direction"] == "response_delivered" {
            assert_ne!(row["content_kind"], "provider_state");
            assert!(!row.to_string().contains(opaque));
        }
        if row["phase"] != "append" {
            continue;
        }
        let parts = if row["direction"] == "response_delivered" {
            &mut response_parts
        } else if row["direction"] == "request_input"
            && row["correlation"]["request_id"] == decisions[1]["correlation"]["request_id"]
            && row["content_kind"] == "provider_state"
        {
            assert_eq!(row["canonical_media_type"], "application/json");
            &mut a2_opaque_parts
        } else {
            continue;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(row["canonical_bytes_base64"].as_str().unwrap())
            .unwrap();
        parts
            .entry(row["content_id"].as_str().unwrap().to_owned())
            .or_default()
            .push((row["chunk_ordinal"].as_u64().unwrap(), bytes));
    }
    let joined = |mut chunks: Vec<(u64, Vec<u8>)>| {
        chunks.sort_by_key(|(ordinal, _)| *ordinal);
        chunks
            .into_iter()
            .flat_map(|(_, bytes)| bytes)
            .collect::<Vec<_>>()
    };
    assert!(
        !response_parts.is_empty(),
        "response capture must actually be enabled"
    );
    for chunks in response_parts.into_values() {
        let bytes = joined(chunks);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !text.contains("provider_state")
                && !text.contains("encrypted_content")
                && !text.contains(opaque),
            "accepted private evidence leaked into response content: {text}"
        );
    }
    assert_eq!(
        a2_opaque_parts.len(),
        1,
        "A2 must capture its actual replayed input state"
    );
    let bytes = joined(a2_opaque_parts.into_values().next().unwrap());
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap(),
        json!(opaque),
        "request capture must resolve Replay bytes, not retain the locator"
    );
    assert!(records.iter().any(|row| row["direction"] == "request_input"
        && row["correlation"]["request_id"] == decisions[1]["correlation"]["request_id"]
        && row["phase"] == "finish"));
}

fn assert_encrypted_tool_retry(proved: bool, correct_phase: bool) {
    let opaque = "synthetic-opaque-state-".repeat(121);
    let regular = NativeProvider::start(vec![
        streamed_phases(
            "a-edit-error",
            vec![
                json!({"type":"reasoning","id":"rs-a","status":"completed","summary":[],"encrypted_content":opaque}),
                message("a-commentary", "commentary", COMMENTARY),
                edit("edit-invalid", ""),
            ],
            correct_phase,
        ),
        streamed(
            "a-edit-retry",
            vec![edit("edit-retry", "The requested fixture must be updated.")],
        ),
        streamed(
            "a-final",
            vec![message("a-final-message", "final_answer", "A_COMPLETE")],
        ),
    ]);
    let primary = NativeProvider::start(vec![streamed(
        "b-final",
        vec![message("b-final-message", "final_answer", "B_COMPLETE")],
    )]);
    let classifier = NativeProvider::start(vec![reply(SIMPLE), reply(ASSESSED_COMPLEX)]);
    let fixture = RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &classifier], true);
    enable_native_provider_state(&fixture);
    let mut input = vec![json!({"role":"user","content":A_USER})];
    let first = send(&fixture, &input, "a", proved);
    let reasoning = first
        .iter()
        .find(|item| item["type"] == "reasoning")
        .unwrap();
    assert_eq!(reasoning["summary"], json!([]));
    assert_eq!(reasoning["encrypted_content"], opaque);
    let text = first.iter().find(|item| item["type"] == "message").unwrap();
    assert_eq!(text["phase"], "commentary");
    assert_eq!(text["content"][0]["text"], COMMENTARY);
    assert_not_closed(&fixture, 1);
    append_receipt(&mut input, first, TOOL_ERROR);

    let retry = send(&fixture, &input, "a", proved);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (2, 0, 1),
        "the accepted opaque/tool edge must inherit A even when origin is unproved"
    );
    let forwarded = upstream_body(&regular, 1);
    let forwarded_reasoning = forwarded["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "reasoning")
        .unwrap();
    assert_eq!(forwarded_reasoning["encrypted_content"], opaque);
    assert_eq!(forwarded_reasoning["summary"], json!([]));
    assert!(forwarded["input"].to_string().contains(TOOL_ERROR));
    assert!(!forwarded.to_string().contains("__hiroute_content_ref"));
    assert_not_closed(&fixture, 2);
    append_receipt(&mut input, retry, "Edit applied successfully.");

    let final_output = send(&fixture, &input, "a", proved);
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (3, 0, 1)
    );
    assert_eq!(final_output[0]["phase"], "final_answer");
    assert_eq!(final_output[0]["content"][0]["text"], "A_COMPLETE");
    let before_b = wait_execution_facts(&fixture, 3);
    assert!(
        !before_b
            .iter()
            .any(|event| event["fact"]["kind"] == "branch_assessment_recorded")
    );
    let closed_before_b = before_b
        .iter()
        .filter(|event| event["fact"]["kind"] == "agent_turn_finished")
        .collect::<Vec<_>>();
    assert_eq!(closed_before_b.len(), 1);
    assert_eq!(closed_before_b[0]["fact"]["status"], "completed");
    assert_eq!(
        closed_before_b[0]["fact"]["history_membership"]["request_ids"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "the delivered final closes all A requests before any new input arrives"
    );
    input.extend(final_output);
    input.push(json!({"role":"user","content":B_USER}));
    let b = send(&fixture, &input, "b", proved);
    assert_eq!(b[0]["content"][0]["text"], "B_COMPLETE");
    assert_eq!(
        (regular.calls(), primary.calls(), classifier.calls()),
        (3, 1, 2)
    );

    // Facts: every actual request/attempt is retained, irrespective of origin
    // proof or whether the native tool returned an error.
    let facts = wait_execution_facts(&fixture, 4);
    let page = super::observed_history::timeline(&facts);
    assert_eq!(page.turns.len(), 2);
    assert_eq!(page.turns[0].requests.len(), 3);
    assert_eq!(page.turns[1].requests.len(), 1);
    assert_eq!(page.turns[0].status, "completed");
    assert_eq!(page.turns[0].assessment.as_ref().unwrap().score, 0.75);
    assert_single_planner_run_per_request(&facts, 4);
    let decisions = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "route_decision")
        .collect::<Vec<_>>();
    assert_observed_retention(&fixture, &decisions, &opaque);
    let sources = decisions
        .iter()
        .map(|event| {
            event["fact"]["complexity"]["decision_source"]
                .as_str()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sources,
        [
            "external_classifier",
            "inherited",
            "inherited",
            "external_classifier"
        ]
    );
    for (index, decision) in decisions.iter().enumerate() {
        assert_eq!(decision["fact"]["complexity"]["fallback_used"], false);
        assert_eq!(
            decision["fact"]["complexity"]["execution_group"],
            if index < 3 { "regular" } else { "primary" }
        );
        let affinity_applied = decision["fact"]["reason_ledger"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason["code"] == "CONTEXT_HOLD_APPLIED");
        if index == 1 {
            assert!(
                !affinity_applied,
                "the opaque response edge cannot grant candidate affinity"
            );
        } else if index == 2 {
            // The next response is an independently captured ordinary tool
            // boundary. Earlier opaque history must not disable affinity forever.
            assert!(
                affinity_applied,
                "later exact tool evidence recovers affinity"
            );
        }
        let request_id = &decision["correlation"]["request_id"];
        let attempts = facts
            .iter()
            .filter(|event| {
                event["fact"]["kind"] == "attempt_finished"
                    && &event["correlation"]["request_id"] == request_id
            })
            .collect::<Vec<_>>();
        assert_eq!(attempts.len(), 1);
        assert_eq!(
            attempts[0]["fact"]["provider_model_event"],
            "response_complete"
        );
    }
    let members = json!(
        decisions[..3]
            .iter()
            .map(|event| event["correlation"]["request_id"].clone())
            .collect::<Vec<_>>()
    );

    // Association: exact delivered opaque bytes allow contextual continuity;
    // source uncertainty stays explicit and does not erase complete capture.
    let a_stages = facts
        .iter()
        .filter(|event| {
            event["fact"]["kind"] == "agent_turn_finished"
                && event["fact"]["history_membership"]["request_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|id| members.as_array().unwrap().contains(id))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        a_stages.len(),
        1,
        "there must be no prematurely sealed A prefix: {a_stages:?}"
    );
    let stage = &a_stages[0]["fact"];
    assert_eq!(stage, &closed_before_b[0]["fact"]);
    assert_eq!(stage["history_membership"]["request_ids"], members);
    assert_eq!(stage["history_membership"]["relation"], "contextual");
    assert_eq!(stage["history_membership"]["association_unverified"], true);
    if !proved {
        assert_eq!(stage["history_membership"]["source_unverified"], true);
    }
    assert_eq!(stage["history_membership"]["capture_partial"], false);
    assert_eq!(stage["history_membership"]["clipped"], false);
    assert_eq!(stage["branch_execution"]["group"], "regular");
    assert!(stage["branch_execution"]["policy"]["criteria_digest"].is_string());

    // Closure and assessment: only the delivered final ends A. B freezes all
    // three actual A executions with the same Plan, profile and rubric target.
    assert_eq!(stage["status"], "completed");
    assert_eq!(stage["history_partial"], false);
    assert_eq!(stage["first_request_id"], members[0]);
    assert_eq!(stage["last_request_id"], members[2]);
    let scores = facts
        .iter()
        .filter(|event| event["fact"]["kind"] == "branch_assessment_recorded")
        .collect::<Vec<_>>();
    assert_eq!(scores.len(), 1);
    assert_eq!(
        scores[0]["correlation"]["request_id"],
        decisions[3]["correlation"]["request_id"]
    );
    let score = &scores[0]["fact"];
    assert_eq!(score["history_membership"], stage["history_membership"]);
    for key in [
        "segment_id",
        "plan_id",
        "plan_revision",
        "model_configuration_id",
        "profile_digest",
    ] {
        assert!(!stage[key].is_null(), "missing {key}");
        assert_eq!(score[key], stage[key], "assessment changed {key}");
    }
    assert_eq!(score["partial"], false);
    assert_eq!(score["score"], 0.75);
    let envelope = upstream_body(&classifier, 1);
    let assessed = &envelope["state"];
    assert_eq!(assessed["history_partial"], false, "{assessed}");
    assert_eq!(assessed["assessment_from"], 0);
    let question = &envelope["questions"]["q1"];
    let instructions = "Evaluate actual smart_saving execution";
    assert!(
        question["instructions"]
            .as_str()
            .unwrap()
            .ends_with(instructions)
    );
    let criteria = question["criteria"]
        .as_array()
        .unwrap()
        .iter()
        .map(|criterion| criterion.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        criteria,
        [
            "smart_saving failed",
            "smart_saving partly competent",
            "smart_saving competent"
        ]
    );
    let rubric_digest = hiroute_domain::CanonicalDigest::of(&(instructions, criteria)).unwrap();
    assert_eq!(
        stage["branch_execution"]["policy"]["criteria_digest"],
        serde_json::to_value(rubric_digest).unwrap(),
        "B must send the rubric frozen for A's actual execution"
    );
    assert!(assessed["latest_user"].to_string().contains(B_USER));
    let turns = assessed["visible_conversation"].as_array().unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0]["status"], "completed");
    assert!(turns[0]["user"].to_string().contains(A_USER));
    assert_eq!(
        turns[0]["steps"],
        json!([
            [{"kind":"text","text":COMMENTARY},{"kind":"tool_activity","tool":"edit","status":"unknown"}],
            [{"kind":"tool_activity","tool":"edit","status":"unknown"}],
            [{"kind":"text","text":"A_COMPLETE"}]
        ])
    );
    assert!(!assessed.to_string().contains(&opaque));
    assert!(!assessed.to_string().contains(TOOL_ERROR));
}

#[path = "cross_protocol_encrypted_continuation.rs"]
mod cross_protocol_encrypted_continuation;
