//! Staged, pre-promotion CLI codecs for the Gateway-unbound Compute/Routing slice.
//!
//! This module owns no discovery, persistence, or routing behavior. It accepts only the exact
//! typed Local Control request DTOs and leaves the public generated command manifest unchanged.

#[cfg(test)]
use std::io::Read;

use hiroute_application_api::{
    AgentConnectionApplyRequestV1, AgentConnectionPreviewRequestV1, ApplyPriceOverrideChangeV2,
    ApplyRequestV1, COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2, ComputeConnectionApplyRequestV1,
    ComputeConnectionAuthorizationRequestV1, ComputeConnectionOptionsRequestV1,
    ComputeConnectionPreviewRequestV1, ComputeConnectionTestRequestV1, ComputeManagementQueryV2,
    ComputeSavePreviewRequestV2, ComputeScanRequestV1, ComputeSubscriptionCheckPreviewRequestV2,
    ErrorCode, GetEffectivePricesV2, PreviewPriceOverrideChangeV2, PreviewRequestV1,
};
#[cfg(test)]
use hiroute_application_api::{CommandDescriptorV1, command_by_id};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

#[cfg(test)]
const TYPED_CONTROL_IDS: [&str; 16] = [
    "models.show",
    "prices.effective",
    "prices.override.preview",
    "prices.override.apply",
    "compute.scan",
    "compute.connection.options",
    "compute.connection.preview",
    "compute.connection.apply",
    "compute.credential.add",
    "routing.preview",
    "routing.apply",
    "agents.connect.preview",
    "agents.connect.apply",
    "agents.connect.status",
    "agents.restore.preview",
    "agents.restore.apply",
];
use crate::request_input::{self, MAX_REQUEST_BYTES};

#[cfg(test)]
pub(crate) fn typed_control_commands() -> impl Iterator<Item = CommandDescriptorV1> {
    TYPED_CONTROL_IDS
        .into_iter()
        .map(|id| command_by_id(id).expect("staged control-plane descriptor exists"))
}

pub(crate) fn accepts_capability(command_id: &str) -> bool {
    matches!(
        command_id,
        "sessions.list"
            | "sessions.show"
            | "sessions.receipt"
            | "sessions.status"
            | "value.show"
            | "observation.plan-quality.samples"
            | "observation.plan-trust"
            | "observation.plan-trust.replay"
            | "observation.plan-trust.suggestions"
    )
}

