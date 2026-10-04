use hiroute_domain::{
    AgentEmulatorError, AgentIngressProtocolV1, AgentProfileV1, AgentTrafficKindV1,
    BuiltInAgentProbeV1, CONNECTIVITY_PROBE_RESPONSE_V1, CanonicalDigest,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RenderedAgentProbeV1 {
    pub protocol: AgentIngressProtocolV1,
    pub traffic_kind: AgentTrafficKindV1,
    pub payload: Value,
}

pub fn render_agent_probe(
    profile: &AgentProfileV1,
    probe: &BuiltInAgentProbeV1,
) -> Result<RenderedAgentProbeV1, AgentProbeAdapterError> {
    let protocol = model_protocol(profile)?;
    probe.validate_for(profile)?;
    let payload = match protocol {
        AgentIngressProtocolV1::Responses => json!({
            "input": [{
                "content": [{"text": probe.prompt, "type": "input_text"}],
                "role": "user"
            }],
            "metadata": {"traffic_kind": "connectivity_probe"},
            "model": probe.model_alias,
            "stream": false,
            "tools": []
        }),
        AgentIngressProtocolV1::Messages => json!({
            "max_tokens": 32,
            "messages": [{
                "content": [{"text": probe.prompt, "type": "text"}],
                "role": "user"
            }],
            "metadata": {"traffic_kind": "connectivity_probe"},
            "model": probe.model_alias,
            "stream": false,
            "tools": []
        }),
    };
    Ok(RenderedAgentProbeV1 {
        protocol,
        traffic_kind: AgentTrafficKindV1::ConnectivityProbe,
        payload,
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProbeResultV1 {
    pub protocol: AgentIngressProtocolV1,
    pub traffic_kind: AgentTrafficKindV1,
    pub ready: bool,
    pub response_digest: CanonicalDigest,
}

pub fn parse_agent_probe_response(
    profile: &AgentProfileV1,
    response: &Value,
) -> Result<AgentProbeResultV1, AgentProbeAdapterError> {
    let protocol = model_protocol(profile)?;
    let text = match protocol {
        AgentIngressProtocolV1::Responses => response
            .get("output")
            .and_then(Value::as_array)
            .and_then(|output| output.first())
            .filter(|message| message.get("type") == Some(&json!("message")))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .filter(|content| content.get("type") == Some(&json!("output_text")))
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str),
        AgentIngressProtocolV1::Messages => response
            .get("content")
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .filter(|content| content.get("type") == Some(&json!("text")))
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str)
            .filter(|_| response.get("stop_reason") == Some(&json!("end_turn"))),
    }
    .ok_or(AgentProbeAdapterError::UnexpectedResponse)?;
    if text != CONNECTIVITY_PROBE_RESPONSE_V1 || contains_tool_result(response) {
        return Err(AgentProbeAdapterError::UnexpectedResponse);
    }
    Ok(AgentProbeResultV1 {
        protocol,
        traffic_kind: AgentTrafficKindV1::ConnectivityProbe,
        ready: true,
        response_digest: CanonicalDigest::of(response)
            .map_err(|_| AgentProbeAdapterError::UnexpectedResponse)?,
    })
}

fn model_protocol(
    profile: &AgentProfileV1,
) -> Result<AgentIngressProtocolV1, AgentProbeAdapterError> {
    profile
        .model_connection()
        .map_err(|_| AgentEmulatorError::InvalidProfile)?
        .map(|model| model.ingress_protocol)
        .ok_or_else(|| AgentEmulatorError::InvalidProfile.into())
}

fn contains_tool_result(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_tool_result),
        Value::Object(values) => values.iter().any(|(key, value)| {
            matches!(key.as_str(), "tool_call" | "tool_use" | "function_call")
                || value.as_str().is_some_and(|value| {
                    matches!(value, "tool_call" | "tool_use" | "function_call")
                })
                || contains_tool_result(value)
        }),
        _ => false,
    }
}

#[derive(Debug, Error)]
pub enum AgentProbeAdapterError {
    #[error(transparent)]
    Contract(#[from] AgentEmulatorError),
    #[error("Agent probe response does not match the exact profile contract")]
    UnexpectedResponse,
}
