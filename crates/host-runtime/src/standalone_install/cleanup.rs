//! Remove only manifest-owned versioned payloads, retaining recovery sources until commit.
use super::*;
use serde::Serialize;
use std::{collections::BTreeSet, fs, path::PathBuf};

#[derive(Default, Serialize)]
pub struct PackageCleanup {
    pub removed: Vec<PathBuf>,
    pub preserved: Vec<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl PackageCleanup {
    #[cfg(any(windows, test))]
    pub(crate) fn best_effort(result: io::Result<Self>) -> Self {
        result.unwrap_or_else(|error| Self {
            error: Some(error.to_string()),
            ..Self::default()
        })
    }
}

pub fn prune_packages(
    base: &Path,
    target: &str,
    retained: &[PathBuf],
) -> io::Result<PackageCleanup> {
    windows_machine(target)?;
    let mut report = PackageCleanup::default();
    // The regular-file helper also admits every ancestor without following links.
    regular(&base.join("package-manifest.json"))?;
    let entries = match fs::read_dir(base) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let root = entry?.path();
        if retained.contains(&root) {
            continue;
        }
        match remove_package(&root, target) {
            Ok(()) => report.removed.push(root),
            // Modified, foreign, busy or unrecognised material remains available for repair.
            Err(_) => report.preserved.push(root),
        }
    }
    Ok(report)
}

fn inventory(root: &Path, expected: &BTreeSet<PathBuf>) -> io::Result<()> {
    regular(&root.join("package-manifest.json"))?;
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if !expected.contains(&path) {
            return Err(invalid());
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() {
            inventory(&path, expected)?;
        } else {
            regular(&path)?.ok_or_else(invalid)?;
        }
    }
    Ok(())
}

fn remove_package(root: &Path, target: &str) -> io::Result<()> {
    let manifest = root.join("package-manifest.json");
    let package: Package = read_json(&manifest, 1024 * 1024)?.ok_or_else(invalid)?;
    package.validate()?;
    if root.file_name() != Some(std::ffi::OsStr::new(&package.directory_name()))
        || package.target != target
    {
        return Err(invalid());
    }
    #[cfg(windows)]
    if let Some(cli) = regular(&root.join("hiroute.exe"))? {
        let running =
            hiroute_platform::windows::security::open_identity(&std::env::current_exe()?, false)?;
        if hiroute_platform::windows::security::file_identity(&cli)?
            == hiroute_platform::windows::security::file_identity(&running)?
        {
            return Err(io::Error::other("INSTALL_PACKAGE_IN_USE"));
        }
    }
    let mut directories = BTreeSet::new();
    let mut expected = BTreeSet::from([manifest.clone()]);
    let mut files = vec![];
    for (name, facts) in &package.files {
        let path = root.join(Package::relative(name));
        expected.insert(path.clone());
        let mut parent = path.parent().ok_or_else(invalid)?;
        while parent != root {
            directories.insert(parent.to_owned());
            expected.insert(parent.to_owned());
            parent = parent.parent().ok_or_else(invalid)?;
        }
        if let Some(file) = regular(&path)? {
            if file.metadata()?.len() != facts.size
                || digest(&path)?.as_ref() != Some(&facts.sha256)
            {
                return Err(invalid());
            }
            files.push((path, facts.sha256.clone()));
        }
    }
    // Admit the entire remaining inventory before the first deletion. Missing owned
    // entries are allowed so a power loss during committed cleanup is resumable.
    inventory(root, &expected)?;
    let manifest_digest = digest(&manifest)?.ok_or_else(invalid)?;
    for (path, expected) in files {
        if digest(&path)?.as_ref() != Some(&expected) {
            return Err(invalid());
        }
        remove_owned(&path)?;
    }
    let mut directories: Vec<_> = directories.into_iter().collect();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for path in directories {
        match fs::remove_dir(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if digest(&manifest)?.as_ref() != Some(&manifest_digest) {
        return Err(invalid());
    }
    remove_owned(&manifest)?;
    fs::remove_dir(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inaccessible_package_base_reports_cleanup_without_failing_the_committed_operation() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("not-a-directory");
        fs::write(&base, b"preserve unknown material").unwrap();
        let report =
            PackageCleanup::best_effort(prune_packages(&base, "x86_64-pc-windows-msvc", &[]));
        assert!(report.error.is_some());
        assert!(report.removed.is_empty());
        assert_eq!(fs::read(base).unwrap(), b"preserve unknown material");
    }
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap().join("packages");
        fs::create_dir(&base).unwrap();
        let (source, package) =
            super::super::package::tests::fixture("x86_64-pc-windows-msvc", 0x8664);
        let root = base.join(package.directory_name());
        fs::rename(source.path(), &root).unwrap();
        (temp, base, root)
    }
    #[test]
    fn current_and_shell_rollback_sources_survive_until_their_owners_commit() {
        let (_temp, base, root) = fixture();
        let report =
            prune_packages(&base, "x86_64-pc-windows-msvc", std::slice::from_ref(&root)).unwrap();
        assert!(report.removed.is_empty());
        assert!(root.join("hirouted.exe").exists());
        let report = prune_packages(&base, "x86_64-pc-windows-msvc", &[]).unwrap();
        assert_eq!(report.removed.as_slice(), std::slice::from_ref(&root));
        assert!(!root.exists());
        assert!(
            prune_packages(&base, "x86_64-pc-windows-msvc", &[])
                .unwrap()
                .removed
                .is_empty()
        );
    }
    #[test]
    fn interrupted_cleanup_resumes_using_the_retained_manifest() {
        let (_temp, base, root) = fixture();
        fs::remove_file(root.join("hiroute.exe")).unwrap();
        assert_eq!(
            prune_packages(&base, "x86_64-pc-windows-msvc", &[])
                .unwrap()
                .removed
                .as_slice(),
            std::slice::from_ref(&root)
        );
        assert!(!root.exists());
    }
    #[test]
    fn changed_files_foreign_entries_and_other_targets_are_preserved_before_any_deletion() {
        for kind in ["changed", "foreign", "directory", "target"] {
            let (_temp, base, root) = fixture();
            let target = if kind == "target" {
                "aarch64-pc-windows-msvc"
            } else {
                "x86_64-pc-windows-msvc"
            };
            match kind {
                "changed" => fs::write(root.join("hirouted.exe"), b"changed").unwrap(),
                "foreign" => fs::write(root.join("my-file"), b"foreign").unwrap(),
                "directory" => fs::create_dir(root.join("my-directory")).unwrap(),
                _ => {}
            }
            let report = prune_packages(&base, target, &[]).unwrap();
            assert_eq!(report.preserved.as_slice(), std::slice::from_ref(&root));
            assert!(root.join("hiroute.exe").exists());
            assert!(root.join("package-manifest.json").exists());
        }
    }
    #[cfg(unix)]
    #[test]
    fn linked_payload_roots_never_redirect_cleanup() {
        use std::os::unix::fs::symlink;
        let (_temp, base, root) = fixture();
        let original = root.with_extension("foreign");
        fs::rename(&root, &original).unwrap();
        symlink(&original, &root).unwrap();
        let report = prune_packages(&base, "x86_64-pc-windows-msvc", &[]).unwrap();
        assert_eq!(report.preserved.len(), 2);
        assert!(original.join("hiroute.exe").exists());
    }
}
