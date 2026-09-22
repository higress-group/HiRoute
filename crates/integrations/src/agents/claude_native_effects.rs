//! Claude native rendering/restoration over the same protected Operation artifact port as Codex.
use super::filesystem_config::{
    claude_change_bytes_are_applied, rebase_claude_change_bytes, render_claude_change_bytes,
    restore_claude_change_bytes,
};
use super::native_effects::replay;
use hiroute_domain::{
    AgentConfigChangeV1, CanonicalDigest, ExternalEffectIntentV1, NativeAgentArtifactPort,
    OperationId, OwnedEffectV1, PortError, PortErrorCode, PortResult,
};
use zeroize::Zeroizing;
const RECORD_LIMIT: usize = 1024 * 1024;

/// The user's settings may be reformatted or gain unrelated keys after Apply. The protected
/// restore record, not whole-file equality, determines whether our exact fields remain owned.
pub fn claude_native_configuration_is_applied(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
) -> PortResult<bool> {
    let record = port
        .load_native_restore(operation, intent)?
        .ok_or_else(|| error(PortErrorCode::NotFound, "claude.status.record"))?;
    let (_, change, _) = decode_record(&record)?;
    let Some(current) = port.read_native_target(intent.target())? else {
        return Ok(false);
    };
    claude_change_bytes_are_applied(&current, &change)
        .map_err(|_| error(PortErrorCode::Conflict, "claude.status.fields"))
}

pub fn stage_claude_configuration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    expected_content: &CanonicalDigest,
    change: &AgentConfigChangeV1,
) -> PortResult<OwnedEffectV1> {
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let original = port.read_native_target(intent.target())?;
    let bytes = original.as_deref().map_or(&[][..], Vec::as_slice);
    if &CanonicalDigest::of_bytes(bytes) != expected_content {
        return Err(error(PortErrorCode::Conflict, "claude.preview.changed"));
    }
    let original_json = if original.is_some() { bytes } else { b"{}" };
    let rendered = render_claude_change_bytes(original_json, change)
        .map_err(|_| error(PortErrorCode::Conflict, "claude.native.render"))?;
    save_claude_configuration(
        port,
        operation,
        intent,
        u8::from(original.is_some()),
        original_json,
        change,
        &rendered,
    )
}

/// Replace an owned Claude configuration while retaining the original user bytes as the one
/// formal restore destination. The preceding protected record supplies both ownership and the
/// three-way restore inputs; arbitrary current helper fields are never adopted.
pub fn stage_claude_reconfiguration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    previous_operation: &OperationId,
    previous_intent: &ExternalEffectIntentV1,
    expected_content: &CanonicalDigest,
    change: &AgentConfigChangeV1,
) -> PortResult<OwnedEffectV1> {
    if operation == previous_operation || intent.target() != previous_intent.target() {
        return Err(error(
            PortErrorCode::InvalidData,
            "claude.reconfigure.binding",
        ));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let current = port
        .read_native_target(intent.target())?
        .ok_or_else(|| error(PortErrorCode::Conflict, "claude.reconfigure.removed"))?;
    if &CanonicalDigest::of_bytes(&current) != expected_content {
        return Err(error(PortErrorCode::Conflict, "claude.reconfigure.changed"));
    }
    let record = port
        .load_native_restore(previous_operation, previous_intent)?
        .ok_or_else(|| error(PortErrorCode::NotFound, "claude.reconfigure.record"))?;
    let (original_exists, previous_change, previous_original) = decode_record(&record)?;
    let base = restore_claude_change_bytes(&current, previous_original, &previous_change)
        .map_err(|_| error(PortErrorCode::Conflict, "claude.reconfigure.fields"))?;
    let (rendered, rebased_change) =
        rebase_claude_change_bytes(&current, &base, &previous_change, change)
            .map_err(|_| error(PortErrorCode::Conflict, "claude.reconfigure.render"))?;
    save_claude_configuration(
        port,
        operation,
        intent,
        original_exists,
        &base,
        &rebased_change,
        &rendered,
    )
}

fn save_claude_configuration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    original_exists: u8,
    original_json: &[u8],
    change: &AgentConfigChangeV1,
    rendered: &[u8],
) -> PortResult<OwnedEffectV1> {
    let change_bytes = Zeroizing::new(
        serde_json::to_vec(change)
            .map_err(|_| error(PortErrorCode::InvalidData, "claude.change.encode"))?,
    );
    // Protected envelope: version, original existence, semantic change length, change, raw before.
    // No original helper/token is reconstructed from the public semantic presence placeholders.
    if change_bytes
        .len()
        .checked_add(original_json.len())
        .and_then(|size| size.checked_add(6))
        .is_none_or(|size| size > RECORD_LIMIT)
    {
        return Err(error(PortErrorCode::InvalidData, "claude.restore.bound"));
    }
    let mut record = Zeroizing::new(vec![1, original_exists]);
    record.extend_from_slice(&(change_bytes.len() as u32).to_le_bytes());
    record.extend_from_slice(&change_bytes);
    record.extend_from_slice(original_json);
    port.save_native_restore(operation, intent, &record)?;
    port.stage_native_target(operation, intent, Some(rendered), true)
}

pub fn stage_claude_restoration(
    port: &dyn NativeAgentArtifactPort,
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    original_operation: &OperationId,
    original_intent: &ExternalEffectIntentV1,
) -> PortResult<OwnedEffectV1> {
    if intent.target() != original_intent.target() || operation == original_operation {
        return Err(error(PortErrorCode::InvalidData, "claude.restore.binding"));
    }
    if let Some(effect) = replay(port, operation, intent)? {
        return Ok(effect);
    }
    let record = port
        .load_native_restore(original_operation, original_intent)?
        .ok_or_else(|| error(PortErrorCode::NotFound, "claude.restore.record"))?;
    let (original_exists, change, original) = decode_record(&record)?;
    let current = port.read_native_target(intent.target())?;
    if current.is_none() && original_exists == 0 {
        return port.stage_native_target(operation, intent, None, true);
    }
    let current =
        current.ok_or_else(|| error(PortErrorCode::Conflict, "claude.restore.removed"))?;
    let restored = restore_claude_change_bytes(&current, original, &change)
        .map_err(|_| error(PortErrorCode::Conflict, "claude.restore.fields"))?;
    let empty = original_exists == 0
        && serde_json::from_slice::<serde_json::Value>(&restored)
            .ok()
            .and_then(|root| root.as_object().map(|object| object.is_empty()))
            .unwrap_or(false);
    port.stage_native_target(
        operation,
        intent,
        if empty { None } else { Some(&restored) },
        true,
    )
}

fn decode_record(record: &[u8]) -> PortResult<(u8, AgentConfigChangeV1, &[u8])> {
    if record.len() < 8 || record.len() > RECORD_LIMIT || record[0] != 1 || record[1] > 1 {
        return Err(error(PortErrorCode::Corrupt, "claude.restore.schema"));
    }
    let length = u32::from_le_bytes(record[2..6].try_into().expect("checked header")) as usize;
    let end = 6usize
        .checked_add(length)
        .filter(|end| *end < record.len())
        .ok_or_else(|| error(PortErrorCode::Corrupt, "claude.restore.length"))?;
    let change: AgentConfigChangeV1 = serde_json::from_slice(&record[6..end])
        .map_err(|_| error(PortErrorCode::Corrupt, "claude.restore.change"))?;
    change
        .validate()
        .map_err(|_| error(PortErrorCode::Corrupt, "claude.restore.change"))?;
    Ok((record[1], change, &record[end..]))
}
fn error(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}
