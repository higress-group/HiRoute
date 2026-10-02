use hiroute_domain::{
    AgentConnectionControlIntentV1, AgentConnectionEffectRoleV1, AgentConnectionTransactionKindV1,
    CanonicalDigest, ExternalEffectIntentV1, NativeAgentArtifactPort, OperationId,
    OperationValidationError, OwnedEffectV1, PortError, PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};

use super::{CODEX_CATALOG_SOURCE_REVISION, CodexCatalogProducerFactsV1, CodexCatalogSelection};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CatalogPayload {
    schema: String,
    context_id: String,
    source_revision: String,
    content_digest: CanonicalDigest,
    producer_kind: super::CodexCatalogMetadataSourceV1,
    producer_path: String,
    producer_content_digest: CanonicalDigest,
    producer_context_digest: CanonicalDigest,
    producer_dependency_digest: CanonicalDigest,
}

pub fn codex_catalog_intent(
    control: &AgentConnectionControlIntentV1,
    context_id: &str,
    catalog: &CodexCatalogSelection,
    producer: &CodexCatalogProducerFactsV1,
    before: Option<CanonicalDigest>,
) -> Result<ExternalEffectIntentV1, OperationValidationError> {
    if control.transaction() != AgentConnectionTransactionKindV1::Settings {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    catalog
        .validate_schema()
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    let bytes = serde_json::to_vec(catalog.original())
        .map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    if bytes.len() > 1024 * 1024 {
        return Err(OperationValidationError::UnregisteredEffectPlan);
    }
    let payload = CatalogPayload {
        schema: "hiroute.codex-catalog-artifact/v1".into(),
        context_id: context_id.into(),
        source_revision: CODEX_CATALOG_SOURCE_REVISION.into(),
        content_digest: CanonicalDigest::of_bytes(&bytes),
        producer_kind: producer.metadata_source,
        producer_path: producer
            .path
            .to_str()
            .ok_or(OperationValidationError::UnregisteredEffectPlan)?
            .into(),
        producer_content_digest: producer.content_digest.clone(),
        producer_context_digest: producer.context_digest.clone(),
        producer_dependency_digest: producer.dependency_digest.clone(),
    };
    let intent = ExternalEffectIntentV1::from_agent_connection_planner(
        control,
        AgentConnectionEffectRoleV1::ModelCatalog,
        before,
        &payload,
        0o600,
    )?;
    decode(&intent).map_err(|_| OperationValidationError::UnregisteredEffectPlan)?;
    Ok(intent)
}

pub fn stage_codex_catalog(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    catalog: &CodexCatalogSelection,
) -> PortResult<OwnedEffectV1> {
    catalog.validate_schema().map_err(|_| invalid())?;
    let bytes = serde_json::to_vec(catalog.original()).map_err(|_| invalid())?;
    validate_bytes(intent, &bytes)?;
    if let Some(effect) = super::native_effects::replay(port, operation, intent)? {
        return Ok(effect);
    }
    port.save_native_restore(operation, intent, &bytes)?;
    restage_codex_catalog(port, operation, intent)
}

pub fn restage_codex_catalog(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<OwnedEffectV1> {
    decode(intent)?;
    if let Some(effect) = super::native_effects::replay(port, operation, intent)? {
        return Ok(effect);
    }
    let bytes = port
        .load_native_restore(operation, intent)?
        .ok_or_else(invalid)?;
    validate_bytes(intent, &bytes)?;
    if port
        .read_native_target(intent.target())?
        .is_some_and(|current| current.as_slice() != bytes.as_slice())
    {
        return Err(PortError::new(
            PortErrorCode::Conflict,
            "codex.catalog.immutable",
        ));
    }
    port.stage_native_target(operation, intent, Some(&bytes), true)
}

fn validate_bytes(intent: &ExternalEffectIntentV1, bytes: &[u8]) -> PortResult<()> {
    let payload = decode(intent)?;
    if bytes.len() > 1024 * 1024 || CanonicalDigest::of_bytes(bytes) != payload.content_digest {
        return Err(invalid());
    }
    CodexCatalogSelection::parse(serde_json::from_slice(bytes).map_err(|_| invalid())?)
        .and_then(|catalog| catalog.validate_schema())
        .map_err(|_| invalid())
}

fn decode(intent: &ExternalEffectIntentV1) -> PortResult<CatalogPayload> {
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
        || !matches!(
            value["subject"]["profile_id"].as_str(),
            Some("codex-responses-v1" | "codex-standalone-profile-v1")
        )
        || value["subject"]["integration_profile_ref"] != "builtin/codex-responses/v1"
    {
        return Err(invalid());
    }
    let payload: CatalogPayload =
        serde_json::from_value(value["payload"].clone()).map_err(|_| invalid())?;
    if payload.schema != "hiroute.codex-catalog-artifact/v1"
        || payload.source_revision != CODEX_CATALOG_SOURCE_REVISION
        || payload.context_id.is_empty()
        || payload.context_id.len() > 256
        || payload.context_id.contains(['\0', '\r', '\n'])
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

fn invalid() -> PortError {
    PortError::new(PortErrorCode::InvalidData, "codex.catalog.intent")
}
