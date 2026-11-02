use super::*;
use crate::server::core_runtime::classification::protocol::assessment_definition_result;
use hiroute_domain::ProtectionEligibilityV1;

#[test]
fn assessment_is_exactly_the_preceding_turn_while_older_history_remains_context() {
    let authority = c_authority();
    let mut history = c_history(&authority);
    history
        .visible_conversation
        .push(history.visible_conversation[0].clone());
    // Even a nominal one-turn identity must not expand to a multi-turn suffix.
    assert!(matches!(
        assessment_definition_result(&history, &authority),
        Err(ProtectionEligibilityV1::InvalidBoundary)
    ));
    history.assessment_from = Some(1);
    assert!(assessment_definition_result(&history, &authority).is_ok());
    let target = history.assessment_target.as_mut().unwrap();
    target.through_ordinal = 2;
    target.through_turn_id = "turn-2".into();
    assert!(matches!(
        assessment_definition_result(&history, &authority),
        Err(ProtectionEligibilityV1::InvalidBoundary)
    ));
    // No historical score/applicability questions can be emitted for an old range.
    let prepared = request_template(&authority, &diagnostic_request(), &history, false).unwrap();
    let bindings = prepared.bindings.unwrap();
    assert!(bindings.assessment.is_none());
    assert!(bindings.applicability.is_none());
}
