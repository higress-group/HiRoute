//! Staged, pre-promotion CLI codecs for the Gateway-unbound Compute/Routing slice.
//!
//! This module owns no discovery, persistence, or routing behavior. It accepts only the exact
//! typed Local Control request DTOs and leaves the public generated command manifest unchanged.

use std::io::Read;

use hiroute_application_api::{
    AgentConnectionApplyRequestV1, AgentConnectionPreviewRequestV1, AgentConnectionStatusRequestV1,
    ApplyPriceOverrideChangeV2, ApplyRequestV1, COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2,
    ComputeConnectionApplyRequestV1, ComputeConnectionAuthorizationRequestV1,
    ComputeConnectionOptionsRequestV1, ComputeConnectionPreviewRequestV1,
    ComputeConnectionTestRequestV1, ComputeManagementQueryV2, ComputeSavePreviewRequestV2,
    ComputeScanRequestV1, ComputeSubscriptionCheckPreviewRequestV2, ErrorCode,
    GetEffectivePricesV2, PreviewPriceOverrideChangeV2, PreviewRequestV1,
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
const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

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
    )
}

pub(crate) fn command_payload(
    command_id: &str,
    options: &[String],
) -> Option<Result<Value, ErrorCode>> {
    match command_id {
        "decision.services.list" => Some(empty::<hiroute_application_api::ClientEmptyRequestV1>(
            options,
        )),
        "decision.services.apply" => Some(preview_or_apply(options)),
        "decision.services.test" => Some(stdin::<
            hiroute_application_api::ClassifierDecisionTestRequestV1,
        >(options)),
        "models.show" => Some(stdin::<hiroute_application_api::ModelCatalogQueryV1>(
            options,
        )),
        "prices.effective" => Some(stdin::<GetEffectivePricesV2>(options)),
        "prices.override.preview" => Some(stdin::<PreviewPriceOverrideChangeV2>(options)),
        "prices.override.apply" => Some(stdin::<ApplyPriceOverrideChangeV2>(options)),
        "compute.scan" => Some(empty::<ComputeScanRequestV1>(options)),
        "compute.list" => Some(empty::<ComputeManagementQueryV2>(options)),
        "compute.show" => Some(positional(options, |source_id| ComputeManagementQueryV2 {
            source_id: Some(source_id),
        })),
        "compute.connection.options" => Some(empty::<ComputeConnectionOptionsRequestV1>(options)),
        "compute.connection.preview" => Some(compute_preview(options)),
        "compute.connection.apply" => Some(stdin::<ComputeConnectionApplyRequestV1>(options)),
        "compute.connection.authorize" => {
            Some(stdin::<ComputeConnectionAuthorizationRequestV1>(options))
        }
        "compute.connection.login" => Some(stdin::<
            hiroute_application_api::ComputeSubscriptionLoginRequestV1,
        >(options)),
        "compute.connection.test" => Some(stdin::<ComputeConnectionTestRequestV1>(options)),
        "compute.credential.add" => Some(preview_or_apply(options)),
        "routing.list" if !options.is_empty() => Some(stdin::<
            hiroute_application_api::AgentPlanCatalogQueryV2,
        >(options)),
        "routing.preview" => Some(routing_request(options, false)),
        "routing.apply" => Some(routing_request(options, true)),
        "agents.connect.preview" => Some(agent_request(options, false)),
        "routing.options" => {
            Some(stdin::<hiroute_application_api::PlanEditorOptionsRequestV1>(options))
        }
        "agents.connect.apply" => Some(agent_request(options, true)),
        "agents.restore.preview" => Some(agent_restore_request(options, false)),
        "agents.restore.apply" => Some(agent_restore_request(options, true)),
        "agents.connect.status" => Some(positional(options, |connection_id| {
            if connection_id.starts_with("agent-context/") {
                serde_json::to_value(hiroute_application_api::AgentSettingsStatusRequestV2 {
                    schema_version: hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2,
                    context_id: connection_id,
                })
                .expect("typed status serializes")
            } else {
                serde_json::to_value(AgentConnectionStatusRequestV1 { connection_id })
                    .expect("typed status serializes")
            }
        })),
        _ => None,
    }
}

fn compute_preview(options: &[String]) -> Result<Value, ErrorCode> {
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    let value = read_value(std::io::stdin())?;
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
    let request: T = serde_json::from_value(value).map_err(|_| ErrorCode::InvalidArguments)?;
    serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
}

