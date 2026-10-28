use super::*;
use crate::server::core_runtime::model_ir::FinalMessagePhaseV1;

impl IncrementalClientSseRenderer {
    pub(super) fn validate_final_message_phase(
        &self,
        event: &ModelEvent,
    ) -> Result<(), ProtocolAdapterError> {
        let final_phase = match event {
            ModelEvent::TextFinished { final_phase, .. }
            | ModelEvent::RefusalFinished { final_phase, .. } => final_phase,
            _ => return Ok(()),
        };
        if let Some(phase) = final_phase {
            if !client_can_represent_message_phase(self.profile.protocol, phase.value.as_deref()) {
                return Err(unrepresentable("final Responses message phase"));
            }
            if phase
                .value
                .as_ref()
                .is_some_and(|value| value.is_empty() || value.chars().any(char::is_control))
            {
                return Err(invalid("final Responses message phase"));
            }
        }
        Ok(())
    }

    pub(super) fn finalize_message_phase(
        &mut self,
        index: u32,
        final_phase: &Option<FinalMessagePhaseV1>,
    ) -> Result<(), ProtocolAdapterError> {
        if let Some(phase) = final_phase {
            if !matches!(
                self.blocks.get(&index),
                Some(ResponseBlockKind::Text | ResponseBlockKind::Refusal)
            ) {
                return Err(invalid("final phase on non-message block"));
            }
            if let Some(value) = &phase.value {
                self.charge_metadata(value.len())?;
                self.message_phases.insert(index, value.clone());
            } else {
                self.message_phases.remove(&index);
            }
        }
        Ok(())
    }
}
