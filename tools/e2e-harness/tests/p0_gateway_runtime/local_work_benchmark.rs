//! Opt-in whole-listener measurements; never substitute a codec-only timing.
use super::*;
use hiroute_gateway::server::request_plan::IngressProtocol;
use serde_json::{Value, json};

#[path = "local_work_membership.rs"]
mod membership;

const CHAT_STREAM: &[u8] = b"data: {\"id\":\"bench\",\"model\":\"runtime-native-model-1\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"42\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"bench\",\"model\":\"runtime-native-model-1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\ndata: [DONE]\n\n";

#[cfg(target_os = "linux")]
fn process_usage(pid: u32) -> Option<(u64, u64)> {
    let root = std::path::PathBuf::from(format!("/proc/{pid}"));
    let stat = std::fs::read_to_string(root.join("stat")).unwrap();
    let fields = stat
        .rsplit_once(") ")
        .unwrap()
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let cpu = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    let status = std::fs::read_to_string(root.join("status")).unwrap();
    let peak_rss = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    Some((cpu, peak_rss))
}

#[cfg(not(target_os = "linux"))]
fn process_usage(_pid: u32) -> Option<(u64, u64)> {
    None
}

#[test]
#[ignore = "release whole-listener performance experiment, not a unit timing gate"]
fn measure_complete_local_work_with_continuity_and_capture() {
    let concurrency = std::env::var("HIROUTE_LOCAL_WORK_CONCURRENCY")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(1);
    assert!(matches!(concurrency, 1 | 2));
    let measured = std::env::var("HIROUTE_LOCAL_WORK_SAMPLES")
        .ok()
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(100)
        .max(2)
        .next_multiple_of(2 * concurrency);
    let warmup = if concurrency == 1 { 10 } else { 12 };
    let wire: Value = match std::env::var("HIROUTE_LOCAL_WORK_INPUT") {
        Ok(path) => serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap(),
        Err(_) => json!({"messages": (0..384).map(|index| json!({
            "role":if index % 2 == 0 {"user"} else {"assistant"},
            "content":format!("message {index}: {}", "a路由🦀\\\"\n".repeat(48))
        })).chain(std::iter::once(json!({"role":"user","content":"Return 42."}))).collect::<Vec<_>>() }),
    };
    let backing = std::env::var("HIROUTE_LOCAL_WORK_BACKING").ok();
    run_reference(&wire, measured, warmup, backing.as_deref(), concurrency);
}

#[test]
fn local_work_reference_assesses_each_sessions_actual_predecessor() {
    let wire = json!({"messages":[
        {"role":"user","content":"Return 42."}
    ]});
    run_reference(&wire, 4, 0, Some("memory"), 1);
}

#[test]
fn concurrent_local_work_reference_assesses_each_actual_session_predecessor() {
    let wire = json!({"messages":[{"role":"user","content":"Return 42."}]});
    run_reference(&wire, 8, 0, Some("memory"), 2);
}

