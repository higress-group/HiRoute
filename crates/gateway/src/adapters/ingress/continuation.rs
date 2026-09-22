use std::collections::BTreeMap;

use crate::server::core_runtime::model_ir::{
    ContentPart, ExactProviderPathV1, ModelIrError, ModelRequestIRV1,
};

/// Only opaque provider state needs trusted, source-specific ownership.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IngressRequestBindings {
    pub provider_state_owner: Option<ExactProviderPathV1>,
}

pub(super) fn validate_bindings(bindings: &IngressRequestBindings) -> Result<(), ModelIrError> {
    if bindings
        .provider_state_owner
        .as_ref()
        .is_some_and(|owner| !owner.is_complete())
    {
        return Err(ModelIrError::ProviderStateOwnershipRequired);
    }
    Ok(())
}

/// A result's callable namespace is a fact in the supplied history, not a
/// capability issued by HiRoute. Result-only native history remains valid.
pub(super) fn pair_tool_history(request: &mut ModelRequestIRV1) -> Result<(), ModelIrError> {
    request.continuation_logical_ids()?;
    let calls: BTreeMap<_, _> = request
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|part| match part {
            ContentPart::ToolCall {
                logical_id,
                tool_kind,
                namespace,
                ..
            } => Some((logical_id.clone(), (*tool_kind, namespace.clone()))),
            _ => None,
        })
        .collect();
    for part in request
        .messages
        .iter_mut()
        .flat_map(|message| &mut message.content)
    {
        if let ContentPart::ToolResult {
            logical_id,
            tool_kind,
            namespace,
            ..
        } = part
            && let Some((call_kind, call_namespace)) = calls.get(logical_id)
        {
            if tool_kind != call_kind || (namespace.is_some() && namespace != call_namespace) {
                return Err(ModelIrError::ToolContinuationConflict);
            }
            *namespace = call_namespace.clone();
        }
    }
    Ok(())
}
