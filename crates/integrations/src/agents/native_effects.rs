//! Main-agent native bytes over the existing staged Operation artifact port.
use super::{
    CodexNativeEdit, CodexNativeRestore, CodexSelectionTarget, configure_codex_native,
    reconfigure_codex_native, restore_codex_native_with_model,
};
use hiroute_domain::{
    AgentAccessGrantMaterial, CanonicalDigest, EffectReconciliation, ExternalEffectIntentV1,
    NativeAgentArtifactPort, OperationId, OwnedEffectV1, PortError, PortErrorCode, PortResult,
};
use zeroize::Zeroizing;

pub struct CodexFileConfiguration<'a> {
    pub expected_content: &'a CanonicalDigest,
    pub selection: CodexSelectionTarget,
    pub provider_id: &'a str,
    pub endpoint: &'a str,
    pub model: Option<&'a str>,
    pub local_grant: &'a AgentAccessGrantMaterial,
    /// Registered target of this Operation's immutable model catalog artifact. The pointer is
    /// rendered natively from the resolved absolute path, never from client input.
    pub model_catalog: Option<&'a str>,
    /// Exact Plan aliases in this Operation's sealed Agent grant.
    pub managed_aliases: &'a [String],
}

/// A later edit to an unrelated Codex setting does not detach the managed route.
/// Preview and Apply still use the exact whole-file snapshot for safe writes.
pub fn codex_native_configuration_is_applied(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<bool> {
    let record = port
        .load_native_restore(operation, intent)?
        .ok_or_else(|| error(PortErrorCode::NotFound, "codex.status.record"))?;
    if record.len() < 3 || record[0] != 1 || record[1] > 1 {
        return Err(error(PortErrorCode::Corrupt, "codex.status.schema"));
    }
    let restore = CodexNativeRestore::decode_protected(&record[2..])
        .map_err(|_| error(PortErrorCode::Corrupt, "codex.status.decode"))?;
    let Some(current) = port.read_native_target(intent.target())? else {
        return Ok(false);
    };
    let Ok(text) = std::str::from_utf8(&current) else {
        return Ok(false);
    };
    Ok(restore.managed_fields_are_applied(text))
}

/// Called only after the Application revalidates capabilities and owns its Operation writer.
/// Returns a staged effect; the existing Operation runner performs activation and recovery.
pub fn stage_codex_configuration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    configuration: CodexFileConfiguration<'_>,
) -> PortResult<OwnedEffectV1> {
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let current = port.read_native_target(intent.target())?;
    let text = std::str::from_utf8(current.as_deref().map_or(&[], |bytes| bytes.as_slice()))
        .map_err(|_| error(PortErrorCode::InvalidData, "codex.encoding"))?;
    let edit = configure_codex_native(
        text,
        configuration.expected_content,
        configuration.selection,
        configuration.provider_id,
        configuration.endpoint,
        configuration.model,
        configuration.local_grant,
    )
    .and_then(|edit| edit.with_managed_aliases(configuration.managed_aliases))
    .map_err(|_| error(PortErrorCode::Conflict, "codex.configure"))?;
    let edit = apply_catalog_pointer(port, edit, configuration.model_catalog)?;
    save_codex_configuration(port, operation, intent, u8::from(current.is_some()), edit)
}

