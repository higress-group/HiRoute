//! Spending admission and settlement own no observation writer or routing state.
use super::*;
use crate::ports::{CredentialLease, KeySpendGuard};

pub(super) fn reserve(
    credential: Option<&CredentialLease>,
    ir: &crate::server::core_runtime::model_ir::ModelRequestIRV1,
    profile: &CandidateProtocolProfile,
    stable_target: &str,
    expected_profile_digest: &str,
) -> Result<Option<KeySpendGuard>, Arc<str>> {
    let currency = credential
        .map(CredentialLease::spending_currency)
        .transpose()
        .map_err(|_| Arc::from(MATERIALIZATION_AUTHORITY_FAILED))?
        .flatten();
    let spend_estimate = if !ir.native_only && ir.web_search.is_none() {
        crate::server::core_runtime::observation::active_request()
            .and_then(|observation| {
                observation.spending_price(
                    stable_target,
                    &profile.capability.model_configuration_id,
                    expected_profile_digest,
                    currency.as_deref()?,
                )
            })
            .and_then(|pricing| {
                Some(hiroute_domain::KeySpendEstimateV1 {
                    pricing,
                    input_bound: *profile.capability.context.max_input_tokens.exact()?,
                    output_bound: ir
                        .requested_max_output_tokens
                        .unwrap_or(*profile.capability.context.max_output_tokens.exact()?)
                        .min(*profile.capability.context.max_output_tokens.exact()?),
                })
            })
    } else {
        None
    };
    credential
        .map(|lease| lease.reserve_spending(spend_estimate.as_ref()))
        .transpose()
        .map(Option::flatten)
        .map_err(|error| match error {
            hiroute_domain::KeyBudgetGateErrorV1::Authority(_) => {
                Arc::from(MATERIALIZATION_AUTHORITY_FAILED)
            }
            _ => Arc::from(MATERIALIZATION_CREDENTIAL_EXHAUSTED),
        })
}

pub(super) fn settle(
    spending: Option<&KeySpendGuard>,
    facts: &ProviderClassificationFacts,
    completion: &ProviderAttemptCompletion,
) {
    if let Some(spending) = spending {
        use hiroute_domain::{KeySpendSettlementV1 as Settlement, KeySpendUsageV1};
        let settlement = if completion.commits.upstream_request
            == hiroute_gateway_core::runtime::attempt::CommitFence::Clear
        {
            Settlement::Unsent
        } else if completion.stream
            == hiroute_gateway_core::runtime::driver::AttemptStreamOutcome::CompletedEos
        {
            facts
                .usage
                .as_ref()
                .map(|usage| Settlement::Usage {
                    usage: KeySpendUsageV1 {
                        input_tokens: usage.input.units,
                        output_tokens: usage.output.units,
                        cache_read_tokens: usage.cache_read.units,
                        cache_write_tokens: usage.cache_write.units,
                        reasoning_tokens: usage.reasoning.units,
                    },
                })
                .unwrap_or(Settlement::Unknown)
        } else {
            Settlement::Unknown
        };
        spending.settle(settlement);
    }
}
