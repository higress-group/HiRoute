//! Frozen Worker token budgets projected into Qoder's native model catalogue.
use hiroute_domain::MaterializedAgentPlanV1;

use super::{QoderNativeError, qoder_error};

pub use hiroute_domain::QoderTokenBudget;

pub(super) fn validate_output(tokens: u64) -> Result<(), QoderNativeError> {
    hiroute_domain::validate_qoder_output_budget(tokens).map_err(Into::into)
}

pub fn qoder_plan_token_budget(
    plan: &MaterializedAgentPlanV1,
) -> Result<QoderTokenBudget, QoderNativeError> {
    // Reuse the Plan policy, including total/output/reasoning reservations and
    // an explicitly chosen smaller window. Do not reconstruct an input capacity.
    let context = plan
        .context_window_tokens()
        .map_err(|_| qoder_error("frozen Plan context budget"))?;
    let mut output = None;
    for candidate in plan
        .attempt_owned
        .groups
        .iter()
        .flat_map(|group| &group.candidates)
    {
        for profile in &candidate.protocol_profiles {
            let current = profile
                .capability
                .context
                .max_output_tokens
                .exact()
                .copied()
                .ok_or_else(|| qoder_error("frozen Plan output budget"))?;
            validate_output(current)?;
            output = Some(output.map_or(current, |previous: u64| previous.min(current)));
        }
    }
    QoderTokenBudget::new(
        context,
        output.ok_or_else(|| qoder_error("frozen Plan output budget"))?,
    )
    .map_err(Into::into)
}

#[cfg(test)]
#[path = "qoder_budget_tests.rs"]
mod tests;