/// Rebase a managed update onto the protected user state owned by the preceding successful
/// settings Operation. Both journal intents are required so an arbitrary provider cannot be
/// claimed merely because its public identifier happens to match.
pub fn stage_codex_reconfiguration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    previous_operation: &OperationId,
    previous_intent: &ExternalEffectIntentV1,
    configuration: CodexFileConfiguration<'_>,
) -> PortResult<OwnedEffectV1> {
    if operation == previous_operation || intent.target() != previous_intent.target() {
        return Err(error(
            PortErrorCode::InvalidData,
            "codex.reconfigure.binding",
        ));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let current = port.read_native_target(intent.target())?;
    let text = std::str::from_utf8(current.as_deref().map_or(&[], |bytes| bytes.as_slice()))
        .map_err(|_| error(PortErrorCode::InvalidData, "codex.encoding"))?;
    let record = port
        .load_native_restore(previous_operation, previous_intent)?
        .ok_or_else(|| error(PortErrorCode::NotFound, "codex.reconfigure.record"))?;
    if record.len() < 3 || record[0] != 1 || record[1] > 1 {
        return Err(error(PortErrorCode::Corrupt, "codex.reconfigure.schema"));
    }
    let previous = CodexNativeRestore::decode_protected(&record[2..])
        .map_err(|_| error(PortErrorCode::Corrupt, "codex.reconfigure.decode"))?;
    let edit = reconfigure_codex_native(
        text,
        configuration.expected_content,
        &previous,
        configuration.selection,
        configuration.provider_id,
        configuration.endpoint,
        configuration.model,
        configuration.local_grant,
    )
    .and_then(|edit| edit.with_managed_aliases(configuration.managed_aliases))
    .map_err(|_| error(PortErrorCode::Conflict, "codex.reconfigure"))?;
    let edit = apply_catalog_pointer(port, edit, configuration.model_catalog)?;
    save_codex_configuration(port, operation, intent, record[1], edit)
}

fn apply_catalog_pointer(
    port: &dyn NativeAgentArtifactPort,
    edit: super::CodexNativeEdit,
    model_catalog: Option<&str>,
) -> PortResult<super::CodexNativeEdit> {
    let Some(target) = model_catalog else {
        return Ok(edit);
    };
    let path = port
        .native_target_path(target)
        .map_err(|_| error(PortErrorCode::PermissionDenied, "codex.catalog.target"))?;
    edit.with_model_catalog(&path)
        .map_err(|_| error(PortErrorCode::Conflict, "codex.catalog.pointer"))
}

fn save_codex_configuration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    original_exists: u8,
    edit: CodexNativeEdit,
) -> PortResult<OwnedEffectV1> {
    let encoded = edit
        .restore
        .encode_protected()
        .map_err(|_| error(PortErrorCode::InvalidData, "codex.restore.encode"))?;
    // Version byte and existence are protected together with the bounded field record.
    let mut record = Zeroizing::new(vec![1, original_exists]);
    record.extend_from_slice(&encoded);
    port.save_native_restore(operation, intent, &record)?;
    port.stage_native_target(operation, intent, Some(edit.rendered.as_bytes()), true)
}

pub fn stage_codex_restoration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    original_operation: &OperationId,
    original_intent: &ExternalEffectIntentV1,
    native_model: Option<&str>,
) -> PortResult<OwnedEffectV1> {
    if intent.target() != original_intent.target() || operation == original_operation {
        return Err(error(PortErrorCode::InvalidData, "codex.restore.binding"));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let record = port
        .load_native_restore(original_operation, original_intent)?
        .ok_or_else(|| error(PortErrorCode::NotFound, "codex.restore.record"))?;
    if record.len() < 3 || record[0] != 1 || record[1] > 1 {
        return Err(error(PortErrorCode::Corrupt, "codex.restore.schema"));
    }
    let restore = CodexNativeRestore::decode_protected(&record[2..])
        .map_err(|_| error(PortErrorCode::Corrupt, "codex.restore.decode"))?;
    let current = port.read_native_target(intent.target())?;
    let text = std::str::from_utf8(current.as_deref().map_or(&[], |bytes| bytes.as_slice()))
        .map_err(|_| error(PortErrorCode::InvalidData, "codex.restore.encoding"))?;
    let restored = restore_codex_native_with_model(text, &restore, native_model)
        .map_err(|_| error(PortErrorCode::Conflict, "codex.restore.fields"))?;
    let desired = if record[1] == 0 && restored.trim().is_empty() {
        None
    } else {
        Some(restored.as_bytes())
    };
    port.stage_native_target(operation, intent, desired, true)
}

pub(super) fn replay(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<Option<OwnedEffectV1>> {
    match port.observe_artifact(operation, intent)? {
        EffectReconciliation::Staged(effect) | EffectReconciliation::Applied(effect) => {
            Ok(Some(effect))
        }
        EffectReconciliation::Missing => Ok(None),
        EffectReconciliation::OwnershipLost(_) => {
            Err(error(PortErrorCode::Conflict, "native.ownership"))
        }
    }
}
fn error(code: PortErrorCode, context: &'static str) -> PortError {
    PortError { code, context }
}
