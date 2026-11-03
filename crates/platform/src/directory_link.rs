//! Exclusive directory links for caller-owned destinations. Never replace an entry.
use std::{io, path::Path};

pub fn create(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.is_absolute() || !destination.is_absolute() || !source.is_dir() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, destination)
    }
    #[cfg(windows)]
    {
        crate::windows::directory_link::create(source, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unlink(path: &Path) {
        #[cfg(unix)]
        std::fs::remove_file(path).unwrap();
        #[cfg(windows)]
        std::fs::remove_dir(path).unwrap();
    }
    #[test]
    fn codex_skill_source_link_replacement_and_removal_flow_through_shared_link() {
        let root = tempfile::tempdir().unwrap();
        let alias = root.path().join("ordinary-skill");
        let shared = root.path().join("private-skill");
        let first = root.path().join("first");
        let second = root.path().join("second");
        for (path, text) in [(&first, "first"), (&second, "second")] {
            std::fs::create_dir(path).unwrap();
            std::fs::write(path.join("SKILL.md"), text).unwrap();
        }
        create(&first, &alias).unwrap();
        create(&alias, &shared).unwrap();
        assert_eq!(
            std::fs::read_to_string(shared.join("SKILL.md")).unwrap(),
            "first"
        );
        unlink(&alias);
        create(&second, &alias).unwrap();
        assert_eq!(
            std::fs::read_to_string(shared.join("SKILL.md")).unwrap(),
            "second"
        );
        unlink(&alias);
        assert!(!shared.join("SKILL.md").exists());
        assert!(std::fs::symlink_metadata(&shared).is_ok());
        assert!(first.join("SKILL.md").is_file() && second.join("SKILL.md").is_file());
        unlink(&shared);
    }
    #[test]
    fn codex_skill_directory_link_shares_updates_without_replacing_private_entries() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("用户技能 source");
        let link = root.path().join("private-link");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "one").unwrap();
        create(&source, &link).unwrap();
        assert_eq!(
            std::fs::read_to_string(link.join("SKILL.md")).unwrap(),
            "one"
        );
        std::fs::write(source.join("SKILL.md"), "two").unwrap();
        assert_eq!(
            std::fs::read_to_string(link.join("SKILL.md")).unwrap(),
            "two"
        );
        assert!(create(&source, &link).is_err());
        let private = root.path().join("existing");
        std::fs::create_dir(&private).unwrap();
        std::fs::write(private.join("SKILL.md"), "mine").unwrap();
        assert!(create(&source, &private).is_err());
        assert_eq!(
            std::fs::read_to_string(private.join("SKILL.md")).unwrap(),
            "mine"
        );
        #[cfg(unix)]
        std::fs::remove_file(&link).unwrap();
        #[cfg(windows)]
        std::fs::remove_dir(&link).unwrap();
        assert_eq!(
            std::fs::read_to_string(source.join("SKILL.md")).unwrap(),
            "two"
        );
    }
}
