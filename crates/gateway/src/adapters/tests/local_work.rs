//! Byte-equivalence at encoder boundaries and an opt-in, fixed-input benchmark.
use super::*;
use crate::content_ref::{
    compact_ingress_document_with_markers, parse_ingress_document, scan_ingress_document,
};
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use hiroute_gateway_core::runtime::body::StreamBudget;
use std::time::{Duration, Instant};

struct Fixture {
    store: ReplayStore,
    budget: StreamBudget,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new(threshold: usize, record_bytes: usize) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hiroute-local-work-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: threshold,
            record_bytes,
            orphan_ttl: Duration::from_secs(60),
        })
        .unwrap();
        let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
            .unwrap()
            .stream(8 * 1024 * 1024)
            .unwrap();
        Self {
            store: manager.begin_request(budget.clone()).unwrap(),
            budget,
            root,
        }
    }

    fn parse(&self, body: &[u8]) -> ModelRequestIRV1 {
        self.parse_protocol(IngressProtocol::Responses, body)
    }

    fn parse_protocol(&self, protocol: IngressProtocol, body: &[u8]) -> ModelRequestIRV1 {
        let mut raw = self.store.begin_raw().unwrap();
        for chunk in body.chunks(16 * 1024) {
            raw.append(chunk).unwrap();
        }
        let raw = raw.seal().unwrap();
        let stats = scan_ingress_document(self.store.reader(&raw).unwrap()).unwrap();
        let (mut document, mut workspace) =
            parse_ingress_document(protocol, &self.store, &raw, stats).unwrap();
        compact_ingress_document_with_markers(
            protocol,
            &mut document,
            &self.store,
            workspace.generated_markers(),
        )
        .unwrap();
        let mut request =
            decode_ingress_request_with_replay(protocol, &document, Some(&self.store)).unwrap();
        externalize_model_request(&mut request, &self.store, 8).unwrap();
        request
    }
}

#[test]
fn owned_ingress_keeps_protocol_meaning_and_moves_the_original_native_tree() {
    for (protocol, wire) in [
        (
            IngressProtocol::Responses,
            json!({"model":"alias","input":[{"role":"user","content":"hello"}],"client_metadata":{"session_id":"native-session"}}),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"model":"alias","messages":[{"role":"user","content":"hello","message_id":"platform-row"}]}),
        ),
        (
            IngressProtocol::Messages,
            json!({"model":"alias","max_tokens":64,"messages":[{"role":"user","content":"hello"}],"metadata":{"user_id":"{\"session_id\":\"native-session\"}"}}),
        ),
    ] {
        let fixture = Fixture::new(1024 * 1024, 16 * 1024);
        let expected = decode_ingress_request(protocol, &wire).unwrap();
        // Vec/String storage, rather than the stack address of the moved map,
        // witnesses that this path retained the original native allocation.
        let model = wire["model"].as_str().unwrap().as_ptr();
        let identity = crate::context_hold::context_document(&wire);
        let actual =
            super::super::decode_owned_ingress_request_with_replay(protocol, wire, &fixture.store)
                .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            actual.native_body.as_ref().unwrap()["model"]
                .as_str()
                .unwrap()
                .as_ptr(),
            model
        );
        assert_eq!(
            crate::context_hold::context_document(actual.native_body.as_ref().unwrap()),
            identity
        );
    }
}

#[test]
fn borrowed_wire_json_matches_the_existing_owned_expansion_exactly() {
    use crate::content_ref::{ContentRef, JsonValueExt, borrowed_wire_value};
    let reference = ContentRef::new(7, 13, 29, 31);
    let marker = reference.json_marker();
    let body = json!({
        "z":[null,true,-17,1.25,"escaped \"\\\n路由🦀",marker],
        "a":{"nested":reference.json_marker(),"literal":reference.wire_marker()},
        // Additional keys make this ordinary JSON, not a locator object.
        "ordinary":{"__hiroute_json_content_ref_v2":reference.wire_marker(),"other":1},
    });
    for body in [body, reference.json_marker(), json!("plain"), json!([])] {
        assert_eq!(
            serde_json::to_vec(&borrowed_wire_value(&body)).unwrap(),
            serde_json::to_vec(&body.wire_value()).unwrap(),
        );
    }
}

