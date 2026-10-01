//! Messages JSON Schema output formatting has equivalent Responses and Chat fields.
//! Preserve the schema; the provider, not this adapter, validates its support.
use serde_json::{Map, Value, json};

use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use crate::server::request_plan::IngressProtocol;

pub(super) fn messages_schema_format(body: &Value) -> Option<&Map<String, Value>> {
    let format = body.pointer("/output_config/format")?.as_object()?;
    (format.get("type").and_then(Value::as_str) == Some("json_schema")
        && format.contains_key("schema")
        && format.keys().all(|key| {
            matches!(
                key.as_str(),
                "type" | "schema" | "name" | "description" | "strict"
            )
        }))
    .then_some(format)
}

pub(super) fn project_messages_format(
    request: &ModelRequestIRV1,
    target: IngressProtocol,
    body: &mut Value,
) {
    if request.ingress_protocol != IngressProtocol::Messages || target == IngressProtocol::Messages
    {
        return;
    }
    let Some(format) = request
        .native_body
        .as_ref()
        .and_then(messages_schema_format)
    else {
        return;
    };
    let mut format = format.clone();
    format.remove("type");
    format
        .entry("name")
        .or_insert_with(|| json!("hiroute_structured_output"));
    format.entry("strict").or_insert_with(|| json!(true));
    match target {
        IngressProtocol::Responses => {
            format.insert("type".into(), json!("json_schema"));
            body["text"] = json!({"format":format});
        }
        IngressProtocol::ChatCompletions => {
            body["response_format"] = json!({"type":"json_schema","json_schema":format});
        }
        IngressProtocol::Messages => unreachable!("native Messages preserves its original format"),
    }
}
