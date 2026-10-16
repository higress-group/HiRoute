//! Real Local Control/settings/observation stores. Synthetic facts test attribution, not native
//! acceptance; actual installed Agent requests remain a separate validation gate.
use super::*;
use hiroute_domain::{
    ExecutionFactV1, ExecutionRequestOutcomeV1, FactsCompleteness, FrozenExecutionTrustV1,
    ObservationStreamV1, ProducerEpoch, ProducerId, StreamId,
};
use hiroute_observation::{LocalObservationWriter, OfferOutcome, WriterCycleOutcome};

fn effective(fixture: &Fixture, plan: &str, request_id: Option<&str>) -> serde_json::Value {
    let response = fixture.service.dispatch_wire(request(
        "GetAgentConnectionStatus",
        json!({
            "schema_version":{"major":2,"minor":0},"context_id":fixture.context,
            "effective":{"plan_id":plan,"request_id":request_id}
        }),
        None,
    ));
    assert!(response.error.is_none(), "{response:?}");
    response.data.unwrap()["effective"].clone()
}

fn trust(fixture: &Fixture) -> FrozenExecutionTrustV1 {
    let join = fixture
        .runtime
        .adapter
        .configured_model_settings_join(&fixture.context)
        .unwrap()
        .unwrap();
    let stores = fixture.runtime.adapter.stores_lock().unwrap();
    let publication = stores
        .control()
        .active_publication(&WorkspaceId::default())
        .unwrap()
        .unwrap()
        .verify()
        .unwrap()
        .gateway_snapshot()
        .unwrap();
    crate::control::runtime::agent_live_check::expected_trust(
        &publication,
        join.grant.grant_id(),
        join.grant
            .scope()
            .model_grant()
            .routes
            .keys()
            .next()
            .unwrap(),
    )
    .unwrap()
}

fn record(
    fixture: &Fixture,
    suffix: &str,
    trust: &FrozenExecutionTrustV1,
    accepted: bool,
    attributed: bool,
) {
    use hiroute_domain::CorrelationProvenance;
    use std::time::{SystemTime, UNIX_EPOCH};
    let stream = ObservationStreamV1 {
        producer_id: ProducerId::parse(format!("producer-{suffix}")).unwrap(),
        producer_epoch: ProducerEpoch::parse(format!("epoch-{suffix}")).unwrap(),
        stream_id: StreamId::parse(format!("stream-{suffix}")).unwrap(),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
        - 100;
    let mut event = crate::control::runtime::tests::envelope(
        &stream,
        suffix,
        trust.agent_plan_id.as_ref().unwrap().as_str(),
        1,
        now,
        ExecutionFactV1::RequestFinished {
            outcome: if accepted {
                ExecutionRequestOutcomeV1::Accepted
            } else {
                ExecutionRequestOutcomeV1::Failed
            },
            attempts_started: 0,
            attempts_finished: 0,
            accepted_attempt_ordinal: None,
            facts_completeness: FactsCompleteness::Unknown,
        },
    );
    event.trust = trust.clone();
    if !attributed {
        event.correlation.correlation_provenance = CorrelationProvenance::Unproven;
    }
    let writer = LocalObservationWriter::new(fixture.runtime.observation.clone());
    let channel = hiroute_observation::FactChannel::new(stream, 128 * 1024);
    assert_eq!(channel.offer(event), OfferOutcome::Accepted);
    assert!(matches!(
        writer.consume_fact(&channel),
        WriterCycleOutcome::Ack(_)
    ));
}

#[test]
fn effective_status_preserves_legacy_shape_and_never_activates_or_calls_models() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::effective::effective_status_preserves_legacy_shape_and_never_activates_or_calls_models",
    ) {
        return;
    }
    let fixture = Fixture::new(true);
    let spec = fixture.model_spec(0);
    let plan = spec["model"]["settings"]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    apply(&fixture.service, &fixture.runtime, spec, "effective-save");
    let before = fs::read(fixture.profile.join("config.yaml")).unwrap();
    assert!(fixture.status().get("effective").is_none());
    let state = effective(&fixture, &plan, None);
    assert_eq!(state["preparation"]["state"], "known");
    assert_eq!(state["native_prerequisites"]["state"], "known");
    assert_eq!(state["native_loading"]["state"], "unknown");
    assert_eq!(state["current_selection"]["state"], "unknown");
    assert_eq!(state["request_evidence"]["state"], "pending");
    assert!(state["observed_request"].is_null());
    assert_eq!(
        before,
        fs::read(fixture.profile.join("config.yaml")).unwrap()
    );
    assert!(state["next_actions"].as_array().unwrap().iter().any(
        |a| a["prerequisite"] == "explicit_model_call_consent_or_an_existing_matching_request"
    ));
    assert_eq!(
        effective(&fixture, "plan/not-granted", None)["request_evidence"]["state"],
        "blocked"
    );

    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.restore(),
        "effective-restore",
    );
    let restored = effective(&fixture, &plan, None);
    assert_eq!(
        restored["preparation"]["reason"],
        "owned_configuration_restored"
    );
    assert_eq!(restored["request_evidence"]["state"], "pending");
    assert!(
        restored["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["action"] == "confirm_restored_selection")
    );
    assert_eq!(
        fs::read(fixture.profile.join("config.yaml")).unwrap(),
        ORIGINAL
    );
}

