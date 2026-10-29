//! Borrowed resources retain their native bytes/mode and use the existing protected journal.
use super::native_artifacts::read_native_file;
use super::*;
use hiroute_domain::{
    NATIVE_FILE_REFERENCE_SCHEMA_V1, NativeAgentArtifactPort, NativeFileReferenceRecordV1,
};

fn accepts(intent: &ExternalEffectIntentV1) -> bool {
    let native_adapter = intent.effect_id() == "agent-connection-native-routing-adapter"
        && matches!(
            intent.desired()["subject"]["agent_id"].as_str(),
            Some("agent_pi_default" | "agent_dsh_default" | "agent_opencode_default")
        )
        && intent.desired()["payload"]["schema"] == "hiroute.settings-native-plugin-file/v1";
    let hermes_plugin = matches!(
        intent.effect_id(),
        "agent-connection-hermes-plugin-module" | "agent-connection-hermes-plugin-manifest"
    ) && intent.desired()["subject"]["agent_id"] == "agent_hermes_default"
        && intent.desired()["payload"]["schema"] == "hiroute.settings-hermes-plugin-file/v1";
    (native_adapter || hermes_plugin)
        && intent.kind() == OwnedEffectKind::AgentArtifact
        && !intent.sensitive()
        && intent.desired_mode() == 0o644
        && intent.desired()["transaction"] == "settings"
}
fn effect(
    operation: &OperationId,
    intent: &ExternalEffectIntentV1,
    fingerprint: Option<CanonicalDigest>,
) -> OwnedEffectV1 {
    OwnedEffectV1 { effect_id: intent.effect_id().into(), kind: intent.kind(), target: intent.target().into(), before_fingerprint: intent.before_fingerprint().cloned(), after_fingerprint: fingerprint,
        compensation: json!({"schema":NATIVE_FILE_REFERENCE_SCHEMA_V1,"operation_id":operation,"effect_id":intent.effect_id(),"desired":intent.desired()}).into() }
}

impl ManagedArtifactStore {
    pub(super) fn observe_native_file_reference(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<EffectReconciliation>> {
        if !accepts(intent) {
            return Ok(None);
        }
        let Some(bytes) = self.load_native_restore(operation, intent)? else {
            return Ok(None);
        };
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| port(PortErrorCode::Corrupt, "native.reference.record"))?;
        if value["schema"] != NATIVE_FILE_REFERENCE_SCHEMA_V1 {
            return Ok(None);
        }
        let record: NativeFileReferenceRecordV1 = serde_json::from_value(value)
            .map_err(|_| port(PortErrorCode::Corrupt, "native.reference.record"))?;
        if record.metadata.len() > 64 * 1024 {
            return Err(port(PortErrorCode::Corrupt, "native.reference.bound"));
        }
        let path = self.native_target_path(intent.target())?;
        let current = read_artifact(&path)
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.reference.read"))?
            .map(|s| s.fingerprint);
        let effect = effect(operation, intent, record.fingerprint.clone());
        Ok(Some(if current == record.fingerprint {
            EffectReconciliation::Applied(effect)
        } else {
            EffectReconciliation::OwnershipLost(effect)
        }))
    }

    pub(super) fn stage_native_file_reference_record(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        expected: Option<&[u8]>,
        metadata: &[u8],
    ) -> PortResult<OwnedEffectV1> {
        if !accepts(intent) || metadata.len() > 64 * 1024 {
            return Err(port(PortErrorCode::InvalidData, "native.reference.intent"));
        }
        let path = self.native_target_path(intent.target())?;
        let bytes = read_native_file(&path, 1024 * 1024)
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.reference.read"))?;
        let snapshot = read_artifact(&path)
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.reference.snapshot"))?;
        if bytes.as_deref().map(|b| b.as_slice()) != expected
            || snapshot.as_ref().map(|s| s.bytes.as_slice()) != expected
            || snapshot.as_ref().map(|s| &s.fingerprint) != intent.before_fingerprint()
        {
            return Err(port(PortErrorCode::Conflict, "native.reference.changed"));
        }
        let record = NativeFileReferenceRecordV1 {
            schema: NATIVE_FILE_REFERENCE_SCHEMA_V1.into(),
            fingerprint: snapshot.map(|s| s.fingerprint),
            metadata: metadata.to_vec(),
        };
        let encoded = zeroize::Zeroizing::new(
            serde_json::to_vec(&record)
                .map_err(|_| port(PortErrorCode::InvalidData, "native.reference.encode"))?,
        );
        self.save_native_restore(operation, intent, &encoded)?;
        match self.observe_native_file_reference(operation, intent)? {
            Some(EffectReconciliation::Applied(effect)) => Ok(effect),
            _ => Err(port(PortErrorCode::Conflict, "native.reference.changed")),
        }
    }

    pub(super) fn authenticate_native_file_reference_effect(
        &self,
        value: &OwnedEffectV1,
    ) -> PortResult<bool> {
        if value.compensation["schema"] != NATIVE_FILE_REFERENCE_SCHEMA_V1 {
            return Ok(false);
        }
        let id = value.compensation["operation_id"]
            .as_str()
            .ok_or_else(|| port(PortErrorCode::InvalidData, "native.reference.operation"))?;
        let operation = OperationId::parse(id.to_owned())
            .map_err(|_| port(PortErrorCode::InvalidData, "native.reference.operation"))?;
        let intent = ExternalEffectIntentV1::from_registered_adapter(
            value.effect_id.clone(),
            value.kind,
            value.target.clone(),
            value.before_fingerprint.clone(),
            value.compensation["desired"].clone(),
            0o644,
            false,
        )
        .map_err(|_| port(PortErrorCode::InvalidData, "native.reference.effect"))?;
        if !accepts(&intent) || value.compensation["effect_id"] != value.effect_id {
            return Err(port(PortErrorCode::InvalidData, "native.reference.effect"));
        }
        let bytes = self
            .load_native_restore(&operation, &intent)?
            .ok_or_else(|| port(PortErrorCode::Corrupt, "native.reference.proof"))?;
        let record: NativeFileReferenceRecordV1 = serde_json::from_slice(&bytes)
            .map_err(|_| port(PortErrorCode::Corrupt, "native.reference.proof"))?;
        if record.schema != NATIVE_FILE_REFERENCE_SCHEMA_V1
            || record.metadata.len() > 64 * 1024
            || effect(&operation, &intent, record.fingerprint) != *value
        {
            return Err(port(PortErrorCode::Corrupt, "native.reference.binding"));
        }
        Ok(true)
    }
}
