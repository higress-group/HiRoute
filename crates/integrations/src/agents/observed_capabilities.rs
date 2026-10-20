//! Read-only filesystem evidence; native authentication is not inferred from a version label.
use hiroute_domain::{
    AgentCapability, CanonicalDigest, CapabilityEvidence, CapabilityReason, CapabilityState,
    SupportedAgentInstallationV1,
};
use std::path::Path;

pub(super) fn attach_file_capabilities(
    installation: &mut SupportedAgentInstallationV1,
    executable: &Path,
    target: &Path,
) {
    attach_capabilities(
        installation,
        dependencies(Some(executable), target),
        "hiroute.native-files/v1",
    );
}

/// Recovery/file editing remains available when a selected native executable disappears.
pub(super) fn attach_optional_file_capabilities(
    installation: &mut SupportedAgentInstallationV1,
    executable: Option<&Path>,
    target: &Path,
) {
    attach_capabilities(
        installation,
        dependencies(executable, target),
        "hiroute.native-files/v1",
    );
}

/// Settings support is derived from the selected configuration target only. A concrete
/// executable is located separately; its inode, digest, release or version is not a save gate.
pub(super) fn attach_target_file_capabilities(
    installation: &mut SupportedAgentInstallationV1,
    target: &Path,
) {
    attach_capabilities(
        installation,
        dependencies(None, target),
        "hiroute.codex-native-files/v2",
    );
}

fn attach_capabilities(
    installation: &mut SupportedAgentInstallationV1,
    sampled: Result<(CanonicalDigest, bool), ()>,
    adapter_contract: &str,
) {
    let native_conflict = installation.capability_evidence.iter().any(|evidence| {
        evidence.capability == AgentCapability::EffectiveConfiguration
            && evidence.state == CapabilityState::Unknown
            && evidence.reason == Some(CapabilityReason::HigherPrecedenceConflict)
    });
    let Ok((dependency, replace)) = sampled else {
        installation.capability_evidence.clear();
        return;
    };
    installation.observation_digest = CanonicalDigest::of(&(
        "hiroute.native-observation/v2",
        &installation.observation_digest,
        dependency,
    ))
    .expect("bounded digest tuple is serializable");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(1)
        .max(1);
    installation.capability_evidence = [
        (
            AgentCapability::EffectiveConfiguration,
            if native_conflict {
                CapabilityState::Unknown
            } else {
                CapabilityState::Proven
            },
            native_conflict.then_some(CapabilityReason::HigherPrecedenceConflict),
        ),
        (
            AgentCapability::AtomicManagedReplace,
            if replace {
                CapabilityState::Proven
            } else {
                CapabilityState::Unknown
            },
            if replace {
                None
            } else {
                Some(CapabilityReason::UnsafeTarget)
            },
        ),
        (
            AgentCapability::IngressAuthentication,
            CapabilityState::Unknown,
            Some(CapabilityReason::NotProbed),
        ),
        (
            AgentCapability::ModelCatalog,
            CapabilityState::Unknown,
            Some(CapabilityReason::NotProbed),
        ),
        (
            AgentCapability::SkillLoading,
            CapabilityState::Unknown,
            Some(CapabilityReason::NotProbed),
        ),
        (
            AgentCapability::TrustedCliExecution,
            CapabilityState::Unknown,
            Some(CapabilityReason::NotProbed),
        ),
        (
            AgentCapability::IsolatedVerification,
            CapabilityState::Unknown,
            Some(CapabilityReason::NotProbed),
        ),
    ]
    .into_iter()
    .map(|(capability, state, reason)| CapabilityEvidence {
        capability,
        state,
        reason,
        adapter_contract: adapter_contract.into(),
        observed_at_unix_ms: now,
        dependency_digest: installation.observation_digest.clone(),
    })
    .collect();
}

#[cfg(unix)]
pub(super) fn dependencies(
    executable: Option<&Path>,
    target: &Path,
) -> Result<(CanonicalDigest, bool), ()> {
    unix_dependencies(executable, target, None)
}

/// Native Skill checks already confirm these exact bytes and validate the native read receipt.
/// A ctime-only bookkeeping change is not a new Skill; identity, permissions, links, size and
/// mtime remain dependencies. Ordinary configuration/Preview keeps its existing version tuple.
pub(super) fn content_bound_dependencies(
    executable: Option<&Path>,
    target: &Path,
    confirmed_content: &CanonicalDigest,
) -> Result<(CanonicalDigest, bool), ()> {
    #[cfg(unix)]
    return unix_dependencies(executable, target, Some(confirmed_content));
    #[cfg(not(unix))]
    {
        let _ = confirmed_content;
        dependencies(executable, target)
    }
}

