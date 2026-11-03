//! Host-process native service effects for the Desktop app.
//!
//! The Desktop application crate is `#![forbid(unsafe_code)]`, so the Objective-C
//! ServiceManagement surface is confined to this crate behind safe entry points. Registration
//! of the main app as a login item must run inside the host process: the calling process's
//! main bundle defines what gets registered, and a spawned daemon child must never be the
//! registration target.
#![deny(unsafe_code)]

#[cfg(target_os = "macos")]
mod launch_reason;
#[cfg(target_os = "macos")]
mod login_item;
#[cfg(target_os = "macos")]
mod workspace;

use std::fmt;

/// Observed state of the main-app login item, mirroring `SMAppServiceStatus`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginItemStatus {
    NotRegistered,
    Enabled,
    RequiresApproval,
    NotFound,
    /// The system returned a status this build does not understand; treated as unavailable.
    Unknown,
}

impl LoginItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRegistered => "not_registered",
            Self::Enabled => "enabled",
            Self::RequiresApproval => "requires_approval",
            Self::NotFound => "not_found",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for LoginItemStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable, non-secret failure reasons for the login-item boundary. The underlying NSError is
/// never surfaced verbatim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginItemError {
    /// The host is not a formally installed, absolutely located .app bundle; worktree, debug,
    /// and relative executables are never registered.
    NotFormalInstallation,
    /// `registerAndReturnError` failed.
    RegistrationFailed,
    /// `unregisterAndReturnError` failed.
    UnregistrationFailed,
    /// The running platform has no ServiceManagement login-item surface.
    UnsupportedPlatform,
}

impl LoginItemError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotFormalInstallation => "not_formal_installation",
            Self::RegistrationFailed => "registration_failed",
            Self::UnregistrationFailed => "unregistration_failed",
            Self::UnsupportedPlatform => "unsupported_platform",
        }
    }
}

impl fmt::Display for LoginItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// True only when the current executable lives inside an absolutely located
/// `<Name>.app/Contents/MacOS/` bundle layout. This is the precondition for registering the
/// main app; worktree, debug, and relative executables are rejected.
pub fn formal_main_app_installation() -> bool {
    #[cfg(target_os = "macos")]
    {
        login_item::formal_installation()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Reads the live main-app login-item status.
pub fn main_app_login_item_status() -> Result<LoginItemStatus, LoginItemError> {
    #[cfg(target_os = "macos")]
    {
        login_item::status()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(LoginItemError::UnsupportedPlatform)
    }
}

/// Registers the main app as a login item and returns the observed post-registration status.
/// A successful call that leaves the item pending user approval returns
/// `Ok(LoginItemStatus::RequiresApproval)`.
pub fn register_main_app_login_item() -> Result<LoginItemStatus, LoginItemError> {
    #[cfg(target_os = "macos")]
    {
        login_item::register()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(LoginItemError::UnsupportedPlatform)
    }
}

/// Unregisters the main-app login item. Only the host calls this, and only for items this
/// feature created; a pre-existing user login item is never touched.
pub fn unregister_main_app_login_item() -> Result<(), LoginItemError> {
    #[cfg(target_os = "macos")]
    {
        login_item::unregister()
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(LoginItemError::UnsupportedPlatform)
    }
}

/// Installs the native open-application launch-reason observer. The `on_launch` closure runs
/// exactly once, on the main thread, when the system dispatches the process's launch event;
/// its argument is true only for a launch caused by the registered login item. Returns
/// false when no observer could be installed and the caller must fall back to foreground
/// presentation.
pub fn observe_launch_reason(on_launch: impl FnOnce(bool) + Send + 'static) -> bool {
    #[cfg(target_os = "macos")]
    {
        launch_reason::observe_launch_reason(on_launch)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = on_launch;
        false
    }
}

/// Reinstalls the launch-reason observer when it was replaced after installation, so the
/// launch event still reaches the host before the main window is presented.
pub fn reinstate_launch_reason_observer() {
    #[cfg(target_os = "macos")]
    {
        launch_reason::reinstate_launch_reason_observer()
    }
}

/// The observed launch reason: `Some(true)` only for a login-item launch, `Some(false)` for
/// a normal foreground launch, `None` until the launch event was dispatched.
pub fn launch_reason_recorded() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        launch_reason::launch_reason_recorded()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Opens this process's own confirmed .app bundle with NSWorkspace so the operating system
/// delivers the reopen event to the already-running instance of the same bundle. Returns
/// false for debug and incomplete installations, which cannot hand off.
pub fn open_own_bundle_in_workspace() -> bool {
    #[cfg(target_os = "macos")]
    {
        workspace::open_own_bundle()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Locates Codex Desktop's application-owned engine through the operating system. This never
/// falls back to a separately installed `codex` on PATH and does not authenticate a release.
pub fn codex_desktop_engine() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        workspace::codex_desktop_engine()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(unix)]
mod codex_instance;
#[cfg(unix)]
pub use codex_instance::{SharedCodexSkills, prepare_codex_instance};

#[cfg(target_os = "macos")]
pub fn launch_codex_instance(
    codex_home: &std::path::Path,
    user_data: &std::path::Path,
) -> Result<(), &'static str> {
    codex_instance::launch(codex_home, user_data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn non_macos_hosts_report_no_login_item_surface() {
        assert!(!formal_main_app_installation());
        assert_eq!(
            main_app_login_item_status(),
            Err(LoginItemError::UnsupportedPlatform)
        );
        assert_eq!(
            register_main_app_login_item(),
            Err(LoginItemError::UnsupportedPlatform)
        );
        assert_eq!(
            unregister_main_app_login_item(),
            Err(LoginItemError::UnsupportedPlatform)
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn a_test_binary_is_never_a_formal_installation() {
        // Cargo's test harness runs the unit-test binary from a build directory, never from
        // inside a .app bundle, so registration must be refused without touching the system.
        assert!(!formal_main_app_installation());
        assert_eq!(
            register_main_app_login_item(),
            Err(LoginItemError::NotFormalInstallation)
        );
        assert_eq!(
            unregister_main_app_login_item(),
            Err(LoginItemError::NotFormalInstallation)
        );
    }
}
