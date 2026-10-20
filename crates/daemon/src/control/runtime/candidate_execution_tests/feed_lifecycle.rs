use super::*;
use crate::model_feed::fixtures;
use hiroute_integrations::model_feed::admit;

#[test]
fn feed_removal_excludes_new_compilation_but_keeps_frozen_execution_self_contained() {
    let embedded = crate::release_catalog::current_fixture_catalog();
    let feed = fixtures::publication(17);
    let catalog = embedded
        .with_feed(&admit(&embedded, &feed, env!("CARGO_PKG_VERSION")).unwrap())
        .unwrap();
    let fact = registered_fact(&catalog, "bailian.payg.cn.v1", fixtures::MODEL);
    let candidate = materialize_registered_management_candidate(&fact, &catalog).unwrap();
    let selection = hiroute_domain::AgentFixedModelSelectionV2 {
        client_model_id: "feed-model".into(),
        candidate: hiroute_domain::CandidateSelectionV1 {
            binding_id: fact.binding_id.clone(),
            reasoning: Some(hiroute_domain::ReasoningSelectionV1::Toggle { enabled: true }),
        },
    };
    let frozen =
        hiroute_application::compiler::compile_fixed_model_bindings(&[selection], &[candidate])
            .unwrap();
    let bytes = serde_json::to_vec(&frozen).unwrap();
    // Current authoring has lost the feed model; a saved source is not new admission evidence.
    assert!(materialize_registered_management_candidate(&fact, &embedded).is_none());
    // Runtime/Worker bindings deserialize without looking up a mutable model catalog.
    let restored: std::collections::BTreeMap<String, hiroute_domain::AttemptOwnedCandidateV1> =
        serde_json::from_slice(&bytes).unwrap();
    let attempt = &restored["feed-model"];
    assert_eq!(attempt.model_configuration_id, fixtures::MODEL);
    assert_eq!(attempt.upstream_model_id, "feed-regression");
    assert_eq!(attempt.credential_refs.len(), 2);
    assert!(!attempt.protocol_profiles.is_empty());
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
}
