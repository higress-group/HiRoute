//! Resume only the accepted current package after storage may have changed.
use super::*;

pub(super) fn resume(
    mut journal: Journal,
    package: &Package,
    layout: &StandaloneLayout,
    host: &mut NativeHost<'_>,
    autostart: bool,
) -> Result<(), InstallFailure> {
    let at = InstallFailure::at;
    let root = layout
        .home
        .join(".local/lib/hiroute")
        .join(package.directory_name());
    let native =
        Definition::new(Manager::Task, root.join("hirouted.exe")).map_err(at("define-task"))?;
    let installed = Package::read(&root).map_err(at("read-repair-package"))?;
    installed
        .verify_directory(&root)
        .map_err(at("verify-repair-package"))?;
    if journal.new_marker.install_root != root
        || journal.new_marker.version != package.version
        || journal.new_marker.target != package.target
        || journal.new_marker.cpa_sha256.as_deref() != Some(package.cpa_digest())
        || serde_json::to_value(&installed).map_err(|_| at("verify-repair-identity")(invalid()))?
            != serde_json::to_value(package).map_err(|_| at("verify-repair-identity")(invalid()))?
        || journal.new_definition
            != serde_json::to_value(&native).map_err(|_| at("verify-repair-identity")(invalid()))?
    {
        return Err(at("verify-repair-identity")(io::Error::other(
            "INSTALL_CURRENT_REPAIR_REQUIRED",
        )));
    }
    journal
        .reapply_install(layout, host)
        .map_err(at("reapply-install-journal"))?;
    task::set_autostart(&native, autostart).map_err(at("set-task-autostart"))?;
    journal
        .commit(layout)
        .map_err(at("commit-install-journal"))?;
    journal.finish(layout).map_err(at("finish-install-journal"))
}

pub(super) fn finish(
    layout: &StandaloneLayout,
    package: &Package,
    request: &InstallRequest,
) -> Result<Value, InstallFailure> {
    if request.console_loopback {
        ConsoleListenerStore::new(layout.gateway_config_root())
            .configure(ConsoleListenerDesiredV1::loopback())
            .map_err(|_| InstallFailure::at("configure-console-listener")(invalid()))?;
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