pub(crate) fn command_payload(
    command_id: &str,
    options: &[String],
) -> Option<Result<Value, ErrorCode>> {
    match command_id {
        "decision.services.list" | "decision.sources.list" | "decision.connections.list" => Some(
            empty::<hiroute_application_api::ClientEmptyRequestV1>(options),
        ),
        "decision.services.apply" | "decision.connections.apply" => Some(preview_or_apply(options)),
        "decision.services.test" => Some(classifier_request(options)),
        "decision.connections.test" | "decision.network.test" => Some(request::<
            hiroute_application_api::DecisionServiceTestRequestV2,
        >(options)),
        "models.feed.status" | "models.feed.refresh" | "models.availability" => {
            Some(empty::<hiroute_application_api::ClientEmptyRequestV1>(
                options,
            ))
        }
        "models.feed.remove.preview" => Some(if options.is_empty() {
            empty::<hiroute_application_api::ModelFeedRemovalQueryV2>(options)
        } else {
            request::<hiroute_application_api::ModelFeedRemovalQueryV2>(options)
        }),
        "models.feed.settings.apply" => Some(feed_settings(options)),
        "models.feed.remove.apply" => {
            Some(request::<hiroute_application_api::ModelFeedRemoveRequestV1>(options))
        }
        "agents.previous-routing.preview" => Some(request::<
            hiroute_application_api::AgentPreviousRoutingQueryV1,
        >(options)),
        "compute.network.show" => {
            Some(request::<hiroute_application_api::NativeSourceNetworkQueryV1>(options))
        }
        "compute.network.preview" => Some(request::<
            hiroute_application_api::NativeSourceNetworkPreviewV1,
        >(options)),
        "compute.network.apply" => Some(request::<
            hiroute_application_api::ComputeConnectionApplyRequestV1,
        >(options)),
        "compute.network.resolve" => Some(request::<
            hiroute_application_api::NativeNetworkRoutePreviewRequestV1,
        >(options)),
        "compute.connection.preview-network" => {
            Some(request::<hiroute_application_api::ComputeManagementChangeV4>(options))
        }
        "compute.defaults.get" => Some(request::<hiroute_application_api::ComputeDefaultsQueryV1>(
            options,
        )),
        "compute.defaults.preview" => Some(request::<
            hiroute_application_api::ComputeSavePreviewRequestV3,
        >(options)),
        "models.metadata.list" => Some(if options.is_empty() {
            empty::<hiroute_application_api::ListModelMetadataV1>(options)
        } else {
            request::<hiroute_application_api::ListModelMetadataV1>(options)
        }),
        "models.metadata.get" => Some(request::<hiroute_application_api::GetModelMetadataV1>(
            options,
        )),
        "models.show" => Some(request::<hiroute_application_api::ModelCatalogQueryV1>(
            options,
        )),
        "prices.effective" => Some(request::<GetEffectivePricesV2>(options)),
        "prices.override.preview" => Some(request::<PreviewPriceOverrideChangeV2>(options)),
        "prices.override.apply" => Some(request::<ApplyPriceOverrideChangeV2>(options)),
        "compute.scan" => Some(if options.is_empty() {
            empty::<ComputeScanRequestV1>(options)
        } else {
            compute_scan(options)
        }),
        "compute.list" => Some(empty::<ComputeManagementQueryV2>(options)),
        "compute.show" => Some(positional(options, |source_id| ComputeManagementQueryV2 {
            source_id: Some(source_id),
        })),
        "compute.connection.prepare" => Some(request::<
            hiroute_application_api::PrepareDiscoveredModelConnectionRequestV1,
        >(options)),
        "compute.connection.options" => Some(if options.is_empty() {
            empty::<ComputeConnectionOptionsRequestV1>(options)
        } else {
            request::<ComputeConnectionOptionsRequestV1>(options)
        }),
        "compute.connection.preview" => Some(compute_preview(options)),
        "compute.connection.apply" => Some(request::<ComputeConnectionApplyRequestV1>(options)),
        "compute.connection.authorize" => {
            Some(request::<ComputeConnectionAuthorizationRequestV1>(options))
        }
        "compute.connection.login" => Some(request::<
            hiroute_application_api::ComputeSubscriptionLoginRequestV1,
        >(options)),
        "compute.connection.test-network" => Some(request::<
            hiroute_application_api::ComputeConnectionTestRequestV2,
        >(options)),
        "compute.connection.test" => Some(request::<ComputeConnectionTestRequestV1>(options)),
        "compute.credential.add" => Some(preview_or_apply(options)),
        "routing.list" if !options.is_empty() => Some(request::<
            hiroute_application_api::AgentPlanCatalogQueryV2,
        >(options)),
        "routing.preview" => Some(routing_request(options, false)),
        "routing.apply" => Some(routing_request(options, true)),
        "agents.connect.preview" => Some(agent_request(options, false)),
        "routing.options" => {
            Some(request::<hiroute_application_api::PlanEditorOptionsRequestV1>(options))
        }
        "agents.connect.apply" => Some(agent_request(options, true)),
        "agents.restore.preview" => Some(agent_restore_request(options, false)),
        "agents.restore.apply" => Some(agent_restore_request(options, true)),
        "agents.connect.status" => Some(crate::effective_status::payload(options)),
        _ => None,
    }
}

