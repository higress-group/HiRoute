use super::*;
use hiroute_application_api::{
    ComputeManagementQueryV2, ComputeManagementSnapshotV2, ComputeSubscriptionModeV1,
};

fn connector_source(connector: &str, candidate: &str) -> hiroute_domain::ComputeManagementSourceV2 {
    let mut source = complete_management_source();
    source.provenance = hiroute_domain::ComputeManagementProvenanceV2::ConnectorOwned {
        connector_id: connector.into(),
        account_ref: "account/private".into(),
    };
    source.validation = Some(hiroute_domain::ComputeManagementValidationV2 {
        approval_operation_id: "operation/approval".into(),
        validation_ref: "validation/connector".into(),
        validation_revision: 1,
    });
    source.last_candidate_ref = candidate.into();
    source.validate().unwrap();
    source
}

#[test]
fn saved_subscription_mode_survives_unavailable_credentials_and_v2_stays_strict() {
    for provider in ["codex", "claude"] {
        for (suffix, expected) in [
            ("managed/account", ComputeSubscriptionModeV1::CpaManaged),
            ("native-account", ComputeSubscriptionModeV1::NativeBorrowed),
        ] {
            let source = connector_source(
                &format!("connector.cpa.{provider}"),
                &format!("candidate/cpa/{provider}/{suffix}"),
            );
            // The saved selection is authoritative even without live credential or runtime facts.
            let repository = OneSourceRepository(source.clone());
            let result = query_compute_management_v3(
                &repository,
                &FailingRuntimeRead,
                &hiroute_domain::WorkspaceId::default(),
                &ComputeManagementQueryV2::default(),
                None,
            )
            .unwrap();
            assert_eq!(result.subscription_modes.len(), 1);
            assert_eq!(result.subscription_modes[0].mode, expected);
            assert_eq!(
                result.subscription_modes[0].source_revision,
                source.revision
            );
            assert_eq!(result.subscription_modes[0].source_id, source.source_id);
            let encoded_v3 = serde_json::to_value(&result).unwrap();
            assert!(serde_json::from_value::<ComputeManagementSnapshotV2>(encoded_v3).is_err());
            let v2 = result.into_v2();
            let old = query_compute_management(
                &repository,
                &FailingRuntimeRead,
                &hiroute_domain::WorkspaceId::default(),
                &ComputeManagementQueryV2::default(),
            )
            .unwrap();
            assert_eq!(v2, old);
            let encoded_v2 = serde_json::to_value(&v2).unwrap();
            assert!(encoded_v2.get("subscription_modes").is_none());
            assert_eq!(
                serde_json::from_value::<ComputeManagementSnapshotV2>(encoded_v2).unwrap(),
                old
            );
            let filtered = query_compute_management_v3(
                &repository,
                &FailingRuntimeRead,
                &hiroute_domain::WorkspaceId::default(),
                &ComputeManagementQueryV2 {
                    source_id: Some(source.source_id.clone()),
                },
                None,
            )
            .unwrap();
            assert_eq!(filtered.sources, old.sources);
            assert_eq!(filtered.subscription_modes.len(), 1);
            assert_eq!(filtered.subscription_modes[0].source_id, source.source_id);
            assert!(matches!(
                query_compute_management_v3(
                    &repository,
                    &FailingRuntimeRead,
                    &hiroute_domain::WorkspaceId::default(),
                    &ComputeManagementQueryV2 {
                        source_id: Some("source/other".into())
                    },
                    None,
                ),
                Err(ComputeManagementQueryErrorV2::NotFound)
            ));
        }
    }
}

#[test]
fn mode_requires_matching_connector_and_committed_candidate_not_a_display_label() {
    for (connector, candidate) in [
        ("connector.other", "candidate/cpa/codex/managed/account"),
        (
            "connector.cpa.codex",
            "candidate/cpa/claude/managed/account",
        ),
        ("connector.cpa.codex", "candidate/cpa/codex/"),
        ("connector.cpa.codex", "candidate/cpa/codex/managed/"),
        ("connector.cpa.codex", "candidate/cpa/codex/unknown/account"),
    ] {
        let mut source = connector_source(connector, candidate);
        source.display_name = "Independent sign-in / 独立登录".into();
        let result = query_compute_management_v3(
            &OneSourceRepository(source),
            &RuntimeFacts::default(),
            &hiroute_domain::WorkspaceId::default(),
            &ComputeManagementQueryV2::default(),
            None,
        )
        .unwrap();
        assert!(result.subscription_modes.is_empty());
    }
}
