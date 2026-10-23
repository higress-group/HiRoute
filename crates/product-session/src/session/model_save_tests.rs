use super::*;
use hiroute_client_core::{Client, LocalEndpoint};
use hiroute_domain::ModelDefaultPreferenceV1;
use std::sync::Arc;
use zeroize::Zeroizing;
struct Authority;
impl HostAuthority for Authority {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn register_operation(
        &self,
        _: &CanonicalDigest,
        _: &RevisionSetV1,
        _: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        Err("UNEXPECTED_NETWORK_AUTHORITY".into())
    }
    fn register_input(
        &self,
        _: &ComputeCandidateRefV2,
        _: Zeroizing<String>,
    ) -> Result<(), String> {
        Err("UNEXPECTED_INPUT".into())
    }
    fn release_model_input(&self, _: &ComputeCandidateRefV2) -> Result<(), String> {
        Err("UNEXPECTED_INPUT".into())
    }
}
#[tokio::test]
async fn model_defaults_v3_confirmation_preserves_exact_request_and_needs_no_second_preview() {
    for version in [2, 3] {
        let root = tempfile::tempdir().unwrap();
        let client = Client::new(
            "default-confirmation-test",
            LocalEndpoint::for_child(root.path(), std::process::id()),
        );
        let mut session = Session::new(client, Arc::new(Authority));
        let revisions = RevisionSetV1 {
            target: 1,
            dependencies: Default::default(),
        };
        let change = ComputeManagementChangeV2 {
            schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
            subject: ComputeManagementSubjectV2::SavedSource {
                source_id: "source/test".into(),
            },
            expected_revisions: revisions.clone(),
            selected_model_refs: vec![],
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: vec![],
            validation: None,
            edit: Some(ComputeManagementEditV1::Rename {
                display_name: "Source".into(),
            }),
        };
        let desired = if version == 3 {
            serde_json::to_value(ComputeManagementChangeV3 {
                schema: hiroute_domain::COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V3.into(),
                change,
                default_preferences: std::collections::BTreeMap::from([(
                    "binding/test".into(),
                    std::collections::BTreeMap::from([(
                        hiroute_domain::ModelDefaultFieldV1::Vision,
                        ModelDefaultPreferenceV1::PreserveSaved,
                    )]),
                )]),
            })
            .unwrap()
        } else {
            serde_json::to_value(change).unwrap()
        };
        let request = ComputeConnectionApplyRequestV1 {
            spec: ChangeSpecV1 {
                schema_version: SchemaVersion::new(1, 0),
                command_id: "compute.connection.apply".into(),
                resource_id: Some("source/test".into()),
                desired_state: desired.clone(),
            },
            accept_digest: CanonicalDigest::of_bytes(b"sealed-preview"),
            expected_revisions: revisions,
            idempotency_key: "default-confirmation".into(),
        };
        let expected_digest = CanonicalDigest::of(&request).unwrap();
        // No socket is listening. A second preview or other network request would fail this test.
        let context = session
            .prepare_model_save_confirmation(request, "en".into())
            .await
            .unwrap();
        assert!(!context.requires_confirmation());
        assert_eq!(context.request.spec.desired_state, desired);
        assert_eq!(
            CanonicalDigest::of(&context.request).unwrap(),
            expected_digest
        );
    }
}
