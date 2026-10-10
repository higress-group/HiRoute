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
fn dependencies(executable: Option<&Path>, target: &Path) -> Result<(CanonicalDigest, bool), ()> {
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
    let digest = CanonicalDigest::of(&(
        binary,
        target,
        file.as_ref().map(file_identity),
        parent,
        directory_identity(&parent_metadata),
    ))
    .map_err(|_| ())?;
    Ok((digest, replace))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

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
#[cfg(not(unix))]
fn dependencies(_executable: Option<&Path>, _target: &Path) -> Result<(CanonicalDigest, bool), ()> {
    Err(())
}