fn compute_preview(options: &[String]) -> Result<Value, ErrorCode> {
    let value = request_input::read(options, MAX_REQUEST_BYTES)?;
    if value
        .get("change")
        .and_then(|change| change.get("schema"))
        .and_then(Value::as_str)
        == Some(COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2)
    {
        return typed_value::<ComputeSavePreviewRequestV2>(value);
    }
    if value.get("candidate").is_some() {
        return typed_value::<ComputeSubscriptionCheckPreviewRequestV2>(value);
    }
    typed_value::<ComputeConnectionPreviewRequestV1>(value)
}

fn typed_value<T: DeserializeOwned + Serialize>(value: Value) -> Result<Value, ErrorCode> {
    let request: T = request_input::deserialize(value)?;
    serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
}

fn classifier_request(options: &[String]) -> Result<Value, ErrorCode> {
    classifier_value(request_input::read(options, MAX_REQUEST_BYTES)?)
}

fn classifier_value(value: Value) -> Result<Value, ErrorCode> {
    match value.get("schema").and_then(Value::as_str) {
        Some(hiroute_application_api::HISTORICAL_APPLICABILITY_TEST_SCHEMA_V1) => {
            typed_value::<hiroute_application_api::HistoricalApplicabilityTestRequestV1>(value)
        }
        _ => typed_value::<hiroute_application_api::ClassifierDecisionTestRequestV1>(value),
    }
}

fn compute_scan(options: &[String]) -> Result<Value, ErrorCode> {
    let value = request_input::read(options, MAX_REQUEST_BYTES)?;
    if value.get("schema").is_some() {
        let parsed: hiroute_application_api::ComputeScanRequestV2 =
            request_input::deserialize(value)?;
        if !parsed.valid() {
            return Err(ErrorCode::InvalidArguments);
        }
        serde_json::to_value(parsed).map_err(|_| ErrorCode::Internal)
    } else {
        typed_value::<ComputeScanRequestV1>(value)
    }
}

fn routing_request(options: &[String], apply: bool) -> Result<Value, ErrorCode> {
    let value = request_input::read(options, MAX_REQUEST_BYTES)?;
    if value.get("change").is_none() {
        request_input::diagnostics::record(
            "missing_field",
            "$.change",
            "a change object inside the Preview/Apply request wrapper",
        );
        return Err(ErrorCode::InvalidArguments);
    }
    if value
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1)
    {
        return if apply {
            typed_value::<hiroute_application_api::PublicationCheckpointApplyRequestV1>(value)
        } else {
            typed_value::<hiroute_application_api::PublicationCheckpointPreviewRequestV1>(value)
        };
    }
    if value
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_LIFECYCLE_CHANGE_SCHEMA_V1)
    {
        return if apply {
            typed_value::<hiroute_application_api::PlanLifecycleApplyRequestV1>(value)
        } else {
            typed_value::<hiroute_application_api::PlanLifecyclePreviewRequestV1>(value)
        };
    }
    if value
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_DRAFT_CHANGE_SCHEMA_V1)
    {
        return if apply {
            typed_value::<hiroute_application_api::PlanDraftApplyRequestV1>(value)
        } else {
            typed_value::<hiroute_application_api::PlanDraftPreviewRequestV1>(value)
        };
    }
    if value
        .get("change")
        .and_then(|change| change.get("schema"))
        .and_then(Value::as_str)
        != Some(hiroute_application_api::PLAN_CONTENT_CHANGE_SCHEMA_V2)
    {
        return Err(ErrorCode::InvalidArguments);
    }
    if apply {
        typed_value::<hiroute_application_api::PlanContentApplyRequestV2>(value)
    } else {
        typed_value::<hiroute_application_api::PlanContentPreviewRequestV2>(value)
    }
}

fn agent_request(options: &[String], apply: bool) -> Result<Value, ErrorCode> {
    validate_agent_request(request_input::read(options, MAX_REQUEST_BYTES)?, apply)
}

fn agent_restore_request(options: &[String], apply: bool) -> Result<Value, ErrorCode> {
    validate_restore_request(request_input::read(options, MAX_REQUEST_BYTES)?, apply)
}

