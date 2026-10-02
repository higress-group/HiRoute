//! Protected native restore metadata using the EXISTING artifact key and Operation identity.
use super::*;
use hiroute_domain::NativeAgentArtifactPort;

const MAX_NATIVE_BYTES: usize = 1024 * 1024;
impl NativeAgentArtifactPort for ManagedArtifactStore {
    fn observe_artifact(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<hiroute_domain::EffectReconciliation> {
        ManagedArtifactStore::observe_artifact(self, operation, intent)
    }
    fn acknowledge_native_restoration(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        expected: Option<&[u8]>,
    ) -> PortResult<OwnedEffectV1> {
        // Observe authenticates the marker, intent binding and backup before any acknowledgment.
        if !matches!(
            self.observe_artifact(operation, intent)?,
            hiroute_domain::EffectReconciliation::OwnershipLost(_)
        ) {
            return Err(port(PortErrorCode::Conflict, "native.restoration.state"));
        }
        let mut marker = self
            .load_marker(operation, intent.effect_id())
            .map_err(|_| port(PortErrorCode::Corrupt, "native.restoration.marker"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "native.restoration.marker"))?;
        if marker.compensated
            || !marker.rendered
            || marker.kind != OwnedEffectKind::AgentArtifact
            || marker.after_mode != intent.desired_mode()
            || marker.target != intent.target()
            || marker.before_digest.as_ref() != intent.before_fingerprint()
            || marker.intent_digest.as_ref()
                != Some(
                    &hiroute_domain::CanonicalDigest::of(intent.desired()).map_err(|_| {
                        port(PortErrorCode::InvalidData, "native.restoration.intent")
                    })?,
                )
        {
            return Err(port(PortErrorCode::Conflict, "native.restoration.binding"));
        }
        self.validated_backup(&marker)
            .map_err(|_| port(PortErrorCode::Corrupt, "native.restoration.backup"))?;
        let path = self
            .target_path(intent.target())
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.restoration.target"))?;
        let current = read_native_file(&path, MAX_NATIVE_BYTES)
            .map_err(|_| port(PortErrorCode::Conflict, "native.restoration.file"))?;
        if current.as_deref().map(|b| b.as_slice()) != expected {
            return Err(port(PortErrorCode::Conflict, "native.restoration.changed"));
        }
        let snapshot = read_artifact(&path)
            .map_err(|_| port(PortErrorCode::Conflict, "native.restoration.snapshot"))?;
        if snapshot.as_ref().map(|s| s.bytes.as_slice()) != expected {
            return Err(port(PortErrorCode::Conflict, "native.restoration.changed"));
        }
        marker.native_restoration = Some(NativeRestorationReceipt {
            fingerprint: snapshot.map(|s| s.fingerprint),
        });
        marker.activated = true;
        self.save_marker(&marker)
            .map_err(|_| port(PortErrorCode::Unavailable, "native.restoration.persist"))?;
        Ok(Self::effect(&marker))
    }

    fn stage_native_skill_target(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        bytes: Option<&[u8]>,
        previous: Option<&hiroute_domain::SkillFileEffectRef>,
    ) -> PortResult<OwnedEffectV1> {
        hiroute_domain::SkillFileEffectRef::from_intent(operation, intent)
            .map_err(|_| port(PortErrorCode::InvalidData, "native.skill.intent"))?;
        if bytes.is_some_and(|bytes| bytes.len() > MAX_NATIVE_BYTES) {
            return Err(port(PortErrorCode::InvalidData, "native.skill.size"));
        }
        let parents = if let Some(previous) = previous {
            previous
                .validate()
                .map_err(|_| port(PortErrorCode::InvalidData, "native.skill.previous"))?;
            if previous.target != intent.target() || previous.operation_id == *operation {
                return Err(port(PortErrorCode::Conflict, "native.skill.lineage"));
            }
            let marker = self
                .load_marker(&previous.operation_id, &previous.effect_id)
                .map_err(|_| port(PortErrorCode::Corrupt, "native.skill.marker"))?
                .ok_or_else(|| port(PortErrorCode::NotFound, "native.skill.marker"))?;
            if !marker.activated
                || !marker.rendered
                || marker.sensitive
                || marker.after_mode != 0o644
                || marker.kind != OwnedEffectKind::AgentArtifact
                || marker.target != previous.target
                || marker.intent_digest.as_ref() != Some(&previous.intent_digest)
                || marker.external_path_digest
                    != self.external_path_digest(intent.target()).map_err(|_| {
                        port(PortErrorCode::PermissionDenied, "native.skill.path.binding")
                    })?
            {
                return Err(port(PortErrorCode::Conflict, "native.skill.marker.binding"));
            }
            marker.created_directories
        } else {
            Vec::new()
        };
        self.apply_external_bytes_with_parents(
            operation,
            intent,
            bytes.unwrap_or_default(),
            true,
            false,
            bytes.is_some(),
            parents,
        )
    }

    fn cleanup_native_parents(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<bool> {
        let marker = self
            .load_marker(operation, intent.effect_id())
            .map_err(|_| port(PortErrorCode::Corrupt, "native.parents.marker"))?
            .ok_or_else(|| port(PortErrorCode::NotFound, "native.parents.marker"))?;
        if (!marker.activated && !marker.compensated)
            || marker.kind != OwnedEffectKind::AgentArtifact
            || !marker.rendered
            || marker.target != intent.target()
            || marker.before_digest.as_ref() != intent.before_fingerprint()
            || marker.after_mode != intent.desired_mode()
            || marker.intent_digest.as_ref()
                != Some(
                    &CanonicalDigest::of(intent.desired())
                        .map_err(|_| port(PortErrorCode::InvalidData, "native.parents.intent"))?,
                )
        {
            return Err(port(PortErrorCode::Conflict, "native.parents.binding"));
        }
        let target = self
            .target_path(intent.target())
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.parents.target"))?;
        native_directories::cleanup(&target, &marker.created_directories)
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.parents.cleanup"))
    }
    fn read_native_target(&self, target: &str) -> PortResult<Option<Zeroizing<Vec<u8>>>> {
        let path = self
            .target_path(target)
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.target"))?;
        let bytes = read_native_file(&path, MAX_NATIVE_BYTES)
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.read"))?;
        if bytes
            .as_ref()
            .is_some_and(|bytes| bytes.len() > MAX_NATIVE_BYTES)
        {
            return Err(port(PortErrorCode::InvalidData, "native.size"));
        }
        Ok(bytes)
    }
    fn native_target_path(&self, target: &str) -> PortResult<std::path::PathBuf> {
        self.target_path(target)
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.target.path"))
    }
    fn save_native_restore(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        bytes: &[u8],
    ) -> PortResult<()> {
        if bytes.len() > MAX_NATIVE_BYTES || intent.kind() != OwnedEffectKind::AgentArtifact {
            return Err(port(PortErrorCode::InvalidData, "native.restore.intent"));
        }
        if let Some(existing) = self.load_native_restore(operation, intent)? {
            return if existing.as_slice() == bytes {
                Ok(())
            } else {
                Err(port(PortErrorCode::Conflict, "native.restore.replay"))
            };
        }
        let aad = self.native_restore_aad(operation, intent)?;
        let mut nonce = [0u8; RESTORE_NONCE_BYTES];
        getrandom::fill(&mut nonce)
            .map_err(|_| port(PortErrorCode::Crypto, "native.restore.entropy"))?;
        let cipher = Aes256Gcm::new_from_slice(&self.restore_key)
            .map_err(|_| port(PortErrorCode::Crypto, "native.restore.key"))?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: &aad,
                },
            )
            .map_err(|_| port(PortErrorCode::Crypto, "native.restore.encrypt"))?;
        let mut encoded = nonce.to_vec();
        encoded.extend_from_slice(&ciphertext);
        atomic_write(
            &self.native_restore_path(operation, intent),
            &encoded,
            0o600,
            true,
        )
        .map_err(|_| port(PortErrorCode::Unavailable, "native.restore.write"))
    }
    fn load_native_restore(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<Zeroizing<Vec<u8>>>> {
        let aad = self.native_restore_aad(operation, intent)?;
        let Some(encoded) = read_native_file(
            &self.native_restore_path(operation, intent),
            MAX_NATIVE_BYTES + 64,
        )
        .map_err(|_| port(PortErrorCode::Corrupt, "native.restore.read"))?
        else {
            return Ok(None);
        };
        if encoded.len() <= RESTORE_NONCE_BYTES || encoded.len() > MAX_NATIVE_BYTES + 64 {
            return Err(port(PortErrorCode::Corrupt, "native.restore.size"));
        }
        let cipher = Aes256Gcm::new_from_slice(&self.restore_key)
            .map_err(|_| port(PortErrorCode::Crypto, "native.restore.key"))?;
        cipher
            .decrypt(
                Nonce::from_slice(&encoded[..RESTORE_NONCE_BYTES]),
                Payload {
                    msg: &encoded[RESTORE_NONCE_BYTES..],
                    aad: &aad,
                },
            )
            .map(|bytes| Some(Zeroizing::new(bytes)))
            .map_err(|_| port(PortErrorCode::Corrupt, "native.restore.authenticate"))
    }
    fn stage_native_target(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        bytes: Option<&[u8]>,
        sensitive: bool,
    ) -> PortResult<OwnedEffectV1> {
        if intent.kind() != OwnedEffectKind::AgentArtifact
            || bytes.is_some_and(|bytes| bytes.len() > MAX_NATIVE_BYTES)
        {
            return Err(port(PortErrorCode::InvalidData, "native.stage.intent"));
        }
        self.apply_external_bytes(
            operation,
            intent,
            bytes.unwrap_or_default(),
            true,
            sensitive,
            bytes.is_some(),
        )
    }
}

/// Bound allocation before reading and check the opened inode, not a following path lookup.
pub(super) fn read_native_file(
    path: &Path,
    limit: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>, LocalStorageError> {
    #[cfg(not(unix))]
    {
        let _ = (path, limit);
        Err(LocalStorageError::Permission)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(
                (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
            )
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let before = file.metadata()?;
        if !before.is_file()
            || before.nlink() != 1
            || before.uid() != rustix::process::getuid().as_raw()
            || !supported_artifact_mode(before.mode() & 0o777)
            || before.len() > limit as u64
        {
            return Err(LocalStorageError::Permission);
        }
        let mut bytes = Zeroizing::new(Vec::new());
        (&file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
        let identity = |m: &fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mode(),
                m.uid(),
                m.gid(),
                m.nlink(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        if bytes.len() > limit
            || identity(&before) != identity(&file.metadata()?)
            || identity(&before) != identity(&fs::symlink_metadata(path)?)
        {
            return Err(LocalStorageError::Permission);
        }
        Ok(Some(bytes))
    }
}
impl ManagedArtifactStore {
    fn native_restore_path(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PathBuf {
        self.restore_root.join(format!(
            "{}.native-restore",
            Self::marker_key(operation, intent.effect_id())
        ))
    }
    fn native_restore_aad(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Vec<u8>> {
        self.target_path(intent.target())
            .map_err(|_| port(PortErrorCode::PermissionDenied, "native.restore.target"))?;
        serde_json::to_vec(&(
            "hiroute.native-field-restore/v1",
            &self.restore_store_uuid,
            &self.restore_key_id,
            operation,
            intent,
            self.external_path_digest(intent.target())
                .map_err(|_| port(PortErrorCode::InvalidData, "native.restore.binding"))?,
        ))
        .map_err(|_| port(PortErrorCode::InvalidData, "native.restore.aad"))
    }
}
