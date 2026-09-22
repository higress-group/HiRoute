//! Plan client context policy, independent of model capability declarations.
use super::{CompiledPlanError, DEFAULT_PLAN_CONTEXT_WINDOW_TOKENS, MaterializedAgentPlanV1};

impl MaterializedAgentPlanV1 {
    /// All active groups and their published ingress profiles must support the chosen window.
    pub fn context_window_upper_bound(&self) -> Result<u64, CompiledPlanError> {
        let mut upper = None;
        for candidate in self.attempt_owned.groups.iter().flat_map(|g| &g.candidates) {
            if candidate.protocol_profiles.is_empty() {
                return Err(CompiledPlanError::InvalidLimits);
            }
            for profile in &candidate.protocol_profiles {
                let context = &profile.capability.context;
                let input = context
                    .max_input_tokens
                    .exact()
                    .copied()
                    .ok_or(CompiledPlanError::InvalidLimits)?;
                let output = context
                    .max_output_tokens
                    .exact()
                    .copied()
                    .ok_or(CompiledPlanError::InvalidLimits)?;
                let total = context
                    .max_total_tokens
                    .exact()
                    .copied()
                    .ok_or(CompiledPlanError::InvalidLimits)?;
                let reasoning = profile.reasoning_profile_for(&candidate.exact_reasoning)?;
                let reservation = output
                    .checked_add(reasoning.additional_reservation_tokens)
                    .ok_or(CompiledPlanError::InvalidLimits)?;
                let window =
                    total.map_or(input, |total| input.min(total.saturating_sub(reservation)));
                if window == 0 || output == 0 || window > i64::MAX as u64 {
                    return Err(CompiledPlanError::InvalidLimits);
                }
                upper = Some(upper.map_or(window, |previous: u64| previous.min(window)));
            }
        }
        upper.ok_or(CompiledPlanError::InvalidLimits)
    }

    pub fn context_window_tokens(&self) -> Result<u64, CompiledPlanError> {
        let upper = self.context_window_upper_bound()?;
        match self.attempt_owned.limits.context_window_tokens {
            Some(value) if value == 0 || value > upper => Err(CompiledPlanError::InvalidLimits),
            Some(value) => Ok(value),
            None => Ok(upper.min(DEFAULT_PLAN_CONTEXT_WINDOW_TOKENS)),
        }
    }
}
