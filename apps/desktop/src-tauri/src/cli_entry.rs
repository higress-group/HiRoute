use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CliEntryState {
    Missing,
    Valid,
    Broken,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CliEntryStatus {
    pub state: CliEntryState,
    pub entry_path: PathBuf,
    pub app_cli_path: Option<PathBuf>,
    pub path_contains_entry_directory: bool,
    pub resolved_command: Option<PathBuf>,
    pub command_resolves_to_app_cli: bool,
}

pub(crate) fn inspect_current() -> Result<CliEntryStatus, String> {
    let home = absolute_home()?;
    let target = std::env::current_exe()
        .map_err(|_| "CLI_ENTRY_APP_UNAVAILABLE".to_owned())?
        .with_file_name("hiroute");
    inspect(
        &home,
        &target,
        &supported_roots(&home),
        std::env::var_os("PATH"),
    )
}

pub(crate) fn install_current() -> Result<CliEntryStatus, String> {
    let home = absolute_home()?;
    let target = std::env::current_exe()
        .map_err(|_| "CLI_ENTRY_APP_UNAVAILABLE".to_owned())?
        .with_file_name("hiroute");
    install(
        &home,
        &target,
        &supported_roots(&home),
        std::env::var_os("PATH"),
    )
}

pub(crate) fn remove_current() -> Result<CliEntryStatus, String> {
    let home = absolute_home()?;
    let target = std::env::current_exe()
        .map_err(|_| "CLI_ENTRY_APP_UNAVAILABLE".to_owned())?
        .with_file_name("hiroute");
    remove(
        &home,
        &target,
        &supported_roots(&home),
        std::env::var_os("PATH"),
    )
}

fn absolute_home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| "CLI_ENTRY_HOME_INVALID".to_owned())
}

fn supported_roots(home: &Path) -> [PathBuf; 2] {
    [PathBuf::from("/Applications"), home.join("Applications")]
}

fn entry_path(home: &Path) -> PathBuf {
    home.join(".local/bin/hiroute")
}

fn inspect(
    home: &Path,
    target: &Path,
    roots: &[PathBuf],
    path: Option<OsString>,
) -> Result<CliEntryStatus, String> {
    if !home.is_absolute() {
        return Err("CLI_ENTRY_HOME_INVALID".into());
    }
    let entry = entry_path(home);
    let validated_target = validate_app_cli(target, roots).ok();
    let state = match std::fs::symlink_metadata(&entry) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => CliEntryState::Missing,
        Err(_) => return Err("CLI_ENTRY_UNAVAILABLE".into()),
        Ok(metadata) if !metadata.file_type().is_symlink() => CliEntryState::Conflict,
        Ok(_) => {
            let link = std::fs::read_link(&entry).map_err(|_| "CLI_ENTRY_UNAVAILABLE")?;
            if !owned_link_target(&link, roots) {
                CliEntryState::Conflict
            } else {
                match (std::fs::canonicalize(&entry), validated_target.as_ref()) {
                    (Ok(actual), Some(expected)) if actual == *expected => CliEntryState::Valid,
                    (Err(error), _) if error.kind() == std::io::ErrorKind::NotFound => {
                        CliEntryState::Broken
                    }
                    (Ok(_), _) | (Err(_), _) => CliEntryState::Broken,
                }
            }
        }
    };
    let bin = entry
        .parent()
        .expect("the fixed user CLI entry has a parent")
        .to_path_buf();
    let path_entries = path
        .as_ref()
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let path_contains_entry_directory = path_entries.iter().any(|value| value == &bin);
    let resolved_command = path_entries
        .iter()
        .map(|directory| directory.join("hiroute"))
        .find(|candidate| std::fs::symlink_metadata(candidate).is_ok());
    let command_resolves_to_app_cli = resolved_command
        .as_ref()
        .and_then(|candidate| std::fs::canonicalize(candidate).ok())
        .zip(validated_target.as_ref())
        .is_some_and(|(actual, expected)| actual == *expected);
    Ok(CliEntryStatus {
        state,
        entry_path: entry,
        app_cli_path: validated_target,
        path_contains_entry_directory,
        resolved_command,
        command_resolves_to_app_cli,
    })
}

