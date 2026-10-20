//! Current-account Task installation. SCM ownership is never adopted by this path.
use super::*;
use crate::{
    ConsoleListenerDesiredV1, ConsoleListenerStore, StandaloneInstallRecordV1, StandaloneLayout,
};
use fs2::FileExt;
use hiroute_platform::windows::{
    background::{self, Definition, Manager},
    lifecycle, security, task,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    path::PathBuf,
    time::Duration,
};
mod repair;
mod space;
pub use space::{ReplacementSpaceFailure, preflight_desktop_space};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRequest {
    pub schema: String,
    pub package_directory: PathBuf,
    #[serde(default = "yes")]
    pub autostart: bool,
    #[serde(default)]
    pub console_loopback: bool,
    #[serde(default)]
    pub allow_legacy_stop: bool,
    #[serde(default)]
    pub defer_package_cleanup: bool,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UninstallRequest {
    pub schema: String,
    #[serde(default)]
    pub remove_data: bool,
    #[serde(default)]
    pub allow_legacy_stop: bool,
    #[serde(default)]
    pub allow_unavailable_agent_restore: bool,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
}
fn yes() -> bool {
    true
}
fn timeout() -> u64 {
    30_000
}
fn deadline(value: u64) -> io::Result<Duration> {
    if !(100..=300_000).contains(&value) {
        return Err(invalid());
    }
    Ok(Duration::from_millis(value))
}
fn layout() -> io::Result<StandaloneLayout> {
    StandaloneLayout::from_environment().map_err(|_| invalid())
}
fn definition_path(layout: &StandaloneLayout) -> PathBuf {
    layout.data_root.join("service/windows-background.json")
}
fn definition(value: &Value) -> io::Result<Definition> {
    let value: Definition = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    value.validate()?;
    if value.manager != Manager::Task {
        return Err(io::Error::other("INSTALL_SCM_MANAGED"));
    }
    Ok(value)
}
fn admit_existing(layout: &StandaloneLayout) -> io::Result<()> {
    let path = definition_path(layout);
    if path.try_exists()? {
        let value = background::read_definition(&path)?;
        if value.manager != Manager::Task {
            return Err(io::Error::other("INSTALL_SCM_MANAGED"));
        }
    }
    if let Some(journal) = Journal::read(layout)? {
        definition(&journal.new_definition)?;
        if let Some(old) = &journal.old_definition {
            definition(old)?;
        }
        if journal.new_marker.target != hiroute_platform::windows::install::native_target()? {
            return Err(io::Error::other("INSTALL_ARCHITECTURE_UNSUPPORTED"));
        }
    }
    if let Some(marker) = read_json::<StandaloneInstallRecordV1>(&layout.marker_path, MAX_RECORD)? {
        marker.validate().map_err(|_| invalid())?;
        if marker.target != hiroute_platform::windows::install::native_target()? {
            return Err(io::Error::other("INSTALL_ARCHITECTURE_UNSUPPORTED"));
        }
    }
    Ok(())
}
pub fn preflight() -> io::Result<()> {
    admit_existing(&layout()?)
}
fn lock(layout: &StandaloneLayout) -> io::Result<File> {
    // Admission is deliberately before creation of directories or the lock file.
    admit_existing(layout)?;
    let file = lock_file(layout)?;
    admit_existing(layout)?;
    Ok(file)
}
fn lock_file(layout: &StandaloneLayout) -> io::Result<File> {
    security::initialize_current_process_private_defaults()?;
    let parent = security::ensure_private_directory(&layout.data_root.join("service"))?;
    let path = layout.data_root.join("service/install.lock");
    let file = match security::open_read_write(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            security::create_owner_only_file_at(&parent, std::ffi::OsStr::new("install.lock"))?
        }
        Err(e) => return Err(e),
    };
    file.try_lock_exclusive()
        .map_err(|_| io::Error::other("INSTALL_MAINTENANCE_BUSY"))?;
    Ok(file)
}

