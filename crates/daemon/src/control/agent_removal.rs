//! Shared Linux Agent mutation admission against exclusive standalone teardown.

use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};

pub(crate) fn try_lock_shared(path: &Path) -> io::Result<File> {
    let parent = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let directory = fs::symlink_metadata(parent)?;
    let owner = nix::unistd::geteuid().as_raw();
    if !directory.is_dir() || directory.uid() != owner || directory.mode() & 0o022 != 0 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.mode() & 0o7777 != 0o600
        || metadata.nlink() != 1
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    FileExt::try_lock_shared(&file)?;
    let current = fs::symlink_metadata(path)?;
    let current_directory = fs::symlink_metadata(parent)?;
    if (current.dev(), current.ino()) != (metadata.dev(), metadata.ino())
        || (current_directory.dev(), current_directory.ino()) != (directory.dev(), directory.ino())
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{LOCAL_CONTROL_SCHEMA_V2, LocalControlDaemon};
    use hiroute_application::ApplicationService;
    use hiroute_application_api::{ErrorCode, LocalControlWireRequestV2};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn shared_mutation_excludes_teardown_and_teardown_refuses_both_apply_entries() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.path().join("agent-removal.lock");
        let active_write = try_lock_shared(&path).unwrap();
        let shared_alias = active_write.try_clone().unwrap();
        let teardown = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(FileExt::try_lock_exclusive(&teardown).is_err());
        // Parallel process tests can inherit a descriptor before exec. Keep
        // aliases alive so this fixture cannot rely on a last-close unlock.
        FileExt::unlock(&active_write).unwrap();
        drop(active_write);
        FileExt::try_lock_exclusive(&teardown).unwrap();
        drop(shared_alias);
        let teardown_alias = teardown.try_clone().unwrap();

        let daemon = LocalControlDaemon::new(ApplicationService::default())
            .with_agent_removal_lock(Some(path));
        let call = |operation: &str| {
            daemon
                .dispatch_wire(LocalControlWireRequestV2 {
                    schema_version: LOCAL_CONTROL_SCHEMA_V2,
                    request_id: "agent-removal-lock-test".into(),
                    operation_id: operation.into(),
                    payload: serde_json::Value::Null,
                    protected_grant: None,
                })
                .error
                .unwrap()
                .code
        };
        assert_eq!(
            call("ApplyAgentConnectionChange"),
            ErrorCode::DaemonUnavailable
        );
        assert_eq!(
            call("ApplyAgentConnectionRestore"),
            ErrorCode::DaemonUnavailable
        );
        assert_eq!(
            call("PreviewAgentConnectionRestore"),
            ErrorCode::InvalidArguments
        );
        FileExt::unlock(&teardown).unwrap();
        drop(teardown);
        assert_eq!(
            call("ApplyAgentConnectionRestore"),
            ErrorCode::InvalidArguments
        );
        drop(teardown_alias);
    }

    #[test]
    fn lock_refuses_a_foreign_link_without_altering_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("foreign");
        fs::write(&target, "untouched").unwrap();
        let path = dir.path().join("agent-removal.lock");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(try_lock_shared(&path).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
    }
}
