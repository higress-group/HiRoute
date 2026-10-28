use super::*;

fn request(id: &str, model: &str, at: i64) -> Request {
    Request {
        view: RoutingTimelineRequestV1 {
            request_id: id.into(),
            started_at_ms: at,
            duration_ms: Some(1),
            native_model: Some(model.into()),
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_read_share: None,
            costs: vec![],
            outcome: Some("accepted".into()),
        },
        position: Position::default(),
        first_fact: at,
        native_turn: None,
        run_id: None,
        decision: None,
        candidate: Some((model.into(), Some("default".into()))),
        identities: Default::default(),
        identity_gap: false,
        request_finished: true,
        stage_finished: false,
        models: BTreeMap::from([(1, model.into())]),
        credentials: BTreeMap::new(),
        partial: false,
        body_bytes: 0,
        request_bytes: 0,
        fact_count: 0,
        usage_count: 0,
    }
}

fn turn(id: &str, ids: &[&str], requests: &BTreeMap<String, Request>) -> RoutingTimelineTurnV1 {
    RoutingTimelineTurnV1 {
        turn_id: ids[0].into(),
        agent_turn_id: Some(id.into()),
        segment_id: None,
        started_at_ms: requests[ids[0]].view.started_at_ms,
        finished_at_ms: Some(requests[*ids.last().unwrap()].view.started_at_ms + 1),
        boundary: RoutingTurnBoundaryV1::NewUserMessage,
        plan_id: None,
        plan_revision: None,
        branch_id: None,
        model_configuration_id: None,
        reasoning_profile_id: None,
        group: None,
        selection: None,
        tool_continuations: 0,
        relays: vec![],
        transitions: vec![],
        assessment_state: "unrated".into(),
        assessment: None,
        competence_floor_millis: None,
        protected_turn_ids: vec![],
        status: "completed".into(),
        requests: ids.iter().map(|id| requests[*id].view.clone()).collect(),
        cost_by_model: vec![],
        evidence_partial: false,
        presentation: None,
        content_digest: String::new(),
    }
}

fn scenario(
    opening_model: &str,
) -> (
    RoutingTimelineTurnV1,
    RoutingTimelineTurnV1,
    BTreeMap<String, Request>,
) {
    let requests: BTreeMap<_, _> = [
        request("prior", "regular", 10),
        request("opening", opening_model, 20),
        request("middle", "primary", 30),
        request("final", "primary", 40),
    ]
    .into_iter()
    .map(|r| (r.view.request_id.clone(), r))
    .collect();
    let previous = turn("turn-a", &["prior"], &requests);
    let current = turn("turn-b", &["opening", "middle", "final"], &requests);
    (previous, current, requests)
}

fn classified(
    previous: &RoutingTimelineTurnV1,
    current: &RoutingTimelineTurnV1,
    requests: &BTreeMap<String, Request>,
) -> Vec<AnalysisTransitionV1> {
    classify(
        "session",
        current,
        Some(&(2, "epoch".into())),
        Some((previous, Some(&(1, "epoch".into())))),
        requests,
    )
}

#[test]
fn completed_turn_comparison_replaces_only_the_same_intermediate_endpoint() {
    let (previous, current, requests) = scenario("primary");
    let values = classified(&previous, &current, &requests);
    assert!(
        values.iter().all(|v| v.request_id != "opening"),
        "a complete final comparison resolves the redundant intermediate context reset"
    );
    let final_comparison = values.iter().find(|v| v.request_id == "final").unwrap();
    assert_eq!(
        final_comparison.kind,
        AnalysisTransitionKindV1::BetweenTurnChange
    );
    assert_eq!(
        final_comparison.previous_request_id.as_deref(),
        Some("prior")
    );
    assert_eq!(final_comparison.completeness, "complete");
    assert_eq!(
        current.requests.len(),
        3,
        "request contributions remain intact"
    );
}

#[test]
fn different_intermediate_model_is_not_erased_by_a_complete_turn_comparison() {
    let (previous, current, requests) = scenario("intermediate");
    let values = classified(&previous, &current, &requests);
    assert_eq!(values.len(), 3);
    let opening = values.iter().find(|v| v.request_id == "opening").unwrap();
    assert_eq!(opening.kind, AnalysisTransitionKindV1::Unknown);
    assert_eq!(opening.model.native_model.as_deref(), Some("intermediate"));
    let within_turn = values.iter().find(|v| v.request_id == "middle").unwrap();
    assert_eq!(within_turn.previous_request_id.as_deref(), Some("opening"));
    assert_eq!(within_turn.model_changed, Some(true));
    assert_eq!(within_turn.completeness, "complete");
    assert_eq!(
        values.last().unwrap().kind,
        AnalysisTransitionKindV1::BetweenTurnChange
    );
}

#[test]
fn incomplete_turn_or_endpoint_evidence_does_not_resolve_an_intermediate_unknown() {
    for variant in [
        "partial",
        "unfinished",
        "pending",
        "missing_model",
        "overlapping",
    ] {
        let (mut previous, mut current, mut requests) = scenario("primary");
        match variant {
            "partial" => current.evidence_partial = true,
            "unfinished" => current.finished_at_ms = None,
            "pending" => current.requests.last_mut().unwrap().duration_ms = None,
            "missing_model" => current.requests.last_mut().unwrap().native_model = None,
            "overlapping" => {
                previous.requests[0].duration_ms = Some(20);
                requests.get_mut("prior").unwrap().view.duration_ms = Some(20);
            }
            _ => unreachable!(),
        }
        let values = classified(&previous, &current, &requests);
        assert!(
            values
                .iter()
                .any(|v| v.request_id == "opening" && v.kind == AnalysisTransitionKindV1::Unknown),
            "{variant} must preserve the intermediate unknown"
        );
    }
}

#[test]
fn model_configuration_and_reasoning_are_part_of_the_comparison_identity() {
    for candidate in [
        ("another-config", "default"),
        ("primary", "different-reasoning"),
    ] {
        let (previous, current, mut requests) = scenario("primary");
        requests.get_mut("opening").unwrap().candidate =
            Some((candidate.0.into(), Some(candidate.1.into())));
        let values = classified(&previous, &current, &requests);
        assert!(
            values
                .iter()
                .any(|v| v.request_id == "opening" && v.kind == AnalysisTransitionKindV1::Unknown)
        );
    }
}

#[test]
fn request_fallback_and_credential_relay_survive_the_final_turn_comparison() {
    let (previous, current, mut requests) = scenario("primary");
    let opening = requests.get_mut("opening").unwrap();
    opening.models = BTreeMap::from([
        (1, "regular".into()),
        (2, "primary".into()),
        (3, "primary".into()),
    ]);
    opening.credentials = BTreeMap::from([
        (1, ("source".into(), "key-a".into())),
        (2, ("source".into(), "key-a".into())),
        (3, ("source".into(), "key-b".into())),
    ]);
    let values = classified(&previous, &current, &requests);
    for kind in [
        AnalysisTransitionKindV1::WithinRequestFallback,
        AnalysisTransitionKindV1::CredentialRelay,
    ] {
        assert!(
            values
                .iter()
                .any(|v| v.request_id == "opening" && v.kind == kind)
        );
    }
    assert!(
        values
            .iter()
            .all(|v| v.request_id != "opening" || v.kind != AnalysisTransitionKindV1::Unknown)
    );
}