struct NativeHost<'a> {
    layout: &'a StandaloneLayout,
    timeout: Duration,
    allow_legacy_stop: bool,
    allow_unavailable_stop: bool,
}
impl ManagerHost for NativeHost<'_> {
    fn quiesce(&mut self, values: &[&Value]) -> io::Result<()> {
        let mut mismatch = false;
        for value in values {
            let value = definition(value)?;
            match task::active(&value) {
                Ok(false) => return Ok(()),
                Ok(true) => {
                    let endpoint = self.layout.lifecycle_endpoint();
                    match lifecycle::drain(&endpoint, &value.daemon, self.timeout) {
                        Ok(()) => {}
                        Err(e)
                            if e.kind() == io::ErrorKind::Unsupported && self.allow_legacy_stop => {
                        }
                        Err(e)
                            if self.allow_unavailable_stop
                                && matches!(
                                    e.kind(),
                                    io::ErrorKind::NotFound
                                        | io::ErrorKind::TimedOut
                                        | io::ErrorKind::ConnectionRefused
                                        | io::ErrorKind::ConnectionReset
                                        | io::ErrorKind::NotConnected
                                        | io::ErrorKind::BrokenPipe
                                ) =>
                        {
                            // Only the exact owner/image-verified Task can be stopped here.
                            task::force_stop(&value)?;
                            let deadline = std::time::Instant::now() + self.timeout;
                            while task::active(&value)? {
                                if std::time::Instant::now() >= deadline {
                                    return Err(io::ErrorKind::TimedOut.into());
                                }
                                std::thread::sleep(Duration::from_millis(50));
                            }
                            return Ok(());
                        }
                        Err(e) => return Err(e),
                    }
                    let stopped = lifecycle::stop_with_fallback(
                        &endpoint,
                        &value.daemon,
                        self.timeout,
                        || task::force_stop(&value),
                    );
                    if stopped.is_err() {
                        let _ = lifecycle::upgrade(
                            &endpoint,
                            &value.daemon,
                            "upgrade-cancel",
                            Duration::from_secs(2),
                        );
                    }
                    return stopped;
                }
                Err(_) => mismatch = true,
            }
        }
        if mismatch { Err(invalid()) } else { Ok(()) }
    }
    fn register(&mut self, value: &Value, previous: Option<&Value>) -> io::Result<()> {
        let value = definition(value)?;
        let previous = previous.map(definition).transpose()?;
        value.verify_image()?;
        task::register(&value, previous.as_ref())
    }
    fn unregister(&mut self, value: &Value) -> io::Result<()> {
        task::unregister(&definition(value)?)
    }
}
/// Resume an interrupted installer only through its explicit maintenance entry.
pub fn recover_pending() -> io::Result<()> {
    let layout = layout()?;
    let _lock = lock(&layout)?;
    let mut host = NativeHost {
        layout: &layout,
        timeout: Duration::from_secs(30),
        allow_legacy_stop: false,
        allow_unavailable_stop: false,
    };
    if let Some(journal) = Journal::read(&layout)? {
        journal.recover(&layout, &mut host)?;
    }
    Ok(())
}
fn cleanup_locked(layout: &StandaloneLayout) -> io::Result<PackageCleanup> {
    let mut retained = vec![];
    if let Some(package) = crate::RetainedRecoveryPackage::read(&layout.storage_root())? {
        retained.push(package.package_root);
    }
    if let Some(marker) = read_json::<StandaloneInstallRecordV1>(&layout.marker_path, MAX_RECORD)? {
        marker.validate().map_err(|_| invalid())?;
        retained.push(marker.install_root);
    }
    if let Some(journal) = Journal::read(layout)? {
        retained.push(journal.new_marker.install_root);
        retained.extend(journal.old_marker.map(|marker| marker.install_root));
    }
    prune_packages(
        &layout.home.join(".local/lib/hiroute"),
        hiroute_platform::windows::install::native_target()?,
        &retained,
    )
}
/// Called by the shell only after its own rollback sources are no longer needed.
pub fn cleanup_packages() -> io::Result<PackageCleanup> {
    let layout = layout()?;
    let _lock = lock(&layout)?;
    cleanup_locked(&layout)
}
fn current(
    layout: &StandaloneLayout,
) -> io::Result<Option<(StandaloneInstallRecordV1, Value, Package)>> {
    let marker: Option<StandaloneInstallRecordV1> = read_json(&layout.marker_path, MAX_RECORD)?;
    let value: Option<Value> = read_json(&definition_path(layout), MAX_RECORD)?;
    let package: Option<Package> =
        read_json(&layout.data_root.join("package-manifest.json"), 1024 * 1024)?;
    match (marker, value, package) {
        (None, None, None) => Ok(None),
        (Some(marker), Some(value), Some(package)) => {
            let native = definition(&value)?;
            package.validate()?;
            marker.validate().map_err(|_| invalid())?;
            if native.daemon != marker.install_root.join("hirouted.exe")
                || marker.service_definition != definition_path(layout)
                || marker.target != hiroute_platform::windows::install::native_target()?
                || package.target != marker.target
                || package.version != marker.version
                || !within(
                    &marker.install_root,
                    &layout.home.join(".local/lib/hiroute"),
                )
            {
                return Err(invalid());
            }
            Ok(Some((marker, value, package)))
        }
        _ => Err(invalid()),
    }
}
fn entries(
    root: &Path,
    layout: &StandaloneLayout,
    package: &Package,
) -> Vec<(PathBuf, PathBuf, EntryKind, String)> {
    let mut entries = vec![];
    for name in ["hiroute.exe", "hirouted.exe"] {
        entries.push((
            layout.home.join(".local/bin").join(name),
            root.join(name),
            EntryKind::Hardlink,
            format!("bin/{name}"),
        ));
    }
    for prefix in [".agents", ".claude"] {
        let name = "skills/hiroute-management/SKILL.md";
        entries.push((
            layout.home.join(prefix).join(name),
            root.join(name),
            EntryKind::Copy,
            name.into(),
        ));
    }
    for name in package
        .files
        .keys()
        .filter(|n| n.starts_with("licenses/") || n.starts_with("docs/"))
    {
        entries.push((
            layout.data_root.join(name),
            root.join(name),
            EntryKind::Copy,
            name.clone(),
        ));
    }
    entries.push((
        layout.data_root.join("package-manifest.json"),
        root.join("package-manifest.json"),
        EntryKind::Copy,
        "__package_manifest__".into(),
    ));
    entries
}
/// The installed transaction owns the complete native file map. Snapshot callers
/// may retain these admitted bytes, but cannot supply filesystem destinations.
pub fn recovery_files() -> io::Result<Vec<(PathBuf, String, String)>> {
    let layout = layout()?;
    let (marker, _, package) = current(&layout)?.ok_or_else(invalid)?;
    let mut output = vec![];
    for (index, (destination, source, _, _)) in entries(&marker.install_root, &layout, &package)
        .into_iter()
        .enumerate()
    {
        let expected = digest(&source)?.ok_or_else(invalid)?;
        if digest(&destination)?.as_deref() != Some(expected.as_str()) {
            return Err(invalid());
        }
        output.push((
            destination,
            format!("installation/entry-{index}.bin"),
            expected,
        ));
    }
    Ok(output)
}

