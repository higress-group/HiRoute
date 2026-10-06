//! Qoder's additional provider uses the existing protected Operation artifact lifecycle.
use hiroute_domain::{
    AdditionalAgentModelV1, AgentAccessGrantMaterial, AgentKindV1, CanonicalDigest,
    ExternalEffectIntentV1, NativeAgentArtifactPort, OperationId, OwnedEffectV1, PortError,
    PortErrorCode, PortResult,
};

use super::{
    QoderNativeError,
    additional_native::{self, Restore},
    native_effects::replay,
};

/// Only the executor receives material; this input deliberately cannot be serialized or logged.
pub struct AdditionalFileConfiguration<'a> {
    pub expected_content: &'a CanonicalDigest,
    pub provider_id: &'a str,
    pub endpoint: &'a str,
    pub models: &'a [AdditionalAgentModelV1],
    pub local_grant: &'a AgentAccessGrantMaterial,
}

/// Preview is read-only and never requires bearer bytes. The protected previous intent proves
/// ownership; an identifier found in an arbitrary user document does not.
pub fn validate_additional_configuration(
    port: &dyn NativeAgentArtifactPort,
    kind: AgentKindV1,
    target: &str,
    provider_id: &str,
    endpoint: &str,
    models: &[AdditionalAgentModelV1],
    previous: Option<(&OperationId, &ExternalEffectIntentV1)>,
) -> PortResult<()> {
    let previous = previous
        .map(|(operation, intent)| {
            if intent.target() != target {
                return Err(error(PortErrorCode::InvalidData, "qoder.previous.target"));
            }
            load(port, operation, intent)
        })
        .transpose()?;
    let current = if previous.is_some() {
        port.read_private_native_target(target)?
    } else {
        port.read_native_target(target)?
    };
    additional_native::validate_configuration(
        kind,
        current.as_deref().map(Vec::as_slice),
        provider_id,
        endpoint,
        models,
        previous.as_ref(),
    )
    .map_err(fields_error)
}

pub fn validate_additional_restoration(
    port: &dyn NativeAgentArtifactPort,
    kind: AgentKindV1,
    target: &str,
    original_operation: &OperationId,
    original_intent: &ExternalEffectIntentV1,
) -> PortResult<()> {
    if original_intent.target() != target {
        return Err(error(PortErrorCode::InvalidData, "qoder.original.target"));
    }
    let restore = load(port, original_operation, original_intent)?;
    if restore.kind().map_err(fields_error)? != kind {
        return Err(error(PortErrorCode::InvalidData, "additional.restore.kind"));
    }
    let current = port.read_native_target(target)?;
    restore
        .validate_restoration(current.as_deref().map(Vec::as_slice))
        .map_err(fields_error)
}

pub fn additional_native_configuration_is_applied(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<bool> {
    let restore = load(port, operation, intent)?;
    let current = match port.read_private_native_target(intent.target()) {
        Ok(current) => current,
        Err(error) if error.code == PortErrorCode::PermissionDenied => return Ok(false),
        Err(error) => return Err(error),
    };
    // Malformed/edited user configuration is drift. A corrupt protected record is not softened.
    Ok(restore
        .applied(current.as_deref().map(Vec::as_slice))
        .unwrap_or(false))
}

pub fn stage_additional_configuration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    configuration: AdditionalFileConfiguration<'_>,
) -> PortResult<OwnedEffectV1> {
    stage(port, operation, intent, configuration, None)
}

pub fn stage_additional_reconfiguration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    previous_operation: &OperationId,
    previous_intent: &ExternalEffectIntentV1,
    configuration: AdditionalFileConfiguration<'_>,
) -> PortResult<OwnedEffectV1> {
    validate_binding(operation, intent, previous_operation, previous_intent)?;
    // Do not require an obsolete previous record for an already staged/applied replay.
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let previous = load(port, previous_operation, previous_intent)?;
    stage(port, operation, intent, configuration, Some(&previous))
}

fn stage(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    configuration: AdditionalFileConfiguration<'_>,
    previous: Option<&Restore>,
) -> PortResult<OwnedEffectV1> {
    if intent.desired_mode() != 0o600 {
        return Err(error(PortErrorCode::InvalidData, "qoder.native.mode"));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let current = if previous.is_some() {
        port.read_private_native_target(intent.target())?
    } else {
        port.read_native_target(intent.target())?
    };
    if CanonicalDigest::of_bytes(current.as_deref().map_or(&[], Vec::as_slice))
        != *configuration.expected_content
    {
        return Err(error(PortErrorCode::Conflict, "qoder.preview.changed"));
    }
    let kind = match intent.desired()["subject"]["agent_id"].as_str() {
        Some("agent_qoder_default") => AgentKindV1::Qoder,
        Some("agent_pi_default") => AgentKindV1::Pi,
        Some("agent_dsh_default") => AgentKindV1::DeepseekHarness,
        _ => return Err(error(PortErrorCode::InvalidData, "additional.native.kind")),
    };
    let edit = additional_native::configure(
        kind,
        current.as_deref().map(Vec::as_slice),
        configuration.provider_id,
        configuration.endpoint,
        configuration.models,
        configuration.local_grant,
        previous,
    )
    .map_err(fields_error)?;
    let record = edit
        .restore
        .encode()
        .map_err(|_| error(PortErrorCode::InvalidData, "qoder.restore.encode"))?;
    port.save_native_restore(operation, intent, &record)?;
    port.stage_native_target(operation, intent, Some(&edit.bytes), true)
}

pub fn stage_additional_restoration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    original_operation: &OperationId,
    original_intent: &ExternalEffectIntentV1,
) -> PortResult<OwnedEffectV1> {
    validate_binding(operation, intent, original_operation, original_intent)?;
    if intent.desired_mode() != 0o600 {
        return Err(error(PortErrorCode::InvalidData, "qoder.native.mode"));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let restore = load(port, original_operation, original_intent)?;
    let current = port.read_native_target(intent.target())?;
    let restored = restore
        .restore(current.as_deref().map(Vec::as_slice))
        .map_err(fields_error)?;
    port.stage_native_target(
        operation,
        intent,
        restored.as_deref().map(Vec::as_slice),
        true,
    )
}

fn validate_binding(
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    previous_operation: &OperationId,
    previous_intent: &ExternalEffectIntentV1,
) -> PortResult<()> {
    if operation == previous_operation || intent.target() != previous_intent.target() {
        return Err(error(PortErrorCode::InvalidData, "qoder.previous.binding"));
    }
    Ok(())
}

fn load(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<Restore> {
    let bytes = port
        .load_native_restore(operation, intent)?
        .ok_or_else(|| error(PortErrorCode::NotFound, "qoder.restore.record"))?;
    Restore::decode(&bytes).map_err(|_| error(PortErrorCode::Corrupt, "qoder.restore.record"))
}

fn fields_error(error: QoderNativeError) -> PortError {
    let context = if error.stage == "default in use" {
        "qoder.default.in-use"
    } else {
        "qoder.native.fields"
    };
    PortError::new(PortErrorCode::Conflict, context)
}

fn error(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}
