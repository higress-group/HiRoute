//! Non-secret native model file intent, bound to the original settings Operation.
use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    AgentConnectionTransactionSubjectV1, AgentFacetIntent, AgentSettingsSpecV2, CanonicalDigest,
    ExternalEffectIntentV1, OperationId, OperationV1, OperationValidationError, PortError,
    PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "hiroute.settings-codex-model-file/v1";
const CATALOG_SCHEMA: &str = "hiroute.codex-catalog-artifact/v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum CodexModelFileAction {
    Configure {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        previous_operation: Option<OperationId>,
        provider_id: String,
        endpoint: String,
        model: Option<String>,
        /// Content digest of the immutable model catalog artifact the configuration points at.
        /// Present exactly when the selection carries permitted plans.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_catalog: Option<CanonicalDigest>,
    },
    Restore {
        original_operation: OperationId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        native_model: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexModelFilePayload {
    schema: String,
    pub context_id: String,
    pub expected_content: CanonicalDigest,
    pub change: CodexModelFileAction,
}

/// The settings renderer supplies the previewed whole-file digest and registered endpoint.
/// Bearer bytes are deliberately absent; the daemon resolves only this Operation's staged grant.
pub fn settings_codex_model_file_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    expected_content: CanonicalDigest,
    before_fingerprint: Option<CanonicalDigest>,
    change: CodexModelFileAction,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = CodexModelFilePayload {
        schema: SCHEMA.into(),
        context_id: context_id.into(),
        expected_content,
        change,
    };
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::ManagedConfiguration,
        before_fingerprint,
        &payload,
        0o600,
    )?;
    decode_settings_codex_model_file(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

/// The immutable model catalog artifact bound to the same settings Operation. The trusted
/// backend supplies the traceable catalog source revision and the merged catalog digest; the
/// artifact bytes themselves never cross this boundary.
pub fn settings_codex_catalog_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    facts: &super::SettingsModelCatalogFacts,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = settings_codex_catalog_payload(context_id, facts);
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::ModelCatalog,
        facts.before_fingerprint.clone(),
        &payload,
        0o600,
    )?;
    decode_settings_codex_catalog(&intent)
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

/// Resolve the same content-addressed target before sealing so an existing immutable artifact
/// can be observed and protected like every other external effect.
pub fn settings_codex_catalog_target(
    subject: &AgentConnectionTransactionSubjectV1,
    context_id: &str,
    facts: &super::SettingsModelCatalogFacts,
) -> Result<String, OperationValidationError> {
    let digest = CanonicalDigest::of(&settings_codex_catalog_payload(context_id, facts))?;
    AgentConnectionEffectRoleV1::ModelCatalog.settings_payload_target_for(subject, &digest)
}