#[test]
fn concurrent_tool_rich_history_preserves_capture_and_actual_predecessors() {
    // Two ordinary requests may finish parsing before the sole optional
    // encoder takes its first snapshot. Exercise that pressure with public,
    // synthetic tool schemas rather than relying on a private conversation.
    let properties = (0..12)
        .map(|index| {
            (
                format!("field_{index}"),
                json!({"type":"string","description":"A bounded tool argument."}),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let wire = json!({
        "messages":(0..237).map(|index| json!({
            "role":if index % 2 == 0 {"user"} else {"assistant"},
            "content":format!("message {index}: {}", "a路由🦀\\\"\n".repeat(80))
        })).collect::<Vec<_>>(),
        "tools":(0..21).map(|index| json!({
            "type":"function","function":{
                "name":format!("tool_{index}"),"description":"Read-only synthetic tool.",
                "parameters":{"type":"object","properties":properties}
            }
        })).collect::<Vec<_>>()
    });
    run_reference(&wire, 8, 0, Some("memory"), 2);
}

fn is_followup(index: usize, concurrency: usize) -> bool {
    index % (2 * concurrency) >= concurrency
}

fn run_reference(
    wire: &Value,
    measured: usize,
    warmup: usize,
    backing: Option<&str>,
    concurrency: usize,
) {
    let count = measured + warmup;
    let original = wire["messages"].as_array().expect("Chat messages");
    // Freeze the shared stock protocol for before/after measurements; a later
    // product default must not change the workload or reject the older binary.
    let protocol = hiroute_domain::NativeProtocolV1::c_v1();
    assert!(backing.is_none_or(|v| matches!(v, "memory" | "disk")));
    for (variant, threshold) in [("memory", 1024 * 1024), ("disk", 16 * 1024)] {
        if backing.is_some_and(|value| value != variant) {
            continue;
        }
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
                    body: if !is_followup(index, concurrency) {
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
        decision_branches::enable_c_in_publication_with(&mut publication, &protocol);
        fixture.publish(&mut publication);
        let mut wire_sizes = Vec::with_capacity(count);
        let mut resource_start = None;
        for wave in (0..count).step_by(concurrency) {
            if wave == warmup {
                resource_start = process_usage(fixture.process.id());
            }
            let address = fixture.address;
            let sizes = std::thread::scope(|scope| {
                let handles = (wave..wave + concurrency)
                    .map(|index| {
                        scope.spawn(move || {
                            let mut input = wire.clone();
                            let mut history = original.clone();
                            if is_followup(index, concurrency) {
                                history.extend([
                                    json!({"role":"assistant","content":"42"}),
                                    json!({"role":"user","content":"Repeat the answer."}),
                                ]);
                            }
                            input["messages"] = json!(history);
                            input["model"] = json!(MODEL);
                            input["stream"] = json!(true);
                            let body = serde_json::to_vec(&input).unwrap();
                            let body_len = body.len();
                            let pair =
                                index / (2 * concurrency) * concurrency + index % concurrency;
                            let session = format!("local-work-{variant}-{pair}");
                            let context = json!({"version":1,"agent":"hermes","session_id":session,
                "purpose":"main","turn_id":format!("turn-{index}"),
                "continuation_requires_existing_turn":false})
                            .to_string();
                            let response = request(
                                address,
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
                            body_len
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| handle.join().unwrap())
                    .collect::<Vec<_>>()
            });
            wire_sizes.extend(sizes);
            // End drained for each reference cell. Async work stays outside
            // foreground timings, but missing capture must fail the experiment.
            let records = request_timing::summaries(&fixture, wave + concurrency);
            for record in &records[wave..wave + concurrency] {
                let timing = &record["event"]["request_timing_summary"];
                assert_eq!(timing["asynchronous"]["dropped"], 0, "{timing}");
                if let Some(dropped) = timing["asynchronous"]["capture_dropped_jobs"].as_u64() {
                    assert_eq!(dropped, 0, "{timing}");
                }
            }
        }
        if let Some((cpu_before, _)) = resource_start {
            let (cpu_after, peak_rss_kib) = process_usage(fixture.process.id()).unwrap();
            let tick_rate = std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()
                .unwrap();
            assert!(tick_rate.status.success());
            let ticks_per_second: u64 = std::str::from_utf8(&tick_rate.stdout)
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            println!(
                "local_work_resources {}",
                json!({
                    "variant":variant,"native_protocol":protocol.schema,
                    "samples":measured,"concurrency":concurrency,"process_cpu_ticks":cpu_after-cpu_before,
                    "clock_ticks_per_second":ticks_per_second,"peak_rss_kib":peak_rss_kib,
                    "includes_async_capture_cpu":true
                })
            );
        }
        let records = request_timing::summaries(&fixture, count);
        let facts = wait_execution_facts(&fixture, count);
        if concurrency == 1 {
            membership::verify_pairs(&facts, count)
        } else {
            membership::verify_scoped_pairs(&facts, count)
        }
        .expect("each follow-up must assess its own complete captured predecessor");
        let diagnostic_bytes = std::fs::read(
            fixture
                .diagnostics_root
                .as_ref()
                .unwrap()
                .join("daemon/current.jsonl"),
        )
        .unwrap();
        let capture_backing: Vec<_> = request_timing::complete_records(&diagnostic_bytes)
            .unwrap()
            .into_iter()
            .filter_map(|record| record["event"].get("spill_begin").cloned())
            .map(|event| event["mode"].clone())
            .collect();
        assert_eq!(
            capture_backing.len(),
            count,
            "one input capture per drained request"
        );
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
                json!({"variant":variant,"sample":index,"concurrency":concurrency,
                "turn": if is_followup(index, concurrency) { "followup" } else { "new_session" },
                "replay_threshold_bytes":threshold,
                "native_protocol":protocol.schema,
                "capture_backing":capture_backing[index + warmup],
                "wire_bytes":wire_sizes[index + warmup],"messages":original.len() + usize::from(is_followup(index, concurrency)) * 2,"complete_local_us":complete_local_us,
                "timing":timing})
            );
        }
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (count, 0, count)
        );
    }
}
