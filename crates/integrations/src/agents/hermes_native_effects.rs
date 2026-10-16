//! Hermes uses the same protected before-image and Operation journal as other Agents.
use super::native_effects::replay;
use super::{HermesConfigRestoreV1, hermes_configure};
use hiroute_domain::{
    AdditionalAgentModelV1, AgentAccessGrantMaterial, CanonicalDigest, ExternalEffectIntentV1,
    NativeAgentArtifactPort, OperationId, OwnedEffectV1, PortError, PortErrorCode, PortResult,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Restore {
    schema: String,
    absent: bool,
    config: HermesConfigRestoreV1,
}

fn load(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<Restore> {
    let bytes = port
        .load_native_restore(operation, intent)?
        .ok_or_else(|| invalid("hermes.restore.missing"))?;
    let restore: Restore =
        serde_json::from_slice(&bytes).map_err(|_| invalid("hermes.restore.record"))?;
    if restore.schema != "hiroute.hermes-native-restore/v1" || restore.config.encode().is_err() {
        return Err(invalid("hermes.restore.record"));
    }
    Ok(restore)
}

pub fn validate_hermes_configuration(
    port: &dyn NativeAgentArtifactPort,
    target: &str,
    provider: &str,
    endpoint: &str,
    model: &AdditionalAgentModelV1,
    previous: Option<(&OperationId, &ExternalEffectIntentV1)>,
) -> PortResult<()> {
    let previous = previous
        .map(|(operation, intent)| {
            if intent.target() != target {
                return Err(invalid("hermes.restore.target"));
            }
            load(port, operation, intent)
        })
        .transpose()?;
    let current = if previous.is_some() {
        port.read_private_native_target(target)?
    } else {
        port.read_native_target(target)?
    };
    hermes_configure(
        current.as_deref().map_or(&[], Vec::as_slice),
        previous.as_ref().map(|p| &p.config),
        provider,
        endpoint,
        model,
        "preview-placeholder-token",
    )
    .map(|_| ())
    .map_err(|_| conflict("hermes.native.fields"))
}

pub fn validate_hermes_restoration(
    port: &dyn NativeAgentArtifactPort,
    target: &str,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<()> {
    if intent.target() != target {
        return Err(invalid("hermes.restore.target"));
    }
    let restore = load(port, operation, intent)?;
    let current = port
        .read_private_native_target(target)?
        .ok_or_else(|| conflict("hermes.native.fields"))?;
    restore
        .config
        .restore(&current)
        .map(|_| ())
        .map_err(|_| conflict("hermes.native.fields"))
}

pub fn hermes_native_configuration_is_applied(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<bool> {
    let restore = load(port, operation, intent)?;
    let current = match port.read_private_native_target(intent.target()) {
        Ok(value) => value,
        Err(error) if error.code == PortErrorCode::PermissionDenied => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(current
        .as_ref()
        .is_some_and(|bytes| restore.config.applied(bytes).unwrap_or(false)))
}

pub struct HermesFileConfiguration<'a> {
    pub expected_content: &'a CanonicalDigest,
    pub provider_id: &'a str,
    pub endpoint: &'a str,
    pub model: &'a AdditionalAgentModelV1,
    pub local_grant: &'a AgentAccessGrantMaterial,
}

pub fn stage_hermes_configuration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    configuration: HermesFileConfiguration<'_>,
    previous: Option<(&OperationId, &ExternalEffectIntentV1)>,
) -> PortResult<OwnedEffectV1> {
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let previous = previous
        .map(|(previous, original)| {
            if previous == operation || intent.target() != original.target() {
                return Err(invalid("hermes.previous.binding"));
            }
            load(port, previous, original)
        })
        .transpose()?;
    let current = if previous.is_some() {
        port.read_private_native_target(intent.target())?
    } else {
        port.read_native_target(intent.target())?
    };
    if CanonicalDigest::of_bytes(current.as_deref().map_or(&[], Vec::as_slice))
        != *configuration.expected_content
    {
        return Err(conflict("hermes.preview.changed"));
    }
    let token = std::str::from_utf8(configuration.local_grant.expose())
        .map_err(|_| invalid("hermes.grant.material"))?;
    let (bytes, config) = hermes_configure(
        current.as_deref().map_or(&[], Vec::as_slice),
        previous.as_ref().map(|p| &p.config),
        configuration.provider_id,
        configuration.endpoint,
        configuration.model,
        token,
    )
    .map_err(|_| conflict("hermes.native.fields"))?;
    let record = Restore {
        schema: "hiroute.hermes-native-restore/v1".into(),
        absent: previous.as_ref().map_or(current.is_none(), |p| p.absent),
        config,
    };
    let encoded =
        Zeroizing::new(serde_json::to_vec(&record).map_err(|_| invalid("hermes.restore.encode"))?);
    port.save_native_restore(operation, intent, &encoded)?;
    port.stage_native_target(operation, intent, Some(&bytes), true)
}

pub fn stage_hermes_restoration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    previous: &OperationId,
    original: &ExternalEffectIntentV1,
) -> PortResult<OwnedEffectV1> {
    if operation == previous || intent.target() != original.target() {
        return Err(invalid("hermes.restore.binding"));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let restore = load(port, previous, original)?;
    let current = port
        .read_private_native_target(intent.target())?
        .ok_or_else(|| conflict("hermes.native.fields"))?;
    let bytes = restore
        .config
        .restore(&current)
        .map_err(|_| conflict("hermes.native.fields"))?;
    // An originally absent file is removed only if no later unowned content survives.
    let desired = if restore.absent && bytes.iter().all(|b| b.is_ascii_whitespace()) {
        None
    } else {
        Some(bytes.as_slice())
    };
    port.stage_native_target(operation, intent, desired, true)
}

fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}
fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}
