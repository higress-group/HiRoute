use super::*;
use hiroute_domain::{
    GatewayCriticalFactV1 as Fact, GatewayPublicationV1, GatewayReasoningAccountingV1,
};

fn plan() -> MaterializedAgentPlanV1 {
    let publication: GatewayPublicationV1 = serde_json::from_slice(include_bytes!(
        "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    publication.plans[0].body.materialized.clone()
}

#[test]
fn mixed_candidates_and_protocols_share_the_smallest_exact_output() {
    let mut plan = plan();
    plan.attempt_owned.limits.context_window_tokens = Some(32_768);
    let mut fallback = plan.attempt_owned.groups[0].clone();
    // The tightest output belongs to a later group and a non-first protocol.
    let profiles = &mut fallback.candidates.last_mut().unwrap().protocol_profiles;
    assert!(profiles.len() > 1);
    profiles
        .last_mut()
        .unwrap()
        .capability
        .context
        .max_output_tokens = Fact::Exact(4096);
    plan.attempt_owned.groups.push(fallback);
    assert_eq!(
        qoder_plan_token_budget(&plan).unwrap(),
        QoderTokenBudget {
            context_window_tokens: 32_768,
            max_output_tokens: 4096
        }
    );
}

#[test]
fn one_unknown_or_zero_output_cannot_be_skipped_in_a_mixed_plan() {
    for invalid in [Fact::Unknown, Fact::Exact(0)] {
        let mut plan = plan();
        plan.attempt_owned
            .groups
            .last_mut()
            .unwrap()
            .candidates
            .last_mut()
            .unwrap()
            .protocol_profiles
            .last_mut()
            .unwrap()
            .capability
            .context
            .max_output_tokens = invalid;
        assert!(qoder_plan_token_budget(&plan).is_err());
    }
}

#[test]
fn total_and_reasoning_reservations_and_the_selected_window_remain_authoritative() {
    let mut plan = plan();
    for candidate in plan
        .attempt_owned
        .groups
        .iter_mut()
        .flat_map(|g| &mut g.candidates)
    {
        for profile in &mut candidate.protocol_profiles {
            profile.capability.context.max_input_tokens = Fact::Exact(100_000);
            profile.capability.context.max_output_tokens = Fact::Exact(4096);
            profile.capability.context.max_total_tokens = Fact::Exact(Some(50_000));
            for reasoning in &mut profile.capability.reasoning_profiles {
                reasoning.accounting = GatewayReasoningAccountingV1::Additive;
                reasoning.additional_reservation_tokens = 8192;
            }
        }
    }
    assert_eq!(
        qoder_plan_token_budget(&plan)
            .unwrap()
            .context_window_tokens,
        37_712
    );
    plan.attempt_owned.limits.context_window_tokens = Some(32_768);
    assert_eq!(
        qoder_plan_token_budget(&plan)
            .unwrap()
            .context_window_tokens,
        32_768
    );
    plan.attempt_owned.limits.context_window_tokens = Some(37_713);
    assert!(qoder_plan_token_budget(&plan).is_err());
}

#[test]
fn zero_native_compaction_threshold_is_rejected_without_rounding_up_the_window() {
    for (context, output) in [
        (0, 4096),
        (32_768, 32_000),
        (33_000, 32_000),
        (17_096, 4096),
    ] {
        assert!(QoderTokenBudget::new(context, output).is_err());
    }
    for (context, output) in [(17_097, 4096), (32_768, 4096), (100_000, 32_000)] {
        let budget = QoderTokenBudget::new(context, output).unwrap();
        assert_eq!(
            (budget.context_window_tokens, budget.max_output_tokens),
            (context, output)
        );
    }
    assert!(QoderTokenBudget::new(100_000, 0).is_err());
    assert!(QoderTokenBudget::new(1 << 53, 4096).is_err());
    assert!(QoderTokenBudget::new(100_000, 1 << 53).is_err());
}

#[test]
fn renderer_requires_a_usable_worker_budget_and_keeps_probe_budget_separate() {
    for (context, output, accepted) in [
        (Some(32_768), 4096, true),
        (Some(32_768), 32_000, false),
        (Some(17_096), 4096, false),
        (Some(100_000), 0, false),
        (None, 2048, true),
    ] {
        let route =
            crate::agents::render_qoder_transient_route(super::super::QoderTransientRouteInput {
                protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
                provider_id: "hiroute-budget-test",
                endpoint: "http://127.0.0.1:4321/v1",
                alias: "plan/branch:cheap",
                credential_env: "HIROUTE_RUN_TOKEN",
                context_window_tokens: context,
                max_output_tokens: output,
            });
        assert_eq!(route.is_ok(), accepted);
        if let Ok(route) = route {
            let model = &route.settings["providers"]["hiroute-budget-test"]["models"][0];
            assert_eq!(model["maxOutputTokens"], output);
            match context {
                Some(context) => assert_eq!(model["contextWindow"], context),
                None => assert!(model.get("contextWindow").is_none()),
            }
        }
    }
}