fn validate_restore_request(value: Value, apply: bool) -> Result<Value, ErrorCode> {
    let value = validate_agent_request(value, apply)?;
    let spec: hiroute_application_api::AgentSettingsSpecV2 =
        request_input::deserialize(value["spec"].clone())?;
    if spec.schema_version != hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2
        || !spec.is_restore_only()
    {
        return Err(ErrorCode::InvalidArguments);
    }
    Ok(value)
}

fn validate_agent_request(value: Value, apply: bool) -> Result<Value, ErrorCode> {
    let v2 = value["spec"]["schema_version"]["major"] == 2;
    let valid =
        match (v2, apply) {
            (true, true) => request_input::deserialize::<
                hiroute_application_api::AgentSettingsApplyV2,
            >(value.clone())
            .is_ok(),
            (true, false) => request_input::deserialize::<
                hiroute_application_api::AgentSettingsPreviewRequestV2,
            >(value.clone())
            .is_ok(),
            (false, true) => {
                request_input::deserialize::<AgentConnectionApplyRequestV1>(value.clone()).is_ok()
            }
            (false, false) => {
                request_input::deserialize::<AgentConnectionPreviewRequestV1>(value.clone()).is_ok()
            }
        };
    if valid {
        Ok(value)
    } else {
        Err(ErrorCode::InvalidArguments)
    }
}

fn preview_or_apply(options: &[String]) -> Result<Value, ErrorCode> {
    let value = request_input::read(options, MAX_REQUEST_BYTES)?;
    let keys = value
        .as_object()
        .ok_or(ErrorCode::InvalidArguments)?
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let preview_keys = ["schema_version", "spec"].into_iter().collect();
    let apply_keys = [
        "schema_version",
        "spec",
        "accept_digest",
        "expected_revisions",
        "idempotency_key",
    ]
    .into_iter()
    .collect();
    if keys == preview_keys {
        request_input::deserialize::<PreviewRequestV1>(value.clone())?;
    } else if keys == apply_keys {
        let apply = request_input::deserialize::<ApplyRequestV1>(value.clone())?;
        if apply.apply_capability.is_some() || apply.idempotency_key.is_empty() {
            return Err(ErrorCode::InvalidArguments);
        }
    } else {
        return Err(ErrorCode::InvalidArguments);
    }
    Ok(value)
}

