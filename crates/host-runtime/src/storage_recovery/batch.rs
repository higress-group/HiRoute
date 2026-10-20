//! Revoke derived UI advice when its original batch or essential material changed.
//! The storage owner still performs full admission before resume or restoration.
use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Component;

fn invalid() -> io::Error {
    io::Error::other("STORAGE_RECOVERY_STALE")
}
fn json(path: &Path, limit: u64) -> io::Result<Value> {
    let mut file = private_fs::open_private_file(path)?;
    private_fs::verify_single_link(&file)?;
    let mut bytes = Vec::new();
    file.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
fn portable(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
pub(super) fn validate(storage: &Path, record: &StorageRecovery) -> io::Result<()> {
    let id = record
        .backup_batch
        .as_deref()
        .filter(|s| portable(s))
        .ok_or_else(invalid)?;
    let name = record
        .backup_name
        .as_deref()
        .filter(|s| portable(s))
        .ok_or_else(invalid)?;
    let link = json(&storage.join("observation-source-backup.json"), 16 * 1024)?;
    let linked_name = link["backup_name"]
        .as_str()
        .filter(|s| portable(s))
        .ok_or_else(invalid)?;
    if link["schema"] != 1
        || link["set_id"] != id
        || (name != linked_name && name != format!("migration-set.{id}"))
    {
        return Err(invalid());
    }
    let mut backup = storage.file_name().ok_or_else(invalid)?.to_os_string();
    backup.push(".upgrade-backups");
    let batch = storage
        .parent()
        .ok_or_else(invalid)?
        .join(backup)
        .join(name);
    let _batch =
        hiroute_diagnostics::files::PrivateDir::open_existing(&batch).map_err(|_| invalid())?;
    let metadata = json(&batch.join("backup-set.manifest.json"), 64 * 1024)?;
    if metadata["schema"] != "hiroute.sqlite-backup-set/v2" || metadata["set_id"] != id {
        return Err(invalid());
    }
    let source = batch.join("source");
    let _source =
        hiroute_diagnostics::files::PrivateDir::open_existing(&source).map_err(|_| invalid())?;
    let manifest = json(&source.join("source-backup.json"), 1024 * 1024)?;
    if manifest["schema"] != "hiroute.upgrade-source-backup/v2"
        || manifest["complete"] != true
        || manifest["source_info"]["batch_id"] != id
    {
        return Err(invalid());
    }
    // Check only the explicit copied-material inventory. Historical Observation
    // content is intentionally absent from this manifest and never traversed.
    let files = manifest["files"]
        .as_object()
        .filter(|f| !f.is_empty() && f.len() <= 10_000)
        .ok_or_else(invalid)?;
    for (name, expected) in files {
        if name.contains(['\\', ':'])
            || !Path::new(name)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(invalid());
        }
        let path = source.join(name);
        let _parent = hiroute_diagnostics::files::PrivateDir::open_existing(
            path.parent().ok_or_else(invalid)?,
        )
        .map_err(|_| invalid())?;
        let mut file = private_fs::open_private_file(&path)?;
        private_fs::verify_single_link(&file)?;
        let expected = expected.as_str().ok_or_else(invalid)?;
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if format!("sha256:{:x}", digest.finalize()) != expected {
            return Err(invalid());
        }
    }
    for name in [
        "storage/live/control.db",
        "storage/live/runtime.db",
        "storage/live/secrets.db",
        "storage/master-key",
        "storage/observation-hmac-key",
    ] {
        if !files.contains_key(name) {
            return Err(invalid());
        }
    }
    if !files
        .keys()
        .any(|name| name.starts_with("recovery-package/"))
    {
        return Err(invalid());
    }
    let history = source.join("storage/observation");
    let _history =
        hiroute_diagnostics::files::PrivateDir::open_existing(&history).map_err(|_| invalid())?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(&history)?;
        [m.dev(), m.ino()]
    };
    #[cfg(windows)]
    let identity = {
        let f = hiroute_platform::windows::security::open_identity(&history, true)?;
        let (volume, index) = hiroute_platform::windows::security::file_identity(&f)?;
        [u64::from(volume), index]
    };
    if manifest["sealed_observation"]["identity"] != serde_json::json!(identity) {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn advice_is_bound_to_the_original_batch_and_essential_material_without_history_scan() {
        let temp = tempfile::tempdir().unwrap();
        let storage = temp.path().join("storage");
        let batch = temp.path().join("storage.upgrade-backups/migration-set");
        let source = batch.join("source");
        let history = source.join("storage/observation");
        for dir in [
            &storage,
            &history,
            &source.join("storage/live"),
            &source.join("recovery-package"),
        ] {
            private_fs::prepare_storage_directory(dir).unwrap();
        }
        let mut files = serde_json::Map::new();
        for name in [
            "storage/live/control.db",
            "storage/live/runtime.db",
            "storage/live/secrets.db",
            "storage/master-key",
            "storage/observation-hmac-key",
            "recovery-package/hirouted",
        ] {
            durable::write_private_new(&source.join(name), b"retained source").unwrap();
            files.insert(
                name.into(),
                serde_json::json!(format!("sha256:{:x}", Sha256::digest(b"retained source"))),
            );
        }
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            let m = std::fs::symlink_metadata(&history).unwrap();
            [m.dev(), m.ino()]
        };
        #[cfg(windows)]
        let identity = {
            let f = hiroute_platform::windows::security::open_identity(&history, true).unwrap();
            let (volume, index) = hiroute_platform::windows::security::file_identity(&f).unwrap();
            [u64::from(volume), index]
        };
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            temp.path().join("missing-history-target"),
            history.join("never-follow"),
        )
        .unwrap();
        let write = |path: &Path, value: Value| {
            durable::write_private_atomic(path, &serde_json::to_vec(&value).unwrap()).unwrap()
        };
        write(
            &storage.join("observation-source-backup.json"),
            serde_json::json!({"schema":1,"set_id":"set_original","backup_name":"migration-set"}),
        );
        write(
            &batch.join("backup-set.manifest.json"),
            serde_json::json!({"schema":"hiroute.sqlite-backup-set/v2","set_id":"set_original"}),
        );
        write(
            &source.join("source-backup.json"),
            serde_json::json!({"schema":"hiroute.upgrade-source-backup/v2","complete":true,"source_info":{"batch_id":"set_original"},"files":files,"sealed_observation":{"identity":identity}}),
        );
        let mut report = StorageRecovery::new(
            StorageRecoveryCause::MigrationFailed,
            StorageRecoveryAction::ResumeCurrent,
        );
        report.backup_batch = Some("set_original".into());
        report.backup_name = Some("migration-set".into());
        report.store(&storage).unwrap();
        assert_eq!(
            StorageRecovery::read(&storage).unwrap(),
            Some(report.clone())
        );
        use hiroute_diagnostics::event::StartupFailureCode as Code;
        for (code, cause) in [
            (
                Code::StorageKeyUnavailable,
                StorageRecoveryCause::MissingKey,
            ),
            (
                Code::UpgradeSourceUnsupported,
                StorageRecoveryCause::UnsupportedFormat,
            ),
        ] {
            let mut admitted = report.clone();
            admitted.cause = cause;
            admitted.action = StorageRecoveryAction::RestoreCompleteGroup;
            admitted.store(&storage).unwrap();
            assert_eq!(
                StorageRecovery::read_for_admission_failure(code, Some(&storage)),
                Some(admitted.clone())
            );
            let prerequisite =
                StorageRecovery::read_for_admission_failure(Code::StorageLocked, Some(&storage))
                    .unwrap();
            assert_eq!(prerequisite.action, StorageRecoveryAction::RetryCurrent);
            assert!(
                prerequisite.backup_batch.is_none(),
                "file use cannot borrow a prior restore target"
            );
            admitted.action = StorageRecoveryAction::ResumeCurrent;
            admitted.store(&storage).unwrap();
            assert_eq!(
                StorageRecovery::read_for_admission_failure(code, Some(&storage))
                    .unwrap()
                    .action,
                StorageRecoveryAction::PreserveAndDiagnose,
                "a diagnostic cannot grant resume"
            );
            admitted.action = StorageRecoveryAction::RestoreCompleteGroup;
            admitted.store(&storage).unwrap();
            std::fs::write(source.join("storage/master-key"), b"corrupted value").unwrap();
            assert_eq!(
                StorageRecovery::read_for_admission_failure(code, Some(&storage))
                    .unwrap()
                    .action,
                StorageRecoveryAction::PreserveAndDiagnose,
                "damaged retained material cannot grant restoration"
            );
            std::fs::write(source.join("storage/master-key"), b"retained source").unwrap();
        }
        report.store(&storage).unwrap();
        // An extant file or unchanged length is insufficient: corruption of a
        // retained key must revoke the otherwise correct batch's restore advice.
        std::fs::write(source.join("storage/master-key"), b"corrupted value").unwrap();
        assert!(StorageRecovery::read(&storage).is_err());
        std::fs::write(source.join("storage/master-key"), b"retained source").unwrap();
        assert_eq!(StorageRecovery::read(&storage).unwrap(), Some(report));
        write(
            &storage.join("observation-source-backup.json"),
            serde_json::json!({"schema":1,"set_id":"set_foreign","backup_name":"migration-set"}),
        );
        assert!(StorageRecovery::read(&storage).is_err());
        write(
            &storage.join("observation-source-backup.json"),
            serde_json::json!({"schema":1,"set_id":"set_original","backup_name":"migration-set"}),
        );
        std::fs::remove_file(source.join("storage/master-key")).unwrap();
        assert!(StorageRecovery::read(&storage).is_err());
    }
}