#[test]
fn local_work_protocol_oracle_preserves_complete_text_and_tool_history() {
    let text = format!("{}\"\\\n中文🦀{}", "a".repeat(24_577), "z".repeat(16_385));
    // Ingress already canonicalizes valid tool JSON. Keep this fixture in
    // canonical key order so byte comparison measures text preservation.
    let args = json!({"empty":"", "nested":[null,true, {"n":17}], "text":text});
    let arguments = serde_json::to_string(&args).unwrap();
    for (protocol, wire) in [
        (
            IngressProtocol::Responses,
            json!({"model":"alias", "tools":[{"type":"function","name":"lookup","parameters":{"type":"object"}}], "input":[
                {"role":"user","content":text},
                {"type":"function_call","call_id":"call-1","name":"lookup","arguments":arguments},
                {"type":"function_call_output","call_id":"call-1","output":text}
            ]}),
        ),
        (
            IngressProtocol::ChatCompletions,
            json!({"model":"alias", "tools":[{"type":"function","function":{"name":"lookup","parameters":{"type":"object"}}}], "messages":[
                {"role":"user","content":text},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"lookup","arguments":arguments}}]},
                {"role":"tool","tool_call_id":"call-1","content":text}
            ]}),
        ),
        (
            IngressProtocol::Messages,
            json!({"model":"alias","max_tokens":128,"tools":[{"name":"lookup","input_schema":{"type":"object"}}],"messages":[
                {"role":"user","content":text},
                {"role":"assistant","content":[{"type":"tool_use","id":"call-1","name":"lookup","input":args}]},
                {"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":text}]}
            ]}),
        ),
    ] {
        // The oracle bypasses ingress parsing and Replay. Neither expected
        // content nor protocol meaning is derived from the optimized path.
        let plain = decode_ingress_request(protocol, &wire).unwrap();
        let profile = CandidateProtocolProfile::exact_portable_path(
            protocol,
            protocol,
            "physical",
            fixed_reasoning("fixed"),
        );
        let expected = project_candidate_request(&plain, &profile).unwrap().body;
        for threshold in [1024 * 1024, 31] {
            let fixture = Fixture::new(threshold, 31);
            let request = fixture.parse_protocol(protocol, &serde_json::to_vec(&wire).unwrap());
            let template = project_candidate_request_template(&request, &profile).unwrap();
            fixture
                .store
                .prevalidate(&model_content_refs(&request))
                .unwrap();
            let encoded = encode(&fixture, template, 127);
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&encoded).unwrap(),
                expected
            );
            assert!(fixture.budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn encode(fixture: &Fixture, template: PreparedNativeTemplate, quantum: usize) -> Vec<u8> {
    let mut reader =
        sequential_attempt_body(template, fixture.store.clone(), &fixture.budget, quantum).unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = reader.next_chunk().unwrap() {
        assert!(chunk.bytes().len() <= quantum);
        bytes.extend_from_slice(chunk.bytes());
    }
    reader.release();
    assert!(reader.next_chunk().unwrap().is_none());
    bytes
}

#[test]
fn sequential_bulk_runs_preserve_all_escapes_and_empty_parts_at_chunk_boundaries() {
    let controls: String = (0..32).map(char::from).collect();
    let text = format!(
        "{}\"\\{controls}路由🦀{}",
        "a".repeat(8191),
        "z".repeat(8193)
    );
    let wire = json!({"model":"alias", "tools":[{"type":"function","name":"test","parameters":{"type":"object"}}], "input":[
        {"role":"user","content":[{"type":"input_text","text":text}, {"type":"input_text","text":""}]},
        {"type":"function_call", "call_id":"call-1", "name":"test", "arguments":serde_json::to_string(&json!({"text":text})).unwrap()},
        {"type":"function_call_output", "call_id":"call-1", "output":{"text":text, "empty":""}}
    ]});
    let canonical = decode_ingress_request(IngressProtocol::Responses, &wire).unwrap();
    for threshold in [31, 1024 * 1024] {
        let fixture = Fixture::new(threshold, 31);
        let mut request = canonical.clone();
        externalize_model_request(&mut request, &fixture.store, 0).unwrap();
        fixture
            .store
            .prevalidate(&model_content_refs(&request))
            .unwrap();
        for target in [
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            IngressProtocol::Messages,
        ] {
            let profile = CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                target,
                "physical",
                fixed_reasoning("fixed"),
            );
            let expected = project_candidate_request(&canonical, &profile)
                .unwrap()
                .body;
            let template = project_candidate_request_template(&request, &profile).unwrap();
            for quantum in [1, 2, 5, 31, 8192, 16384] {
                let live = fixture.budget.snapshot().unwrap().live;
                let actual = encode(&fixture, template.clone(), quantum);
                assert_eq!(actual.len(), template.wire_len);
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&actual).unwrap(),
                    expected
                );
                assert_eq!(fixture.budget.snapshot().unwrap().live, live);
            }
        }
    }
}

