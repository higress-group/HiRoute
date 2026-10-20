//! Resume only the accepted current package after storage may have changed.
use super::*;

pub(super) fn resume(
    mut journal: Journal,
    package: &Package,
    layout: &StandaloneLayout,
    host: &mut NativeHost<'_>,
    autostart: bool,
) -> io::Result<()> {
    let root = layout
        .home
        .join(".local/lib/hiroute")
        .join(package.directory_name());
    let native = Definition::new(Manager::Task, root.join("hirouted.exe"))?;
    let installed = Package::read(&root)?;
    installed.verify_directory(&root)?;
    if journal.new_marker.install_root != root
        || journal.new_marker.version != package.version
        || journal.new_marker.target != package.target
        || journal.new_marker.cpa_sha256.as_deref() != Some(package.cpa_digest())
        || serde_json::to_value(&installed).map_err(|_| invalid())?
            != serde_json::to_value(package).map_err(|_| invalid())?
        || journal.new_definition != serde_json::to_value(&native).map_err(|_| invalid())?
    {
        return Err(io::Error::other("INSTALL_CURRENT_REPAIR_REQUIRED"));
    }
    journal.reapply_install(layout, host)?;
    task::set_autostart(&native, autostart)?;
    journal.commit(layout)?;
    journal.finish(layout)
}

pub(super) fn finish(
    layout: &StandaloneLayout,
    package: &Package,
    request: &InstallRequest,
) -> io::Result<Value> {
    if request.console_loopback {
        ConsoleListenerStore::new(layout.gateway_config_root())
            .configure(ConsoleListenerDesiredV1::loopback())
            .map_err(|_| invalid())?;
    }
    let cleanup = if request.defer_package_cleanup {
        None
    } else {
        Some(PackageCleanup::best_effort(cleanup_locked(layout)))
    };
    Ok(
        json!({"schema":"hiroute.windows-install-result/v1","installed":true,"version":package.version,"manager":"task","cli":layout.home.join(".local/bin/hiroute.exe"),"started":false,"package_cleanup":cleanup}),
    )
}
