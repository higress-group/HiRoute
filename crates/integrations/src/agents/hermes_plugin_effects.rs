//! Ownership and rollback for dependency-free bundled native plugin resources.
use super::native_effects::replay;
use hiroute_domain::{
    CanonicalDigest, ExternalEffectIntentV1, NativeAgentArtifactPort, OperationId, OwnedEffectV1,
    PortError, PortErrorCode, PortResult, SkillFileEffectRef,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const HERMES_PLUGIN_MODULE: &str = include_str!("../../../../assets/agents/hermes/__init__.py");
pub const HERMES_PLUGIN_MANIFEST: &str =
    include_str!("../../../../assets/agents/hermes/plugin.yaml");

fn restore_schema(intent: &ExternalEffectIntentV1) -> &'static str {
    if intent.effect_id() == "agent-connection-native-routing-adapter" {
        "hiroute.native-plugin-restore/v1"
    } else {
        "hiroute.hermes-plugin-restore/v1"
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Restore {
    schema: String,
    borrowed: bool,
    content_digest: CanonicalDigest,
}

fn load(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<Restore> {
    let bytes = port
        .load_native_restore(operation, intent)?
        .ok_or_else(|| invalid("hermes.plugin.restore.missing"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| invalid("hermes.plugin.restore.record"))?;
    let bytes = if value["schema"] == hiroute_domain::NATIVE_FILE_REFERENCE_SCHEMA_V1 {
        let reference: hiroute_domain::NativeFileReferenceRecordV1 =
            serde_json::from_value(value).map_err(|_| invalid("hermes.plugin.reference.record"))?;
        Zeroizing::new(reference.metadata)
    } else {
        bytes
    };
    let restore: Restore =
        serde_json::from_slice(&bytes).map_err(|_| invalid("hermes.plugin.restore.record"))?;
    if restore.schema != restore_schema(intent) {
        return Err(invalid("hermes.plugin.restore.record"));
    }
    Ok(restore)
}

pub fn validate_native_plugin_configuration(
    port: &dyn NativeAgentArtifactPort,
    target: &str,
    content: &[u8],
    previous: Option<(&OperationId, &ExternalEffectIntentV1)>,
) -> PortResult<()> {
    let current = port.read_native_target(target)?;
    if let Some((operation, intent)) = previous {
        if intent.target() != target {
            return Err(invalid("hermes.plugin.target"));
        }
        let restore = load(port, operation, intent)?;
        if current.as_ref().map(|b| CanonicalDigest::of_bytes(b)) != Some(restore.content_digest) {
            return Err(conflict("hermes.plugin.changed"));
        }
    } else if current.as_ref().is_some_and(|b| b.as_slice() != content) {
        return Err(conflict("hermes.plugin.collision"));
    }
    Ok(())
}

pub fn validate_native_plugin_restoration(
    port: &dyn NativeAgentArtifactPort,
    target: &str,
    operation: &OperationId,
    original: &ExternalEffectIntentV1,
) -> PortResult<()> {
    if target != original.target() {
        return Err(invalid("hermes.plugin.target"));
    }
    let restore = load(port, operation, original)?;
    let current = port.read_native_target(target)?;
    if !restore.borrowed
        && current.as_ref().map(|b| CanonicalDigest::of_bytes(b)) != Some(restore.content_digest)
    {
        return Err(conflict("hermes.plugin.changed"));
    }
    Ok(())
}

pub fn native_plugin_is_applied(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<bool> {
    let restore = load(port, operation, intent)?;
    Ok(port
        .read_native_target(intent.target())?
        .as_ref()
        .map(|b| CanonicalDigest::of_bytes(b))
        == Some(restore.content_digest))
}

pub fn stage_native_plugin_configuration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    expected_content: &CanonicalDigest,
    content: &[u8],
    previous: Option<(&OperationId, &ExternalEffectIntentV1)>,
) -> PortResult<OwnedEffectV1> {
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    validate_native_plugin_configuration(port, intent.target(), content, previous)?;
    let current = port.read_native_target(intent.target())?;
    if CanonicalDigest::of_bytes(current.as_deref().map_or(&[], Vec::as_slice)) != *expected_content
    {
        return Err(conflict("hermes.plugin.preview.changed"));
    }
    let previous_restore = previous
        .map(|(id, original)| load(port, id, original))
        .transpose()?;
    let restore = Restore {
        schema: restore_schema(intent).into(),
        borrowed: previous_restore
            .as_ref()
            .map_or(current.is_some(), |p| p.borrowed),
        content_digest: CanonicalDigest::of_bytes(content),
    };
    // An identical preexisting resource is a reference, never a file takeover.
    if restore.borrowed && current.as_deref().map(Vec::as_slice) != Some(content) {
        return Err(conflict("hermes.plugin.borrowed.version"));
    }
    let encoded = Zeroizing::new(
        serde_json::to_vec(&restore).map_err(|_| invalid("hermes.plugin.restore.encode"))?,
    );
    if restore.borrowed {
        return port.stage_native_file_reference(
            operation,
            intent,
            current.as_deref().map(Vec::as_slice),
            &encoded,
        );
    }
    port.save_native_restore(operation, intent, &encoded)?;
    let lineage = previous
        .map(|(id, original)| SkillFileEffectRef::from_intent(id, original))
        .transpose()
        .map_err(|_| invalid("hermes.plugin.lineage"))?;
    port.stage_native_skill_target(operation, intent, Some(content), lineage.as_ref())
}

pub fn stage_native_plugin_restoration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    expected_content: &CanonicalDigest,
    original_operation: &OperationId,
    original: &ExternalEffectIntentV1,
) -> PortResult<OwnedEffectV1> {
    if operation == original_operation {
        return Err(invalid("hermes.plugin.restore.binding"));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    validate_native_plugin_restoration(port, intent.target(), original_operation, original)?;
    let restore = load(port, original_operation, original)?;
    let current = port.read_native_target(intent.target())?;
    if CanonicalDigest::of_bytes(current.as_deref().map_or(&[], Vec::as_slice)) != *expected_content
    {
        return Err(conflict("hermes.plugin.preview.changed"));
    }
    if restore.borrowed {
        let encoded = Zeroizing::new(
            serde_json::to_vec(&restore).map_err(|_| invalid("hermes.plugin.restore.encode"))?,
        );
        return port.stage_native_file_reference(
            operation,
            intent,
            current.as_deref().map(Vec::as_slice),
            &encoded,
        );
    }
    let lineage = SkillFileEffectRef::from_intent(original_operation, original)
        .map_err(|_| invalid("hermes.plugin.lineage"))?;
    port.stage_native_skill_target(operation, intent, None, Some(&lineage))
}

fn invalid(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::InvalidData, context)
}
fn conflict(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::Conflict, context)
}
