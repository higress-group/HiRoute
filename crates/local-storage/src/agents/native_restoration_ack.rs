//! Protected proof of user cleanup when restoration failed before any file was staged.
use super::native_artifacts::read_native_file;
use super::*;
use hiroute_domain::NativeAgentArtifactPort;

pub(super) const ACK_SCHEMA: &str = "hiroute.native-restoration-ack/v1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestorationAck {
    schema: String,
    fingerprint: Option<CanonicalDigest>,
}

fn accepts_ack(intent: &ExternalEffectIntentV1) -> bool {
    hiroute_domain::is_settings_managed_configuration(intent)
        && intent.desired()["payload"]["schema"] == "hiroute.settings-codex-model-file/v1"
        && intent.desired()["payload"]["change"]["action"] == "restore"
}

impl ManagedArtifactStore {
    fn unstaged_ack_effect(
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        fingerprint: Option<CanonicalDigest>,
    ) -> OwnedEffectV1 {
        OwnedEffectV1 {
            effect_id: intent.effect_id().into(),
            kind: intent.kind(),
            target: intent.target().into(),
            before_fingerprint: intent.before_fingerprint().cloned(),
            after_fingerprint: fingerprint,
            compensation:
                json!({"schema":ACK_SCHEMA,"operation_id":operation,"effect_id":intent.effect_id()})
                    .into(),
        }
    }

    pub(super) fn observe_unstaged_native_restoration(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<EffectReconciliation>> {
        if !accepts_ack(intent) {
            return Ok(None);
        }
        let Some(record) = self.load_native_restore(operation, intent)? else {
            return Ok(None);
        };
        let ack: RestorationAck = serde_json::from_slice(&record)
            .map_err(|_| port(PortErrorCode::Corrupt, "native.restoration.ack.decode"))?;
        if ack.schema != ACK_SCHEMA {
            return Err(port(
                PortErrorCode::Corrupt,
                "native.restoration.ack.schema",
            ));
        }
        let target = self.native_target_path(intent.target())?;
        let safe = read_native_file(&target, 1024 * 1024).map_err(|_| {
            port(
                PortErrorCode::PermissionDenied,
                "native.restoration.ack.read",
            )
        })?;
        let snapshot = read_artifact(&target).map_err(|_| {
            port(
                PortErrorCode::PermissionDenied,
                "native.restoration.ack.snapshot",
            )
        })?;
        if safe.as_deref().map(|b| b.as_slice()) != snapshot.as_ref().map(|s| s.bytes.as_slice()) {
            return Err(port(
                PortErrorCode::Conflict,
                "native.restoration.ack.changed",
            ));
        }
        let current = snapshot.map(|s| s.fingerprint);
        let effect = Self::unstaged_ack_effect(operation, intent, ack.fingerprint.clone());
        Ok(Some(if current == ack.fingerprint {
            EffectReconciliation::Applied(effect)
        } else {
            EffectReconciliation::OwnershipLost(effect)
        }))
    }

    pub(super) fn acknowledge_unstaged_native_restoration(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        expected: Option<&[u8]>,
    ) -> PortResult<OwnedEffectV1> {
        if !accepts_ack(intent) {
            return Err(port(
                PortErrorCode::InvalidData,
                "native.restoration.ack.intent",
            ));
        }
        let target = self.native_target_path(intent.target())?;
        let current = read_native_file(&target, 1024 * 1024).map_err(|_| {
            port(
                PortErrorCode::PermissionDenied,
                "native.restoration.ack.read",
            )
        })?;
        let snapshot = read_artifact(&target).map_err(|_| {
            port(
                PortErrorCode::PermissionDenied,
                "native.restoration.ack.snapshot",
            )
        })?;
        if current.as_deref().map(|b| b.as_slice()) != expected
            || snapshot.as_ref().map(|s| s.bytes.as_slice()) != expected
        {
            return Err(port(
                PortErrorCode::Conflict,
                "native.restoration.ack.changed",
            ));
        }
        let fingerprint = snapshot.map(|s| s.fingerprint);
        let encoded = serde_json::to_vec(&RestorationAck {
            schema: ACK_SCHEMA.into(),
            fingerprint: fingerprint.clone(),
        })
        .map_err(|_| port(PortErrorCode::InvalidData, "native.restoration.ack.encode"))?;
        // Existing encryption authenticates the exact Operation, intent, target and store key.
        self.save_native_restore(operation, intent, &encoded)?;
        match self.observe_unstaged_native_restoration(operation, intent)? {
            Some(EffectReconciliation::Applied(effect)) => Ok(effect),
            _ => Err(port(
                PortErrorCode::Conflict,
                "native.restoration.ack.changed",
            )),
        }
    }
}
