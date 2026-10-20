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

mod recovery;

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
fn same_repair_payload(
    pending: &BTreeMap<String, String>,
    requested: &BTreeMap<String, String>,
) -> bool {
    !requested.is_empty()
        && requested
            .iter()
            .all(|(name, hash)| pending.get(name) == Some(hash))
        && pending.keys().all(|name| {
            requested.contains_key(name)
                || matches!(
                    name.as_str(),
                    "uninstall.exe" | "shell-install.json" | INVENTORY
                )
        })
}
pub(super) fn recover_for_candidate(manifest: &Path, daemon: &Path, allow: bool) -> io::Result<()> {
    let layout =
        hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
    if has_pending()?
        && pending()?.phase == Phase::Prepared
        && hiroute_host_runtime::storage_upgrade_started(&layout.storage_root())?
    {
        return prepare(manifest, daemon);
    }
    windows::recover_pending()?;
    rollback(allow)
}
pub(super) fn prepare(manifest: &Path, daemon: &Path) -> io::Result<()> {
    windows::preflight()?;
    let package: Package = read(manifest, false)?;
    package.validate()?;
    if package.target != "x86_64-pc-windows-msvc" {
        return Err(invalid());
    }
    let daemon_digest = &package
        .files
        .get("bin/hirouted.exe")
        .ok_or_else(invalid)?
        .sha256;
    if hash(daemon)? != *daemon_digest {
        return Err(invalid());
    }
    if has_pending()? {
        let layout =
            hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
        if hiroute_host_runtime::storage_upgrade_started(&layout.storage_root())? {
            let value = pending()?;
            let mut requested: BTreeMap<String, String> = package
                .files
                .iter()
                .map(|(name, item)| {
                    (
                        format!("standalone/{}", Package::relative(name)),
                        item.sha256.clone(),
                    )
                })
                .collect();
            requested.insert("standalone/package-manifest.json".into(), hash(manifest)?);
            requested.insert("hiroute-shell.exe".into(), hash(&std::env::current_exe()?)?);
            if value.phase != Phase::Prepared || !same_repair_payload(&value.new, &requested) {
                return Err(invalid());
            }
            // Re-running the exact current installer continues its existing
            // journal forward. Never replace its retained original rollback set.
            for name in value.old.keys().chain(value.new.keys()) {
                let path = root()?.join(name);
                if present(&path)? {
                    let actual = hash(&path)?;
                    if value.old.get(name) != Some(&actual) && value.new.get(name) != Some(&actual)
                    {
                        return Err(invalid());
                    }
                }
            }
            return Ok(());
        }
        rollback(false)?;
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
    let value = Pending {
        schema: "hiroute.windows-shell-transaction/v1".into(),
        phase: Phase::Prepared,
        old,
        new,
        service_active,
        previous_package,
        previous_autostart,
        console_desired,
        shell_login,
    };
    recovery::prepare(&value, &package, daemon)?;
    write(&data.join(PENDING), &value)
}
/// NSIS overlays files, so old content-hashed assets must leave the package's
/// closed file set before the Rust package reader admits the new payload.
pub(super) fn prune_payload() -> io::Result<()> {
    let value = pending()?;
    if value.phase != Phase::Prepared {
        return Err(invalid());
    }
    prune_old_payload(
        &root()?,
        &data()?.join("install-backups"),
        &value.old,
        &value.new,
    )
}
fn prune_old_payload(
    root: &Path,
    backups: &Path,
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
) -> io::Result<()> {
    validate(old)?;
    validate(new)?;
    let obsolete: Vec<_> = old
        .iter()
        .filter(|(name, _)| name.starts_with("standalone/") && !new.contains_key(*name))
        .collect();
    // Check the whole deletion set and every retained rollback copy first.
    for (name, expected) in &obsolete {
        if hash(&backups.join(name))? != **expected
            || (present(&root.join(name))? && hash(&root.join(name))? != **expected)
        {
            return Err(invalid());
        }
    }
    for (name, _) in obsolete {
        let path = root.join(name);
        if present(&path)? {
            fs::remove_file(path)?;
        }
    }
    Ok(())
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
    let actual_inventory = if present(&root.join(INVENTORY))? {
        Some(hash(&root.join(INVENTORY))?)
    } else {
        None
    };
    prepare_inventory(&value.old, &mut value.new, actual_inventory.as_deref())?;
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
    complete_commit(&data()?, &value.old)?;
    cleanup_committed();
    Ok(())
}

fn prepare_inventory(
    old: &BTreeMap<String, String>,
    new: &mut BTreeMap<String, String>,
    actual: Option<&str>,
) -> io::Result<()> {
    if actual.is_some_and(|hash| {
        old.get(INVENTORY).map(String::as_str) != Some(hash)
            && new.get(INVENTORY).map(String::as_str) != Some(hash)
    }) {
        return Err(invalid());
    }
    // A failed publication can leave this generated digest in the prepared
    // journal while the old inventory is still present. It is metadata about
    // the inventory, never a member of that inventory's own closed file set.
    new.remove(INVENTORY);
    Ok(())
}

fn complete_commit(data: &Path, old: &BTreeMap<String, String>) -> io::Result<()> {
    // A manual install has no update finisher to replace an earlier failure. Keep
    // the committed journal until reporting succeeds so recovery can repeat it.
    write(
        &data.join("update-outcome.json"),
        &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"succeeded"}),
    )?;
    clear(data, old)
}