fn install(
    home: &Path,
    target: &Path,
    roots: &[PathBuf],
    path: Option<OsString>,
) -> Result<CliEntryStatus, String> {
    let target = validate_app_cli(target, roots)?;
    let before = inspect(home, &target, roots, path.clone())?;
    match before.state {
        CliEntryState::Valid => return Ok(before),
        CliEntryState::Conflict => return Err("CLI_ENTRY_CONFLICT".into()),
        CliEntryState::Missing | CliEntryState::Broken => {}
    }
    let local = home.join(".local");
    let bin = local.join("bin");
    ensure_owned_directory(&local)?;
    ensure_owned_directory(&bin)?;
    let entry = bin.join("hiroute");
    // The containing directory is owner-controlled. Re-read immediately before replacing an
    // owned broken link so an unrelated entry is never intentionally overwritten.
    if std::fs::symlink_metadata(&entry).is_ok() {
        let link = std::fs::read_link(&entry).map_err(|_| "CLI_ENTRY_CONFLICT")?;
        if !owned_link_target(&link, roots) {
            return Err("CLI_ENTRY_CONFLICT".into());
        }
    }
    let temporary = bin.join(format!(".hiroute-{}.tmp", crate::random_id()?));
    std::os::unix::fs::symlink(&target, &temporary)
        .map_err(|_| "CLI_ENTRY_INSTALL_FAILED".to_owned())?;
    let replaced = std::fs::rename(&temporary, &entry);
    if replaced.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err("CLI_ENTRY_INSTALL_FAILED".into());
    }
    let after = inspect(home, &target, roots, path)?;
    if after.state != CliEntryState::Valid {
        return Err("CLI_ENTRY_INSTALL_FAILED".into());
    }
    Ok(after)
}

fn remove(
    home: &Path,
    target: &Path,
    roots: &[PathBuf],
    path: Option<OsString>,
) -> Result<CliEntryStatus, String> {
    let before = inspect(home, target, roots, path.clone())?;
    match before.state {
        CliEntryState::Missing => return Ok(before),
        CliEntryState::Conflict => return Err("CLI_ENTRY_CONFLICT".into()),
        CliEntryState::Valid | CliEntryState::Broken => {}
    }
    let entry = entry_path(home);
    let link = std::fs::read_link(&entry).map_err(|_| "CLI_ENTRY_CONFLICT")?;
    if !owned_link_target(&link, roots) {
        return Err("CLI_ENTRY_CONFLICT".into());
    }
    std::fs::remove_file(&entry).map_err(|_| "CLI_ENTRY_REMOVE_FAILED".to_owned())?;
    inspect(home, target, roots, path)
}

fn validate_app_cli(target: &Path, roots: &[PathBuf]) -> Result<PathBuf, String> {
    if !target.is_absolute() {
        return Err("CLI_ENTRY_APP_NOT_INSTALLED".into());
    }
    let target = std::fs::canonicalize(target).map_err(|_| "CLI_ENTRY_APP_NOT_INSTALLED")?;
    let metadata = std::fs::metadata(&target).map_err(|_| "CLI_ENTRY_APP_NOT_INSTALLED")?;
    use std::os::unix::fs::PermissionsExt;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err("CLI_ENTRY_APP_NOT_INSTALLED".into());
    }
    let Some(bundle) = app_bundle_for_cli(&target) else {
        return Err("CLI_ENTRY_APP_NOT_INSTALLED".into());
    };
    if !roots
        .iter()
        .any(|root| bundle.parent() == Some(root.as_path()))
    {
        return Err("CLI_ENTRY_APP_NOT_INSTALLED".into());
    }
    Ok(target)
}

fn app_bundle_for_cli(target: &Path) -> Option<&Path> {
    if target.file_name()? != "hiroute" || target.parent()?.file_name()? != "MacOS" {
        return None;
    }
    let contents = target.parent()?.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    let bundle = contents.parent()?;
    (bundle.file_name()? == "HiRoute.app").then_some(bundle)
}

fn owned_link_target(target: &Path, roots: &[PathBuf]) -> bool {
    target.is_absolute()
        && app_bundle_for_cli(target).is_some_and(|bundle| {
            roots
                .iter()
                .any(|root| bundle.parent() == Some(root.as_path()))
        })
}

