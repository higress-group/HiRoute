//! Controls observed on the installed Codex Responses client. Preserve their exact values
//! only on a Responses upstream; they convey no HiRoute identity or provider-state ownership.
use super::*;

pub(super) fn annotations(
    input: Option<&Value>,
) -> Result<
    std::collections::BTreeMap<usize, std::collections::BTreeMap<usize, Vec<UrlCitationV1>>>,
    ModelIrError,
> {
    let mut output = std::collections::BTreeMap::new();
    if let Some(Value::Array(items)) = input {
        let mut message_index = 0_usize;
        for item in items {
            if item.get("type").and_then(Value::as_str) == Some("additional_tools") {
                continue;
            }
            if let Some(parts) = item.get("content").and_then(Value::as_array) {
                for (part_index, part) in parts.iter().enumerate() {
                    if let Some(value) = part.get("annotations") {
                        let annotations: Vec<UrlCitationV1> = serde_json::from_value(value.clone())
                            .map_err(|_| ModelIrError::InvalidField("annotations"))?;
                        if !annotations.is_empty() {
                            if item.get("role").and_then(Value::as_str) != Some("assistant")
                                || part.get("type").and_then(Value::as_str) != Some("output_text")
                                || annotations.len() > 128
                                || annotations.iter().any(|a| {
                                    a.start_index > a.end_index
                                        || a.title.len() > 4096
                                        || a.url.len() > 8192
                                })
                            {
                                return Err(ModelIrError::InvalidField("annotations"));
                            }
                            output
                                .entry(message_index)
                                .or_insert_with(std::collections::BTreeMap::new)
                                .insert(part_index, annotations);
                        }
                    }
                }
            }
            message_index += 1;
        }
    }
    Ok(output)
}

pub(super) fn item_ids(
    input: Option<&Value>,
) -> Result<std::collections::BTreeMap<usize, String>, ModelIrError> {
    let mut ids = std::collections::BTreeMap::new();
    if let Some(Value::Array(items)) = input {
        let mut message_index = 0_usize;
        for item in items {
            if item.get("type").and_then(Value::as_str) == Some("additional_tools") {
                continue;
            }
            if let Some(id) = item.get("id") {
                let id = id
                    .as_str()
                    .ok_or(ModelIrError::InvalidField("input[].id"))?;
                if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
                    return Err(ModelIrError::InvalidField("input[].id"));
                }
                ids.insert(message_index, id.to_owned());
            }
            message_index += 1;
        }
    }
    Ok(ids)
}

pub(super) fn message_phase(
    value: &Value,
) -> Result<Option<ResponsesMessagePhaseV1>, ModelIrError> {
    let object = value
        .as_object()
        .ok_or(ModelIrError::InvalidField("input[]"))?;
    object
        .get("phase")
        .map(|phase| match phase.as_str() {
            Some("commentary") => Ok(ResponsesMessagePhaseV1::Commentary),
            Some("final_answer") => Ok(ResponsesMessagePhaseV1::FinalAnswer),
            _ => Err(ModelIrError::InvalidField("input[].phase")),
        })
        .transpose()
}

pub(super) fn internal_chat_message_metadata(
    value: &Value,
) -> Result<Option<ResponsesInternalChatMessageMetadataV1>, ModelIrError> {
    let object = value
        .as_object()
        .ok_or(ModelIrError::InvalidField("input[]"))?;
    let Some(metadata) = object.get("internal_chat_message_metadata_passthrough") else {
        return Ok(None);
    };
    let metadata = checked_object(
        metadata,
        &["turn_id"],
        "internal_chat_message_metadata_passthrough",
    )?;
    let turn_id = required_string(metadata, "turn_id")?;
    if turn_id.is_empty() || turn_id.len() > 256 || turn_id.chars().any(char::is_control) {
        return Err(ModelIrError::InvalidField(
            "internal_chat_message_metadata_passthrough.turn_id",
        ));
    }
    Ok(Some(ResponsesInternalChatMessageMetadataV1 { turn_id }))
}

