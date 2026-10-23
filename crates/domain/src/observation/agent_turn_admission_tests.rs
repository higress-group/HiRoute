use super::*;

const PRE_ADMISSION_DIGEST: &str =
    "sha256:57592f8645e74ed2c13a968d8b782bde1819f5f1aa8729b2b5fefa9f5f8d20e6";

fn admission() -> AgentTurnAdmissionV1 {
    AgentTurnAdmissionV1 {
        version: 1,
        agent_turn_id: "turn/admitted".into(),
        branch_id: "history/branch".into(),
        revision: 2,
        ordinal: 3,
        opening_cause: AgentTurnOpeningCauseV1::NoComparableHistory,
    }
}

#[test]
fn admission_is_request_scoped_identity_without_completion_or_source_proof() {
    let mut fact = envelope(1, ExecutionFactV1::AgentTurnAdmitted(admission()), None);
    fact.correlation.correlation_provenance = CorrelationProvenance::Unproven;
    fact.validate().unwrap();
    let bytes = serde_json::to_vec(&fact).unwrap();
    let decoded: ExecutionFactEnvelopeV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, fact);
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
    assert_eq!(
        serde_json::to_value(&decoded.fact).unwrap(),
        serde_json::json!({
            "kind": "agent_turn_admitted", "version": 1,
            "agent_turn_id": "turn/admitted", "branch_id": "history/branch",
            "revision": 2, "ordinal": 3, "opening_cause": "no_comparable_history"
        })
    );
    assert_eq!(decoded.correlation.request_id.as_str(), "request-1");
    assert_eq!(decoded.producer.stream.producer_epoch.as_str(), "epoch-1");
    assert_eq!(
        decoded.correlation.correlation_provenance,
        CorrelationProvenance::Unproven
    );
    fact.attempt_id = Some(AttemptId::parse("attempt/1").unwrap());
    assert_eq!(
        fact.validate(),
        Err(ExecutionFactError::InvalidAttemptScope)
    );
    fact.attempt_id = None;
    fact.pricing = Some(pricing());
    assert_eq!(fact.validate(), Err(ExecutionFactError::InvalidFact));
}

#[test]
fn admission_rejects_invalid_version_identity_and_unowned_proof_fields() {
    let wire = serde_json::to_value(admission()).unwrap();
    for (field, value) in [
        ("version", serde_json::json!(0)),
        ("version", serde_json::json!(2)),
        ("agent_turn_id", serde_json::json!("")),
        ("agent_turn_id", serde_json::json!("../other")),
        ("branch_id", serde_json::json!("")),
        ("branch_id", serde_json::json!(" \t")),
        ("branch_id", serde_json::json!("x".repeat(129))),
        ("revision", serde_json::json!(0)),
        ("ordinal", serde_json::json!(0)),
    ] {
        let mut invalid = wire.clone();
        invalid[field] = value;
        assert_eq!(
            serde_json::from_value::<AgentTurnAdmissionV1>(invalid)
                .unwrap()
                .validate(),
            Err(ExecutionFactError::InvalidFact)
        );
    }
    for field in [
        "segment_id",
        "complete",
        "source_proof",
        "association_unverified",
    ] {
        let mut invalid = wire.clone();
        invalid[field] = serde_json::json!(true);
        assert!(serde_json::from_value::<AgentTurnAdmissionV1>(invalid.clone()).is_err());
        invalid["kind"] = serde_json::json!("agent_turn_admitted");
        assert!(serde_json::from_value::<ExecutionFactV1>(invalid).is_err());
    }
    for field in [
        "version",
        "agent_turn_id",
        "branch_id",
        "revision",
        "ordinal",
        "opening_cause",
    ] {
        let mut invalid = wire.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<AgentTurnAdmissionV1>(invalid).is_err());
    }
}

