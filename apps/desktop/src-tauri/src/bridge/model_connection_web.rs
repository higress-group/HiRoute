//! WebView-safe compute-management DTOs.
//!
//! `validation_revision` is a server-issued `u64` derived from sealed evidence and regularly
//! exceeds JavaScript's exact integer range. Keep the Local Control contract numeric, but carry
//! this one value across the WebView boundary as canonical decimal text.

use hiroute_application_api::*;
use serde::{Deserialize, Serialize};

/// The only discovery handle allowed to cross into the WebView. The revision is decimal text so
/// JavaScript cannot round a server-issued `u64` before returning it to the native bridge.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeDiscoveryRefV1 {
    discovery_ref: String,
    discovery_revision: String,
}

impl From<ComputeDiscoveryRefV1> for WebComputeDiscoveryRefV1 {
    fn from(value: ComputeDiscoveryRefV1) -> Self {
        Self {
            discovery_ref: value.discovery_ref,
            discovery_revision: value.discovery_revision.to_string(),
        }
    }
}

impl TryFrom<WebComputeDiscoveryRefV1> for ComputeDiscoveryRefV1 {
    type Error = &'static str;

    fn try_from(value: WebComputeDiscoveryRefV1) -> Result<Self, Self::Error> {
        let discovery_revision = value
            .discovery_revision
            .parse::<u64>()
            .map_err(|_| "REQUEST_INVALID")?;
        if discovery_revision.to_string() != value.discovery_revision {
            return Err("REQUEST_INVALID");
        }
        let discovery = Self {
            discovery_ref: value.discovery_ref,
            discovery_revision,
        };
        discovery
            .valid()
            .then_some(discovery)
            .ok_or("REQUEST_INVALID")
    }
}

/// Read-only local discovery facts safe to expose to the WebView. Protected-input slots, source
/// locators, filesystem paths, endpoint details, and catalog digests deliberately remain in Rust.
#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeScanItemV1 {
    agent_id: String,
    supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_provider_id: Option<String>,
    configuration_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    connection_option_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_model_id: Option<String>,
    inventory_eligible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    discovery: Option<WebComputeDiscoveryRefV1>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    actions_required: Vec<String>,
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeScanResultV1 {
    items: Vec<WebComputeScanItemV1>,
}