pub(super) fn reasoning_history(
    value: &Value,
) -> Result<ResponsesReasoningHistoryV1, ModelIrError> {
    let object = value
        .as_object()
        .ok_or(ModelIrError::InvalidField("input[]"))?;
    let native_fields = object
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "type"
                    | "id"
                    | "status"
                    | "encrypted_content"
                    | "internal_chat_message_metadata_passthrough"
            )
        })
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let encrypted_content = match object.get("encrypted_content") {
        None => ResponsesReasoningEncryptedContentV1::Absent,
        Some(Value::Null) => ResponsesReasoningEncryptedContentV1::Null,
        Some(Value::String(value)) if value.is_empty() => {
            ResponsesReasoningEncryptedContentV1::Empty
        }
        Some(Value::String(_)) => ResponsesReasoningEncryptedContentV1::Opaque,
        Some(_) => return Err(ModelIrError::InvalidField("encrypted_content")),
    };
    Ok(ResponsesReasoningHistoryV1 {
        native_fields,
        encrypted_content,
    })
}

pub(super) fn decode(
    object: &Map<String, Value>,
) -> Result<Option<ResponsesRequestOptionsV1>, ModelIrError> {
    let reasoning = object
        .get("reasoning")
        .map(|value| {
            let reasoning = checked_object(
                value,
                &["effort", "summary", "context"],
                "responses reasoning",
            )?;
            for field in ["effort", "summary"] {
                if let Some(value) = optional_string(reasoning, field)?
                    && (value.is_empty() || value.len() > 64 || value.chars().any(char::is_control))
                {
                    return Err(ModelIrError::InvalidField(field));
                }
            }
            let context = optional_string(reasoning, "context")?;
            if context.as_deref().is_some_and(|value| {
                value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
            }) {
                return Err(ModelIrError::InvalidField("reasoning.context"));
            }
            Ok((optional_string(reasoning, "summary")?, context))
        })
        .transpose()?;
    let (reasoning_summary, reasoning_context) = reasoning.unwrap_or((None, None));
    let store = optional_bool(object, "store")?;
    // Server-stored conversations need the existing exact provider-state authority, not a
    // boolean supplied by the caller. The current stateless Worker explicitly sends false.
    if store == Some(true) {
        return Err(ModelIrError::ProviderStateOwnershipRequired);
    }
    let include = object
        .get("include")
        .map(|value| {
            let values = value
                .as_array()
                .ok_or(ModelIrError::InvalidField("include"))?;
            if values.len() > 16 {
                return Err(ModelIrError::InvalidField("include"));
            }
            values
                .iter()
                .map(|value| {
                    let value = value
                        .as_str()
                        .ok_or(ModelIrError::InvalidField("include"))?;
                    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
                    {
                        return Err(ModelIrError::InvalidField("include"));
                    }
                    Ok(value.to_owned())
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let prompt_cache_key = optional_string(object, "prompt_cache_key")?;
    if prompt_cache_key
        .as_ref()
        .is_some_and(|key| key.len() > 256 || key.chars().any(char::is_control))
    {
        return Err(ModelIrError::InvalidField("prompt_cache_key"));
    }
    let client_metadata = object
        .get("client_metadata")
        .map(|value| {
            let values = value
                .as_object()
                .ok_or(ModelIrError::InvalidField("client_metadata"))?;
            if values.len() > 16 {
                return Err(ModelIrError::InvalidField("client_metadata"));
            }
            values
                .iter()
                .map(|(key, value)| {
                    let value = value
                        .as_str()
                        .ok_or(ModelIrError::InvalidField("client_metadata"))?;
                    if key.len() > 128 || value.len() > 2048 || key.chars().any(char::is_control) {
                        return Err(ModelIrError::InvalidField("client_metadata"));
                    }
                    Ok((key.clone(), value.to_owned()))
                })
                .collect::<Result<std::collections::BTreeMap<_, _>, ModelIrError>>()
        })
        .transpose()?;
    if store.is_none()
        && include.is_none()
        && prompt_cache_key.is_none()
        && client_metadata.is_none()
        && reasoning_summary.is_none()
        && reasoning_context.is_none()
    {
        return Ok(None);
    }
    Ok(Some(ResponsesRequestOptionsV1 {
        store,
        include,
        prompt_cache_key,
        client_metadata,
        reasoning_context,
        reasoning_summary,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn reasoning_context_roundtrips_only_on_responses_without_dropping_history_policy() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
        let document =
            json!({"model":"alias","input":"hello", "reasoning":{"context":"all_turns"}});
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        let restored: ModelRequestIRV1 =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        assert_eq!(restored, request);
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
            let projection = project_candidate_request(&request, &profile);
            if target == IngressProtocol::Responses {
                assert_eq!(
                    projection.unwrap().body["reasoning"]["context"],
                    "all_turns"
                );
            } else {
                assert!(projection.is_err());
            }
        }
        for invalid in [
            json!(null),
            json!(false),
            json!(""),
            json!("x".repeat(257)),
            json!("line\nbreak"),
        ] {
            let mut document = document.clone();
            document["reasoning"]["context"] = invalid;
            assert!(decode_ingress_request(IngressProtocol::Responses, &document).is_err());
        }
    }
    #[test]
    fn bounded_native_responses_options_are_provider_decided_not_gateway_enumerated() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

        let document = json!({
            "model":"alias", "input":"hello",
            "reasoning":{"context":"provider_future_context"},
            "include":["reasoning.encrypted_content", "provider.future_output"]
        });
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
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
            let projected = project_candidate_request(&request, &profile);
            if target == IngressProtocol::Responses {
                let body = projected.unwrap().body;
                assert_eq!(body["reasoning"], document["reasoning"]);
                assert_eq!(body["include"], document["include"]);
            } else {
                assert!(projected.is_err());
            }
        }
    }
    #[test]
    fn hosted_search_declaration_is_typed_not_a_function_and_not_silently_converted() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
        for external in [false, true] {
            let body = json!({"model":"alias","input":"search", "tools":[
                {"type":"web_search","external_web_access":external}]});
            let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
            assert!(request.tools.is_empty());
            assert_eq!(
                request.web_search.as_ref().unwrap().external_web_access,
                Some(external)
            );
            for target in [
                IngressProtocol::Responses,
                IngressProtocol::Messages,
                IngressProtocol::ChatCompletions,
            ] {
                let profile = CandidateProtocolProfile::exact_portable_path(
                    IngressProtocol::Responses,
                    target,
                    "physical",
                    fixed_reasoning("fixed"),
                );
                let result = project_candidate_request(&request, &profile);
                if target == IngressProtocol::Responses {
                    assert_eq!(result.unwrap().body["tools"], body["tools"]);
                } else {
                    assert!(result.is_err());
                }
            }
        }
        for tools in [
            json!([{"type":"web_search","external_web_access":"true"}]),
            json!([{"type":"web_search","unknown":true}]),
            json!([{"type":"web_search","external_web_access":true},{"type":"web_search","external_web_access":false}]),
        ] {
            assert!(
                decode_ingress_request(
                    IngressProtocol::Responses,
                    &json!({"model":"alias","input":"search","tools":tools})
                )
                .is_err()
            );
        }
        let duplicate = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({
                "model":"alias",
                "input":"search",
                "tools":[{"type":"web_search"},{"type":"web_search"}]
            }),
        )
        .unwrap();
        assert!(duplicate.web_search.is_some());
        assert_eq!(duplicate.responses_tool_order.len(), 1);
    }
    #[test]
    fn acp_tool_item_ids_do_not_replace_trusted_call_bindings() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
        let document = json!({"model":"alias","input":[
            {"type":"function_call","id":"client-item-call","call_id":"logical","name":"lookup","arguments":"{}"},
            {"type":"function_call_output","id":"client-item-result","call_id":"logical","output":"done"}
        ]});
        let mut request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        assert!(project_candidate_request(&request, &profile).is_err());
        request.tool_id_map = vec![ToolIdMapEntryV1 {
            logical_id: "logical".into(),
            native_id: "trusted-native-call".into(),
            kind: ToolKindV1::Function,
            name: "lookup".into(),
            namespace: None,
            owner: profile.exact_provider_path().unwrap(),
        }];
        let projected = project_candidate_request(&request, &profile).unwrap();
        for index in 0..2 {
            assert_eq!(
                projected.body["input"][index]["id"],
                document["input"][index]["id"]
            );
            assert_eq!(
                projected.body["input"][index]["call_id"],
                "trusted-native-call"
            );
        }
    }
    #[test]
    fn acp_message_ids_survive_ir_and_responses_projection() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
        let document = json!({"model":"alias","input":[
            {"type":"message","id":"client-message-1","role":"developer","content":[{"type":"input_text","text":"bounds"}]},
            {"type":"message","id":"client-message-2","role":"user","content":[{"type":"input_text","text":"goal"}]}
        ]});
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        let restored: ModelRequestIRV1 =
            serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
        assert_eq!(restored, request);
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        let projected = project_candidate_request(&restored, &profile).unwrap();
        assert_eq!(projected.body["input"], document["input"]);
        for invalid in [Value::Null, json!(5), json!(""), json!("x".repeat(257))] {
            let mut document = document.clone();
            document["input"][0]["id"] = invalid;
            assert!(decode_ingress_request(IngressProtocol::Responses, &document).is_err());
        }
    }
    #[test]
    fn installed_codex_shape_projects_without_losing_options_and_rejects_cross_protocol() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};
        let document = json!({"model":"alias","stream":true,
            "instructions":"isolated Worker", "input":[{"type":"message","role":"developer","content":[{"type":"input_text","text":"bounds"}]},{"type":"message","role":"user","content":[{"type":"input_text","text":"goal"}]}],
            "store":false,"include":["reasoning.encrypted_content"], "prompt_cache_key":"session",
            "client_metadata":{"session_id":"session"}, "reasoning":{"effort":"low","summary":"auto"},
            "parallel_tool_calls":false,"tool_choice":"auto","tools":[{"type":"function","name":"update_plan","parameters":{"type":"object"},"strict":false}]});
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        let projected = project_candidate_request(&request, &profile).unwrap();
        for key in ["store", "include", "prompt_cache_key", "client_metadata"] {
            assert_eq!(projected.body[key], document[key]);
        }
        assert_eq!(projected.body["reasoning"], json!({"summary":"auto"}));
        let other = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            "physical",
            fixed_reasoning("fixed"),
        );
        assert!(project_candidate_request(&request, &other).is_err());
    }

    #[test]
    fn installed_codex_request_matches_same_protocol_projection_except_owned_rewrites() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request_with_state_and_tool_resolver;
        use crate::server::core_runtime::profiles::{
            CandidateProtocolProfile, Fidelity, NativeProviderStateEmission, StateAffinity,
            fixed_reasoning,
        };

        let native = json!({
            "model": "served-alias",
            "stream": true,
            "instructions": "isolated Worker",
            "input": [
                {"type":"message","id":"developer-message","role":"developer","content":[{"type":"input_text","text":"bounds"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}},
                {"type":"message","id":"user-message","role":"user","content":[{"type":"input_text","text":"goal"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}},
                {"type":"reasoning","id":"reasoning-item","summary":[{"type":"summary_text","text":""}],"content":null,"encrypted_content":"opaque-state","internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}},
                {"type":"message","id":"assistant-message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"checking"}],"internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}},
                {"type":"function_call","id":"call-item","call_id":"native-call","namespace":"tools","name":"lookup","arguments":"{\"q\":\"test\"}","internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}},
                {"type":"function_call_output","id":"result-item","call_id":"native-call","output":"done","internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}}
            ],
            "store": false,
            "include": ["reasoning.encrypted_content"],
            "prompt_cache_key": "session",
            "client_metadata": {"session_id":"session"},
            "reasoning": {"effort":"low","summary":"auto"},
            "max_output_tokens": 2048,
            "parallel_tool_calls": false,
            "tool_choice": "auto",
            "tools": [{
                "type":"namespace",
                "name":"tools",
                "description":"native tools",
                "tools":[{"type":"function","name":"lookup","description":"look up","parameters":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]},"strict":true}]
            }]
        });
        let mut profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical-model",
            fixed_reasoning("fixed"),
        );
        profile.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
        profile.capability.request.provider_state = Fidelity::Exact;
        profile.capability.request.state_affinity = StateAffinity::ExactOwner;
        profile.capability.response.provider_state = Fidelity::Exact;
        profile.capability.response.state_affinity = StateAffinity::ExactOwner;
        let bindings = vec![ToolIdMapEntryV1 {
            logical_id: "native-call".into(),
            native_id: "native-call".into(),
            kind: ToolKindV1::Function,
            name: "lookup".into(),
            namespace: Some("tools".into()),
            owner: profile.exact_provider_path().unwrap(),
        }];
        let request = decode_ingress_request_with_state_and_tool_resolver(
            IngressProtocol::Responses,
            &native,
            profile.exact_provider_path().ok(),
            |_| Ok(bindings.clone()),
        )
        .unwrap();
        let projected = project_candidate_request(&request, &profile).unwrap().body;
        let mut expected = native;
        expected["model"] = json!("physical-model");
        expected["reasoning"] = json!({"summary":"auto"});
        expected["max_output_tokens"] = json!(256);

        assert_eq!(projected, expected);

        let cross = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            "physical-model",
            fixed_reasoning("fixed"),
        );
        assert!(project_candidate_request(&request, &cross).is_err());
    }

    #[test]
    fn installed_codex_item_metadata_and_phase_remain_bounded_and_fail_closed() {
        use super::super::decode_ingress_request;
        let valid = json!({"model":"alias","input":[{
            "type":"message","role":"assistant","phase":"commentary",
            "content":[{"type":"output_text","text":"status"}],
            "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}
        }]});
        decode_ingress_request(IngressProtocol::Responses, &valid).unwrap();

        for invalid in [
            json!(null),
            json!({}),
            json!({"turn_id":""}),
            json!({"turn_id":"x".repeat(257)}),
            json!({"turn_id":"turn-1","unknown":true}),
        ] {
            let mut document = valid.clone();
            document["input"][0]["internal_chat_message_metadata_passthrough"] = invalid;
            assert!(decode_ingress_request(IngressProtocol::Responses, &document).is_err());
        }
        for phase in [
            json!(null),
            json!(""),
            json!("analysis"),
            json!("x".repeat(65)),
        ] {
            let mut document = valid.clone();
            document["input"][0]["phase"] = phase;
            assert!(decode_ingress_request(IngressProtocol::Responses, &document).is_err());
        }
    }
    #[test]
    fn reasoning_history_preserves_native_fields_but_requires_opaque_owner() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request_with_state_and_tool_resolver;
        use crate::server::core_runtime::model_ir::ToolIdMapEntryV1;
        use crate::server::core_runtime::profiles::{
            CandidateProtocolProfile, Fidelity, NativeProviderStateEmission, StateAffinity,
            fixed_reasoning,
        };

        let native = json!({"model":"alias","input":[{
            "type":"reasoning","id":"reasoning-item",
            "summary":[{"type":"summary_text","text":"retained summary"}],
            "encrypted_content":"opaque-state"
        }]});
        let mut profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        profile.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
        profile.capability.request.provider_state = Fidelity::Exact;
        profile.capability.request.state_affinity = StateAffinity::ExactOwner;
        profile.capability.response.provider_state = Fidelity::Exact;
        profile.capability.response.state_affinity = StateAffinity::ExactOwner;
        let request = decode_ingress_request_with_state_and_tool_resolver(
            IngressProtocol::Responses,
            &native,
            profile.exact_provider_path().ok(),
            |_| Ok(Vec::<ToolIdMapEntryV1>::new()),
        )
        .unwrap();
        let projected = project_candidate_request(&request, &profile).unwrap().body;
        assert_eq!(
            projected["input"][0]["summary"],
            native["input"][0]["summary"]
        );
        assert!(projected["input"][0].get("content").is_none());

        for reasoning in [
            json!({"type":"reasoning","summary":[{"type":"summary_text","text":"x","unknown":true}],"encrypted_content":"state"}),
            json!({"type":"reasoning","summary":[{"type":"other","text":"x"}],"encrypted_content":"state"}),
            json!({"type":"reasoning","summary":[{"type":"summary_text","text":7}],"encrypted_content":"state"}),
            json!({"type":"reasoning","summary":(0..65).map(|_| json!({"type":"summary_text","text":""})).collect::<Vec<_>>(),"encrypted_content":"state"}),
            json!({"type":"reasoning","summary":[],"content":[{"type":"reasoning_text","text":"native reasoning"}],"provider_extension":{"mode":1},"encrypted_content":"state"}),
        ] {
            let native = json!({"model":"alias","input":[reasoning]});
            let request = decode_ingress_request_with_state_and_tool_resolver(
                IngressProtocol::Responses,
                &native,
                profile.exact_provider_path().ok(),
                |_| Ok(Vec::<ToolIdMapEntryV1>::new()),
            )
            .unwrap();
            assert_eq!(
                project_candidate_request(&request, &profile).unwrap().body["input"],
                native["input"]
            );
        }
        assert!(
            decode_ingress_request_with_state_and_tool_resolver(
                IngressProtocol::Responses,
                &native,
                None,
                |_| Ok(Vec::<ToolIdMapEntryV1>::new()),
            )
            .is_err()
        );
    }

    #[test]
    fn summary_only_reasoning_preserves_native_encrypted_content_shape() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request_with_state_and_tool_resolver;
        use crate::server::core_runtime::model_ir::ToolIdMapEntryV1;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        for content in [
            None,
            Some(Value::Null),
            Some(json!([])),
            Some(json!([{"type":"reasoning_text","text":"provider-owned plain text"}])),
        ] {
            for encrypted_content in [None, Some(Value::Null), Some(json!(""))] {
                let mut reasoning = json!({
                    "type":"reasoning",
                    "id":"reasoning-item",
                    "summary":[{"type":"summary_text","text":"used a tool"}]
                });
                if let Some(content) = &content {
                    reasoning["content"] = content.clone();
                }
                if let Some(value) = encrypted_content {
                    reasoning["encrypted_content"] = value;
                }
                let native = json!({"model":"alias","input":[reasoning]});
                let request = decode_ingress_request_with_state_and_tool_resolver(
                    IngressProtocol::Responses,
                    &native,
                    None,
                    |_| Ok(Vec::<ToolIdMapEntryV1>::new()),
                )
                .unwrap();
                assert!(request.messages[0].content.is_empty());
                let projected = project_candidate_request(&request, &profile).unwrap().body;
                assert_eq!(projected["input"], native["input"]);
            }
        }

        let invalid = json!({"model":"alias","input":[{
            "type":"reasoning","summary":[],"encrypted_content":7
        }]});
        assert!(
            decode_ingress_request_with_state_and_tool_resolver(
                IngressProtocol::Responses,
                &invalid,
                None,
                |_| Ok(Vec::<ToolIdMapEntryV1>::new()),
            )
            .is_err()
        );
    }
    #[test]
    fn generic_namespace_functions_preserve_structure_order_and_ordinary_requirements() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::profiles::{
            CandidateProtocolProfile, Fidelity, fixed_reasoning,
        };
        let tools = json!([
            {"type":"function","name":"flat","parameters":{"type":"object"}},
            {"type":"namespace","name":"group-a","description":"first group","tools":[
                {"type":"function","name":"shared","description":"one","parameters":{"type":"object"},"strict":true}
            ]},
            {"type":"web_search","external_web_access":true},
            {"type":"namespace","name":"group-b","tools":[
                {"type":"function","name":"shared","parameters":{"type":"object"},"strict":false}
            ]}
        ]);
        let request = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({
                "model":"alias",
                "stream":true,
                "input":"goal",
                "tools":tools.clone()
            }),
        )
        .unwrap();
        let requirements = request.requirements();
        assert!(!request.tool_namespaces.is_empty());
        assert!(requirements.function_tools);
        assert!(requirements.strict_tools);
        assert!(requirements.stream_tool_arguments);
        assert_eq!(request.tool_namespaces[0].tools[0].name, "shared");
        assert_eq!(request.tool_namespaces[1].tools[0].name, "shared");

        let profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        );
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["tools"],
            tools
        );

        let mut unsupported = profile.clone();
        unsupported.capability.request.function_tools = Fidelity::Unsupported;
        assert!(project_candidate_request(&request, &unsupported).is_err());
        let cross = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            "physical",
            fixed_reasoning("fixed"),
        );
        assert!(project_candidate_request(&request, &cross).is_err());
    }

    #[test]
    fn additional_tools_merge_deterministically_and_conflicting_identities_fail_ingress() {
        use super::super::super::project_candidate_request;
        use super::super::decode_ingress_request;
        use crate::server::core_runtime::model_ir::ResponsesToolOrderEntryV1;
        use crate::server::core_runtime::profiles::{CandidateProtocolProfile, fixed_reasoning};

        let lookup = json!({
            "type":"function",
            "name":"lookup",
            "description":"lookup records",
            "parameters":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]},
            "strict":true
        });
        let namespace = json!({
            "type":"namespace",
            "name":"shell",
            "tools":[{"type":"custom","name":"exec","description":"run a command"}]
        });
        let document = json!({
            "model":"alias",
            "input":[
                {"type":"message","role":"user","content":[{"type":"input_text","text":"goal"}]},
                {"type":"additional_tools","tools":[
                    lookup.clone(),
                    namespace.clone(),
                    {"type":"custom","name":"scratch"}
                ]}
            ],
            "tools":[lookup.clone(), namespace.clone()],
            "parallel_tool_calls":true
        });
        let request = decode_ingress_request(IngressProtocol::Responses, &document).unwrap();
        assert_eq!(request.tools.len(), 2);
        assert_eq!(request.tools[0].name, "lookup");
        assert_eq!(request.tools[1].name, "scratch");
        assert_eq!(request.tool_namespaces.len(), 1);
        assert_eq!(
            request.responses_tool_order,
            vec![
                ResponsesToolOrderEntryV1::Tool { index: 0 },
                ResponsesToolOrderEntryV1::Namespace { index: 0 },
                ResponsesToolOrderEntryV1::Tool { index: 1 },
            ]
        );

        let chat = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::ChatCompletions,
            "physical",
            fixed_reasoning("fixed"),
        );
        let projected = project_candidate_request(&request, &chat).unwrap().body;
        assert_eq!(projected["tools"][0]["function"]["name"], "lookup");
        assert_eq!(projected["tools"][0]["function"]["strict"], true);
        assert_eq!(
            projected["tools"][0]["function"]["parameters"],
            lookup["parameters"]
        );
        assert_eq!(projected["tools"][1]["function"]["name"], "shell__exec");
        assert_eq!(
            projected["tools"][1]["function"]["parameters"],
            json!({
                "type":"object",
                "properties":{"input":{"type":"string"}},
                "required":["input"],
                "additionalProperties":false
            })
        );
        assert_eq!(projected["tools"][2]["function"]["name"], "scratch");
        assert_eq!(projected["parallel_tool_calls"], true);

        let mut conflicting_flat = document.clone();
        conflicting_flat["input"][1]["tools"][0]["strict"] = json!(false);
        assert!(decode_ingress_request(IngressProtocol::Responses, &conflicting_flat).is_err());
        let mut conflicting_namespace = document;
        conflicting_namespace["input"][1]["tools"][1]["description"] = json!("different");
        assert!(
            decode_ingress_request(IngressProtocol::Responses, &conflicting_namespace).is_err()
        );

        for tools in [
            json!([
                {"type":"function","name":"same","parameters":{"type":"object"}},
                {"type":"custom","name":"same"}
            ]),
            json!([{
                "type":"namespace",
                "name":"group",
                "tools":[
                    {"type":"function","name":"same","parameters":{"type":"object"}},
                    {"type":"custom","name":"same"}
                ]
            }]),
        ] {
            assert!(
                decode_ingress_request(
                    IngressProtocol::Responses,
                    &json!({"model":"alias","input":"goal","tools":tools}),
                )
                .is_err(),
                "different kinds with the same namespace/local name must fail ingress"
            );
        }

        let exact_namespace_child_duplicate = decode_ingress_request(
            IngressProtocol::Responses,
            &json!({
                "model":"alias",
                "input":"goal",
                "tools":[{
                    "type":"namespace",
                    "name":"group",
                    "tools":[
                        {"type":"custom","name":"same"},
                        {"type":"custom","name":"same"}
                    ]
                }]
            }),
        )
        .unwrap();
        assert_eq!(
            exact_namespace_child_duplicate.tool_namespaces[0]
                .tools
                .len(),
            1
        );
    }

    #[test]
    fn namespace_children_and_named_choice_fail_closed_when_unsupported_or_ambiguous() {
        use super::super::decode_ingress_request;
        for tool in [
            json!({"type":"namespace","name":"group","tools":[{"type":"namespace","name":"nested","tools":[{"type":"function","name":"child","parameters":{}}]}]}),
            json!({"type":"namespace","name":"group","execution":"hosted","tools":[{"type":"function","name":"child","parameters":{}}]}),
            json!({"type":"namespace","name":"group","tools":[]}),
        ] {
            assert!(
                decode_ingress_request(
                    IngressProtocol::Responses,
                    &json!({"model":"alias","input":"goal","tools":[tool]})
                )
                .is_err()
            );
        }
        assert!(
            decode_ingress_request(
                IngressProtocol::Responses,
                &json!({
                    "model":"alias",
                    "input":"goal",
                    "tools":[{"type":"namespace","name":"group","tools":[{"type":"custom","name":"child"}]}]
                })
            )
            .is_ok()
        );
        let ambiguous = json!({
            "model":"alias","input":"goal",
            "tools":[
                {"type":"namespace","name":"a","tools":[{"type":"function","name":"same","parameters":{}}]},
                {"type":"namespace","name":"b","tools":[{"type":"function","name":"same","parameters":{}}]}
            ],
            "tool_choice":{"type":"function","name":"same"}
        });
        assert!(decode_ingress_request(IngressProtocol::Responses, &ambiguous).is_err());

        let mut unique = ambiguous;
        unique["tools"].as_array_mut().unwrap().pop();
        assert!(decode_ingress_request(IngressProtocol::Responses, &unique).is_ok());
    }
    #[test]
    fn native_responses_controls_are_typed_and_preserved() {
        let value = json!({"store":false,"include":["reasoning.encrypted_content"],
            "prompt_cache_key":"native-session", "client_metadata":{"session_id":"native-session"}});
        let options = decode(value.as_object().unwrap()).unwrap().unwrap();
        assert_eq!(serde_json::to_value(options).unwrap(), value);
        assert_eq!(decode(json!({}).as_object().unwrap()).unwrap(), None);
    }
    #[test]
    fn stateful_unknown_and_unbounded_controls_fail_closed() {
        for value in [
            json!({"store":true}),
            json!({"store":"false"}),
            json!({"include":[false]}),
            json!({"include":[""]}),
            json!({"include":["x".repeat(257)]}),
            json!({"include":vec!["x"; 17]}),
            json!({"client_metadata":{"nested":{}}}),
            json!({"prompt_cache_key":"x".repeat(257)}),
        ] {
            assert!(decode(value.as_object().unwrap()).is_err());
        }
    }
}