#[test]
fn no_retained_contract_can_authenticate_a_new_admission() {
    for (schema, digest) in [
        (EXECUTION_FACT_SCHEMA_V2, PRE_ADMISSION_DIGEST),
        (
            EXECUTION_FACT_SCHEMA_V2,
            "sha256:3cb9b100385c1f05a92097c2f28eb5c47d61ca495d32d41a8528b7227c81a465",
        ),
        (
            EXECUTION_FACT_SCHEMA_V2,
            "sha256:806871b6b7a3c1a563e2e214e9af5f0a0b18d1f704e093494c2e1b2316b08069",
        ),
        (
            EXECUTION_FACT_SCHEMA_V2,
            "sha256:7ff01a649de96b1f62e069b872b23609d64a3734b5ad42c337324d3eb7cfc9f2",
        ),
        (
            EXECUTION_FACT_SCHEMA_V2,
            "sha256:4f0dbcc9f7862133e73e451f64d7a6d6e5de146d958ac8bf3a4d1acc885b6b1d",
        ),
        (
            EXECUTION_FACT_SCHEMA_V2,
            "sha256:de91d4f2333db66f0ec3f8b63ce192267dee0a56b65f34180b06996b232fb2c6",
        ),
        (
            EXECUTION_FACT_SCHEMA_V2,
            "sha256:80b4ff13af8abc79d7721fea42141300beade2acf1d2659983ceaafcdc6e4799",
        ),
        (
            "hiroute.observation.execution-fact-envelope/v1",
            "sha256:5aad4c450a2a296ec557b7a934bfbf70ad69a8e8fe9bc7539db46830a2940069",
        ),
    ] {
        let mut fact = envelope(1, ExecutionFactV1::AgentTurnAdmitted(admission()), None);
        fact.schema_version = schema.into();
        fact.schema_digest = CanonicalDigest::parse(digest).unwrap();
        assert_eq!(fact.validate(), Err(ExecutionFactError::UnsupportedSchema));
        assert_eq!(
            fact.validate_persisted_contract(),
            Err(ExecutionFactError::InvalidFact)
        );
    }
}

fn pricing() -> ExecutionPricingEvidenceV1 {
    serde_json::from_value(serde_json::json!({
        "schema_version": "hiroute.observation.execution-pricing/v1",
        "request_generation": null, "captured_at_ms": 0, "attempt_execution_at_ms": 0,
        "quote": null, "reference_quote": null, "unknown_reason": "snapshot_unavailable"
    }))
    .unwrap()
}

#[test]
fn pre_admission_recovery_keeps_every_existing_gateway_shape_and_pricing() {
    for mut fact in gateway_fact_envelopes() {
        fact.schema_digest = CanonicalDigest::parse(PRE_ADMISSION_DIGEST).unwrap();
        if let ExecutionFactV1::RouteDecision(route) = &mut fact.fact {
            route.native_auxiliary = true;
            route.requirements.tool_choice = ToolChoiceRequirementV1::RequiredNamed {
                tool_kind: ToolKindRequirementV1::Custom,
                name: "read_number".into(),
            };
        }
        if matches!(fact.fact, ExecutionFactV1::AttemptStarted { .. }) {
            fact.pricing = Some(pricing());
        }
        assert_eq!(fact.validate(), Err(ExecutionFactError::UnsupportedSchema));
        fact.validate_persisted_contract().unwrap();
        let bytes = serde_json::to_vec(&fact).unwrap();
        let decoded: ExecutionFactEnvelopeV1 = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
        assert_eq!(
            CanonicalDigest::of(&decoded).unwrap(),
            CanonicalDigest::of(&fact).unwrap()
        );
    }
}

#[test]
fn frozen_pre_admission_finished_bytes_keep_unknown_and_membership_without_new_identity() {
    // Synthetic frozen storage bytes matching the Product serializer and digest
    // at 968d3abb3003b55cc03cc594c366d4625fd19801; no current writer creates this input.
    let bytes: &[u8] = include_bytes!("fixtures/pre-admission-968-finished.json");
    let fact: ExecutionFactEnvelopeV1 = serde_json::from_slice(bytes).unwrap();
    assert_eq!(fact.validate(), Err(ExecutionFactError::UnsupportedSchema));
    fact.validate_persisted_contract().unwrap();
    assert_eq!(serde_json::to_vec(&fact).unwrap(), bytes);
    assert_eq!(
        CanonicalDigest::of(&fact).unwrap().as_str(),
        "sha256:931de650bb321ff486755bb4c853246de303f1a708d128381ebbc74582ee39c2"
    );
    let ExecutionFactV1::AgentTurnFinished {
        status,
        opening_cause,
        history_membership,
        history_partial,
        ..
    } = fact.fact
    else {
        panic!("the frozen event must not become an admission")
    };
    assert_eq!(status, AgentTurnStatusV1::Unknown);
    assert_eq!(
        opening_cause,
        Some(AgentTurnOpeningCauseV1::NoComparableHistory)
    );
    assert!(history_partial);
    let membership = history_membership.unwrap();
    assert!(membership.source_unverified && membership.association_unverified);
    assert!(!membership.capture_partial);
    assert_eq!(membership.request_ids, ["request-1"]);
}
