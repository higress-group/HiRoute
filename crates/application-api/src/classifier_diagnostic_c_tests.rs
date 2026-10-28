use super::*;
use hiroute_domain::{
    CategoryOptionV1, DecisionDefinitionV1, JudgmentSettingsV1, NativeProtocolV1,
    SMART_SAVING_SCOPE_ID,
};

fn request(
    protocol: NativeProtocolV1,
    shape: CCapabilityShapeV1,
) -> HistoricalApplicabilityTestRequestV1 {
    let judgment = JudgmentSettingsV1::default();
    let (decision, scopes) = match shape {
        CCapabilityShapeV1::Standard => (
            judgment.degree.definition().into(),
            vec![(SMART_SAVING_SCOPE_ID.to_owned(), judgment.clone())],
        ),
        CCapabilityShapeV1::MaxBranches => {
            let options = (0..16)
                .map(|i| CategoryOptionV1 {
                    id: format!("category-{i}"),
                    criterion: format!("Synthetic category {i}"),
                    refinement: Some(judgment.degree.definition()),
                })
                .collect::<Vec<_>>();
            let scopes = options
                .iter()
                .map(|o| (o.id.clone(), judgment.clone()))
                .collect();
            (
                DecisionDefinitionV1::Categorical {
                    instructions: "Choose a category".into(),
                    options,
                },
                scopes,
            )
        }
    };
    HistoricalApplicabilityTestRequestV1 {
        schema: HISTORICAL_APPLICABILITY_TEST_SCHEMA_V1.into(),
        service_id: "fixture".into(),
        service_revision: 1,
        protocol: FrozenNativeProtocolV1::freeze(&protocol, decision, scopes).unwrap(),
        shape,
    }
}

#[test]
fn diagnostic_accepts_both_sealed_versions_and_rejects_mismatches() {
    for protocol in [NativeProtocolV1::c_v1(), NativeProtocolV1::c_v2()] {
        for shape in [
            CCapabilityShapeV1::Standard,
            CCapabilityShapeV1::MaxBranches,
        ] {
            let valid = request(protocol.clone(), shape);
            assert!(valid.validate());
            let bytes = serde_json::to_vec(&valid).unwrap();
            let decoded: HistoricalApplicabilityTestRequestV1 =
                serde_json::from_slice(&bytes).unwrap();
            assert_eq!(decoded, valid);
            let mut swapped = decoded.clone();
            swapped.protocol.protocol.fixed = if protocol.schema == hiroute_domain::SYSTEM_ONE_C_V1
            {
                hiroute_domain::NativeFixedContentV1::c_v2()
            } else {
                hiroute_domain::NativeFixedContentV1::c_v1()
            };
            assert!(!swapped.validate());
            let mut unknown = decoded.clone();
            unknown.protocol.protocol.schema = "hiroute.system-one-c/v99".into();
            assert!(!unknown.validate());
            let mut cap = decoded.clone();
            cap.protocol.protocol.prior_context_max_turns = 7;
            assert!(!cap.validate());
            let mut digest = decoded;
            digest.protocol.digest = CanonicalDigest::of_bytes(b"tampered");
            assert!(!digest.validate());
        }
    }
}
