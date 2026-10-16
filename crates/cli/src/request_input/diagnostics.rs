//! Bounded presentation diagnostics. Never return serde's raw message or a submitted value.
use hiroute_application_api::{ErrorCode, ErrorV1, MachineEnvelopeV2};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeSet};

use hiroute_application_api::CliInputDiagnosticV1 as InputDiagnostic;
struct State {
    command: String,
    rich: bool,
    diagnostic: Option<InputDiagnostic>,
}
thread_local! { static CURRENT: RefCell<Option<State>> = const { RefCell::new(None) }; }
pub(crate) struct Scope(Option<State>);
impl Scope {
    pub(crate) fn enter(command: &str, rich: bool) -> Self {
        Self(CURRENT.with(|current| {
            current.replace(Some(State {
                command: command.into(),
                rich,
                diagnostic: None,
            }))
        }))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.0.take());
        });
    }
}

pub(crate) fn record(category: &'static str, path: &str, expected: &'static str) {
    CURRENT.with(|current| {
        if let Some(state) = current.borrow_mut().as_mut() {
            state.diagnostic = Some(InputDiagnostic {
                category: category.into(),
                path: path.chars().take(256).collect(),
                expected: expected.into(),
            });
        }
    });
}
fn fields(schema: &Value, names: &mut BTreeSet<String>) {
    match schema {
        Value::Object(object) => {
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                names.extend(properties.keys().cloned());
            }
            for child in object.values() {
                fields(child, names);
            }
        }
        Value::Array(values) => {
            for value in values {
                fields(value, names);
            }
        }
        _ => {}
    }
}

pub(crate) fn typed(error: &serde_path_to_error::Error<serde_json::Error>) {
    let mut names = BTreeSet::new();
    CURRENT.with(|current| {
        if let Some(state) = current.borrow().as_ref()
            && let Some(schemas) = hiroute_application_api::command_schemas(&state.command)
        {
            fields(&schemas.request, &mut names);
        }
    });
    let mut path = String::from("$");
    for segment in error.path().iter().take(16) {
        match segment {
            serde_path_to_error::Segment::Seq { .. } => path.push_str("[*]"),
            serde_path_to_error::Segment::Map { key } if names.contains(key) => {
                path.push('.');
                path.push_str(key);
            }
            _ => path.push_str(".*"),
        }
    }
    let message = error.inner().to_string();
    let (category, expected) = if let Some(field) = message
        .strip_prefix("missing field `")
        .and_then(|m| m.split('`').next())
    {
        if names.contains(field) {
            path.push('.');
            path.push_str(field);
        }
        ("missing_field", "required field from the command schema")
    } else if message.starts_with("unknown field") {
        ("unknown_field", "only declared request fields")
    } else if message.starts_with("unknown variant") {
        ("invalid_enum", "one of the declared enum variants")
    } else if message.starts_with("invalid type") {
        let expected = match message.rsplit(", expected ").next().unwrap_or("") {
            "a string" => "string",
            "a boolean" => "boolean",
            "a sequence" => "array",
            "a map" => "object",
            value
                if value.starts_with("u32")
                    || value.starts_with("u64")
                    || value.starts_with("usize")
                    || value.starts_with("i64") =>
            {
                "integer"
            }
            value if value.starts_with("struct ") => "object with the declared fields",
            _ => "type declared by the command schema",
        };
        ("invalid_type", expected)
    } else {
        ("invalid_value", "value allowed by the command schema")
    };
    record(category, &path, expected);
}

pub(crate) fn json(error: &serde_json::Error) {
    let category = if error.to_string().starts_with("duplicate JSON member") {
        "duplicate_field"
    } else if error.to_string().starts_with("trailing characters") {
        "trailing_json"
    } else {
        "invalid_json"
    };
    record(
        category,
        "$",
        "one complete UTF-8 JSON document without duplicate fields",
    );
}

pub(crate) fn failure(code: ErrorCode, request_id: Option<String>) -> crate::CliExecution {
    if code != ErrorCode::InvalidArguments {
        return crate::failure(code, request_id);
    }
    CURRENT.with(|current| {
        let current = current.borrow();
        let Some(state) = current.as_ref() else { return crate::failure(code, request_id); };
        let diagnostic = state.diagnostic.clone().unwrap_or(InputDiagnostic { category: "invalid_request".into(), path: "$".into(), expected: "the command's declared options and request wrapper".into() });
        let envelope = if state.rich { MachineEnvelopeV2::failed_with_data(ErrorV1::new(code), json!({
            "schema":"hiroute.cli-input-diagnostic/v1", "command_id":state.command, "diagnostic":diagnostic,
        }), request_id) } else { MachineEnvelopeV2::failed(ErrorV1::new(code), request_id) };
        let mut result = crate::render(envelope);
        result.stderr = format!("Invalid input at {} ({}): expected {}.\nRun: hiroute schema show --command-id {} --include-schema\n", diagnostic.path, diagnostic.category, diagnostic.expected, state.command);
        result
    })
}