impl From<ComputeScanResultV1> for WebComputeScanResultV1 {
    fn from(value: ComputeScanResultV1) -> Self {
        Self {
            items: value
                .items
                .into_iter()
                .map(|item| WebComputeScanItemV1 {
                    agent_id: item.agent_id,
                    supported: item.supported,
                    native_provider_id: item.native_provider_id,
                    configuration_state: item.configuration_state,
                    connection_option_id: item.connection_option_id,
                    observed_model_id: item.observed_model_id,
                    inventory_eligible: item.inventory_eligible,
                    discovery: item.discovery.map(Into::into),
                    actions_required: item.actions_required,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebValidationRefV2 {
    approval_operation: OperationReferenceV1,
    validation_ref: String,
    validation_revision: String,
}

impl From<ComputeValidationRefV2> for WebValidationRefV2 {
    fn from(value: ComputeValidationRefV2) -> Self {
        Self {
            approval_operation: value.approval_operation,
            validation_ref: value.validation_ref,
            validation_revision: value.validation_revision.to_string(),
        }
    }
}

impl TryFrom<WebValidationRefV2> for ComputeValidationRefV2 {
    type Error = &'static str;

    fn try_from(value: WebValidationRefV2) -> Result<Self, Self::Error> {
        let validation_revision = value
            .validation_revision
            .parse::<u64>()
            .map_err(|_| "REQUEST_INVALID")?;
        if validation_revision.to_string() != value.validation_revision {
            return Err("REQUEST_INVALID");
        }
        Ok(Self {
            approval_operation: value.approval_operation,
            validation_ref: value.validation_ref,
            validation_revision,
        })
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeCandidateViewV2 {
    candidate: ComputeCandidateRefV2,
    correlation: ComputeCheckCorrelationV2,
    producer: ComputeCandidateProducerV2,
    provenance: ComputeCandidateProvenanceKindV2,
    display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    existing_source_id: Option<String>,
    models: Vec<ComputeCandidateModelViewV2>,
    input_state: ComputeCandidateInputStateV2,
    fact_state: ComputeCandidateFactStateV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<WebValidationRefV2>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    issues: Vec<ComputeCandidateIssueV2>,
}

impl From<ComputeCandidateViewV2> for WebComputeCandidateViewV2 {
    fn from(value: ComputeCandidateViewV2) -> Self {
        Self {
            candidate: value.candidate,
            correlation: value.correlation,
            producer: value.producer,
            provenance: value.provenance,
            display_name: value.display_name,
            existing_source_id: value.existing_source_id,
            models: value.models,
            input_state: value.input_state,
            fact_state: value.fact_state,
            validation: value.validation.map(Into::into),
            issues: value.issues,
        }
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeSubscriptionCandidatesV2 {
    candidates: Vec<WebComputeCandidateViewV2>,
    discovery_state: ComputeSubscriptionDiscoveryStateV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
}

impl From<ComputeSubscriptionCandidatesV2> for WebComputeSubscriptionCandidatesV2 {
    fn from(value: ComputeSubscriptionCandidatesV2) -> Self {
        Self {
            candidates: value.candidates.into_iter().map(Into::into).collect(),
            discovery_state: value.discovery_state,
            reason_code: value.reason_code,
        }
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeSubscriptionCheckResultV2 {
    candidate: ComputeCandidateRefV2,
    approval_operation: OperationReferenceV1,
    status: ComputeSubscriptionCheckStatusV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    save_operation: Option<OperationReferenceV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<WebValidationRefV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_candidate: Option<WebComputeCandidateViewV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl From<ComputeSubscriptionCheckResultV2> for WebComputeSubscriptionCheckResultV2 {
    fn from(value: ComputeSubscriptionCheckResultV2) -> Self {
        Self {
            candidate: value.candidate,
            approval_operation: value.approval_operation,
            status: value.status,
            save_operation: value.save_operation,
            validation: value.validation.map(Into::into),
            checked_candidate: value.checked_candidate.map(Into::into),
            reason: value.reason,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeManagementChangeV2 {
    schema: String,
    subject: ComputeManagementSubjectV2,
    expected_revisions: RevisionSetV1,
    selected_model_refs: Vec<String>,
    intent: ComputeManagementIntentV2,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    key_edits: Vec<ComputeKeyEditV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation: Option<WebValidationRefV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edit: Option<hiroute_application_api::ComputeManagementEditV1>,
}

impl From<ComputeManagementChangeV2> for WebComputeManagementChangeV2 {
    fn from(value: ComputeManagementChangeV2) -> Self {
        Self {
            schema: value.schema,
            subject: value.subject,
            expected_revisions: value.expected_revisions,
            selected_model_refs: value.selected_model_refs,
            intent: value.intent,
            key_edits: value.key_edits,
            validation: value.validation.map(Into::into),
            edit: value.edit,
        }
    }
}

impl TryFrom<WebComputeManagementChangeV2> for ComputeManagementChangeV2 {
    type Error = &'static str;

    fn try_from(value: WebComputeManagementChangeV2) -> Result<Self, Self::Error> {
        Ok(Self {
            schema: value.schema,
            subject: value.subject,
            expected_revisions: value.expected_revisions,
            selected_model_refs: value.selected_model_refs,
            intent: value.intent,
            key_edits: value.key_edits,
            validation: value.validation.map(TryInto::try_into).transpose()?,
            edit: value.edit,
        })
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeSavePreviewV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<ComputeCandidateRefV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<WebValidationRefV2>,
    spec: ChangeSpecV1,
    accept_digest: CanonicalDigest,
    expected_revisions: RevisionSetV1,
    changes: Vec<ComputeSaveChangeViewV2>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    affected_plan_refs: Vec<String>,
}

impl TryFrom<ComputeSavePreviewV2> for WebComputeSavePreviewV2 {
    type Error = &'static str;

    fn try_from(value: ComputeSavePreviewV2) -> Result<Self, Self::Error> {
        let mut spec = value.spec;
        let change = serde_json::from_value::<ComputeManagementChangeV2>(spec.desired_state)
            .map_err(|_| "RESPONSE_DATA_INVALID")?;
        spec.desired_state = serde_json::to_value(WebComputeManagementChangeV2::from(change))
            .map_err(|_| "RESPONSE_DATA_INVALID")?;
        Ok(Self {
            candidate: value.candidate,
            validation: value.validation.map(Into::into),
            spec,
            accept_digest: value.accept_digest,
            expected_revisions: value.expected_revisions,
            changes: value.changes,
            affected_plan_refs: value.affected_plan_refs,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeConnectionApplyRequestV1 {
    spec: ChangeSpecV1,
    accept_digest: CanonicalDigest,
    expected_revisions: RevisionSetV1,
    idempotency_key: String,
}

impl TryFrom<WebComputeConnectionApplyRequestV1> for ComputeConnectionApplyRequestV1 {
    type Error = &'static str;

    fn try_from(value: WebComputeConnectionApplyRequestV1) -> Result<Self, Self::Error> {
        let mut spec = value.spec;
        let change = serde_json::from_value::<WebComputeManagementChangeV2>(spec.desired_state)
            .map_err(|_| "REQUEST_INVALID")?;
        spec.desired_state = serde_json::to_value(ComputeManagementChangeV2::try_from(change)?)
            .map_err(|_| "REQUEST_INVALID")?;
        Ok(Self {
            spec,
            accept_digest: value.accept_digest,
            expected_revisions: value.expected_revisions,
            idempotency_key: value.idempotency_key,
        })
    }
}

#[derive(Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WebComputeSaveResultV2 {
    #[serde(skip_serializing_if = "Option::is_none")]
    candidate: Option<ComputeCandidateRefV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    validation: Option<WebValidationRefV2>,
    disposition: ComputeSaveDispositionV2,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bindings: Vec<ComputeSavedBindingV2>,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    management_state: Option<MaterializationState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation: Option<OperationReferenceV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

impl From<ComputeSaveResultV2> for WebComputeSaveResultV2 {
    fn from(value: ComputeSaveResultV2) -> Self {
        Self {
            candidate: value.candidate,
            validation: value.validation.map(Into::into),
            disposition: value.disposition,
            source_id: value.source_id,
            bindings: value.bindings,
            saved_revision: value.saved_revision,
            management_state: value.management_state,
            operation: value.operation,
            reason: value.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const LARGE_REVISION: u64 = 8_091_638_379_121_373_450;

    fn validation() -> ComputeValidationRefV2 {
        ComputeValidationRefV2 {
            approval_operation: OperationReferenceV1 {
                operation_id: "op_validation".into(),
                state: "succeeded".into(),
                sequence: 15,
                cancellable: false,
            },
            validation_ref: "validation/cpa/example".into(),
            validation_revision: LARGE_REVISION,
        }
    }

    #[test]
    fn compute_scan_web_view_omits_protected_discovery_material() {
        let mut scan = ComputeScanResultV1 {
            schema: COMPUTE_SCAN_RESULT_SCHEMA_V1.into(),
            catalog: ComputeCatalogProvenanceViewV1 {
                product_release: "fixture-release".into(),
                catalog_binding_id: "fixture-catalog".into(),
                release_sequence: 7,
                connector_registry_digest: CanonicalDigest::of_bytes(b"connector-registry"),
                model_data_digest: CanonicalDigest::of_bytes(b"model-data"),
                cross_reference_digest: CanonicalDigest::of_bytes(b"cross-reference"),
            },
            items: vec![ComputeScanItemV1 {
                native_provider_id: None,
                agent_id: "agent_claude_default".into(),
                supported: true,
                configuration_state: "registered_with_protected_input".into(),
                discovered_source_ref: Some("private/source/ref".into()),
                connection_option_id: Some("zhipu.coding-plan.cn.v1".into()),
                endpoint_profile_id: Some("private-endpoint-profile".into()),
                registered_base_url: Some("https://private.example.invalid/api".into()),
                observed_model_id: Some("glm-fixture".into()),
                model_configuration_id: Some("model.zhipu.fixture".into()),
                inventory_eligible: true,
                discovery: Some(ComputeDiscoveryRefV1 {
                    discovery_ref: format!("discovery/{}", "a".repeat(64)),
                    discovery_revision: LARGE_REVISION,
                }),
                actions_required: Vec::new(),
                permission_action: Some(ComputePermissionActionV1 {
                    discovered_source_ref: "private/source/ref".into(),
                    observed_identity: CanonicalDigest::of_bytes(b"private-source"),
                    observed_revision: 9,
                    display_path: "/private/config/path".into(),
                    required_mode: 0o600,
                }),
                credential_import: Some(ComputeDiscoveredCredentialImportV1 {
                    input_slot: "protected-input-slot".into(),
                    scanner_id: "scanner-fixture".into(),
                    scanner_version: "1".into(),
                    source_ref: "private/source/ref".into(),
                    field_selector: "api-key".into(),
                    observed_revision: 9,
                }),
            }],
        };

        scan.items.push(ComputeScanItemV1 {
            agent_id: "agent_pi_default".into(),
            native_provider_id: Some("native-static-provider".into()),
            connection_option_id: None,
            ..scan.items[0].clone()
        });

        let web = serde_json::to_value(WebComputeScanResultV1::from(scan)).unwrap();
        assert_eq!(web["items"][0]["agent_id"], "agent_claude_default");
        assert_eq!(web["items"][0]["supported"], true);
        assert!(web["items"][0].get("native_provider_id").is_none());
        assert_eq!(web["items"][1]["agent_id"], "agent_pi_default");
        assert_eq!(
            web["items"][1]["native_provider_id"],
            "native-static-provider"
        );
        assert!(web["items"][1].get("connection_option_id").is_none());
        assert_eq!(
            web["items"][0]["configuration_state"],
            "registered_with_protected_input"
        );
        assert_eq!(web["items"][0]["observed_model_id"], "glm-fixture");
        assert_eq!(
            web["items"][0]["discovery"]["discovery_revision"],
            LARGE_REVISION.to_string()
        );
        let encoded = serde_json::to_string(&web).unwrap();
        for private_value in [
            "private/source/ref",
            "private-endpoint-profile",
            "private.example.invalid",
            "/private/config/path",
            "protected-input-slot",
            "scanner-fixture",
            "fixture-catalog-binding",
        ] {
            assert!(!encoded.contains(private_value));
        }
    }

    #[test]
    fn discovery_revision_round_trips_without_javascript_rounding() {
        let discovery = ComputeDiscoveryRefV1 {
            discovery_ref: format!("discovery/{}", "b".repeat(64)),
            discovery_revision: LARGE_REVISION,
        };
        let encoded =
            serde_json::to_value(WebComputeDiscoveryRefV1::from(discovery.clone())).unwrap();
        assert_eq!(encoded["discovery_revision"], LARGE_REVISION.to_string());
        let web: WebComputeDiscoveryRefV1 = serde_json::from_value(encoded).unwrap();
        assert_eq!(ComputeDiscoveryRefV1::try_from(web).unwrap(), discovery);
    }

    #[test]
    fn invalid_or_non_canonical_web_discovery_is_rejected() {
        for encoded in [
            serde_json::json!({
                "discovery_ref": format!("discovery/{}", "c".repeat(64)),
                "discovery_revision": format!("0{LARGE_REVISION}")
            }),
            serde_json::json!({
                "discovery_ref": "discovery/not-a-digest",
                "discovery_revision": "7"
            }),
        ] {
            let web: WebComputeDiscoveryRefV1 = serde_json::from_value(encoded).unwrap();
            assert_eq!(ComputeDiscoveryRefV1::try_from(web), Err("REQUEST_INVALID"));
        }
    }

    #[test]
    fn validation_revision_round_trips_beyond_javascript_integer_range() {
        let web = WebValidationRefV2::from(validation());
        let encoded = serde_json::to_value(&web).unwrap();
        assert_eq!(encoded["validation_revision"], LARGE_REVISION.to_string());
        let decoded: WebValidationRefV2 = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            ComputeValidationRefV2::try_from(decoded).unwrap(),
            validation()
        );
    }

    #[test]
    fn non_canonical_web_revision_is_rejected() {
        let mut encoded = serde_json::to_value(WebValidationRefV2::from(validation())).unwrap();
        encoded["validation_revision"] = serde_json::Value::String(format!("0{LARGE_REVISION}"));
        let decoded: WebValidationRefV2 = serde_json::from_value(encoded).unwrap();
        assert_eq!(
            ComputeValidationRefV2::try_from(decoded),
            Err("REQUEST_INVALID")
        );
    }

    #[test]
    fn preview_and_apply_restore_the_exact_native_change() {
        let revisions = RevisionSetV1 {
            target: 3,
            dependencies: BTreeMap::new(),
        };
        let change = ComputeManagementChangeV2 {
            edit: None,
            schema: COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2.into(),
            subject: ComputeManagementSubjectV2::Candidate {
                candidate: ComputeCandidateRefV2 {
                    candidate_ref: "candidate/cpa/codex/example".into(),
                    candidate_revision: 6,
                },
            },
            expected_revisions: revisions.clone(),
            selected_model_refs: vec!["cpa-model/gpt-example".into()],
            intent: ComputeManagementIntentV2::SaveReady,
            key_edits: Vec::new(),
            validation: Some(validation()),
        };
        let preview = ComputeSavePreviewV2 {
            candidate: None,
            validation: Some(validation()),
            spec: ChangeSpecV1 {
                schema_version: SchemaVersion::new(1, 0),
                command_id: "compute.connection.apply".into(),
                resource_id: Some("cpa/example".into()),
                desired_state: serde_json::to_value(&change).unwrap(),
            },
            accept_digest: CanonicalDigest::of_bytes(b"preview"),
            expected_revisions: revisions.clone(),
            changes: Vec::new(),
            affected_plan_refs: Vec::new(),
        };

        let web =
            serde_json::to_value(WebComputeSavePreviewV2::try_from(preview).unwrap()).unwrap();
        assert_eq!(
            web["spec"]["desired_state"]["validation"]["validation_revision"],
            LARGE_REVISION.to_string()
        );
        let request: WebComputeConnectionApplyRequestV1 =
            serde_json::from_value(serde_json::json!({
                "spec": web["spec"].clone(),
                "accept_digest": web["accept_digest"].clone(),
                "expected_revisions": revisions,
                "idempotency_key": "subscription-save:example"
            }))
            .unwrap();
        let native = ComputeConnectionApplyRequestV1::try_from(request).unwrap();
        let restored: ComputeManagementChangeV2 =
            serde_json::from_value(native.spec.desired_state).unwrap();
        assert_eq!(restored, change);
    }
}