#[test]
fn effective_status_accepts_exact_receipts_and_rejects_stale_or_unrelated_evidence() {
    if isolate(
        "control::runtime::native_model::tests::settings_entry_tests::hermes::effective::effective_status_accepts_exact_receipts_and_rejects_stale_or_unrelated_evidence",
    ) {
        return;
    }
    let fixture = Fixture::new(true);
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.model_spec(0),
        "effective-receipts",
    );
    let expected = trust(&fixture);
    let plan = expected.agent_plan_id.as_ref().unwrap().as_str();
    for (suffix, accepted) in [("effective-success", true), ("effective-failure", false)] {
        record(&fixture, suffix, &expected, accepted, true);
        let state = effective(&fixture, plan, Some(&format!("request-{suffix}")));
        assert_eq!(state["request_evidence"]["state"], "known", "{state}");
        assert_eq!(
            state["observed_request"]["outcome"],
            if accepted { "accepted" } else { "failed" }
        );
        assert_eq!(
            state["observed_request"]["grant_generation"],
            expected.grant_generation
        );
        assert!(state["observed_request"]["actual_model"].is_null());
        assert_eq!(state["observed_request"]["usage"], json!([]));
        assert_eq!(state["native_loading"]["state"], "unknown");
    }
    let changes: [fn(&mut FrozenExecutionTrustV1); 7] = [
        |t: &mut FrozenExecutionTrustV1| t.grant_generation += 1,
        |t: &mut FrozenExecutionTrustV1| t.grant_id = "grant-other-profile".into(),
        |t: &mut FrozenExecutionTrustV1| {
            t.gateway_publication_digest = CanonicalDigest::of_bytes(b"old-publication")
        },
        |t: &mut FrozenExecutionTrustV1| t.gateway_publication_revision = "999999".into(),
        |t: &mut FrozenExecutionTrustV1| t.authority_epoch += 1,
        |t: &mut FrozenExecutionTrustV1| {
            if let hiroute_domain::ModelRequestRouteV2::Plan { revision, .. } = &mut t.route {
                *revision += 1;
            }
        },
        |t: &mut FrozenExecutionTrustV1| {
            t.agent_plan_id = Some(hiroute_domain::AgentPlanId::parse("plan/unrelated").unwrap())
        },
    ];
    for (index, change) in changes.into_iter().enumerate() {
        let suffix = format!("effective-mismatch-{index}");
        let mut other = expected.clone();
        change(&mut other);
        record(&fixture, &suffix, &other, true, true);
        let state = effective(&fixture, plan, Some(&format!("request-{suffix}")));
        assert_eq!(state["request_evidence"]["state"], "pending", "{state}");
        assert!(state["observed_request"].is_null());
    }
    record(&fixture, "unattributed", &expected, true, false);
    assert_eq!(
        effective(&fixture, plan, Some("request-unattributed"))["request_evidence"]["state"],
        "pending"
    );
    // Reconfiguration gives a different current grant/Plan; the old success cannot complete it.
    apply(
        &fixture.service,
        &fixture.runtime,
        fixture.model_spec(1),
        "effective-reconfigure",
    );
    let state = effective(&fixture, plan, Some("request-effective-success"));
    assert_ne!(state["request_evidence"]["state"], "known");
}