fn publish_package(source: &Path, root: &Path, package: &Package) -> io::Result<()> {
    if root.try_exists()? {
        return package.verify_directory(root);
    }
    directory(root.parent().ok_or_else(invalid)?)?;
    let stage = sibling(root)?;
    directory(&stage)?;
    for name in package.files.keys() {
        let relative = Package::relative(name);
        let target = stage.join(relative);
        directory(target.parent().ok_or_else(invalid)?)?;
        copy_new(&source.join(relative), &target)?;
    }
    hiroute_platform::durable::write_private_new(
        &stage.join("package-manifest.json"),
        &package.canonical()?,
    )?;
    package.verify_directory(&stage)?;
    hiroute_platform::durable::rename(&stage, root)
}
fn retained_source(
    layout: &StandaloneLayout,
    previous: &(StandaloneInstallRecordV1, Value, Package),
    target: &std::path::Path,
) -> io::Result<PathBuf> {
    if let Some(retained) = crate::RetainedRecoveryPackage::read(&layout.storage_root())?
        && digest(target)?.as_deref() == Some(retained.target_daemon_sha256.as_str())
    {
        retained.verify(target)?;
        if retained.package_root == previous.0.install_root {
            return Ok(retained.package_root);
        }
        // The NSIS owner retained both program namespaces. Bind its original
        // standalone payload, marker and Task to this exact installation, rather
        // than accepting any same-version directory from the handoff.
        if Package::read(&retained.package_root.join("program"))?.canonical()?
            != previous.2.canonical()?
            || read_json::<Value>(
                &retained.package_root.join("installation/standalone.json"),
                MAX_RECORD,
            )? != Some(serde_json::to_value(&previous.0).map_err(|_| invalid())?)
            || read_json::<Value>(
                &retained
                    .package_root
                    .join("installation/windows-background.json"),
                MAX_RECORD,
            )? != Some(previous.1.clone())
        {
            return Err(invalid());
        }
        return Ok(retained.package_root);
    }
    Ok(previous.0.install_root.clone())
}

