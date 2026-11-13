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
    pub independent_dsh_provider: bool,
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
    if kind == AgentKindV1::DeepseekHarness
        && !previous
            .as_ref()
            .is_some_and(Restore::independent_dsh_provider)
    {
        let path = port.native_target_path(target)?;
        additional_native::attach_routing_adapter(
            current.as_deref().map_or(b"[]\n", Vec::as_slice),
            &path,
            None,
        )
        .map_err(fields_error)?;
    }
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
    restore_managed_changes: bool,
) -> PortResult<()> {
    if original_intent.target() != target {
        return Err(error(PortErrorCode::InvalidData, "qoder.original.target"));
    }
    let restore = load(port, original_operation, original_intent)?;
    if restore.kind().map_err(fields_error)? != kind {
        return Err(error(PortErrorCode::InvalidData, "additional.restore.kind"));
    }
    let current = port.read_native_target(target)?;
    let current = current.as_deref().map(Vec::as_slice);
    if restore_managed_changes {
        render_restoration(port, target, &restore, current, true).map(|_| ())
    } else {
        restore
            .render_restoration(current, false)
            .map(|_| ())
            .map_err(fields_error)
    }
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

pub fn additional_restore_provider_ids(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<Vec<String>> {
    Ok(load(port, operation, intent)?
        .owned_providers()
        .into_keys()
        .collect())
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
        Some("agent_dsh_default" | "agent_dsh_desktop") => AgentKindV1::DeepseekHarness,
        Some("agent_opencode_default") => AgentKindV1::OpenCode,
        _ => return Err(error(PortErrorCode::InvalidData, "additional.native.kind")),
    };
    let current = current.as_deref().map(Vec::as_slice);
    let mut edit = if configuration.independent_dsh_provider {
        if intent.desired()["subject"]["agent_id"] != "agent_dsh_desktop" {
            return Err(error(PortErrorCode::InvalidData, "additional.native.kind"));
        }
        additional_native::configure_dsh_desktop(
            &port.native_target_path(intent.target())?,
            current,
            configuration.provider_id,
            configuration.endpoint,
            configuration.models,
            configuration.local_grant,
            previous,
        )
    } else {
        additional_native::configure(
            kind,
            current,
            configuration.provider_id,
            configuration.endpoint,
            configuration.models,
            configuration.local_grant,
            previous,
        )
    }
    .map_err(fields_error)?;
    if kind == AgentKindV1::DeepseekHarness && !configuration.independent_dsh_provider {
        let path = port.native_target_path(intent.target())?;
        if let Some(previous) = previous {
            // Provider-only removal preserves unrelated user edits, including
            // the previous adapter insert. Carry its original ownership forward
            // instead of adopting our own insert as user-owned configuration.
            edit.restore.original = additional_native::attach_routing_adapter(
                &edit.restore.original,
                &path,
                Some(&previous.original),
            )
            .map_err(fields_error)?
            .to_vec();
        }
        edit.bytes = additional_native::attach_routing_adapter(&edit.bytes, &path, None)
            .map_err(fields_error)?;
        edit.restore.rendered_digest = CanonicalDigest::of_bytes(&edit.bytes);
    }
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
    restore_managed_changes: bool,
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
    let restored = render_restoration(
        port,
        intent.target(),
        &restore,
        current.as_deref().map(Vec::as_slice),
        restore_managed_changes,
    )?;
    port.stage_native_target(
        operation,
        intent,
        restored.as_deref().map(Vec::as_slice),
        true,
    )
}

fn render_restoration(
    port: &dyn NativeAgentArtifactPort,
    target: &str,
    restore: &Restore,
    current: Option<&[u8]>,
    restore_managed_changes: bool,
) -> PortResult<Option<zeroize::Zeroizing<Vec<u8>>>> {
    let mut restored = restore
        .render_restoration(current, restore_managed_changes)
        .map_err(fields_error)?;
    if restore.kind().map_err(fields_error)? == AgentKindV1::DeepseekHarness
        && !restore.independent_dsh_provider()
        && let Some(bytes) = &restored
    {
        restored = Some(if restore_managed_changes {
            let original =
                super::dsh_config::Patch::parse(Some(&restore.original)).map_err(fields_error)?;
            super::dsh_config::Patch::parse(Some(bytes))
                .and_then(|patch| {
                    let bytes = patch.restore_routing_adapter(&original, &Default::default())?;
                    super::dsh_config::Patch::parse(Some(&bytes))?
                        .remove_empty_legacy_provider_row(&original)
                })
                .map_err(fields_error)?
        } else {
            let path = port.native_target_path(target)?;
            additional_native::attach_routing_adapter(bytes, &path, Some(&restore.original))
                .map_err(fields_error)?
        });
    }
    Ok(restored)
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
