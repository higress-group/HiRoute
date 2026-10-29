//! The production ingress/Replay path must negotiate exactly like an inline request.
use super::*;
use crate::content_ref::{
    ContentValueExt, MARKER_PREFIX, compact_ingress_document_with_markers,
    externalize_model_request, model_content_refs, parse_ingress_document, scan_ingress_document,
};
use crate::replay::{ReplayConfig, ReplayManager, ReplayStore};
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use hiroute_gateway_core::runtime::body::{BudgetTree, StreamBudget};

struct OwnedDir(std::path::PathBuf);
impl Drop for OwnedDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Backing {
    store: ReplayStore,
    budget: StreamBudget,
    _manager: ReplayManager,
    _root: OwnedDir,
}
impl Backing {
    fn new() -> Self {
        let root = OwnedDir(std::env::temp_dir().join(format!(
            "hiroute-v2-replay-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        )));
        std::fs::create_dir(&root.0).unwrap();
        let manager = ReplayManager::open(ReplayConfig {
            root: root.0.join("replay"),
            memory_threshold_bytes: 1024,
            record_bytes: 127,
            orphan_ttl: std::time::Duration::from_secs(60),
        })
        .unwrap();
        let budget = BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
            .unwrap()
            .stream(8 * 1024 * 1024)
            .unwrap();
        let store = manager.begin_request(budget.clone()).unwrap();
        Self {
            store,
            budget,
            _manager: manager,
            _root: root,
        }
    }
    fn request(&self, original: &Value) -> ModelRequestIRV1 {
        self.request_with_budget(original, 0).unwrap()
    }
    fn request_with_budget(
        &self,
        original: &Value,
        inline_budget: usize,
    ) -> Result<ModelRequestIRV1, ModelIrError> {
        let bytes = serde_json::to_vec(original).unwrap();
        let mut writer = self.store.begin_raw().unwrap();
        for bytes in bytes.chunks(1024) {
            writer.append(bytes).unwrap();
        }
        let raw = writer.seal().unwrap();
        let stats = scan_ingress_document(self.store.reader(&raw).unwrap()).unwrap();
        let (mut document, mut workspace) =
            parse_ingress_document(IngressProtocol::Responses, &self.store, &raw, stats).unwrap();
        compact_ingress_document_with_markers(
            IngressProtocol::Responses,
            &mut document,
            &self.store,
            workspace.generated_markers(),
        )
        .unwrap();
        let mut request = decode_ingress_request_with_replay(
            IngressProtocol::Responses,
            &document,
            Some(&self.store),
        )?;
        externalize_model_request(&mut request, &self.store, inline_budget).unwrap();
        assert!(self.store.snapshot().disk_backed);
        let mut references = model_content_refs(&request);
        assert!(!references.is_empty());
        references.push(raw);
        drop(document);
        drop(workspace);
        self.store.prevalidate(&references).unwrap();
        Ok(request)
    }
    fn project(
        &self,
        request: &ModelRequestIRV1,
        profile: &CandidateProtocolProfile,
    ) -> (Vec<u8>, FunctionEncodingProjection) {
        let template = project_candidate_request_template(request, profile).unwrap();
        let encoding = template.function_encoding_projection;
        let mut reader =
            sequential_attempt_body(template, self.store.clone(), &self.budget, 37).unwrap();
        let mut actual = Vec::new();
        while let Some(chunk) = reader.next_chunk().unwrap() {
            actual.extend_from_slice(chunk.bytes());
        }
        reader.release();
        (actual, encoding)
    }
    fn release(self) {
        let budget = self.budget.clone();
        assert!(budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
        drop(self);
        assert_eq!(budget.snapshot().unwrap().live, 0);
    }
}

#[path = "responses_dialect_reasoning_replay_tests.rs"]
mod reasoning;

fn business_schema() -> Value {
    json!({
        "type":"object",
        "description":"quoted \"description\" 中文\n".repeat(3000),
        "properties":{
            "encrypted":{"type":"boolean"},
            "description":{"type":"string"},
            "message":{"type":"string","encrypted":true,"default":"large default ".repeat(3000)}
        },
        "default":{"encrypted":true,"parameters":{"encrypted":true},"text":"default data ".repeat(3000)},
        "examples":[{"encrypted":true,"input_schema":{"encrypted":false}}]
    })
}
fn portable_request() -> Value {
    json!({"model":"alias","input":"read", "max_output_tokens":128,
        "instructions":"stable instruction ".repeat(4096),
        "tools":[{"type":"function","name":"read","parameters":business_schema()}]})
}

#[test]
fn codex_v2_full_shape_schema_and_peer_production_replay_matches_inline_bytes() {
    let mut body = fixture();
    body["instructions"] = json!("stable instruction ".repeat(4096));
    body["tools"][0]["parameters"] = business_schema();
    body["input"].as_array_mut().unwrap().push(
        json!({"type":"agent_message", "id":"original_peer_id",
        "author":"parent_original", "recipient":"child_original", "content":[
            {"type":"input_text", "text":"actual quoted \"peer\" 中文\n".repeat(3000)},
            {"type":"input_text", "text":"second exact block"}]}),
    );
    let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let backing = Backing::new();
    let request = backing.request(&body);
    let native = request.native_body.as_ref().unwrap();
    // Large data is backed, while identities and the declaration stay inspectable.
    assert_eq!(native["input"][1]["author"], "parent_original");
    assert_eq!(native["input"][1]["recipient"], "child_original");
    assert_eq!(native["input"][1]["id"], "original_peer_id");
    assert_eq!(
        native["tools"][0]["parameters"]["properties"]["message"]["encrypted"],
        true
    );
    assert!(
        native["tools"][0]["parameters"]["description"]
            .as_str()
            .unwrap()
            .content_ref()
            .is_some()
    );
    for standard in [true, false] {
        let p = profile(standard);
        let expected = project_candidate_request(&inline, &p).unwrap();
        let (actual, encoding) = backing.project(&request, &p);
        assert_eq!(actual, expected.bytes);
        assert_eq!(encoding, expected.function_encoding_projection);
        let actual: Value = serde_json::from_slice(&actual).unwrap();
        if standard {
            assert_eq!(encoding, FunctionEncodingProjection::Plaintext);
            let peer = &actual["input"][1];
            assert_eq!(peer["role"], "assistant");
            assert_eq!(
                serde_json::from_str::<Value>(peer["content"][0]["text"].as_str().unwrap())
                    .unwrap(),
                json!({"type":"agent_message","id":"original_peer_id","author":"parent_original","recipient":"child_original"})
            );
            assert_eq!(
                peer["content"][1]["text"],
                body["input"][1]["content"][0]["text"]
            );
            assert_eq!(
                actual["tools"][0]["parameters"]["default"],
                body["tools"][0]["parameters"]["default"]
            );
            assert_eq!(
                actual["tools"][0]["parameters"]["examples"],
                body["tools"][0]["parameters"]["examples"]
            );
        } else {
            assert_eq!(actual["tools"], body["tools"]);
            assert_eq!(actual["input"], body["input"]);
        }
        assert!(
            !serde_json::to_string(&actual)
                .unwrap()
                .contains(MARKER_PREFIX)
        );
    }
    drop(request);
    backing.release();
}

#[test]
fn codex_v2_portable_schema_replay_preserves_business_data_for_both_json_producers() {
    let body = portable_request();
    let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let backing = Backing::new();
    let request = backing.request(&body);
    for upstream in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
        let p = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            upstream,
            "physical",
            fixed_reasoning("fixed"),
        );
        let expected = project_candidate_request(&inline, &p).unwrap();
        let (actual, encoding) = backing.project(&request, &p);
        assert_eq!(encoding, FunctionEncodingProjection::Plaintext);
        assert_eq!(actual, expected.bytes);
        let actual: Value = serde_json::from_slice(&actual).unwrap();
        let schema = if upstream == IngressProtocol::ChatCompletions {
            &actual["tools"][0]["function"]["parameters"]
        } else {
            &actual["tools"][0]["input_schema"]
        };
        let mut expected_schema = business_schema();
        expected_schema["properties"]["message"]
            .as_object_mut()
            .unwrap()
            .remove("encrypted");
        assert_eq!(schema, &expected_schema);
    }
    drop(request);
    backing.release();
}

#[test]
fn codex_v2_ordinary_standard_replay_does_not_negotiate_on_business_encrypted_keys() {
    let mut body = portable_request();
    body["tools"][0]["parameters"]["properties"]["message"]
        .as_object_mut()
        .unwrap()
        .remove("encrypted");
    body["input"] = json!([{"role":"user","content":"read"},
        {"type":"function_call","id":"f","call_id":"c","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"c","output":"actual result"}]);
    let backing = Backing::new();
    let request = backing.request(&body);
    let (actual, encoding) = backing.project(&request, &profile(true));
    assert_eq!(encoding, FunctionEncodingProjection::Preserve);
    let expected = project_candidate_request(
        &decode_ingress_request(IngressProtocol::Responses, &body).unwrap(),
        &profile(true),
    )
    .unwrap();
    assert_eq!(actual, expected.bytes);
    let actual: Value = serde_json::from_slice(&actual).unwrap();
    assert!(actual["input"][1].get("encrypted_function_args").is_none());
    assert_eq!(actual["tools"], body["tools"]);
    drop(request);
    backing.release();
}

#[test]
fn codex_v2_large_function_argument_replay_keeps_three_encoding_states() {
    for declaration in [None, Some(json!([])), Some(json!(["message"]))] {
        let args = json!({"message":"quoted \"arguments\" 中文\n".repeat(3000)}).to_string();
        let mut body = fixture();
        body["instructions"] = json!("stable instruction ".repeat(4096));
        body["input"]
            .as_array_mut()
            .unwrap()
            .push(item(declaration.clone(), "completed", &args));
        let backing = Backing::new();
        let request = backing.request(&body);
        let (actual, _) = backing.project(&request, &profile(false));
        let actual: Value = serde_json::from_slice(&actual).unwrap();
        assert_eq!(
            actual["input"][1].get("encrypted_function_args"),
            declaration.as_ref()
        );
        assert_eq!(actual["input"][1]["arguments"], args);
        if declaration
            .as_ref()
            .is_none_or(|value| value.as_array().unwrap().is_empty())
        {
            let (actual, encoding) = backing.project(&request, &profile(true));
            let actual: Value = serde_json::from_slice(&actual).unwrap();
            assert_eq!(encoding, FunctionEncodingProjection::Plaintext);
            assert!(actual["input"][1].get("encrypted_function_args").is_none());
            assert_eq!(actual["input"][1]["arguments"], args);
            assert_eq!(actual["input"][1]["call_id"], "call_native");
            let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
            assert_eq!(
                serde_json::to_vec(&actual).unwrap(),
                project_candidate_request(&inline, &profile(true))
                    .unwrap()
                    .bytes
            );
            assert_eq!(
                request.native_body.as_ref().unwrap()["input"][1].get("encrypted_function_args"),
                declaration.as_ref()
            );
        } else {
            assert!(project_candidate_request_template(&request, &profile(true)).is_err());
        }
        drop(request);
        backing.release();
    }
}

#[test]
fn codex_v2_retained_peer_identity_cannot_bypass_the_request_memory_budget() {
    let mut body = fixture();
    body["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"agent_message",
        "author":"a".repeat(8 * 1024 * 1024),"recipient":"child", "content":[]}));
    let mut request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let backing = Backing::new();
    let error = externalize_model_request(&mut request, &backing.store, 0).unwrap_err();
    assert!(
        matches!(
            error,
            crate::replay::ReplayError::Body(
                hiroute_gateway_core::runtime::body::BodyError::BudgetExceeded
            )
        ),
        "{error:?}"
    );
    drop(request);
    backing.release();
}

#[test]
fn codex_v2_backed_peer_identity_is_not_embedded_as_a_literal_marker() {
    let mut body = fixture();
    let author = format!("author-{MARKER_PREFIX}literal");
    body["input"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"agent_message",
        "author":author,"recipient":"child", "content":[{"type":"input_text","text":"read"}]}));
    let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    // This boundary does not claim to express a reserved marker namespace inside
    // its metadata text. Native passthrough retains those original bytes instead.
    assert!(project_candidate_request(&inline, &profile(true)).is_err());
    let backing = Backing::new();
    let request = backing.request(&body);
    assert!(project_candidate_request_template(&request, &profile(true)).is_err());
    let (actual, _) = backing.project(&request, &profile(false));
    let actual: Value = serde_json::from_slice(&actual).unwrap();
    assert_eq!(actual["input"], body["input"]);
    drop(request);
    backing.release();
}

#[test]
fn codex_v2_invalid_schema_shape_is_not_hidden_by_replay() {
    for schema in [
        json!({"properties":"invalid"}),
        json!({"allOf":{}}),
        json!({"items":17}),
        json!({"encrypted":"invalid"}),
    ] {
        let mut body = portable_request();
        body["tools"][0]["parameters"] = schema;
        let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert!(project_candidate_request(&inline, &profile(true)).is_err());
        let backing = Backing::new();
        let request = backing.request(&body);
        assert!(project_candidate_request_template(&request, &profile(true)).is_err());
        // The original native dialect still carries the exact schema value.
        let (actual, _) = backing.project(&request, &profile(false));
        assert_eq!(
            serde_json::from_slice::<Value>(&actual).unwrap()["tools"],
            body["tools"]
        );
        drop(request);
        backing.release();
    }
}

#[test]
fn codex_v2_retained_schema_structure_cannot_bypass_the_request_memory_budget() {
    let properties = (0..16_000)
        .map(|i| {
            (
                format!("field_{i}_{}", "k".repeat(512)),
                json!({"type":"string"}),
            )
        })
        .collect::<serde_json::Map<String, Value>>();
    // This isolates retained metadata accounting; it does not claim such a large
    // document passed the production parser's independent initial workspace limit.
    for wrapper in ["wrap", "type", "role"] {
        let mut body = portable_request();
        body["tools"][0]["parameters"] = json!({"type":"object","properties":{
            wrapper:{"type":"object","properties":properties.clone()}
        }});
        let mut request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        let backing = Backing::new();
        let error = externalize_model_request(&mut request, &backing.store, 0).unwrap_err();
        assert!(
            matches!(
                error,
                crate::replay::ReplayError::Body(
                    hiroute_gateway_core::runtime::body::BodyError::BudgetExceeded
                )
            ),
            "{wrapper}: {error:?}"
        );
        drop(request);
        backing.release();
    }
}

#[test]
fn codex_v2_schema_literal_replay_marker_shape_does_not_alias_backing() {
    let mut body = portable_request();
    // The direct internal IR API reserves the marker namespace. Build the wire
    // expectation from an ordinary request, then add the literal client data.
    let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let literal = json!({"__hiroute_json_content_ref_v2":"__hiroute_content_ref_v2_1_0_4_4__"});
    body["tools"][0]["parameters"]["properties"]["literal"] = literal.clone();
    body["tools"][0]["parameters"]["default"] = literal.clone();
    let backing = Backing::new();
    let request = backing.request(&body);
    for standard in [false, true] {
        let p = profile(standard);
        let (actual, _) = backing.project(&request, &p);
        let mut expected = project_candidate_request(&inline, &p).unwrap().body;
        expected["tools"][0]["parameters"]["properties"]["literal"] = literal.clone();
        expected["tools"][0]["parameters"]["default"] = literal.clone();
        assert_eq!(serde_json::from_slice::<Value>(&actual).unwrap(), expected);
    }
    drop(request);
    backing.release();
}

#[test]
fn codex_v2_schema_type_and_role_containers_have_equal_retained_replay_accounting() {
    let mut retained = Vec::new();
    for wrapper in ["wrap", "type", "role"] {
        let properties = (0..128)
            .map(|i| {
                (
                    format!("field_{i}"),
                    json!({"type":"string", "description":"leaf"}),
                )
            })
            .collect::<serde_json::Map<String, Value>>();
        let mut body = portable_request();
        body["tools"][0]["parameters"] = json!({"type":"object","properties":{
            wrapper:{"type":"object","properties":properties}
        }});
        let backing = Backing::new();
        // request() executes scan/parse/compact/IR and drops its ingress workspace
        // before returning. Compare only live retained request ownership here.
        let request = backing.request(&body);
        retained.push(backing.budget.snapshot().unwrap().live);
        for p in [profile(false), profile(true)] {
            let expected = project_candidate_request(
                &decode_ingress_request(IngressProtocol::Responses, &body).unwrap(),
                &p,
            )
            .unwrap();
            assert_eq!(backing.project(&request, &p).0, expected.bytes);
        }
        drop(request);
        backing.release();
    }
    // Equal-length wrapper names deliberately avoid a string-size discrepancy.
    assert!(retained[0] > 0);
    assert_eq!(
        retained[1], retained[0],
        "type subtree was not fully charged"
    );
    assert_eq!(
        retained[2], retained[0],
        "role subtree was not fully charged"
    );
}

#[test]
fn codex_v2_literal_schema_keys_survive_all_protocols_and_keep_real_replay_values() {
    let keys = [
        "__hiroute_content_ref_v2_99_0_4_4__".to_owned(),
        "before___hiroute_content_ref_v2_99_0_4_4___after".to_owned(),
        "literal___hiroute_content_ref_v2_not-a-reference".to_owned(),
        "__hiroute_content_ref_v2_99_0_4_4__-__hiroute_content_ref_v2_99_0_4_4__".to_owned(),
    ];
    let mut body = portable_request();
    for position in ["properties", "patternProperties", "$defs"] {
        let mut children = serde_json::Map::new();
        for key in &keys {
            children.insert(
                key.clone(),
                json!({"type":"string", "encrypted":true,
                "description":"real backed content with quotes \" 中\n".repeat(1000)}),
            );
        }
        body["tools"][0]["parameters"][position] = Value::Object(children);
    }
    // Data objects remain one canonical raw JSON range. Their literal keys must
    // keep the same wire bytes as keys in retained schema structure.
    body["tools"][0]["parameters"]["default"] = json!({keys[0].clone(): "default value"});
    body["tools"][0]["parameters"]["examples"] = json!([{keys[1].clone(): "example value"}]);
    let inline = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
    let backing = Backing::new();
    let request = backing.request(&body);
    let mut profiles = vec![profile(false), profile(true)];
    profiles.extend(
        [IngressProtocol::ChatCompletions, IngressProtocol::Messages].map(|upstream| {
            CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                upstream,
                "physical",
                fixed_reasoning("fixed"),
            )
        }),
    );
    for p in profiles {
        let expected = project_candidate_request(&inline, &p).unwrap();
        let (actual, encoding) = backing.project(&request, &p);
        assert_eq!(actual, expected.bytes);
        assert_eq!(encoding, expected.function_encoding_projection);
        let actual: Value = serde_json::from_slice(&actual).unwrap();
        let schema = match p.capability.upstream_protocol {
            IngressProtocol::Responses => &actual["tools"][0]["parameters"],
            IngressProtocol::ChatCompletions => &actual["tools"][0]["function"]["parameters"],
            IngressProtocol::Messages => &actual["tools"][0]["input_schema"],
        };
        for position in ["properties", "patternProperties", "$defs"] {
            for key in &keys {
                assert_eq!(
                    schema[position][key]["description"],
                    body["tools"][0]["parameters"][position][key]["description"]
                );
                assert_eq!(schema[position][key]["type"], "string");
                assert_eq!(
                    schema[position][key].get("encrypted").cloned(),
                    (encoding == FunctionEncodingProjection::Preserve).then(|| json!(true))
                );
            }
        }
    }
    drop(request);
    backing.release();
}
