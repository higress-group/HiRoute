//! Shared native provider/model wire projection; no settings-scope or token ownership policy.
use super::{QoderNativeError, QoderTokenBudget, qoder_error};
use serde_json::{Value, json};

pub(super) fn validate_route(provider_id: &str, endpoint: &str) -> Result<(), QoderNativeError> {
    validate_route_at(provider_id, endpoint, "/v1")
}

pub(super) fn validate_model_route(
    provider_id: &str,
    endpoint: &str,
) -> Result<(), QoderNativeError> {
    validate_route_at(provider_id, endpoint, hiroute_domain::QODER_MODEL_BASE_PATH)
}

fn validate_route_at(
    provider_id: &str,
    endpoint: &str,
    path: &str,
) -> Result<(), QoderNativeError> {
    if provider_id.is_empty()
        || provider_id.len() > 128
        || provider_id == "qoder"
        || !provider_id.as_bytes()[0].is_ascii_alphanumeric()
        || !provider_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(qoder_error("provider identity"));
    }
    let endpoint: reqwest::Url = endpoint
        .parse()
        .map_err(|_| qoder_error("loopback route"))?;
    if endpoint.scheme() != "http"
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || endpoint.path() != path
        || !endpoint
            .host_str()
            .and_then(|h| h.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok())
            .is_some_and(|h| h.is_loopback())
    {
        return Err(qoder_error("loopback route"));
    }
    Ok(())
}

pub(super) fn model(
    alias: &str,
    context: Option<u64>,
    output: u64,
) -> Result<Value, QoderNativeError> {
    if alias.is_empty()
        || alias.len() > 512
        || alias.chars().any(char::is_control)
        || alias.contains("${")
        || alias.trim() != alias
    {
        return Err(qoder_error("model alias"));
    }
    hiroute_domain::validate_qoder_output_budget(output)?;
    if let Some(context) = context {
        QoderTokenBudget::new(context, output)?;
    }
    let mut model = json!({"model":alias,"capabilities":{"tools":true},"maxOutputTokens":output});
    if let Some(context) = context {
        model["contextWindow"] = context.into();
    }
    Ok(model)
}

pub(super) fn provider(endpoint: &str, credential: &str, models: Vec<Value>) -> Value {
    json!({"type":"openai-compatible", "protocol":"openai-responses", "authType":"bearer",
        "baseUrl":endpoint, "apiKey":credential, "models":models})
}