fn settings_codex_catalog_payload(
    context_id: &str,
    facts: &super::SettingsModelCatalogFacts,
) -> CodexCatalogFilePayload {
    CodexCatalogFilePayload {
        schema: CATALOG_SCHEMA.into(),
        context_id: context_id.into(),
        source_revision: facts.source_revision.clone(),
        content_digest: facts.content_digest.clone(),
        producer_kind: facts.producer_kind,
        producer_path: facts.producer_path.clone(),
        producer_content_digest: facts.producer_content_digest.clone(),
        producer_context_digest: facts.producer_context_digest.clone(),
        producer_dependency_digest: facts.producer_dependency_digest.clone(),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexCatalogFilePayload {
    schema: String,
    pub context_id: String,
    pub source_revision: String,
    pub content_digest: CanonicalDigest,
    pub producer_kind: super::CodexCatalogProducerKindV1,
    pub producer_path: String,
    pub producer_content_digest: CanonicalDigest,
    pub producer_context_digest: CanonicalDigest,
    pub producer_dependency_digest: CanonicalDigest,
}

pub fn decode_settings_codex_catalog(
    intent: &ExternalEffectIntentV1,
) -> PortResult<CodexCatalogFilePayload> {
    ExternalEffectIntentV1::from_registered_adapter(
        intent.effect_id(),
        intent.kind(),
        intent.target(),
        intent.before_fingerprint().cloned(),
        intent.desired().clone(),
        intent.desired_mode(),
        intent.sensitive(),
    )
    .map_err(|_| invalid())?;
    let value = intent.desired();
    if intent.effect_id() != "agent-connection-model-catalog"
        || intent.desired_mode() != 0o600
        || value["transaction"] != "settings"
        || value["subject"]["agent_id"] != "agent_codex_default"
        || value["subject"]["profile_id"] != "codex-responses-v1"
        || value["subject"]["integration_profile_ref"] != "builtin/codex-responses/v1"
    {
        return Err(invalid());
    }
    let payload: CodexCatalogFilePayload =
        serde_json::from_value(value["payload"].clone()).map_err(|_| invalid())?;
    if payload.schema != CATALOG_SCHEMA
        || !super::settings::identity(&payload.context_id)
        || payload.source_revision.is_empty()
        || payload.source_revision.len() > 128
        || payload.source_revision.contains(['\0', '\r', '\n'])
        || !absolute_catalog_source_path(&payload.producer_path)
    {
        return Err(invalid());
    }
    Ok(payload)
}

fn absolute_catalog_source_path(value: &str) -> bool {
    let path = std::path::Path::new(value);
    value.len() <= 4096
        && !value.contains(['\0', '\r', '\n'])
        && path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
}

pub fn decode_settings_codex_model_file(
    intent: &ExternalEffectIntentV1,
) -> PortResult<CodexModelFilePayload> {
    ExternalEffectIntentV1::from_registered_adapter(
        intent.effect_id(),
        intent.kind(),
        intent.target(),
        intent.before_fingerprint().cloned(),
        intent.desired().clone(),
        intent.desired_mode(),
        intent.sensitive(),
    )
    .map_err(|_| invalid())?;
    let value = intent.desired();
    if intent.effect_id() != "agent-connection-managed-configuration"
        || intent.desired_mode() != 0o600
        || value["transaction"] != "settings"
        || value["subject"]["agent_id"] != "agent_codex_default"
        || value["subject"]["profile_id"] != "codex-responses-v1"
        || value["subject"]["integration_profile_ref"] != "builtin/codex-responses/v1"
    {
        return Err(invalid());
    }
    let payload: CodexModelFilePayload =
        serde_json::from_value(value["payload"].clone()).map_err(|_| invalid())?;
    if payload.schema != SCHEMA || !super::settings::identity(&payload.context_id) {
        return Err(invalid());
    }
    if let CodexModelFileAction::Configure {
        previous_operation,
        provider_id,
        endpoint,
        model,
        model_catalog,
    } = &payload.change
        && (provider_id != "hiroute"
            || !local_endpoint(endpoint)
            || endpoint.len() > 2048
            || endpoint.contains(['\0', '\n', '\r'])
            || model
                .as_deref()
                .is_some_and(|name| !hiroute_domain::valid_client_model_name(name))
            || previous_operation
                .as_ref()
                .is_some_and(|previous| OperationId::parse(previous.as_str().to_owned()).is_err())
            || model_catalog
                .as_ref()
                .is_some_and(|digest| CanonicalDigest::parse(digest.as_str().to_owned()).is_err()))
    {
        return Err(invalid());
    }
    Ok(payload)
}

/// Validate membership and facet again when consuming a persisted journal during execution.
/// The original payload, not a new current-state preview, supplies recovery inputs.
pub fn settings_codex_model_file_for_operation(
    operation: &OperationV1,
    intent: &ExternalEffectIntentV1,
) -> PortResult<CodexModelFilePayload> {
    let payload = decode_settings_codex_model_file(intent)?;
    let spec: AgentSettingsSpecV2 =
        serde_json::from_value(operation.plan.spec().desired_state.clone())
            .map_err(|_| invalid())?;
    if operation.plan.spec().command_id != "agents.settings.apply"
        || spec.context_id != payload.context_id
        || !operation.plan.external().contains(intent)
    {
        return Err(invalid());
    }
    let state = &operation.plan.control()["payload"]["state"];
    if state["accept_digest"]
        != serde_json::to_value(&operation.accepted_digest).map_err(|_| invalid())?
    {
        return Err(invalid());
    }
    match (&spec.model, &payload.change) {
        (
            AgentFacetIntent::Configure { settings },
            CodexModelFileAction::Configure {
                previous_operation,
                model,
                model_catalog,
                ..
            },
        ) => {
            let grant: hiroute_domain::AgentModelGrantV2 =
                serde_json::from_value(state["model_grant"].clone()).map_err(|_| invalid())?;
            let [mutation] = operation.plan.agent_access_grants() else {
                return Err(invalid());
            };
            let scope = mutation.desired_scope().ok_or_else(invalid)?;
            if scope.model_grant() != &grant {
                return Err(invalid());
            }
            if grant
                .codex_default_override(settings)
                .map_err(|_| invalid())?
                != *model
                || previous_operation.as_ref() == Some(&operation.operation_id)
            {
                return Err(invalid());
            }
            // Every managed Codex model selection has one filtered catalog artifact.
            if model_catalog.is_none() {
                return Err(invalid());
            }
            if let Some(digest) = model_catalog
                && !operation.plan.external().iter().any(|intent| {
                    decode_settings_codex_catalog(intent)
                        .is_ok_and(|catalog| &catalog.content_digest == digest)
                })
            {
                return Err(invalid());
            }
        }
        (
            AgentFacetIntent::Restore { restore_point_ref },
            CodexModelFileAction::Restore {
                original_operation,
                native_model,
            },
        ) if original_operation != &operation.operation_id
            && *restore_point_ref == codex_model_restore_point_ref(original_operation)
            && &spec.restore_native_model == native_model => {}
        _ => return Err(invalid()),
    }
    Ok(payload)
}

fn invalid() -> PortError {
    PortError::new(PortErrorCode::InvalidData, "codex.settings.intent")
}

fn local_endpoint(value: &str) -> bool {
    value
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| value.strip_prefix("http://[::1]:"))
        .and_then(|value| value.strip_suffix("/v1"))
        .is_some_and(|port| {
            !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        })
}

/// Opaque reference exposed only for an owned original model configuration Operation.
pub fn codex_model_restore_point_ref(operation: &OperationId) -> String {
    format!("model-restore/{}", operation.as_str())
}
