use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::content_ref::JsonValueExt;
use crate::server::core_runtime::model_ir::{
    CanonicalTool, ModelRequestIRV1, ResponsesToolOrderEntryV1, ToolChoice, ToolKindV1,
};

use super::ProtocolAdapterError;

const MAX_CHAT_TOOL_NAME_BYTES: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ChatToolIdentity {
    pub(crate) emitted_name: String,
    pub(crate) kind: ToolKindV1,
    pub(crate) namespace: Option<String>,
    pub(crate) local_name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChatToolProjection {
    entries: Vec<ChatToolIdentity>,
}

impl ChatToolProjection {
    pub(crate) fn retained_bytes(&self) -> Option<usize> {
        let entries = self.entries.iter().try_fold(0_usize, |total, identity| {
            total
                .checked_add(identity.emitted_name.capacity())?
                .checked_add(identity.local_name.capacity())?
                .checked_add(identity.namespace.as_ref().map_or(0, String::capacity))?
                .checked_add(std::mem::size_of::<ChatToolIdentity>())
        })?;
        Some(entries)
    }

    pub(crate) fn for_request(request: &ModelRequestIRV1) -> Result<Self, ProtocolAdapterError> {
        if request.web_search.is_some() {
            return Err(unrepresentable(
                "Responses hosted tools cannot be projected to Chat Completions",
            ));
        }
        if request
            .tool_namespaces
            .iter()
            .any(|namespace| namespace.description.is_some())
        {
            return Err(unrepresentable(
                "Responses namespace descriptions cannot be projected to Chat Completions",
            ));
        }

        let declarations = ordered_declarations(request)?;

        let mut entries = Vec::with_capacity(declarations.len());
        let mut emitted_names = BTreeSet::new();
        for (namespace, tool) in declarations {
            validate_declaration(tool)?;
            let emitted_name = match namespace {
                Some(namespace) => format!("{namespace}__{}", tool.name),
                None => tool.name.clone(),
            };
            validate_chat_name(&emitted_name)?;
            let identity = ChatToolIdentity {
                emitted_name: emitted_name.clone(),
                kind: tool.kind,
                namespace: namespace.map(str::to_owned),
                local_name: tool.name.clone(),
            };
            if !emitted_names.insert(emitted_name) {
                return Err(unrepresentable(
                    "Responses tools collide after Chat name projection",
                ));
            }
            entries.push(identity);
        }
        Ok(Self { entries })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn render_tools(
        &self,
        request: &ModelRequestIRV1,
    ) -> Result<Vec<Value>, ProtocolAdapterError> {
        let declarations = ordered_declarations(request)?;
        if declarations.len() != self.entries.len() {
            return Err(unrepresentable(
                "Chat tool projection changed after construction",
            ));
        }
        declarations
            .into_iter()
            .zip(&self.entries)
            .map(|((namespace, tool), identity)| {
                if identity.kind != tool.kind
                    || identity.namespace.as_deref() != namespace
                    || identity.local_name != tool.name
                {
                    return Err(unrepresentable(
                        "Chat tool projection changed after construction",
                    ));
                }
                Ok(Self::render_chat_tool(identity, tool))
            })
            .collect()
    }

    pub(crate) fn resolve_emitted(&self, emitted: &str) -> Option<&ChatToolIdentity> {
        self.entries
            .iter()
            .find(|identity| identity.emitted_name == emitted)
    }

    pub(crate) fn emitted_for(
        &self,
        kind: ToolKindV1,
        namespace: Option<&str>,
        local_name: &str,
    ) -> Result<&str, ProtocolAdapterError> {
        let mut matches = self.entries.iter().filter(|identity| {
            identity.kind == kind
                && identity.namespace.as_deref() == namespace
                && identity.local_name == local_name
        });
        let found = matches
            .next()
            .ok_or_else(|| unrepresentable("tool call has no projected declaration"))?;
        if matches.next().is_some() {
            return Err(unrepresentable("tool call projection is ambiguous"));
        }
        Ok(&found.emitted_name)
    }

    pub(crate) fn named_choice(
        &self,
        choice: &ToolChoice,
    ) -> Result<Option<&str>, ProtocolAdapterError> {
        let ToolChoice::RequiredNamed { tool_kind, name } = choice else {
            return Ok(None);
        };
        let mut matches = self
            .entries
            .iter()
            .filter(|identity| identity.kind == *tool_kind && identity.local_name == *name);
        let found = matches
            .next()
            .ok_or_else(|| unrepresentable("named tool choice has no declaration"))?;
        if matches.next().is_some() {
            return Err(unrepresentable("named tool choice is ambiguous"));
        }
        Ok(Some(found.emitted_name.as_str()))
    }

    pub(crate) fn chat_arguments(
        identity: &ChatToolIdentity,
        canonical: &Value,
    ) -> Result<Value, ProtocolAdapterError> {
        match identity.kind {
            ToolKindV1::Function => Ok(canonical.clone()),
            ToolKindV1::Custom => {
                let input = canonical.wire_value();
                if !input.is_string() {
                    return Err(unrepresentable(
                        "custom tool input must be a freeform string",
                    ));
                }
                Ok(json!({"input": input}))
            }
        }
    }

    pub(crate) fn canonical_arguments(
        identity: &ChatToolIdentity,
        chat: Value,
    ) -> Result<Value, ProtocolAdapterError> {
        match identity.kind {
            ToolKindV1::Function => Ok(chat),
            ToolKindV1::Custom => {
                let object = chat.as_object().ok_or_else(|| {
                    unrepresentable("custom Chat tool arguments must be an object")
                })?;
                if object.len() != 1 {
                    return Err(unrepresentable(
                        "custom Chat tool arguments must contain only input",
                    ));
                }
                let input = object
                    .get("input")
                    .and_then(Value::as_str)
                    .ok_or_else(|| unrepresentable("custom Chat tool input must be a string"))?;
                Ok(Value::String(input.to_owned()))
            }
        }
    }

    pub(crate) fn render_chat_tool(identity: &ChatToolIdentity, tool: &CanonicalTool) -> Value {
        let mut function = Map::new();
        function.insert("name".into(), Value::String(identity.emitted_name.clone()));
        if let Some(description) = &tool.description {
            function.insert("description".into(), Value::String(description.clone()));
        }
        match tool.kind {
            ToolKindV1::Function => {
                function.insert(
                    "parameters".into(),
                    tool.input_schema
                        .as_ref()
                        .expect("validated function tool has an input schema")
                        .wire_value(),
                );
                if let Some(strict) = tool.strict {
                    function.insert("strict".into(), Value::Bool(strict));
                }
            }
            ToolKindV1::Custom => {
                function.insert(
                    "parameters".into(),
                    json!({
                        "type": "object",
                        "properties": {"input": {"type": "string"}},
                        "required": ["input"],
                        "additionalProperties": false
                    }),
                );
            }
        }
        json!({"type": "function", "function": function})
    }
}

fn ordered_declarations(
    request: &ModelRequestIRV1,
) -> Result<Vec<(Option<&str>, &CanonicalTool)>, ProtocolAdapterError> {
    let mut declarations = Vec::new();
    if request.responses_tool_order.is_empty() {
        if !request.tool_namespaces.is_empty() {
            return Err(unrepresentable(
                "Responses namespace tools require an exact declaration order",
            ));
        }
        declarations.extend(request.tools.iter().map(|tool| (None, tool)));
        return Ok(declarations);
    }

    let mut flat_seen = vec![false; request.tools.len()];
    let mut namespace_seen = vec![false; request.tool_namespaces.len()];
    for order in &request.responses_tool_order {
        match *order {
            ResponsesToolOrderEntryV1::Tool { index } => {
                let index = usize::try_from(index)
                    .map_err(|_| unrepresentable("Responses tool order index is invalid"))?;
                let tool = request.tools.get(index).ok_or_else(|| {
                    unrepresentable("Responses tool order references a missing tool")
                })?;
                if std::mem::replace(&mut flat_seen[index], true) {
                    return Err(unrepresentable("Responses tool order repeats a tool"));
                }
                declarations.push((None, tool));
            }
            ResponsesToolOrderEntryV1::Namespace { index } => {
                let index = usize::try_from(index)
                    .map_err(|_| unrepresentable("Responses tool order index is invalid"))?;
                let namespace = request.tool_namespaces.get(index).ok_or_else(|| {
                    unrepresentable("Responses tool order references a missing namespace")
                })?;
                if std::mem::replace(&mut namespace_seen[index], true) {
                    return Err(unrepresentable("Responses tool order repeats a namespace"));
                }
                declarations.extend(
                    namespace
                        .tools
                        .iter()
                        .map(|tool| (Some(namespace.name.as_str()), tool)),
                );
            }
            ResponsesToolOrderEntryV1::WebSearch => {
                return Err(unrepresentable(
                    "Responses hosted tools cannot be projected to Chat Completions",
                ));
            }
        }
    }
    if flat_seen.iter().any(|seen| !seen) || namespace_seen.iter().any(|seen| !seen) {
        return Err(unrepresentable("Responses tool order is incomplete"));
    }
    Ok(declarations)
}

fn validate_declaration(tool: &CanonicalTool) -> Result<(), ProtocolAdapterError> {
    match tool.kind {
        ToolKindV1::Function if tool.input_schema.is_some() && tool.format.is_none() => Ok(()),
        ToolKindV1::Custom
            if tool.input_schema.is_none() && tool.strict.is_none() && tool.format.is_none() =>
        {
            Ok(())
        }
        ToolKindV1::Function => Err(unrepresentable(
            "function tool declaration is incomplete or contains custom format",
        )),
        ToolKindV1::Custom => Err(unrepresentable(
            "custom tool format cannot be represented by Chat Completions",
        )),
    }
}

fn validate_chat_name(name: &str) -> Result<(), ProtocolAdapterError> {
    if name.is_empty()
        || name.len() > MAX_CHAT_TOOL_NAME_BYTES
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(unrepresentable(
            "projected Chat tool name violates length or character limits",
        ));
    }
    Ok(())
}

fn unrepresentable(message: impl Into<String>) -> ProtocolAdapterError {
    ProtocolAdapterError::ClientUnrepresentable(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::core_runtime::model_ir::{
        MODEL_REQUEST_IR_SCHEMA, RequestedReasoningControl,
    };
    use crate::server::request_plan::IngressProtocol;

    fn request() -> ModelRequestIRV1 {
        ModelRequestIRV1 {
            schema_version: MODEL_REQUEST_IR_SCHEMA.into(),
            ingress_protocol: IngressProtocol::Responses,
            served_model_id: "route".into(),
            stream: true,
            instructions: Vec::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            tool_namespaces: Vec::new(),
            responses_tool_order: Vec::new(),
            web_search: None,
            responses_search_history: Default::default(),
            responses_annotations: Default::default(),
            tool_choice: ToolChoice::Auto,
            parallel_tool_calls: true,
            requested_reasoning: RequestedReasoningControl::absent(),
            requested_max_output_tokens: None,
            provider_state: Vec::new(),
            responses_options: None,
            responses_item_ids: Default::default(),
            responses_item_statuses: Default::default(),
            responses_message_phases: Default::default(),
            responses_internal_chat_message_metadata: Default::default(),
            responses_reasoning_history: Default::default(),
        }
    }

    fn function(name: &str) -> CanonicalTool {
        CanonicalTool {
            kind: ToolKindV1::Function,
            name: name.into(),
            description: None,
            input_schema: Some(json!({"type":"object"})),
            strict: Some(true),
            format: None,
        }
    }

    fn custom(name: &str) -> CanonicalTool {
        CanonicalTool {
            kind: ToolKindV1::Custom,
            name: name.into(),
            description: None,
            input_schema: None,
            strict: None,
            format: None,
        }
    }

    #[test]
    fn same_child_name_in_two_namespaces_has_distinct_reversible_names() {
        let mut request = request();
        request.tool_namespaces = vec![
            crate::server::core_runtime::model_ir::CanonicalToolNamespaceV1 {
                name: "first".into(),
                description: None,
                tools: vec![function("shared")],
            },
            crate::server::core_runtime::model_ir::CanonicalToolNamespaceV1 {
                name: "second".into(),
                description: None,
                tools: vec![function("shared")],
            },
        ];
        request.responses_tool_order = vec![
            ResponsesToolOrderEntryV1::Namespace { index: 0 },
            ResponsesToolOrderEntryV1::Namespace { index: 1 },
        ];
        let projection = ChatToolProjection::for_request(&request).unwrap();
        assert_eq!(
            projection
                .resolve_emitted("first__shared")
                .unwrap()
                .namespace
                .as_deref(),
            Some("first")
        );
        assert_eq!(
            projection
                .resolve_emitted("second__shared")
                .unwrap()
                .namespace
                .as_deref(),
            Some("second")
        );
    }

    #[test]
    fn flat_and_qualified_collision_fails_closed() {
        let mut request = request();
        request.tools = vec![function("editor__patch")];
        request.tool_namespaces = vec![
            crate::server::core_runtime::model_ir::CanonicalToolNamespaceV1 {
                name: "editor".into(),
                description: None,
                tools: vec![function("patch")],
            },
        ];
        request.responses_tool_order = vec![
            ResponsesToolOrderEntryV1::Tool { index: 0 },
            ResponsesToolOrderEntryV1::Namespace { index: 0 },
        ];
        assert!(ChatToolProjection::for_request(&request).is_err());
    }

    #[test]
    fn namespace_description_fails_closed_instead_of_being_dropped() {
        let mut request = request();
        request.tool_namespaces = vec![
            crate::server::core_runtime::model_ir::CanonicalToolNamespaceV1 {
                name: "records".into(),
                description: Some("record tools".into()),
                tools: vec![function("lookup")],
            },
        ];
        request.responses_tool_order = vec![ResponsesToolOrderEntryV1::Namespace { index: 0 }];
        assert!(matches!(
            ChatToolProjection::for_request(&request),
            Err(ProtocolAdapterError::ClientUnrepresentable(_))
        ));
    }

    #[test]
    fn custom_wrapper_requires_exact_single_string_input() {
        let identity = ChatToolIdentity {
            emitted_name: "exec".into(),
            kind: ToolKindV1::Custom,
            namespace: None,
            local_name: "exec".into(),
        };
        let wrapped =
            ChatToolProjection::chat_arguments(&identity, &Value::String("echo ok".into()))
                .unwrap();
        assert_eq!(wrapped, json!({"input":"echo ok"}));
        assert_eq!(
            ChatToolProjection::canonical_arguments(&identity, wrapped).unwrap(),
            Value::String("echo ok".into())
        );
        assert!(
            ChatToolProjection::canonical_arguments(
                &identity,
                json!({"input":"echo ok","extra":true})
            )
            .is_err()
        );
    }

    #[test]
    fn invalid_chat_names_and_custom_grammars_fail_candidate_projection() {
        for name in ["", "bad.name", &"x".repeat(MAX_CHAT_TOOL_NAME_BYTES + 1)] {
            let mut request = request();
            request.tools = vec![function(name)];
            assert!(ChatToolProjection::for_request(&request).is_err());
        }

        let mut request = request();
        let mut grammar = custom("shell");
        grammar.format = Some(json!({"type":"grammar","syntax":"lark","definition":"start: WORD"}));
        request.tools = vec![grammar];
        assert!(ChatToolProjection::for_request(&request).is_err());
    }

    #[test]
    fn unique_namespaced_named_choice_projects_to_flat_chat_name() {
        let mut request = request();
        request.tool_namespaces = vec![
            crate::server::core_runtime::model_ir::CanonicalToolNamespaceV1 {
                name: "records".into(),
                description: None,
                tools: vec![function("lookup")],
            },
        ];
        request.responses_tool_order = vec![ResponsesToolOrderEntryV1::Namespace { index: 0 }];
        request.tool_choice = ToolChoice::RequiredNamed {
            tool_kind: ToolKindV1::Function,
            name: "lookup".into(),
        };
        let projection = ChatToolProjection::for_request(&request).unwrap();
        assert_eq!(
            projection.named_choice(&request.tool_choice).unwrap(),
            Some("records__lookup")
        );
    }
}
