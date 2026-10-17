//! Shell files and the standalone journal have separate owners. Retain shell bytes until
//! the native standalone transaction, login settings and readiness check all succeed.
use hiroute_host_runtime::standalone_install::{Package, windows};
use hiroute_platform::windows::security;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

const INVENTORY: &str = "shell-package.json";
const PENDING: &str = "install-pending.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    schema: String,
    files: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    schema: String,
    phase: Phase,
    old: BTreeMap<String, String>,
    new: BTreeMap<String, String>,
    service_active: bool,
    previous_package: Option<PathBuf>,
    previous_autostart: bool,
    console_desired: hiroute_host_runtime::ConsoleListenerDesiredV1,
    shell_login: bool,
}
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Committed,
}
fn invalid() -> io::Error {
    io::Error::other("SHELL_INSTALL_RECOVERY_REQUIRED")
}
fn present(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
pub(super) fn data() -> io::Result<PathBuf> {
    Ok(hiroute_platform::windows::background::local_app_data()?.join("HiRoute/shell"))
}
pub(super) fn root() -> io::Result<PathBuf> {
    Ok(hiroute_platform::windows::background::local_app_data()?.join("Programs/HiRoute"))
}
pub(super) fn hash(path: &Path) -> io::Result<String> {
    let mut file = security::open_identity(path, false)?;
    if file.metadata()?.len() > 2 * 1024 * 1024 * 1024 {
        return Err(invalid());
    }
    let mut sha = Sha256::new();
    let mut buf = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 512
        && !name.contains('\\')
        && name.split('/').all(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
        })
}
fn validate(files: &BTreeMap<String, String>) -> io::Result<()> {
    if files.len() > 6000
        || files.iter().any(|(n, h)| {
            !safe_name(n) || h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err(invalid());
    }
    Ok(())
}
fn read<T: serde::de::DeserializeOwned>(path: &Path, private: bool) -> io::Result<T> {
    let mut file = if private {
        security::open_owner_only(path, false)?
    } else {
        security::open_identity(path, false)?
    };
    let mut bytes = vec![];
    file.by_ref()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}
fn write(path: &Path, value: &impl Serialize) -> io::Result<()> {
    hiroute_platform::durable::write_private_atomic(
        path,
        &serde_json::to_vec(value).map_err(|_| invalid())?,
    )
}
fn backup_file(source: &Path, dest: &Path, expected: &str) -> io::Result<()> {
    let parent = security::ensure_private_directory(dest.parent().ok_or_else(invalid)?)?;
    if !present(dest)? {
        let mut source = security::open_identity(source, false)?;
        let mut output =
            security::create_owner_only_file_at(&parent, dest.file_name().ok_or_else(invalid)?)?;
        io::copy(&mut source, &mut output)?;
        output.sync_all()?;
        // The owner-only writer denies all sharing on Windows. Close it before
        // reopening the completed backup for identity and content verification.
        drop(output);
    }
    if hash(dest)? != expected {
        return Err(invalid());
    }
    Ok(())
}
fn pending() -> io::Result<Pending> {
    let value: Pending = read(&data()?.join(PENDING), true)?;
    if value.schema != "hiroute.windows-shell-transaction/v1" {
        return Err(invalid());
    }
    validate(&value.old)?;
    validate(&value.new)?;
    if let Some(package) = &value.previous_package {
        let layout =
            hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
        if package.parent() != Some(layout.home.join(".local/lib/hiroute").as_path())
            || package.components().any(|p| {
                matches!(
                    p,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
        {
            return Err(invalid());
        }
    }
    value.console_desired.validate().map_err(|_| invalid())?;
    Ok(value)
}
pub(super) fn has_pending() -> io::Result<bool> {
    present(&data()?.join(PENDING))
}
pub(super) fn validate_installed() -> io::Result<()> {
    let root = root()?;
    let inventory: Inventory = read(&root.join(INVENTORY), false)?;
    if inventory.schema != "hiroute.windows-shell-package/v1" {
        return Err(invalid());
    }
    validate(&inventory.files)?;
    for (name, expected) in inventory.files {
        let path = root.join(name);
        if present(&path)? && hash(&path)? != expected {
            return Err(invalid());
        }
    }
    Ok(())
}
pub(super) fn prepare(manifest: &Path) -> io::Result<()> {
    windows::preflight()?;
    if has_pending()? {
        return Err(invalid());
    }
    let package: Package = read(manifest, false)?;
    package.validate()?;
    if package.target != "x86_64-pc-windows-msvc" {
        return Err(invalid());
    }
    let root = root()?;
    let mut old = BTreeMap::new();
    let inventory = root.join(INVENTORY);
    let existed = present(&inventory)?;
    if existed {
        let before: Inventory = read(&inventory, false)?;
        if before.schema != "hiroute.windows-shell-package/v1" {
            return Err(invalid());
        }
        validate(&before.files)?;
        for (name, expected) in before.files {
            if !present(&root.join(&name))? {
                continue;
            }
            if hash(&root.join(&name))? != expected {
                return Err(invalid());
            }
            old.insert(name, expected);
        }
        old.insert(INVENTORY.into(), hash(&inventory)?);
    } else if present(&root.join("hiroute-shell.exe"))? {
        return Err(invalid());
    }
    let mut new: BTreeMap<String, String> = package
        .files
        .iter()
        .map(|(name, item)| {
            (
                format!("standalone/{}", Package::relative(name)),
                item.sha256.clone(),
            )
        })
        .collect();
    new.insert("standalone/package-manifest.json".into(), hash(manifest)?);
    new.insert("hiroute-shell.exe".into(), hash(&std::env::current_exe()?)?);
    validate(&new)?;
    for name in new.keys() {
        if present(&root.join(name))? && !old.contains_key(name) {
            return Err(invalid());
        }
    }
    let layout =
        hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
    let previous_package =
        match hiroute_host_runtime::read_standalone_install_record(&layout.marker_path) {
            Ok(record) => Some(record.install_root),
            Err(_) if !present(&layout.marker_path)? => None,
            Err(_) => return Err(invalid()),
        };
    let (service_active, previous_autostart) = if previous_package.is_some() {
        let definition = hiroute_platform::windows::background::read_definition(
            &layout.data_root.join("service/windows-background.json"),
        )?;
        (
            hiroute_platform::windows::task::active(&definition)?,
            hiroute_platform::windows::task::autostart(&definition)?,
        )
    } else {
        (false, false)
    };
    let console_desired =
        hiroute_host_runtime::ConsoleListenerStore::new(layout.gateway_config_root())
            .read()
            .map_err(|_| invalid())?
            .desired;
    let shell_login = if existed {
        hiroute_platform::windows::shell_login::enabled(&root.join("hiroute-shell.exe"))?
    } else {
        false
    };
    let data = data()?;
    security::ensure_private_directory(&data)?;
    let backups = data.join("install-backups");
    security::ensure_private_directory(&backups)?;
    for (name, expected) in &old {
        backup_file(&root.join(name), &backups.join(name), expected)?;
    }
    write(
        &data.join(PENDING),
        &Pending {
            schema: "hiroute.windows-shell-transaction/v1".into(),
            phase: Phase::Prepared,
            old,
            new,
            service_active,
            previous_package,
            previous_autostart,
            console_desired,
            shell_login,
        },
    )
}
/// Record NSIS-generated files before any service mutation. Unknown replacements stay for repair.
pub(super) fn record_generated() -> io::Result<()> {
    let mut value = pending()?;
    let root = root()?;
    for name in ["uninstall.exe", "shell-install.json"] {
        let path = root.join(name);
        if present(&path)? {
            value.new.insert(name.into(), hash(&path)?);
        }
    }
    write(&data()?.join(PENDING), &value)
}
pub(super) fn write_preferences(bytes: &[u8]) -> io::Result<()> {
    let mut value = pending()?;
    value.new.insert(
        "shell-install.json".into(),
        format!("{:x}", Sha256::digest(bytes)),
    );
    write(&data()?.join(PENDING), &value)?;
    hiroute_platform::durable::write_private_atomic(&root()?.join("shell-install.json"), bytes)
}
fn cleanup_committed() {
    let report = match windows::cleanup_packages() {
        Ok(report) => serde_json::json!({"status":"complete","packages":report}),
        Err(_) => serde_json::json!({"status":"pending","code":"INSTALL_PACKAGE_CLEANUP_REQUIRED"}),
    };
    if let Ok(data) = data() {
        let _ = write(&data.join("package-cleanup.json"), &report);
    }
}
pub(super) fn commit() -> io::Result<()> {
    record_generated()?;
    let mut value = pending()?;
    let root = root()?;
    for (name, expected) in &value.new {
        if hash(&root.join(name))? != *expected {
            return Err(invalid());
        }
    }
    for (name, expected) in &value.old {
        let path = root.join(name);
        if name != INVENTORY && !value.new.contains_key(name) && present(&path)? {
            if hash(&path)? != *expected {
                return Err(invalid());
            }
            fs::remove_file(path)?;
        }
    }
    let inventory = Inventory {
        schema: "hiroute.windows-shell-package/v1".into(),
        files: value.new.clone(),
    };
    let bytes = serde_json::to_vec(&inventory).map_err(|_| invalid())?;
    value
        .new
        .insert(INVENTORY.into(), format!("{:x}", Sha256::digest(&bytes)));
    write(&data()?.join(PENDING), &value)?;
    hiroute_platform::durable::write_private_atomic(&root.join(INVENTORY), &bytes)?;
    value.phase = Phase::Committed;
    write(&data()?.join(PENDING), &value)?;
    clear(&value.old)?;
    cleanup_committed();
    Ok(())
}

fn clear(old: &BTreeMap<String, String>) -> io::Result<()> {
    let data = data()?;
    for (name, expected) in old {
        let path = data.join("install-backups").join(name);
        if present(&path)? {
            if hash(&path)? != *expected {
                return Err(invalid());
            }
            fs::remove_file(path)?;
        }
    }
    fs::remove_file(data.join(PENDING))
}
pub(super) fn rollback(allow_unavailable_agent_restore: bool) -> io::Result<()> {
    if !has_pending()? {
        return Ok(());
    }
    let value = pending()?;
    let root = root()?;
    if value.phase == Phase::Committed {
        validate_installed()?;
        clear(&value.old)?;
        cleanup_committed();
        return Ok(());
    }
    windows::recover_pending()?;
    // Validate the complete replacement set before restoring the first file.
    for name in value.old.keys().chain(value.new.keys()) {
        let path = root.join(name);
        if present(&path)? {
            let actual = hash(&path)?;
            if value.old.get(name) != Some(&actual) && value.new.get(name) != Some(&actual) {
                return Err(invalid());
            }
        }
    }
    let remove_added_path = super::installer::path_added(&root).map_err(|_| invalid())?
        && !super::installer::path_added(&data()?.join("install-backups"))
            .map_err(|_| invalid())?;
    // A failed post-install hook can follow a committed standalone transaction. Reinstall
    // the exact retained package through the same Rust core instead of editing its marker.
    if let Some(package) = &value.previous_package {
        windows::install(serde_json::from_value(serde_json::json!({"schema":"hiroute.windows-install-request/v1","package_directory":package,"autostart":value.previous_autostart,"console_loopback":false,"defer_package_cleanup":true})).map_err(|_|invalid())?)?;
    } else {
        let layout =
            hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
        if present(&layout.marker_path)? {
            // The bundled CLI is outside the version root that uninstall removes.
            let cli = root.join("standalone/hiroute.exe");
            let (success, reply) = super::service::execute_reply(
                &cli,
                &["__windows-uninstall", "--request-stdin"],
                Some(&serde_json::to_vec(&serde_json::json!({"schema":"hiroute.windows-uninstall-request/v1","allow_unavailable_agent_restore":allow_unavailable_agent_restore})).map_err(|_|invalid())?),
                std::time::Duration::from_secs(600),
            )
            .map_err(|_| invalid())?;
            if !success {
                return Err(io::Error::other(
                    if reply["error"]["code"] == "AGENT_CONTROL_UNAVAILABLE" {
                        "AGENT_CONTROL_UNAVAILABLE"
                    } else {
                        "SHELL_INSTALL_RECOVERY_REQUIRED"
                    },
                ));
            }
        }
    }
    if remove_added_path {
        hiroute_platform::windows::user_path::configure(false)?;
    }
    for (name, expected) in &value.old {
        let backup = data()?.join("install-backups").join(name);
        if hash(&backup)? != *expected {
            return Err(invalid());
        }
        let dest = root.join(name);
        if let Some(parent) = dest.parent() {
            security::open_identity(parent, true)?;
        }
        fs::copy(backup, dest)?;
    }
    let layout =
        hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
    hiroute_host_runtime::ConsoleListenerStore::new(layout.gateway_config_root())
        .configure(value.console_desired.clone())
        .map_err(|_| invalid())?;
    if value.shell_login {
        hiroute_platform::windows::shell_login::set(&root.join("hiroute-shell.exe"), true)?;
    } else {
        hiroute_platform::windows::shell_login::remove()?;
    }
    for (name, expected) in &value.new {
        let path = root.join(name);
        if !value.old.contains_key(name) && present(&path)? {
            if hash(&path)? != *expected {
                return Err(invalid());
            }
            fs::remove_file(path)?;
        }
    }
    if value.service_active {
        super::service::manager(&["service", "start", "--output", "json"])
            .map_err(|_| invalid())?;
    }
    write(
        &data()?.join("update-outcome.json"),
        &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"failed","code":"UPGRADE_INSTALL_FAILED","service_available":value.service_active}),
    )?;
    clear(&value.old)?;
    cleanup_committed();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_copy_closes_exclusive_writer_before_verification() {
        struct Temporary(PathBuf);
        impl Drop for Temporary {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let temporary = Temporary(std::env::temp_dir().join(format!(
            "hiroute-shell-backup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )));
        let parent = security::ensure_private_directory(&temporary.0).unwrap();
        let source = temporary.0.join("previous-shell.exe");
        fs::write(&source, b"retained shell bytes").unwrap();
        let expected = hash(&source).unwrap();
        let dest = temporary.0.join("backup.exe");
        let writer =
            security::create_owner_only_file_at(&parent, dest.file_name().unwrap()).unwrap();
        assert_eq!(hash(&dest).unwrap_err().raw_os_error(), Some(32));
        drop(writer);
        fs::remove_file(&dest).unwrap();

        backup_file(&source, &dest, &expected).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"retained shell bytes");
        backup_file(&source, &dest, &expected).unwrap();
        fs::write(&dest, b"changed backup").unwrap();
        assert!(backup_file(&source, &dest, &expected).is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"changed backup");
    }
}