fn positional<T>(options: &[String], build: impl FnOnce(String) -> T) -> Result<Value, ErrorCode>
where
    T: Serialize,
{
    let [value] = options else {
        return Err(ErrorCode::InvalidArguments);
    };
    if !request_input::valid_positional(value) {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(build(value.clone())).map_err(|_| ErrorCode::Internal)
}

fn feed_settings(options: &[String]) -> Result<Value, ErrorCode> {
    if options.len() == 2 && options[0] == "--retrieval" {
        let enabled = match options[1].as_str() {
            "on" => true,
            "off" => false,
            _ => return Err(ErrorCode::InvalidArguments),
        };
        return serde_json::to_value(hiroute_application_api::ModelFeedSettingsRequestV1 {
            retrieval_enabled: Some(enabled),
            acknowledge_disclosure: None,
        })
        .map_err(|_| ErrorCode::Internal);
    }
    request::<hiroute_application_api::ModelFeedSettingsRequestV1>(options)
}

fn empty<T>(options: &[String]) -> Result<Value, ErrorCode>
where
    T: Default + Serialize,
{
    if !options.is_empty() {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(T::default()).map_err(|_| ErrorCode::Internal)
}

fn request<T>(options: &[String]) -> Result<Value, ErrorCode>
where
    T: DeserializeOwned + Serialize,
{
    request_input::typed::<T>(options, MAX_REQUEST_BYTES)
}

#[cfg(test)]
fn typed_request<T>(reader: impl Read) -> Result<Value, ErrorCode>
where
    T: DeserializeOwned + Serialize,
{
    typed_value::<T>(request_input::read_json(reader, MAX_REQUEST_BYTES)?)
}

#[cfg(test)]
mod tests {
    use hiroute_application_api::{
        COMPUTE_CONNECTION_CHANGE_SCHEMA_V1, ComputeConnectionChangeV1,
        ComputeConnectionPreviewRequestV1,
    };

    use super::*;

    #[test]
    fn control_plane_owns_the_sixteen_exact_typed_descriptors() {
        let commands = typed_control_commands().collect::<Vec<_>>();
        assert_eq!(commands.len(), 16);
        assert_eq!(
            commands
                .iter()
                .map(|command| command.command_id.as_str())
                .collect::<Vec<_>>(),
            TYPED_CONTROL_IDS
        );
    }

    #[test]
    fn control_plane_typed_codec_rejects_unknown_fields() {
        let request = ComputeConnectionPreviewRequestV1 {
            change: ComputeConnectionChangeV1 {
                schema: COMPUTE_CONNECTION_CHANGE_SCHEMA_V1.into(),
                discovered_source_ref: "discovered.fixture".into(),
                connection_option_id: "option.fixture".into(),
                model_configuration_id: "model.fixture".into(),
                expected_source_revision: 0,
                expected_binding_revision: 0,
                expected_inventory_revision: 0,
                explicit_materialization: true,
            },
        };
        let bytes = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            typed_request::<ComputeConnectionPreviewRequestV1>(bytes.as_slice()).unwrap(),
            serde_json::to_value(request).unwrap()
        );
        assert_eq!(
            typed_request::<ComputeConnectionPreviewRequestV1>(
                br#"{"change":{"schema":"hiroute.compute-connection-change/v1","discovered_source_ref":"x","connection_option_id":"y","model_configuration_id":"z","expected_source_revision":0,"expected_binding_revision":0,"expected_inventory_revision":0,"explicit_materialization":true},"secret":"forbidden"}"#.as_slice(),
            )
            .unwrap_err(),
            ErrorCode::InvalidArguments
        );
    }

    #[test]
    fn local_configuration_apply_needs_no_capability_channel() {
        assert!(!accepts_capability("compute.connection.login"));
        assert!(!accepts_capability("compute.connection.apply"));
        assert!(!accepts_capability("compute.credential.add"));
        assert!(!accepts_capability("routing.apply"));
        assert!(!accepts_capability("agents.connect.apply"));
        assert!(!accepts_capability("agents.restore.apply"));
        assert!(!accepts_capability("agents.restore.preview"));
        assert!(!accepts_capability("compute.connection.preview"));
        assert!(!accepts_capability("routing.preview"));
        assert!(!accepts_capability("prices.override.apply"));
        assert!(!accepts_capability("prices.override.preview"));
        assert!(!accepts_capability("prices.effective"));
        assert!(!accepts_capability("models.show"));
        assert!(accepts_capability("sessions.list"));
    }

    #[test]
    fn subscription_login_codec_rejects_callback_secrets_but_preserves_protected_references() {
        use hiroute_application_api::ComputeSubscriptionLoginRequestV1;
        let value = serde_json::json!({"action":"callback","login_ref":"login/one",
            "input_candidate":{"candidate_ref":"candidate/subscription-login/one","candidate_revision":1}});
        let encoded = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            typed_request::<ComputeSubscriptionLoginRequestV1>(encoded.as_slice()).unwrap(),
            value
        );
        let mut with_secret = value;
        with_secret["callback_url"] = serde_json::json!("callback-secret-must-not-be-forwarded");
        let encoded = serde_json::to_vec(&with_secret).unwrap();
        assert_eq!(
            typed_request::<ComputeSubscriptionLoginRequestV1>(encoded.as_slice()).unwrap_err(),
            ErrorCode::InvalidArguments
        );
        assert_eq!(
            command_payload(
                "compute.connection.login",
                &["--code".into(), "forbidden".into()]
            )
            .unwrap()
            .unwrap_err(),
            ErrorCode::InvalidArguments
        );
    }
}

