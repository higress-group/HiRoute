//! Authenticate only the requested restore marker; legacy conversion stays on use.
use super::*;

impl ManagedArtifactStore {
    pub(super) fn load_marker_compatible(
        &self,
        path: &Path,
    ) -> Result<Option<ArtifactMarker>, LocalStorageError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        validate_owner_regular_file(&metadata).map_err(|_| LocalStorageError::Locked)?;
        #[cfg(windows)]
        let bytes = read_private_restore_file(path)?;
        #[cfg(not(windows))]
        let bytes = fs::read(path)?;
        let encoded: Value =
            serde_json::from_slice(&bytes).map_err(|_| LocalStorageError::Locked)?;
        let has_backup_aad = encoded.get("backup_aad").is_some();
        let mut marker: ArtifactMarker =
            serde_json::from_value(encoded).map_err(|_| LocalStorageError::Locked)?;
        let operation = OperationId::parse(marker.operation_id.clone())
            .map_err(|_| LocalStorageError::Locked)?;
        if path != self.marker_path(&operation, &marker.effect_id)
            || marker.restore_store_uuid != self.restore_store_uuid
            || marker.restore_key_id != self.restore_key_id
            || marker.external_path_digest != self.external_path_digest(&marker.target)?
        {
            return Err(LocalStorageError::Locked);
        }
        if self
            .external_targets
            .read()
            .map_err(|_| LocalStorageError::InvalidData)?
            .contains_key(&marker.target)
        {
            let _ = self.target_path(&marker.target)?;
        }
        let changed = match marker.schema.as_str() {
            "hiroute.managed-artifact-marker/v2" => {
                marker.backup_aad = if marker.backup_name.is_some() {
                    ArtifactBackupAad::LegacyV2
                } else {
                    ArtifactBackupAad::CurrentV4
                };
                true
            }
            "hiroute.managed-artifact-marker/v3" => {
                marker.backup_aad = if marker.backup_name.is_some() {
                    ArtifactBackupAad::LegacyV3
                } else {
                    ArtifactBackupAad::CurrentV4
                };
                true
            }
            #[cfg(windows)]
            "hiroute.windows-managed-artifact-marker/v5" if has_backup_aad => false,
            "hiroute.managed-artifact-marker/v4" if has_backup_aad => false,
            _ => return Err(LocalStorageError::Locked),
        };
        if changed {
            marker.schema = "hiroute.managed-artifact-marker/v4".to_owned();
        }
        if marker.schema != current_artifact_schema() {
            return Err(LocalStorageError::Locked);
        }
        self.validate_marker_shape(&marker)
            .map_err(|_| LocalStorageError::Locked)?;
        if changed {
            // Validate the original AAD and exact backup before replacing metadata.
            self.validated_backup(&marker)
                .map_err(|_| LocalStorageError::Locked)?;
            self.save_marker(&marker)
                .map_err(|_| LocalStorageError::Locked)?;
        }
        Ok(Some(marker))
    }
}
