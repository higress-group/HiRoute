//! Bounded startup recovery facts. Only the storage owner assigns an action.
use hiroute_platform::{durable, private_fs};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read},
    path::Path,
};

pub const STORAGE_RECOVERY_SCHEMA: &str = "hiroute.storage-startup-recovery/v1";
pub const STORAGE_RECOVERY_FILE: &str = "startup-recovery.json";
mod admission;
pub use admission::{StartupFailureBoundary, startup_failure_host_code};
mod batch;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecoverySpace {
    pub required_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StorageRecoveryCause {
    MissingPackage,
    InvalidPackage,
    InsufficientSpace,
    AccessDenied,
    Locked,
    MissingKey,
    UnsupportedFormat,
    NamespaceUnavailable,
    MigrationFailed,
    UnknownState,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StorageRecoveryAction {
    RetryCurrent,
    ResumeCurrent,
    RestoreCompleteGroup,
    PreserveAndDiagnose,
}
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StorageRecovery {
    pub schema: String,
    pub cause: StorageRecoveryCause,
    pub action: StorageRecoveryAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_batch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_space: Option<RecoverySpace>,
}
impl StorageRecovery {
    pub fn new(cause: StorageRecoveryCause, action: StorageRecoveryAction) -> Self {
        Self {
            schema: STORAGE_RECOVERY_SCHEMA.into(),
            cause,
            action,
            backup_batch: None,
            backup_name: None,
            storage_space: None,
        }
    }
    pub fn read(storage: &Path) -> io::Result<Option<Self>> {
        let mut file = match private_fs::open_private_file(&storage.join(STORAGE_RECOVERY_FILE)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        private_fs::verify_single_link(&file)?;
        let mut bytes = Vec::new();
        file.by_ref().take(4097).read_to_end(&mut bytes)?;
        let record: Self = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if bytes.len() > 4096 || record.schema != STORAGE_RECOVERY_SCHEMA {
            return Err(io::Error::other("STORAGE_RECOVERY_INVALID"));
        }
        if matches!(
            record.action,
            StorageRecoveryAction::ResumeCurrent | StorageRecoveryAction::RestoreCompleteGroup
        ) {
            batch::validate(storage, &record)?;
        }
        Ok(Some(record))
    }
    pub fn store(&self, storage: &Path) -> io::Result<()> {
        durable::write_private_atomic(
            &storage.join(STORAGE_RECOVERY_FILE),
            &serde_json::to_vec(self)?,
        )
    }
    pub fn clear(storage: &Path) -> io::Result<()> {
        let path = storage.join(STORAGE_RECOVERY_FILE);
        match private_fs::open_private_file(&path) {
            Ok(file) => {
                private_fs::verify_single_link(&file)?;
                drop(file);
                std::fs::remove_file(path)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub fn operator_guidance(&self) -> &'static str {
        match (self.cause, self.action) {
            (_, StorageRecoveryAction::ResumeCurrent) => {
                "Keep all generations and reopen the current version to resume the identified original upgrade batch. Do not run an older program. If it cannot continue, preserve newer data separately and follow this batch's complete-group recovery guide."
            }
            (_, StorageRecoveryAction::RestoreCompleteGroup) => {
                "Stop HiRoute and preserve newer data with its matching program separately. Follow the verified original batch's RECOVERY.md or recovery guide to restore the entire old group and matching complete program. Never restore individual databases or mix generations."
            }
            (StorageRecoveryCause::InsufficientSpace, _) => {
                "Keep all data and recovery material. Free unrelated space on the storage volume, then retry the current version."
            }
            (StorageRecoveryCause::MissingPackage | StorageRecoveryCause::InvalidPackage, _) => {
                "Keep all storage generations. Supply the verified complete matching old package through the supported installer or HIROUTE_RECOVERY_PACKAGE for the actual daemon process. Do not launch an old program against this data group. Follow the complete-group recovery guide if current-version recovery cannot continue."
            }
            (StorageRecoveryCause::AccessDenied | StorageRecoveryCause::Locked, _) => {
                "Close other owned HiRoute instances and resolve the reported file access or ownership problem. Preserve all data and retry the current version. Do not change permissions recursively or delete recovery material."
            }
            (StorageRecoveryCause::NamespaceUnavailable, _) => {
                "Keep all data and recovery material. The original history and recovery destination must be on the same filesystem and permit an atomic directory move. Correct the recorded locations or file-use problem before retrying the current version; do not copy or delete history to bypass this check."
            }
            _ => {
                "Keep all storage generations, keys, packages and journals. Do not downgrade or delete data. Inspect startup diagnostics and the matching complete-group recovery guide; preserve newer data separately before restoration. Missing keys or source material may require support and cannot be repaired by repeated retries."
            }
        }
    }
}

/// Installation rollback is insufficient evidence of storage downgrade safety.
/// Refuse conservatively if any storage-upgrade authority is present or unreadable.
pub fn storage_upgrade_started(storage: &Path) -> io::Result<bool> {
    for name in [
        "observation-source-backup.json",
        "observation.upgrade.json",
        "observation.pre-v5-source",
        "observation.v5-target",
        "migration-set",
    ] {
        match std::fs::symlink_metadata(storage.join(name)) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let Some(parent) = storage.parent() else {
        return Err(io::Error::other("STORAGE_ROOT_INVALID"));
    };
    let Some(name) = storage.file_name() else {
        return Err(io::Error::other("STORAGE_ROOT_INVALID"));
    };
    let mut backup = name.to_os_string();
    backup.push(".upgrade-backups");
    let backup = parent.join(backup);
    let metadata = match std::fs::symlink_metadata(&backup) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    if let Some(metadata) = metadata {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(std::io::Error::other("STORAGE_BACKUP_ROOT_INVALID"));
        }
        // Unrecognized recovery material also prevents an unsafe program rollback.
        if std::fs::read_dir(backup)?.next().is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retired_program_only_action_and_fields_are_not_a_recovery_contract() {
        let mut value = serde_json::to_value(StorageRecovery::new(
            StorageRecoveryCause::MissingPackage,
            StorageRecoveryAction::PreserveAndDiagnose,
        ))
        .unwrap();
        value["action"] = serde_json::json!("reinstall_original");
        assert!(serde_json::from_value::<StorageRecovery>(value.clone()).is_err());
        value["action"] = serde_json::json!("preserve_and_diagnose");
        value["original_state_digest"] = serde_json::json!("unproduced proof");
        assert!(serde_json::from_value::<StorageRecovery>(value).is_err());
    }
    #[test]
    fn unknown_recovery_cannot_authorize_downgrade() {
        for field in ["cause", "action"] {
            let mut value = serde_json::to_value(StorageRecovery::new(
                StorageRecoveryCause::MissingPackage,
                StorageRecoveryAction::PreserveAndDiagnose,
            ))
            .unwrap();
            value[field] = serde_json::json!("future_value");
            assert!(serde_json::from_value::<StorageRecovery>(value).is_err());
        }
    }
}