#[cfg(unix)]
fn unix_dependencies(
    executable: Option<&Path>,
    target: &Path,
    confirmed_content: Option<&CanonicalDigest>,
) -> Result<(CanonicalDigest, bool), ()> {
    use std::os::unix::fs::MetadataExt;
    let file_identity = |metadata: &std::fs::Metadata| {
        (
            metadata.dev(),
            metadata.ino(),
            metadata.uid(),
            metadata.gid(),
            metadata.mode(),
            metadata.nlink(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    };
    // A sibling appearing in the directory changes its size and timestamps, but does not make
    // the already checked target unsafe. Bind the parent only to its stable identity and access
    // policy; replacement, ownership and permission changes must still invalidate Preview.
    let directory_identity = |metadata: &std::fs::Metadata| {
        (
            metadata.dev(),
            metadata.ino(),
            metadata.uid(),
            metadata.gid(),
            metadata.mode(),
            metadata.file_type().is_dir(),
            metadata.file_type().is_symlink(),
        )
    };
    let binary = executable
        .map(|executable| {
            let executable = super::executable::resolve(executable)
                .map_err(|_| ())?
                .ok_or(())?;
            let metadata = std::fs::metadata(&executable).map_err(|_| ())?;
            Ok::<_, ()>((executable, file_identity(&metadata)))
        })
        .transpose()?;
    let file = match std::fs::symlink_metadata(target) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(()),
    };
    let mut parent = target.parent().ok_or(())?;
    let mut parent_metadata = None;
    for _ in 0..32 {
        match std::fs::symlink_metadata(parent) {
            Ok(metadata) => {
                parent_metadata = Some(metadata);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                parent = parent.parent().ok_or(())?;
            }
            Err(_) => return Err(()),
        }
    }
    let parent_metadata = parent_metadata.ok_or(())?;
    let replace = parent_metadata.is_dir()
        && !parent_metadata.file_type().is_symlink()
        && file.as_ref().is_none_or(|metadata| {
            metadata.is_file() && !metadata.file_type().is_symlink() && metadata.nlink() == 1
        });
    let digest = if let Some(content) = confirmed_content {
        let skill_identity = file.as_ref().map(|metadata| {
            (
                metadata.dev(),
                metadata.ino(),
                metadata.uid(),
                metadata.gid(),
                metadata.mode(),
                metadata.nlink(),
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
            )
        });
        CanonicalDigest::of(&(
            binary,
            target,
            skill_identity,
            parent,
            directory_identity(&parent_metadata),
            content,
        ))
    } else {
        CanonicalDigest::of(&(
            binary,
            target,
            file.as_ref().map(file_identity),
            parent,
            directory_identity(&parent_metadata),
        ))
    }
    .map_err(|_| ())?;
    Ok((digest, replace))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn confirmed_skill_content_ignores_ctime_only_churn_and_retains_native_guards() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("SKILL.md");
        std::fs::write(&target, b"original Skill body\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let content = CanonicalDigest::of_bytes(b"original Skill body\n");
        let original = std::fs::metadata(&target).unwrap();
        let strict = dependencies(None, &target).unwrap();
        let confirmed = content_bound_dependencies(None, &target, &content).unwrap();
        // Reproduce the observed native read: ctime changed while bytes, inode,
        // access policy and mtime remained identical. chmod with the same policy
        // provides this metadata event without depending on a macOS background service.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let after = loop {
            std::fs::set_permissions(&target, original.permissions()).unwrap();
            let after = std::fs::metadata(&target).unwrap();
            if (original.ctime(), original.ctime_nsec()) != (after.ctime(), after.ctime_nsec()) {
                break after;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "filesystem ctime did not advance"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(original.modified().unwrap(), after.modified().unwrap());
        assert_ne!(strict.0, dependencies(None, &target).unwrap().0);
        assert_eq!(
            confirmed,
            content_bound_dependencies(None, &target, &content).unwrap()
        );

        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert_ne!(
            confirmed.0,
            content_bound_dependencies(None, &target, &content)
                .unwrap()
                .0
        );
        std::fs::set_permissions(&target, original.permissions()).unwrap();
        let parent_permissions = std::fs::metadata(root.path()).unwrap().permissions();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        assert_ne!(
            confirmed.0,
            content_bound_dependencies(None, &target, &content)
                .unwrap()
                .0
        );
        std::fs::set_permissions(root.path(), parent_permissions).unwrap();

        std::fs::write(&target, b"modified Skill body\n").unwrap();
        std::fs::File::open(&target)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap()))
            .unwrap();
        let changed = CanonicalDigest::of_bytes(b"modified Skill body\n");
        assert_ne!(
            confirmed.0,
            content_bound_dependencies(None, &target, &changed)
                .unwrap()
                .0
        );
        std::fs::rename(&target, root.path().join("previous.md")).unwrap();
        std::fs::write(&target, b"original Skill body\n").unwrap();
        std::fs::set_permissions(&target, original.permissions()).unwrap();
        std::fs::File::open(&target)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap()))
            .unwrap();
        assert_ne!(
            confirmed.0,
            content_bound_dependencies(None, &target, &content)
                .unwrap()
                .0
        );
    }

    #[test]
    fn parent_sibling_churn_does_not_change_native_dependency() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("config.toml");
        std::fs::write(&target, b"model = 'original'\n").unwrap();
        let before = dependencies(None, &target).unwrap().0;

        std::fs::write(root.path().join("unrelated.tmp"), b"churn").unwrap();
        let after_create = dependencies(None, &target).unwrap().0;
        std::fs::remove_file(root.path().join("unrelated.tmp")).unwrap();
        let after_remove = dependencies(None, &target).unwrap().0;

        assert_eq!(before, after_create);
        assert_eq!(before, after_remove);
    }

    #[test]
    fn target_change_and_parent_replacement_change_native_dependency() {
        use std::os::unix::fs::PermissionsExt;

        let outer = tempfile::tempdir().unwrap();
        let parent = outer.path().join("config");
        std::fs::create_dir(&parent).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        let target = parent.join("config.toml");
        std::fs::write(&target, b"model = 'original'\n").unwrap();
        let before = dependencies(None, &target).unwrap().0;

        std::fs::write(&target, b"model = 'changed'\n").unwrap();
        let changed = dependencies(None, &target).unwrap().0;
        assert_ne!(before, changed);

        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o750)).unwrap();
        let permission_changed = dependencies(None, &target).unwrap().0;
        assert_ne!(changed, permission_changed);

        // Keep the removed inodes allocated so a fast remove/create on a workbench
        // filesystem cannot reuse both identities before the second observation.
        let _old_parent = std::fs::File::open(&parent).unwrap();
        let _old_target = std::fs::File::open(&target).unwrap();
        std::fs::remove_file(&target).unwrap();
        std::fs::remove_dir(&parent).unwrap();
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(&target, b"model = 'original'\n").unwrap();
        let replaced = dependencies(None, &target).unwrap().0;
        assert_ne!(before, replaced);
    }
}
#[cfg(not(any(unix, windows)))]
fn dependencies(_executable: Option<&Path>, _target: &Path) -> Result<(CanonicalDigest, bool), ()> {
    Err(())
}