#[cfg(test)]
mod agent_settings_codec_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn v2_settings_codec_preserves_schema_and_rejects_untyped_authority() {
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":"agent-context/codex/test",
            "model":{"intent":"restore","restore_point_ref":"model-restore/original"}});
        let preview = json!({"spec":spec});
        assert_eq!(
            validate_agent_request(preview.clone(), false).unwrap(),
            preview
        );
        assert_eq!(
            validate_restore_request(preview.clone(), false).unwrap(),
            preview
        );
        let mut no_restore = preview.clone();
        no_restore["spec"]["model"] = json!({"intent":"keep"});
        assert!(validate_restore_request(no_restore, false).is_err());
        let digest = hiroute_application_api::CanonicalDigest::of_bytes(b"codec");
        let apply = json!({"spec":spec,"accept_digest":digest,"dependency_digest":digest,
            "expected_revisions":{"target":0,"dependencies":{}},"idempotency_key":"settings-codec"});
        assert_eq!(validate_agent_request(apply.clone(), true).unwrap(), apply);
        let mut injected = apply.clone();
        injected["apply_capability"] = json!("forbidden-in-ordinary-stdin");
        assert!(validate_agent_request(injected, true).is_err());
        let mut missing = apply;
        missing
            .as_object_mut()
            .unwrap()
            .remove("expected_revisions");
        assert!(validate_agent_request(missing, true).is_err());
        let mut unknown = preview;
        unknown["spec"]["schema_version"]["major"] = json!(3);
        assert!(validate_agent_request(unknown, false).is_err());
    }
}

#[cfg(test)]
mod classifier_codec_tests {
    use super::*;
    use hiroute_domain::*;
    use serde_json::json;

    #[test]
    fn explicit_c_request_reaches_control_while_v1_stays_unchanged() {
        let legacy = json!({"schema":"hiroute.classifier-decision-test/v1","classifier":{"kind":"local_rules"}});
        assert_eq!(classifier_value(legacy.clone()).unwrap(), legacy);
        let protocol = NativeProtocolV1::default();
        let mut judgment = JudgmentSettingsV1::default();
        protocol.stock_assessment(&mut judgment);
        let frozen = FrozenNativeProtocolV1::freeze(
            &protocol,
            judgment.degree.definition().into(),
            [(SMART_SAVING_SCOPE_ID.into(), judgment)],
        )
        .unwrap();
        let request = json!({"schema":"hiroute.historical-applicability-test/v1","service_id":"saved-fixture","service_revision":7,"shape":"standard","protocol":frozen});
        assert_eq!(classifier_value(request.clone()).unwrap(), request);
        let mut injected = request.clone();
        injected["state"] = json!("verified");
        assert_eq!(classifier_value(injected), Err(ErrorCode::InvalidArguments));
        let mut unknown = request;
        unknown["schema"] = json!("unknown");
        assert_eq!(classifier_value(unknown), Err(ErrorCode::InvalidArguments));
    }

    #[test]
    fn maximum_c_shape_preserves_all_nested_degree_definitions() {
        let protocol = NativeProtocolV1::default();
        let mut judgment = JudgmentSettingsV1::default();
        protocol.stock_assessment(&mut judgment);
        let options = (0..16)
            .map(|i| CategoryOptionV1 {
                id: format!("category-{i}"),
                criterion: format!("Synthetic category {i}"),
                refinement: Some(judgment.degree.definition()),
            })
            .collect::<Vec<_>>();
        let frozen = FrozenNativeProtocolV1::freeze(
            &protocol,
            DecisionDefinitionV1::Categorical {
                instructions: "Select the current task category".into(),
                options: options.clone(),
            },
            options.into_iter().map(|o| (o.id, judgment.clone())),
        )
        .unwrap();
        let request = json!({"schema":"hiroute.historical-applicability-test/v1","service_id":"saved-fixture","service_revision":7,"shape":"max_branches","protocol":frozen});
        let decoded: hiroute_application_api::HistoricalApplicabilityTestRequestV1 =
            serde_json::from_value(request.clone()).unwrap();
        assert!(decoded.validate());
        assert_eq!(classifier_value(request.clone()).unwrap(), request);
    }
}
