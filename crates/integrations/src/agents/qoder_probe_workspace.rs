//! Qoder and its Git Bash shell must receive the same ordinary drive spelling.
//! Rust's canonical verbatim path is not a portable shell working directory.
use hiroute_platform::windows::security;
use std::{
    fs::File,
    path::{Path, PathBuf},
};

pub(super) use super::super::probe_files::native_directory as retain;

/// Qoder 1.1.65 appends this fixed stderr notice when its Bash cwd is reset.
/// It is a tool envelope, not CLI stdout. Accept it only for this held workspace.
pub(super) struct Receipt {
    path: PathBuf,
    held: File,
}

impl Receipt {
    pub(super) fn retain(path: &Path) -> io::Result<Self> {
        let (path, held) = retain(path)?;
        Ok(Self { path, held })
    }

    pub(super) fn matches(&self, actual: &str, expected: &str) -> bool {
        let Some(path) = self.path.to_str() else {
            return false;
        };
        if expected.trim().is_empty()
            || actual.trim().strip_prefix(expected.trim())
                != Some(format!("\n\nShell cwd was reset to {path}").as_str())
        {
            return false;
        }
        security::open_owner_only(&self.path, true).is_ok_and(|current| {
            security::file_identity(&current)
                .and_then(|current| security::file_identity(&self.held).map(|held| current == held))
                .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Component, Prefix};

    #[test]
    fn shell_workspace_uses_an_ordinary_name_for_the_same_private_directory() {
        let root = super::super::super::probe_files::temporary("hiroute-qoder-workspace-").unwrap();
        let directory = root.path().join("shell workspace 测试");
        drop(security::create_owner_only_directory(&directory).unwrap());
        let canonical = std::fs::canonicalize(&directory).unwrap();
        assert!(matches!(
            canonical.components().next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_))
        ));
        for original in [&directory, &canonical] {
            let (ordinary, held) = retain(original).unwrap();
            assert!(matches!(
                ordinary.components().next(),
                Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
            ));
            assert_eq!(std::fs::canonicalize(&ordinary).unwrap(), canonical);
            let opened = security::open_owner_only(&directory, true).unwrap();
            assert_eq!(
                security::file_identity(&held).unwrap(),
                security::file_identity(&opened).unwrap()
            );
        }
        let file = directory.join("not-a-directory");
        drop(security::create_owner_only_file(&file).unwrap());
        assert!(retain(&file).is_err());
        for invalid in [
            r"relative",
            r"C:relative",
            r"\\server\share",
            r"\\?\UNC\server\share",
            r"\\.\C:\workspace",
        ] {
            assert!(retain(Path::new(invalid)).is_err());
        }
    }

    #[test]
    fn cwd_notice_requires_exact_baseline_and_this_held_private_workspace() {
        let root = super::super::super::probe_files::temporary("hiroute-qoder-notice-").unwrap();
        let binding = Receipt::retain(root.path()).unwrap();
        let expected = "{\n\"status\":\"succeeded\"\n}";
        let notice = format!("\n\nShell cwd was reset to {}", binding.path.display());
        let actual = format!("{expected}{notice}");
        assert!(binding.matches(&actual, expected));
        for rejected in [
            format!("Exit code 2\n{actual}"),
            format!("{actual}\nPermission denied"),
            format!("{actual}{notice}"),
            format!("{{\"status\":\"succeeded\"}}{notice}"),
            format!(
                "{expected}\nShell cwd was reset to {}",
                binding.path.display()
            ),
            format!("{expected}\n\nShell cwd was reset to C:\\foreign"),
            format!("{actual}\\child"),
        ] {
            assert!(!binding.matches(&rejected, expected));
        }
        assert!(!binding.matches(&actual, ""));
        let foreign =
            super::super::super::probe_files::temporary("hiroute-qoder-foreign-").unwrap();
        let wrong_binding = Receipt {
            path: binding.path.clone(),
            held: security::open_owner_only(foreign.path(), true).unwrap(),
        };
        assert!(!wrong_binding.matches(&actual, expected));
    }
}