fn routing_request(options: &[String], apply: bool) -> Result<Value, ErrorCode> {
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    let value = read_value(std::io::stdin())?;
    fn typed<T: DeserializeOwned + Serialize>(value: Value) -> Result<Value, ErrorCode> {
        let request: T = serde_json::from_value(value).map_err(|_| ErrorCode::InvalidArguments)?;
        serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
    }
    if value
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PUBLICATION_CHECKPOINT_CHANGE_SCHEMA_V1)
    {
        return if apply {
            typed::<hiroute_application_api::PublicationCheckpointApplyRequestV1>(value)
        } else {
            typed::<hiroute_application_api::PublicationCheckpointPreviewRequestV1>(value)
        };
    }
    if value
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_LIFECYCLE_CHANGE_SCHEMA_V1)
    {
        return if apply {
            typed::<hiroute_application_api::PlanLifecycleApplyRequestV1>(value)
        } else {
            typed::<hiroute_application_api::PlanLifecyclePreviewRequestV1>(value)
        };
    }
    if value
        .get("change")
        .and_then(|c| c.get("schema"))
        .and_then(Value::as_str)
        == Some(hiroute_application_api::PLAN_DRAFT_CHANGE_SCHEMA_V1)
    {
        return if apply {
            typed::<hiroute_application_api::PlanDraftApplyRequestV1>(value)
        } else {
            typed::<hiroute_application_api::PlanDraftPreviewRequestV1>(value)
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
        typed::<hiroute_application_api::PlanContentApplyRequestV2>(value)
    } else {
        typed::<hiroute_application_api::PlanContentPreviewRequestV2>(value)
    }
}

fn agent_request(options: &[String], apply: bool) -> Result<Value, ErrorCode> {
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    validate_agent_request(read_value(std::io::stdin())?, apply)
}

fn agent_restore_request(options: &[String], apply: bool) -> Result<Value, ErrorCode> {
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    validate_restore_request(read_value(std::io::stdin())?, apply)
}

fn validate_restore_request(value: Value, apply: bool) -> Result<Value, ErrorCode> {
    let value = validate_agent_request(value, apply)?;
    let spec: hiroute_application_api::AgentSettingsSpecV2 =
        serde_json::from_value(value["spec"].clone()).map_err(|_| ErrorCode::InvalidArguments)?;
    if spec.schema_version != hiroute_application_api::AGENT_SETTINGS_SCHEMA_V2
        || !spec.is_restore_only()
    {
        return Err(ErrorCode::InvalidArguments);
    }
    Ok(value)
}

fn validate_agent_request(value: Value, apply: bool) -> Result<Value, ErrorCode> {
    let v2 = value["spec"]["schema_version"]["major"] == 2;
    let valid = match (v2, apply) {
        (true, true) => {
            serde_json::from_value::<hiroute_application_api::AgentSettingsApplyV2>(value.clone())
                .is_ok()
        }
        (true, false) => serde_json::from_value::<
            hiroute_application_api::AgentSettingsPreviewRequestV2,
        >(value.clone())
        .is_ok(),
        (false, true) => {
            serde_json::from_value::<AgentConnectionApplyRequestV1>(value.clone()).is_ok()
        }
        (false, false) => {
            serde_json::from_value::<AgentConnectionPreviewRequestV1>(value.clone()).is_ok()
        }
    };
    if valid {
        Ok(value)
    } else {
        Err(ErrorCode::InvalidArguments)
    }
}

fn preview_or_apply(options: &[String]) -> Result<Value, ErrorCode> {
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    let value = read_value(std::io::stdin())?;
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
        serde_json::from_value::<PreviewRequestV1>(value.clone())
            .map_err(|_| ErrorCode::InvalidArguments)?;
    } else if keys == apply_keys {
        let apply = serde_json::from_value::<ApplyRequestV1>(value.clone())
            .map_err(|_| ErrorCode::InvalidArguments)?;
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
    if value.is_empty() {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(build(value.clone())).map_err(|_| ErrorCode::Internal)
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

fn stdin<T>(options: &[String]) -> Result<Value, ErrorCode>
where
    T: DeserializeOwned + Serialize,
{
    if options != ["--request-stdin"] {
        return Err(ErrorCode::InvalidArguments);
    }
    typed_request::<T>(std::io::stdin())
}

fn typed_request<T>(reader: impl Read) -> Result<Value, ErrorCode>
where
    T: DeserializeOwned + Serialize,
{
    let mut bytes = Vec::new();
    reader
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(ErrorCode::InvalidArguments);
    }
    let request: T = serde_json::from_slice(&bytes).map_err(|_| ErrorCode::InvalidArguments)?;
    serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
}

fn read_value(reader: impl Read) -> Result<Value, ErrorCode> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::from_slice(&bytes).map_err(|_| ErrorCode::InvalidArguments)
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