#[cfg(windows)]
pub(super) fn dependencies(
    executable: Option<&Path>,
    target: &Path,
) -> Result<(CanonicalDigest, bool), ()> {
    use hiroute_platform::file_security::capture;
    use hiroute_platform::windows::security::{file_identity, file_link_count, open_identity};
    use std::os::windows::fs::MetadataExt;
    let identity = |path: &Path, directory: bool| -> std::io::Result<_> {
        let file = open_identity(path, directory)?;
        let meta = file.metadata()?;
        let permissions = capture(&file)?;
        let version = if directory {
            None
        } else {
            Some((
                meta.file_size(),
                meta.creation_time(),
                meta.last_write_time(),
            ))
        };
        Ok((
            file_identity(&file)?,
            permissions,
            version,
            file_link_count(&file)?,
        ))
    };
    let binary = executable
        .map(|path| {
            let path = super::executable::resolve(path)
                .map_err(|_| ())?
                .ok_or(())?;
            Ok::<_, ()>((path.clone(), identity(&path, false).map_err(|_| ())?))
        })
        .transpose()?;
    let file = match identity(target, false) {
        Ok(value) => Some(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(()),
    };
    let mut parent = target.parent().ok_or(())?;
    let mut parent_identity = None;
    for _ in 0..32 {
        match identity(parent, true) {
            Ok(value) => {
                parent_identity = Some(value);
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                parent = parent.parent().ok_or(())?;
            }
            Err(_) => return Err(()),
        }
    }
    let parent_identity = parent_identity.ok_or(())?;
    let replace = file
        .as_ref()
        .is_none_or(|entry| entry.3 == 1 && !entry.1.read_only);
    let digest =
        CanonicalDigest::of(&(binary, target, file, parent, parent_identity)).map_err(|_| ())?;
    Ok((digest, replace))
}
