//! Persisted custom defaults must survive real connector-owned candidate construction.
use super::*;
use hiroute_application::control::RoutingFactsPort;
use hiroute_domain::{
    ModelDefaultFieldV1, ModelDefaultPreferenceV1, ModelDefaultValueV1, NativeReasoningCapabilityV1,
};

#[test]
fn qualified_claude_custom_fixed_remains_routable_through_cpa() {
    if crate::test_support::isolated_agent_home(concat!(
        "control::runtime::subscriptions::login_admission_tests::model_defaults_tests::",
        "qualified_claude_custom_fixed_remains_routable_through_cpa"
    )) {
        return;
    }
    let fixture = Fixture::new();
    let login = fixture.authorize(SubscriptionLoginProviderV1::Claude);
    let checked = fixture.check(&login);
    fixture.save(fixture.current_candidate_request(&checked, "claude-custom-initial"));
    let before = fixture.snapshot();
    let source = &before.sources[0];
    let binding = &source.models[0].binding_id;
    let fixed = NativeReasoningCapabilityV1::Fixed {
        profile: "provider-default".into(),
    };
    let old = fixture.saved_request(
        &checked,
        ComputeManagementIntentV2::SaveReady,
        "claude-custom-template",
    );
    let change = hiroute_application_api::saved_compute_change(old.spec.desired_state).unwrap();
    let preview = fixture
        .adapter()
        .preview_compute_save_v3(hiroute_application_api::ComputeManagementChangeV3 {
            schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
            change,
            api_link: None,
            default_preferences: [(
                binding.clone(),
                [(
                    ModelDefaultFieldV1::NativeReasoning,
                    ModelDefaultPreferenceV1::Custom {
                        value: ModelDefaultValueV1::Reasoning(fixed.clone()),
                    },
                )]
                .into_iter()
                .collect(),
            )]
            .into_iter()
            .collect(),
        })
        .unwrap();
    fixture.save(ComputeConnectionApplyRequestV1 {
        spec: preview.spec,
        accept_digest: preview.accept_digest,
        expected_revisions: preview.expected_revisions,
        idempotency_key: "claude-custom-fixed".into(),
    });
    let snapshot = fixture
        .adapter()
        .routing_compilation_snapshot(&WorkspaceId::default())
        .unwrap();
    let candidate = snapshot
        .facts
        .candidates
        .iter()
        .find(|c| &c.binding.binding_id == binding)
        .expect("qualified connector-owned candidate must survive a valid custom fixed choice");
    assert!(candidate.is_routable());
    assert_eq!(candidate.reasoning, fixed);
    candidate.validate().unwrap();
    assert_eq!(
        fixture.snapshot().sources[0].models,
        before.sources[0].models
    );
}
