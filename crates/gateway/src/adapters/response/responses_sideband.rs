//! Recognized response notifications carry no canonical model output.
//!
//! Native delivery preserves their original bytes. This does not admit arbitrary
//! metadata: safety buffering, verification and unknown variants need their own
//! semantics before they can enter the canonical decoder.
use serde_json::{Map, Value};

use super::wire::{object_field, optional_u64, required_str};
use super::{ModelIrError, ProtocolAdapterError};

pub(super) fn moderation_notification(
    object: &Map<String, Value>,
    response_id: Option<&str>,
) -> Result<(), ProtocolAdapterError> {
    known_fields(
        object,
        &["type", "response_id", "sequence_number", "metadata"],
        "response.metadata envelope extension",
    )?;
    let id = required_str(object, "response_id")?;
    if id.is_empty() || response_id != Some(id) {
        return Err(ModelIrError::InvalidResponseLifecycle(
            "moderation notification does not belong to the started response".into(),
        )
        .into());
    }
    optional_u64(object, "sequence_number")?
        .ok_or(ModelIrError::InvalidField("sequence_number"))?;
    let metadata = object_field(object, "metadata")?;
    known_fields(
        metadata,
        &["openai_chatgpt_moderation_metadata"],
        "response.metadata variant",
    )?;
    let moderation = object_field(metadata, "openai_chatgpt_moderation_metadata")?;
    known_fields(
        moderation,
        &["prompt", "generation", "tool_call", "tool_response"],
        "response.metadata moderation variant",
    )?;
    if moderation.values().any(|value| !value.is_object()) {
        return Err(ModelIrError::InvalidField("moderation notification").into());
    }
    // The inner notification objects remain opaque. They cannot add text/tools,
    // establish response identity, alter refusal/error or supply a terminal event.
    Ok(())
}

fn known_fields(
    object: &Map<String, Value>,
    fields: &[&str],
    context: &'static str,
) -> Result<(), ProtocolAdapterError> {
    if object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(ModelIrError::UnsupportedField(context.into()).into());
    }
    Ok(())
}