pub fn install(request: InstallRequest) -> io::Result<Value> {
    if request.schema != "hiroute.windows-install-request/v1" {
        return Err(invalid());
    }
    let package = Package::read(&request.package_directory)?;
    if package.target != hiroute_platform::windows::install::native_target()? {
        return Err(io::Error::other("INSTALL_ARCHITECTURE_UNSUPPORTED"));
    }
    let timeout = deadline(request.timeout_ms)?;
    let layout = layout()?;
    let _lock = lock(&layout)?;
    let mut host = NativeHost {
        layout: &layout,
        timeout,
        allow_legacy_stop: request.allow_legacy_stop,
        allow_unavailable_stop: false,
    };
    if let Some(journal) = Journal::read(&layout)? {
        if journal.operation == journal::Operation::Install
            && journal.phase == journal::Phase::Prepared
            && crate::storage_upgrade_started(&layout.storage_root())?
        {
            repair::resume(journal, &package, &layout, &mut host, request.autostart)?;
            return repair::finish(&layout, &package, &request);
        }
        journal.recover(&layout, &mut host)?;
    }
    let old = current(&layout)?;
    if let Some(previous) = &old {
        previous.2.verify_directory(&previous.0.install_root)?;
    }
    let root = layout
        .home
        .join(".local/lib/hiroute")
        .join(package.directory_name());
    publish_package(&request.package_directory, &root, &package)?;
    let recovery_root = if let Some(previous) = &old
        && root != previous.0.install_root
    {
        let target = root.join("hirouted.exe");
        let source = retained_source(&layout, previous, &target)?;
        let report = crate::checked_upgrade_preflight(&target, &layout.storage_root(), &source)
            .map_err(io::Error::other)?;
        report.history_cutover_required.then_some(source)
    } else {
        None
    };
    let native = Definition::new(Manager::Task, root.join("hirouted.exe"))?;
    let marker = StandaloneInstallRecordV1 {
        schema_version: crate::STANDALONE_INSTALL_SCHEMA_V1.into(),
        version: package.version.clone(),
        target: package.target.clone(),
        install_root: root,
        service_definition: definition_path(&layout),
        installed_skills: [".agents", ".claude"]
            .iter()
            .map(|p| layout.home.join(p).join("skills/hiroute-management"))
            .collect(),
        cpa_binary: Some(
            native
                .daemon
                .parent()
                .ok_or_else(invalid)?
                .join("libexec/cliproxyapi.exe"),
        ),
        cpa_sha256: Some(package.cpa_digest().into()),
    };
    let planned = journal::plan(
        entries(&marker.install_root, &layout, &package),
        &layout,
        old.as_ref().map(|o| &o.2),
        false,
    )?;
    let mut journal = Journal {
        schema: JOURNAL_SCHEMA.into(),
        operation: Operation::Install,
        phase: Phase::Prepared,
        old_marker: old.as_ref().map(|o| o.0.clone()),
        old_definition: old.as_ref().map(|o| o.1.clone()),
        new_marker: marker,
        new_definition: serde_json::to_value(&native).map_err(|_| invalid())?,
        files: planned,
    };
    let was_active = journal
        .old_definition
        .as_ref()
        .map(definition)
        .transpose()?
        .as_ref()
        .map(task::active)
        .transpose()?
        .unwrap_or(false);
    // Drain precedes preparation: an old daemon refusal cannot leave its admission blocked.
    if let Some(old) = &journal.old_definition {
        host.quiesce(&[old])?;
    }
    if let Err(error) = journal.persist(&layout) {
        if was_active && let Some(old) = &journal.old_definition {
            let _ = task::start(&definition(old)?);
        }
        return Err(error);
    }
    let result = (|| {
        journal.apply_files()?;
        write_json(&layout.marker_path, &journal.new_marker)?;
        host.register(&journal.new_definition, journal.old_definition.as_ref())?;
        write_json(
            &journal.new_marker.service_definition,
            &journal.new_definition,
        )?;
        task::set_autostart(&native, request.autostart)?;
        if let Some(source) = &recovery_root {
            crate::RetainedRecoveryPackage::capture(
                &layout.storage_root(),
                source,
                &journal.new_marker.install_root.join("hirouted.exe"),
            )?
            .store()?;
        }
        journal.commit(&layout)?;
        journal.finish(&layout)
    })();
    if let Err(error) = result {
        if journal.recover(&layout, &mut host).is_ok()
            && was_active
            && let Some(old) = &old
        {
            let _ = task::start(&definition(&old.1)?);
        }
        return Err(error);
    }
    repair::finish(&layout, &package, &request)
}
pub fn uninstall(
    request: UninstallRequest,
    restore_agents: impl FnOnce(&StandaloneLayout) -> io::Result<()>,
) -> io::Result<Value> {
    if request.schema != "hiroute.windows-uninstall-request/v1" {
        return Err(invalid());
    }
    if request.allow_unavailable_agent_restore && request.remove_data {
        return Err(io::Error::other("AGENT_RESTORE_DATA_REQUIRED"));
    }
    let timeout = deadline(request.timeout_ms)?;
    let layout = layout()?;
    let lock = lock(&layout)?;
    let mut host = NativeHost {
        layout: &layout,
        timeout,
        allow_legacy_stop: request.allow_legacy_stop,
        allow_unavailable_stop: false,
    };
    if let Some(journal) = Journal::read(&layout)? {
        journal.recover(&layout, &mut host)?;
    }
    let Some((marker, value, package)) = current(&layout)? else {
        let cleanup = PackageCleanup::best_effort(cleanup_locked(&layout));
        return Ok(
            json!({"schema":"hiroute.windows-uninstall-result/v1","uninstalled":true,"already_absent":true,"package_cleanup":cleanup}),
        );
    };
    let native = definition(&value)?;
    if let Some(installed_cli) = regular(&marker.install_root.join("hiroute.exe"))? {
        let running_cli = security::open_identity(&std::env::current_exe()?, false)?;
        if security::file_identity(&installed_cli)? == security::file_identity(&running_cli)? {
            return Err(io::Error::other("INSTALL_PACKAGE_IN_USE"));
        }
    }
    let restoration = super::uninstall::restore(
        &layout,
        request.allow_unavailable_agent_restore,
        request.remove_data,
        || {
            if !task::active(&native)? {
                task::start(&native).map_err(|_| io::Error::other("AGENT_CONTROL_UNAVAILABLE"))?;
            }
            Ok(())
        },
        || restore_agents(&layout),
    )?;
    host.allow_unavailable_stop = restoration != super::uninstall::Restoration::Restored;
    let planned = journal::plan(
        entries(&marker.install_root, &layout, &package),
        &layout,
        Some(&package),
        true,
    )?;
    host.quiesce(&[&value])?;
    let mut journal = Journal {
        schema: JOURNAL_SCHEMA.into(),
        operation: Operation::Uninstall,
        phase: Phase::Prepared,
        old_marker: Some(marker.clone()),
        new_marker: marker,
        old_definition: Some(value.clone()),
        new_definition: value,
        files: planned,
    };
    if let Err(error) = journal.persist(&layout) {
        // No uninstall record became durable: restore the service stopped for preparation.
        let _ = task::start(&native);
        return Err(error);
    }
    host.unregister(&journal.new_definition)?;
    journal.apply_files()?;
    journal.commit(&layout)?;
    journal.finish(&layout)?;
    hiroute_platform::windows::shell_login::remove()?;
    let cleanup = PackageCleanup::best_effort(cleanup_locked(&layout));
    drop(host);
    drop(lock);
    if request.remove_data {
        remove_tree(&layout.data_root)?;
        remove_tree(&layout.state_root)?;
    }
    Ok(
        json!({"schema":"hiroute.windows-uninstall-result/v1","uninstalled":true,"data_removed":request.remove_data,"state_preserved":!request.remove_data,"agent_restoration_unverified":restoration == super::uninstall::Restoration::Unverified,"package_cleanup":cleanup}),
    )
}
fn remove_tree(root: &Path) -> io::Result<()> {
    fn admit(path: &Path) -> io::Result<()> {
        let _held = security::open_identity(path, true)?;
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(invalid());
            }
            if metadata.is_dir() {
                admit(&path)?;
            } else {
                let _held = regular(&path)?.ok_or_else(invalid)?;
            }
        }
        Ok(())
    }
    if root.try_exists()? {
        admit(root)?;
        fs::remove_dir_all(root)?;
    }
    Ok(())
}
