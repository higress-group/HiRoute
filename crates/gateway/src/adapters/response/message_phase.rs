//! Added-item metadata is provisional; done-item metadata seals the message.
use super::wire::optional_str;
use super::*;
use crate::server::core_runtime::model_ir::FinalMessagePhaseV1;

impl DecoderCore {
    pub(super) fn observe_message_phase(
        &mut self,
        native_index: u32,
        item: &Map<String, Value>,
        complete_item: bool,
        final_snapshot: bool,
    ) -> Result<(), ProtocolAdapterError> {
        let phase = optional_str(item, "phase")?;
        validate_phase(phase)?;
        if complete_item {
            if let Some(established) = self.final_message_phases.get(&native_index) {
                // A response-level snapshot can omit optional metadata already
                // sealed by item.done. Explicit null is still an observation.
                if final_snapshot && !item.contains_key("phase") {
                    return Ok(());
                }
                if established.as_deref() != phase {
                    return Err(ModelIrError::InvalidResponseLifecycle(
                        "Responses final message phase disagrees with output_item.done".into(),
                    )
                    .into());
                }
                return Ok(());
            }
            self.charge_message_phase(phase)?;
            self.final_message_phases
                .insert(native_index, phase.map(str::to_owned));
        } else if let Some(phase) = phase {
            if self.native_message_phases.contains_key(&native_index)
                || self.final_message_phases.contains_key(&native_index)
                || self
                    .native_blocks
                    .contains_key(&(NativeBlockKind::Text, native_index))
                || self
                    .native_blocks
                    .contains_key(&(NativeBlockKind::Refusal, native_index))
            {
                return Err(ModelIrError::InvalidResponseLifecycle(
                    "Responses message metadata started after its message".into(),
                )
                .into());
            }
            self.charge_message_phase(Some(phase))?;
            self.native_message_phases
                .insert(native_index, phase.to_owned());
        }
        Ok(())
    }

    fn charge_message_phase(&mut self, phase: Option<&str>) -> Result<(), ProtocolAdapterError> {
        let bytes = phase.map_or(0, str::len);
        self.retention.add(bytes.saturating_add(64))?;
        self.semantic_bytes = self
            .semantic_bytes
            .checked_add(bytes)
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        Ok(())
    }

    pub(super) fn native_message_phase(&self, native_index: u32) -> Option<String> {
        self.final_message_phases
            .get(&native_index)
            .cloned()
            .unwrap_or_else(|| self.native_message_phases.get(&native_index).cloned())
    }

    pub(super) fn final_message_phase(&self, native_index: u32) -> Option<FinalMessagePhaseV1> {
        self.final_message_phases
            .get(&native_index)
            .map(|value| FinalMessagePhaseV1 {
                value: value.clone(),
            })
    }

    pub(super) fn apply_final_message_phase(
        &mut self,
        index: u32,
        final_phase: &Option<FinalMessagePhaseV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if let Some(phase) = final_phase {
            validate_phase(phase.value.as_deref())?;
            if let Some(value) = &phase.value {
                self.accumulator.message_phases.insert(index, value.clone());
            } else {
                self.accumulator.message_phases.remove(&index);
            }
        }
        Ok(())
    }
}

fn validate_phase(phase: Option<&str>) -> Result<(), ProtocolAdapterError> {
    if phase.is_some_and(|phase| phase.is_empty() || phase.chars().any(char::is_control)) {
        return Err(ModelIrError::InvalidField("response output message phase").into());
    }
    Ok(())
}
