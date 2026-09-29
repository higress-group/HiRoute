//! Duplicate-host handoff through NSWorkspace. The Desktop application crate stays
//! `#![forbid(unsafe_code)]`; Objective-C interop stays confined to this crate.
#![allow(unsafe_code)]

use objc2_app_kit::NSWorkspace;
use objc2_foundation::{NSString, NSURL};

use super::login_item;

/// Resolves the installed Codex Desktop bundle through LaunchServices and returns its own
/// packaged engine path. The daemon performs the ordinary safe executable/path checks; this host
/// helper deliberately does not compare a version, digest, release artifact, or PATH fallback.
pub(super) fn codex_desktop_engine() -> Option<std::path::PathBuf> {
    let bundle_identifier = NSString::from_str("com.openai.codex");
    let bundle =
        NSWorkspace::sharedWorkspace().URLForApplicationWithBundleIdentifier(&bundle_identifier)?;
    Some(
        bundle
            .to_file_path()?
            .join("Contents/Resources/codex-cli/bin/codex"),
    )
}

/// Opens this process's own confirmed .app bundle so the operating system delivers the
/// reopen event to the already-running instance. Called only on the main thread of a
/// foreground duplicate; a debug or incomplete bundle never reaches the system.
pub(super) fn open_own_bundle() -> bool {
    let Some(bundle) = login_item::own_bundle() else {
        return false;
    };
    let path = NSString::from_str(&bundle.to_string_lossy());
    let url = NSURL::fileURLWithPath(&path);
    NSWorkspace::sharedWorkspace().openURL(&url)
}
