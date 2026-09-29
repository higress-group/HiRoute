use super::*;
use crate::{NativeReasoningCapabilityV1, ReasoningRenderModeV1};

fn exact(profile: &str) -> ExactNativeReasoningV1 {
    ExactNativeReasoningV1::Profile {
        parameter: "reasoning_effort".into(),
        profile: profile.into(),
        render_mode: ReasoningRenderModeV1::ExplicitNative,
    }
}
fn snapshot() -> RatingSnapshotV2 {
    let mut result = RatingSnapshotV2 {
        schema: RATING_SNAPSHOT_SCHEMA_V2.into(),
        version: "test-1".into(),
        scale_version: "test-scale-1".into(),
        model_catalog_digest: CanonicalDigest::of_bytes(b"models"),
        models: vec![ModelNativeReasoningV1 {
            native_render_convention: None,
            model_configuration_id: "spark".into(),
            capability: NativeReasoningCapabilityV1::Discrete {
                parameter: "reasoning_effort".into(),
                profiles: vec!["low".into(), "xhigh".into()],
                default_profile: None,
            },
        }],
        records: vec![NativeConfigurationRatingV2 {
            model_configuration_id: "spark".into(),
            native_configuration: NativeRatingConfigurationV1::Profile {
                profile: "xhigh".into(),
            },
            overall: RatingValueV1::Reference {
                score_tenths: 45,
                evidence_ref: "test-evidence".into(),
                method_revision: "test-method".into(),
            },
            coding: RatingValueV1::Estimated {
                score_tenths: 43,
                evidence_ref: "test-evidence".into(),
                method_revision: "test-method".into(),
            },
            tool: RatingValueV1::unknown(RatingUnknownReasonV1::RatingNotCollected),
        }],
        digest: CanonicalDigest::of_bytes(b"pending"),
    };
    result.digest = result.computed_digest().unwrap();
    result
}

#[test]
fn ratings_match_only_exact_configuration_and_keep_each_dimension_state() {
    let s = snapshot();
    s.validate().unwrap();
    let high = s.resolve("spark", &exact("xhigh")).unwrap();
    assert!(matches!(
        high.overall,
        RatingValueV1::Reference {
            score_tenths: 45,
            ..
        }
    ));
    assert!(matches!(high.coding, RatingValueV1::Estimated { .. }));
    assert!(matches!(high.tool, RatingValueV1::Unknown { .. }));
    let low = s.resolve("spark", &exact("low")).unwrap();
    assert_eq!(
        low.overall,
        RatingValueV1::unknown(RatingUnknownReasonV1::ConfigurationNotInSnapshot)
    );
    assert!(low.matched_configuration.is_none());
    assert_eq!(
        s.resolve("another-model", &exact("xhigh")).unwrap().overall,
        RatingValueV1::unknown(RatingUnknownReasonV1::IdentityUnresolved)
    );
}

#[test]
fn ratings_budget_never_interpolates_and_rejects_ultra() {
    let s = snapshot();
    let budget = ExactNativeReasoningV1::Budget {
        parameter: "thinking_budget".into(),
        tokens: 4096,
        render_mode: ReasoningRenderModeV1::ExplicitNative,
    };
    assert_eq!(
        s.resolve("spark", &budget).unwrap().overall,
        RatingValueV1::unknown(RatingUnknownReasonV1::ConfigurationNotInSnapshot)
    );
    assert!(s.resolve("spark", &exact("ultra")).is_err());
}

#[test]
fn ratings_reject_duplicate_dangling_out_of_range_and_digest_corruption() {
    let mut s = snapshot();
    s.records.push(s.records[0].clone());
    s.digest = s.computed_digest().unwrap();
    assert!(s.validate().is_err());
    let mut s = snapshot();
    s.records[0].model_configuration_id = "missing".into();
    s.digest = s.computed_digest().unwrap();
    assert!(s.validate().is_err());
    let mut s = snapshot();
    s.records[0].overall = RatingValueV1::Reference {
        score_tenths: 0,
        evidence_ref: "evidence".into(),
        method_revision: "method".into(),
    };
    s.digest = s.computed_digest().unwrap();
    assert!(s.validate().is_err());
    let mut s = snapshot();
    s.version = "changed".into();
    assert!(s.validate().is_err());
    let mut s = snapshot();
    s.records.clear();
    s.digest = s.computed_digest().unwrap();
    s.validate().unwrap();
}

#[test]
fn ratings_wire_parameter_is_not_a_cross_model_identity() {
    let s = snapshot();
    let other_wire = ExactNativeReasoningV1::Profile {
        parameter: "effort".into(),
        profile: "xhigh".into(),
        render_mode: ReasoningRenderModeV1::ExplicitNative,
    };
    assert_eq!(
        s.resolve("spark", &other_wire).unwrap(),
        s.resolve("spark", &exact("xhigh")).unwrap()
    );
}

#[test]
fn ratings_legacy_only_projects_proven_native_effort() {
    let capability = snapshot().models.remove(0).capability;
    assert_eq!(
        legacy_rating_configuration(&capability, "xhigh"),
        Some(NativeRatingConfigurationV1::Profile {
            profile: "xhigh".into()
        })
    );
    assert_eq!(legacy_rating_configuration(&capability, "default"), None);
    assert_eq!(legacy_rating_configuration(&capability, "highest"), None);
}