#[test]
fn buffered_ingress_still_rejects_trailing_invalid_and_truncated_json() {
    for threshold in [31, 1024 * 1024] {
        let fixture = Fixture::new(threshold, 31);
        let mut valid = fixture.store.begin_raw().unwrap();
        valid.append(br#"{"model":"alias","input":"ok"}"#).unwrap();
        let valid = valid.seal().unwrap();
        let stats = scan_ingress_document(fixture.store.reader(&valid).unwrap()).unwrap();
        for bytes in [
            br#"{"model":"alias","input":"ok"} {}"#.as_slice(),
            br#"{"model":"alias","input":"\q"}"#,
            br#"{"model":"alias","input":"ok""#,
        ] {
            let mut raw = fixture.store.begin_raw().unwrap();
            raw.append(bytes).unwrap();
            let raw = raw.seal().unwrap();
            let live = fixture.budget.snapshot().unwrap().live;
            assert!(
                parse_ingress_document(IngressProtocol::Responses, &fixture.store, &raw, stats)
                    .is_err()
            );
            assert_eq!(fixture.budget.snapshot().unwrap().live, live);
        }
    }
}

#[test]
fn prepared_candidate_reuse_keeps_bytes_and_rejects_profile_or_cleanup_changes() {
    use crate::server::composition::RequestTemplates;
    let wire =
        json!({"model":"alias","input":[{"role":"user","content":"text 中文".repeat(4096)}]});
    for (candidate, digest, cleanup, hit) in [
        ("selected", "profile", None, true),
        ("other", "profile", None, false),
        ("selected", "changed", None, false),
        ("selected", "profile", Some(1), false),
    ] {
        let fixture = Fixture::new(1024 * 1024, 16 * 1024);
        let request = fixture.parse(&serde_json::to_vec(&wire).unwrap());
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        let template = project_candidate_request_template(&request, &profile).unwrap();
        let bytes = template.bytes.clone();
        let before = fixture.budget.snapshot().unwrap().live;
        let mut templates = RequestTemplates::new(fixture.budget.clone());
        templates.insert("selected", "profile", template);
        assert!(fixture.budget.snapshot().unwrap().live > before);
        let cached = templates.take(candidate, digest, cleanup);
        assert_eq!(cached.is_some(), hit);
        drop(templates);
        assert_eq!(fixture.budget.snapshot().unwrap().live, before);
        if let Some(template) = cached {
            assert!(std::sync::Arc::ptr_eq(&bytes, &template.bytes));
            fixture
                .store
                .prevalidate(&model_content_refs(&request))
                .unwrap();
            let actual = encode(&fixture, template, 127);
            let expected = project_candidate_request(
                &decode_ingress_request(IngressProtocol::Responses, &wire).unwrap(),
                &profile,
            )
            .unwrap()
            .body;
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&actual).unwrap(),
                expected
            );
        }
    }
}

#[test]
#[ignore = "opt-in release benchmark; reports timings without a flaky deadline assertion"]
fn benchmark_local_text_processing() {
    let text = "ordinary text 中文, quote \" and slash \\, newline\n".repeat(15);
    let input: Vec<_> = (0..492)
        .map(|index| json!({"role":if index % 2 == 0 {"user"} else {"assistant"}, "content":text}))
        .collect();
    let wire = json!({"model":"alias", "input":input});
    let body = serde_json::to_vec(&wire).unwrap();
    let profile = CandidateProtocolProfile::exact_portable_path(
        IngressProtocol::Responses,
        IngressProtocol::Responses,
        "physical",
        fixed_reasoning("fixed"),
    );
    let expected = project_candidate_request(
        &decode_ingress_request(IngressProtocol::Responses, &wire).unwrap(),
        &profile,
    )
    .unwrap()
    .body;
    for threshold in [1024 * 1024, 16 * 1024] {
        let mut samples = Vec::new();
        for iteration in 0..21 {
            let fixture = Fixture::new(threshold, 16 * 1024);
            let started = Instant::now();
            let request = fixture.parse(&body);
            let parsed = started.elapsed();
            let template = project_candidate_request_template(&request, &profile).unwrap();
            fixture
                .store
                .prevalidate(&model_content_refs(&request))
                .unwrap();
            let started = Instant::now();
            let actual = encode(&fixture, template, 16 * 1024);
            let encoded = started.elapsed();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&actual).unwrap(),
                expected
            );
            if iteration > 0 {
                samples.push((parsed.as_micros(), encoded.as_micros()));
            }
        }
        eprintln!(
            "local_text_benchmark bytes={} threshold={} samples_us={samples:?}",
            body.len(),
            threshold
        );
    }
}
