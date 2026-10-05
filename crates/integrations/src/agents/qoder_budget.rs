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
    let (context, output) = plan_token_budgets(plan)?;
    QoderTokenBudget::new(context, output).map_err(Into::into)
}

pub fn pi_plan_token_budget(
    plan: &MaterializedAgentPlanV1,
) -> Result<QoderTokenBudget, QoderNativeError> {
    let (context, output) = plan_token_budgets(plan)?;
    let declaration = hiroute_domain::AdditionalAgentModelV1 {
        alias: "hiroute-budget".into(),
        context_window_tokens: context,
        max_output_tokens: output,
    };
    declaration
        .validate_pi()
        .map_err(|_| qoder_error("Pi frozen budget"))?;
    Ok(QoderTokenBudget {
        context_window_tokens: context,
        max_output_tokens: output,
    })
}

fn plan_token_budgets(plan: &MaterializedAgentPlanV1) -> Result<(u64, u64), QoderNativeError> {
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
    Ok((
        context,
        output.ok_or_else(|| qoder_error("frozen Plan output budget"))?,
    ))
}

#[cfg(test)]
#[path = "qoder_budget_tests.rs"]
mod tests;
