//! Restoration policy is separate from native Task effects and never opens user stores.
use crate::StandaloneLayout;
use std::{fs, io, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Restoration {
    NotNeeded,
    Restored,
    Unverified,
}

fn present(path: &Path) -> io::Result<bool> {
    // A dangling ancestor must not turn unknown/restorable state into proof
    // that this is a pristine installation. Do not follow links or junctions.
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(super::invalid());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if metadata.file_attributes() & 0x400 != 0 {
                        return Err(super::invalid());
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn restore(
    layout: &StandaloneLayout,
    allow_unavailable: bool,
    remove_data: bool,
    start: impl FnOnce() -> io::Result<()>,
    restore: impl FnOnce() -> io::Result<()>,
) -> io::Result<Restoration> {
    // Exceptional removal must retain the complete restoration material.
    if allow_unavailable && remove_data {
        return Err(io::Error::other("AGENT_RESTORE_DATA_REQUIRED"));
    }
    if !present(&layout.storage_root())?
        && !present(&layout.state_root.join("storage.upgrade-backups"))?
    {
        return Ok(Restoration::NotNeeded);
    }
    match start().and_then(|()| restore()) {
        Ok(()) => Ok(Restoration::Restored),
        Err(error) if allow_unavailable && error.to_string() == "AGENT_CONTROL_UNAVAILABLE" => {
            Ok(Restoration::Unverified)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout() -> (tempfile::TempDir, StandaloneLayout) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let layout = StandaloneLayout::from_values(false, Some(home), None, None, None).unwrap();
        (temp, layout)
    }
    #[test]
    fn first_install_without_agent_storage_never_requires_a_daemon() {
        let (_temp, layout) = layout();
        assert_eq!(
            restore(
                &layout,
                false,
                false,
                || panic!("daemon start"),
                || panic!("Agent query")
            )
            .unwrap(),
            Restoration::NotNeeded
        );
        assert!(!layout.state_root.exists());
    }
    #[cfg(unix)]
    #[test]
    fn a_dangling_state_ancestor_is_not_evidence_of_an_empty_installation() {
        let (_temp, layout) = layout();
        fs::create_dir_all(layout.state_root.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(layout.home.join("missing"), &layout.state_root).unwrap();
        assert!(
            restore(
                &layout,
                false,
                false,
                || panic!("start"),
                || panic!("restore")
            )
            .is_err()
        );
        assert!(
            fs::symlink_metadata(&layout.state_root)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
    #[test]
    fn unavailable_service_requires_consent_and_preserves_all_recovery_material() {
        for backup in [false, true] {
            let (_temp, layout) = layout();
            let root = if backup {
                layout.state_root.join("storage.upgrade-backups")
            } else {
                layout.storage_root()
            };
            fs::create_dir_all(&root).unwrap();
            let sentinel = root.join("keep");
            fs::write(&sentinel, b"restore material").unwrap();
            for allowed in [false, true] {
                let result = restore(
                    &layout,
                    allowed,
                    false,
                    || Err(io::Error::other("AGENT_CONTROL_UNAVAILABLE")),
                    || panic!("unavailable service"),
                );
                if allowed {
                    assert_eq!(result.unwrap(), Restoration::Unverified);
                } else {
                    assert_eq!(result.unwrap_err().to_string(), "AGENT_CONTROL_UNAVAILABLE");
                }
                assert_eq!(fs::read(&sentinel).unwrap(), b"restore material");
            }
            assert_eq!(
                restore(
                    &layout,
                    true,
                    true,
                    || panic!("start"),
                    || panic!("restore")
                )
                .unwrap_err()
                .to_string(),
                "AGENT_RESTORE_DATA_REQUIRED"
            );
        }
    }
    #[test]
    fn consent_never_bypasses_an_enumerated_conflict_or_foreign_registration() {
        let (_temp, layout) = layout();
        fs::create_dir_all(layout.storage_root()).unwrap();
        for reason in [
            "AGENT_RESTORE_REQUIRED:[\"codex\"]",
            "INSTALL_OWNERSHIP_CONFLICT",
        ] {
            let result = restore(
                &layout,
                true,
                false,
                || Ok(()),
                || Err(io::Error::other(reason)),
            );
            assert_eq!(result.unwrap_err().to_string(), reason);
        }
        assert_eq!(
            restore(&layout, false, false, || Ok(()), || Ok(())).unwrap(),
            Restoration::Restored
        );
    }
}