fn ensure_owned_directory(path: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err("CLI_ENTRY_DIRECTORY_UNSAFE".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path).map_err(|_| "CLI_ENTRY_DIRECTORY_UNAVAILABLE")?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .map_err(|_| "CLI_ENTRY_DIRECTORY_UNAVAILABLE")?;
        }
        Err(_) => return Err("CLI_ENTRY_DIRECTORY_UNAVAILABLE".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        root: tempfile::TempDir,
        home: PathBuf,
        applications: PathBuf,
        target: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            // macOS tempfile roots use /var, while the installed CLI is canonicalized to /private/var.
            let canonical_root = std::fs::canonicalize(root.path()).unwrap();
            let home = canonical_root.join("Users/test user");
            let applications = home.join("Applications");
            let target = applications.join("HiRoute.app/Contents/MacOS/hiroute");
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(&target, b"binary").unwrap();
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self {
                root,
                home,
                applications,
                target,
            }
        }

        fn roots(&self) -> [PathBuf; 1] {
            [self.applications.clone()]
        }
    }

    #[test]
    fn install_is_idempotent_and_path_and_shadowing_are_separate_facts() {
        let fixture = Fixture::new();
        let roots = fixture.roots();
        let missing = inspect(&fixture.home, &fixture.target, &roots, None).unwrap();
        assert_eq!(missing.state, CliEntryState::Missing);
        let installed = install(
            &fixture.home,
            &fixture.target,
            &roots,
            Some(OsString::from("/usr/bin")),
        )
        .unwrap();
        assert_eq!(installed.state, CliEntryState::Valid);
        assert!(!installed.path_contains_entry_directory);
        assert!(!installed.command_resolves_to_app_cli);
        assert_eq!(
            install(&fixture.home, &fixture.target, &roots, None)
                .unwrap()
                .state,
            CliEntryState::Valid
        );

        let shadow = fixture.root.path().join("shadow");
        std::fs::create_dir(&shadow).unwrap();
        std::fs::write(shadow.join("hiroute"), b"unknown").unwrap();
        let bin = fixture.home.join(".local/bin");
        let path = std::env::join_paths([shadow, bin]).unwrap();
        let status = inspect(&fixture.home, &fixture.target, &roots, Some(path)).unwrap();
        assert!(status.path_contains_entry_directory);
        assert!(!status.command_resolves_to_app_cli);
    }

    #[test]
    fn broken_owned_link_repairs_and_remove_never_deletes_directories() {
        let fixture = Fixture::new();
        let roots = fixture.roots();
        install(&fixture.home, &fixture.target, &roots, None).unwrap();
        std::fs::remove_file(&fixture.target).unwrap();
        assert_eq!(
            inspect(&fixture.home, &fixture.target, &roots, None)
                .unwrap()
                .state,
            CliEntryState::Broken
        );
        std::fs::write(&fixture.target, b"replacement").unwrap();
        std::fs::set_permissions(&fixture.target, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            install(&fixture.home, &fixture.target, &roots, None)
                .unwrap()
                .state,
            CliEntryState::Valid
        );
        assert_eq!(
            remove(&fixture.home, &fixture.target, &roots, None)
                .unwrap()
                .state,
            CliEntryState::Missing
        );
        assert!(fixture.home.join(".local/bin").is_dir());
    }

    #[test]
    fn unrelated_entry_and_uninstalled_app_are_never_modified() {
        let fixture = Fixture::new();
        let roots = fixture.roots();
        std::fs::create_dir_all(fixture.home.join(".local/bin")).unwrap();
        let entry = entry_path(&fixture.home);
        std::fs::write(&entry, b"unrelated").unwrap();
        assert_eq!(
            inspect(&fixture.home, &fixture.target, &roots, None)
                .unwrap()
                .state,
            CliEntryState::Conflict
        );
        assert!(install(&fixture.home, &fixture.target, &roots, None).is_err());
        assert!(remove(&fixture.home, &fixture.target, &roots, None).is_err());
        assert_eq!(std::fs::read(&entry).unwrap(), b"unrelated");

        let temporary = fixture
            .root
            .path()
            .join("Downloads/HiRoute.app/Contents/MacOS/hiroute");
        std::fs::create_dir_all(temporary.parent().unwrap()).unwrap();
        std::fs::write(&temporary, b"binary").unwrap();
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(validate_app_cli(&temporary, &roots).is_err());
    }
}