fn clear(data: &Path, old: &BTreeMap<String, String>) -> io::Result<()> {
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
        complete_commit(&data()?, &value.old)?;
        cleanup_committed();
        return Ok(());
    }
    let layout =
        hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
    if hiroute_host_runtime::storage_upgrade_started(&layout.storage_root())? {
        return Err(io::Error::other("STORAGE_UPGRADE_RECOVERY_REQUIRED"));
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
    // the retained package only if that core changed. An early payload rejection must
    // not drain unrelated work in the still-running previous installation.
    if let Some(package) = &value.previous_package {
        let layout =
            hiroute_host_runtime::StandaloneLayout::from_environment().map_err(|_| invalid())?;
        let current = if present(&layout.marker_path)? {
            Some(
                hiroute_host_runtime::read_standalone_install_record(&layout.marker_path)
                    .map_err(|_| invalid())?,
            )
        } else {
            None
        };
        if let Some(current) = current.filter(|record| record.install_root == *package) {
            if current.service_definition
                != layout.data_root.join("service/windows-background.json")
            {
                return Err(invalid());
            }
            let definition = hiroute_platform::windows::background::read_definition(
                &current.service_definition,
            )?;
            if definition.daemon != package.join("hirouted.exe") {
                return Err(invalid());
            }
            definition.verify_image()?;
            // Task verification preserves foreign registrations; changing the login
            // trigger does not stop the existing process or run it again.
            hiroute_platform::windows::task::set_autostart(&definition, value.previous_autostart)?;
        } else {
            windows::install(serde_json::from_value(serde_json::json!({"schema":"hiroute.windows-install-request/v1","package_directory":package,"autostart":value.previous_autostart,"console_loopback":false,"defer_package_cleanup":true})).map_err(|_|invalid())?)?;
        }
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
    let service_available = value.service_active && super::service::available_blocking();
    write(
        &data()?.join("update-outcome.json"),
        &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"failed","code":"UPGRADE_INSTALL_FAILED","service_available":service_available}),
    )?;
    clear(&data()?, &value.old)?;
    cleanup_committed();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PayloadDirectory(PathBuf);
    impl PayloadDirectory {
        fn new() -> Self {
            let value = Self(std::env::temp_dir().join(format!(
                    "hiroute-shell-payload-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                )));
            security::ensure_private_directory(&value.0).unwrap();
            value
        }
        fn file(&self, name: &str, bytes: &[u8]) -> String {
            let path = self.0.join(name);
            security::ensure_private_directory(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            hash(&path).unwrap()
        }
    }
    impl Drop for PayloadDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn forward_storage_repair_requires_the_exact_prepared_payload() {
        let pending = BTreeMap::from([
            ("hiroute-shell.exe".into(), "shell-new".into()),
            ("standalone/hirouted.exe".into(), "daemon-new".into()),
            ("uninstall.exe".into(), "generated".into()),
        ]);
        let mut request = pending.clone();
        request.remove("uninstall.exe");
        assert!(same_repair_payload(&pending, &request));
        request.insert("standalone/hirouted.exe".into(), "daemon-old".into());
        assert!(!same_repair_payload(&pending, &request));
        assert!(!same_repair_payload(&pending, &BTreeMap::new()));
    }

    #[test]
    fn retrying_inventory_publication_accepts_only_old_or_admitted_new_metadata_without_self_hashing()
     {
        let old = BTreeMap::from([(INVENTORY.into(), "a".repeat(64))]);
        let payload = BTreeMap::from([("hiroute-shell.exe".into(), "b".repeat(64))]);
        let bytes = serde_json::to_vec(&Inventory {
            schema: "hiroute.windows-shell-package/v1".into(),
            files: payload.clone(),
        })
        .unwrap();
        let published = format!("{:x}", Sha256::digest(&bytes));
        for actual in [old[INVENTORY].as_str(), published.as_str()] {
            let mut new = payload.clone();
            new.insert(INVENTORY.into(), published.clone());
            prepare_inventory(&old, &mut new, Some(actual)).unwrap();
            assert_eq!(new, payload);
            assert_eq!(
                serde_json::to_vec(&Inventory {
                    schema: "hiroute.windows-shell-package/v1".into(),
                    files: new
                })
                .unwrap(),
                bytes
            );
        }
        let mut changed = payload.clone();
        changed.insert(INVENTORY.into(), published);
        let before = changed.clone();
        assert!(prepare_inventory(&old, &mut changed, Some(&"c".repeat(64))).is_err());
        assert_eq!(changed, before);
    }
    #[test]
    fn committed_manual_repair_replaces_stale_failure_before_retiring_journal() {
        let temporary = PayloadDirectory::new();
        let previous = temporary.file("install-backups/previous.exe", b"previous shell");
        temporary.file(PENDING, b"committed shell transaction");
        write(
            &temporary.0.join("update-outcome.json"),
            &serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"failed","code":"UPGRADE_INSTALL_FAILED","service_available":true}),
        )
        .unwrap();
        complete_commit(
            &temporary.0,
            &BTreeMap::from([("previous.exe".into(), previous)]),
        )
        .unwrap();
        let outcome: serde_json::Value =
            read(&temporary.0.join("update-outcome.json"), true).unwrap();
        assert_eq!(
            outcome,
            serde_json::json!({"schema":"hiroute.windows-shell-update-outcome/v1","status":"succeeded"})
        );
        assert!(!temporary.0.join(PENDING).exists());
        assert!(!temporary.0.join("install-backups/previous.exe").exists());
    }

    #[test]
    fn failed_committed_reporting_preserves_journal_and_backups_for_retry() {
        let temporary = PayloadDirectory::new();
        let previous = temporary.file("install-backups/previous.exe", b"previous shell");
        temporary.file(PENDING, b"committed shell transaction");
        let old = BTreeMap::from([("previous.exe".into(), previous)]);
        let outcome = temporary.0.join("update-outcome.json");
        security::ensure_private_directory(&outcome).unwrap();
        assert!(complete_commit(&temporary.0, &old).is_err());
        assert!(temporary.0.join(PENDING).exists());
        assert_eq!(
            fs::read(temporary.0.join("install-backups/previous.exe")).unwrap(),
            b"previous shell"
        );
        fs::remove_dir(outcome).unwrap();
        complete_commit(&temporary.0, &old).unwrap();
        assert!(!temporary.0.join(PENDING).exists());
    }

    #[test]
    fn interrupted_committed_cleanup_repeats_reporting_without_losing_journal() {
        let temporary = PayloadDirectory::new();
        let previous = temporary.file("install-backups/previous.exe", b"previous shell");
        temporary.file(PENDING, b"committed shell transaction");
        temporary.file("install-backups/previous.exe", b"foreign change");
        let old = BTreeMap::from([("previous.exe".into(), previous)]);
        assert!(complete_commit(&temporary.0, &old).is_err());
        let first: serde_json::Value =
            read(&temporary.0.join("update-outcome.json"), true).unwrap();
        assert_eq!(first["status"], "succeeded");
        assert!(temporary.0.join(PENDING).exists());
        temporary.file("install-backups/previous.exe", b"previous shell");
        complete_commit(&temporary.0, &old).unwrap();
        let retried: serde_json::Value =
            read(&temporary.0.join("update-outcome.json"), true).unwrap();
        assert_eq!(retried, first);
        assert!(!temporary.0.join(PENDING).exists());
    }

    #[test]
    fn changed_frontend_names_retire_only_verified_old_payload_and_keep_rollback_bytes() {
        let temporary = PayloadDirectory::new();
        let name = "standalone/console/assets/index-old.js";
        let previous = temporary.file(&format!("program/{name}"), b"old frontend");
        temporary.file(&format!("backups/{name}"), b"old frontend");
        let next = temporary.file(
            "program/standalone/console/assets/index-new.js",
            b"new frontend",
        );
        temporary.file("program/standalone/foreign.txt", b"foreign content");
        temporary.file("program/shell-install.json", b"previous preferences");
        let old = BTreeMap::from([(name.into(), previous)]);
        let new = BTreeMap::from([("standalone/console/assets/index-new.js".into(), next)]);
        let root = temporary.0.join("program");
        let backups = temporary.0.join("backups");
        prune_old_payload(&root, &backups, &old, &new).unwrap();
        assert!(!root.join(name).exists());
        assert_eq!(fs::read(backups.join(name)).unwrap(), b"old frontend");
        assert_eq!(
            fs::read(root.join("standalone/console/assets/index-new.js")).unwrap(),
            b"new frontend"
        );
        assert_eq!(
            fs::read(root.join("standalone/foreign.txt")).unwrap(),
            b"foreign content"
        );
        assert_eq!(
            fs::read(root.join("shell-install.json")).unwrap(),
            b"previous preferences"
        );
        prune_old_payload(&root, &backups, &old, &new).unwrap();
        fs::copy(backups.join(name), root.join(name)).unwrap();
        assert_eq!(fs::read(root.join(name)).unwrap(), b"old frontend");
    }

    #[test]
    fn changed_payload_or_backup_blocks_the_whole_prune_before_first_removal() {
        for change_backup in [false, true] {
            let temporary = PayloadDirectory::new();
            let mut old = BTreeMap::new();
            for name in ["standalone/console/a-old.js", "standalone/console/z-old.js"] {
                old.insert(
                    name.into(),
                    temporary.file(&format!("program/{name}"), b"previous"),
                );
                temporary.file(&format!("backups/{name}"), b"previous");
            }
            temporary.file(
                &format!(
                    "{}/standalone/console/z-old.js",
                    if change_backup { "backups" } else { "program" }
                ),
                b"changed content",
            );
            let root = temporary.0.join("program");
            assert!(
                prune_old_payload(&root, &temporary.0.join("backups"), &old, &BTreeMap::new())
                    .is_err()
            );
            assert_eq!(
                fs::read(root.join("standalone/console/a-old.js")).unwrap(),
                b"previous"
            );
            assert!(root.join("standalone/console/z-old.js").exists());
        }
    }

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
