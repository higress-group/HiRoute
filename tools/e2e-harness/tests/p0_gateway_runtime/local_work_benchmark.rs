//! Opt-in whole-listener measurements; never substitute a codec-only timing.
use super::*;
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

const CHAT_STREAM: &[u8] = b"data: {\"id\":\"bench\",\"model\":\"runtime-native-model-1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"42\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"bench\",\"model\":\"runtime-native-model-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\ndata: [DONE]\n\n";

#[test]
#[ignore = "release whole-listener performance experiment, not a unit timing gate"]
fn measure_complete_local_work_with_continuity_and_capture() {
    let measured = std::env::var("HIROUTE_LOCAL_WORK_SAMPLES")
        .ok()
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(100)
        .max(2)
        .next_multiple_of(2);
    let warmup = 10;
    let count = measured + warmup;
    let wire: Value = match std::env::var("HIROUTE_LOCAL_WORK_INPUT") {
        Ok(path) => serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(),
        Err(_) => json!({"messages": (0..384).map(|index| json!({
            "role":if index % 2 == 0 {"user"} else {"assistant"},
            "content":format!("message {index}: {}", "a路由🦀\\\"\n".repeat(48))
        })).chain(std::iter::once(json!({"role":"user","content":"Return 42."}))).collect::<Vec<_>>() }),
    };
    let original = wire["messages"].as_array().expect("Chat messages");
    for (variant, threshold) in [("memory", 1024 * 1024), ("disk", 16 * 1024)] {
        let regular = NativeProvider::start(vec![
            ProviderReply::StreamComplete {
                status: 200,
                body: CHAT_STREAM
            };
            count
        ]);
        let primary = NativeProvider::start(vec![]);
        let plain = br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#;
        let decider = NativeProvider::start(
            (0..count)
                .map(|index| ProviderReply::Complete {
                    status: 200,
                    error_kind: None,
                    body: if index % 2 == 0 {
                        plain
                    } else {
                        decision_branches::c_answer(true, false)
                    },
                })
                .collect(),
        );
        let fixture =
            RuntimeFixture::launch_local_work_benchmark(&[&regular, &primary, &decider], threshold);
        let mut publication = fixture.publication();
        for (candidate, provider) in publication.aliases[0]
            .candidates
            .iter_mut()
            .zip([&regular, &primary])
        {
            *candidate = hiroute_e2e::gateway_fixture::sealed_native_candidate(
                candidate.local_id,
                &candidate.stable_target_key,
                &candidate.credential_refs,
                provider.authority(),
                &candidate.upstream_model_id,
                &[
                    (IngressProtocol::Responses, IngressProtocol::ChatCompletions),
                    (
                        IngressProtocol::ChatCompletions,
                        IngressProtocol::ChatCompletions,
                    ),
                    (IngressProtocol::Messages, IngressProtocol::ChatCompletions),
                ],
            );
        }
        publication
            .grants
            .iter_mut()
            .find(|grant| grant.protocol == IngressProtocol::ChatCompletions)
            .unwrap()
            .consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes);
        decision_branches::enable_c_in_publication(&mut publication);
        fixture.publish(&mut publication);
        let mut wire_bytes = 0;
        for index in 0..count {
            let mut input = wire.clone();
            let mut history = original.clone();
            if index % 2 == 1 {
                history.extend([
                    json!({"role":"assistant","content":"42"}),
                    json!({"role":"user","content":"Repeat the answer."}),
                ]);
            }
            input["messages"] = json!(history);
            input["model"] = json!(MODEL);
            input["stream"] = json!(true);
            let body = serde_json::to_vec(&input).unwrap();
            wire_bytes = body.len();
            let session = format!("local-work-{variant}-{}", index / 2);
            let context = json!({"version":1,"agent":"hermes","session_id":session,
                "purpose":"main","turn_id":format!("turn-{index}"),
                "continuation_requires_existing_turn":false})
            .to_string();
            let response = request(
                fixture.address,
                "POST",
                "/v1/chat/completions",
                &[
                    ("X-HiRoute-Token", CHAT_TOKEN),
                    ("x-hermes-session-id", &session),
                    ("x-hiroute-agent", "hermes"),
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
            assert!(String::from_utf8_lossy(&response.body).contains("[DONE]"));
            // End drained for each reference cell. Async work stays outside
            // foreground timings, but missing capture must fail the experiment.
            let records = request_timing::summaries(&fixture, index + 1);
            let timing = &records.last().unwrap()["event"]["request_timing_summary"];
            assert_eq!(timing["asynchronous"]["dropped"], 0, "{timing}");
            if let Some(dropped) = timing["asynchronous"]["capture_dropped_jobs"].as_u64() {
                assert_eq!(dropped, 0, "{timing}");
            }
        }
        let records = request_timing::summaries(&fixture, count);
        let timings = records
            .iter()
            .skip(warmup)
            .map(|record| &record["event"]["request_timing_summary"]);
        for (index, timing) in timings.enumerate() {
            let local_us: u64 = timing["local"]
                .as_object()
                .unwrap()
                .values()
                .map(|value| value.as_u64().unwrap())
                .sum();
            let complete_local_us = local_us + timing["wait"]["local_work_us"].as_u64().unwrap();
            println!(
                "local_work_benchmark {}",
                json!({"variant":variant,"sample":index,
                "wire_bytes":wire_bytes,"messages":original.len(),"complete_local_us":complete_local_us,
                "timing":timing})
            );
        }
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (count, 0, count)
        );
    }
}
