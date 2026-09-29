//! One explicit, bounded function-call probe; no protocol discovery or model fan-out.
use super::*;
use hiroute_application_api::{
    ModelConnectionAuthenticationStatusV1 as Authentication,
    ModelConnectionInferenceStatusV1 as Inference, ModelConnectionReachabilityV1 as Reachability,
};
use std::io::Read;

pub(super) fn send(
    target: &NormalizedModelConnectionTargetV1,
    model: &str,
    credential: Option<ModelConnectionProbeCredentialV1<'_>>,
    limits: ModelConnectionProbeLimitsV1,
) -> Result<ModelDirectoryHttpResponseV1, ModelDirectoryTransportErrorV1> {
    use ModelDirectoryTransportErrorV1 as Error;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(limits.total_timeout)
        .build()
        .map_err(|_| Error::Unavailable)?;
    let messages =
        serde_json::json!([{"role":"user","content":"Call hiroute_probe with value ok."}]);
    let parameters = serde_json::json!({"type":"object","properties":{"value":{"type":"string","enum":["ok"]}},"required":["value"],"additionalProperties":false});
    // Let the user instruction elicit the call. Forced tool_choice is rejected by some
    // providers in their default thinking mode; leave that mode unchanged and allow
    // enough output for a short reasoning prelude before the tool call.
    let body = match target.candidate_target.upstream_protocol {
        UpstreamProtocol::Responses => {
            serde_json::json!({"model":model,"input":messages,"max_output_tokens":4096,"stream":false,
                "tools":[{"type":"function","name":"hiroute_probe","parameters":parameters}]})
        }
        UpstreamProtocol::ChatCompletions => {
            serde_json::json!({"model":model,"messages":messages,"max_tokens":4096,"stream":false,
                "tools":[{"type":"function","function":{"name":"hiroute_probe","parameters":parameters}}]})
        }
        UpstreamProtocol::Messages => {
            serde_json::json!({"model":model,"messages":messages,"max_tokens":4096,"stream":false,
                "tools":[{"name":"hiroute_probe","input_schema":parameters}]})
        }
    };
    let mut request = client
        .post(target.request_url())
        .header(reqwest::header::HOST, target.host_header())
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&body).map_err(|_| Error::Unavailable)?);
    for (name, value) in target.directory_headers() {
        request = request.header(name.clone(), value.clone());
    }
    if let Some(credential) = credential {
        request = probe::apply_authorization(request, credential)?;
    }
    let response = request.send().map_err(|_| Error::Unavailable)?;
    let status = response.status().as_u16();
    let mut body = Vec::new();
    // Error responses may echo credentials. They are neither read nor returned.
    if (200..300).contains(&status) {
        response
            .take(limits.response_bytes as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|_| Error::Unavailable)?;
    }
    let truncated = body.len() > limits.response_bytes;
    Ok(ModelDirectoryHttpResponseV1 {
        status,
        body,
        truncated,
    })
}

pub(super) fn observe<T: ModelDirectoryTransportV1>(
    transport: &T,
    target: &NormalizedModelConnectionTargetV1,
    model: &str,
    credential: Option<ModelConnectionProbeCredentialV1<'_>>,
    cancellation: &ModelConnectionProbeCancellationV1,
    limits: ModelConnectionProbeLimitsV1,
) -> ModelDirectoryProbeObservationV1 {
    let authenticated = credential.is_some();
    let mut observation = ModelDirectoryProbeObservationV1::missing_credential();
    observation.issues.clear();
    observation.authentication = if authenticated {
        Authentication::Unknown
    } else {
        Authentication::NotRequired
    };
    observation.inference = Inference::Failed;
    match transport.infer(target, model, credential, limits) {
        Ok(response) if !cancellation.is_cancelled() => {
            observation.reachability = Reachability::Reachable;
            if matches!(response.status, 401 | 403) {
                observation.authentication = Authentication::Rejected;
            } else if (200..300).contains(&response.status) && !response.truncated {
                let value: serde_json::Value =
                    serde_json::from_slice(&response.body).unwrap_or_default();
                let valid = valid_tool_call(&value, target.candidate_target.upstream_protocol);
                if valid {
                    observation.inference = Inference::Verified;
                    observation.authentication = if authenticated {
                        Authentication::Verified
                    } else {
                        Authentication::NotRequired
                    };
                } else {
                    observation
                        .issues
                        .push(hiroute_application_api::ModelConnectionCheckIssueV1 {
                            code: "TOOL_CALL_NOT_VERIFIED".into(),
                            message_key: "model_connections.tool_call_not_verified".into(),
                            retryable: true,
                        });
                }
            }
        }
        _ => {
            observation.reachability = Reachability::TransportFailed;
        }
    }
    if observation.inference == Inference::Failed {
        observation
            .issues
            .push(hiroute_application_api::ModelConnectionCheckIssueV1 {
                code: "INFERENCE_FAILED".into(),
                message_key: "model_connections.inference_failed".into(),
                retryable: true,
            });
    }
    observation
}

fn valid_tool_call(value: &serde_json::Value, protocol: UpstreamProtocol) -> bool {
    use serde_json::Value;
    let (items, kind, id, name, arguments) = match protocol {
        UpstreamProtocol::Responses => (
            value.get("output"),
            "function_call",
            "call_id",
            "/name",
            "/arguments",
        ),
        UpstreamProtocol::ChatCompletions => (
            value.pointer("/choices/0/message/tool_calls"),
            "function",
            "id",
            "/function/name",
            "/function/arguments",
        ),
        UpstreamProtocol::Messages => (value.get("content"), "tool_use", "id", "/name", "/input"),
    };
    items.and_then(Value::as_array).is_some_and(|items| {
        items.iter().any(|item| {
            let args = item.pointer(arguments).cloned().unwrap_or(Value::Null);
            let args = if protocol == UpstreamProtocol::Messages {
                args
            } else {
                args.as_str()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .unwrap_or(Value::Null)
            };
            item.get("type").and_then(Value::as_str) == Some(kind)
                && item
                    .get(id)
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
                && item.pointer(name).and_then(Value::as_str) == Some("hiroute_probe")
                && args == serde_json::json!({"value":"ok"})
        })
    })
}
