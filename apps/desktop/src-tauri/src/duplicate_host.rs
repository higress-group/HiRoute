//! Handling for a duplicate host process: the root's desktop.lock is already held by
//! another HiRoute host, so this process must never start a second daemon or show UI.
use tauri::AppHandle;

/// The exit code for a duplicate host. Only a foreground duplicate inside a confirmed
/// formal bundle hands the reopen to the operating system; every other duplicate just
/// reports the existing instance without claiming anything about its service health.
pub(crate) fn duplicate_exit(background: bool, formal_bundle: bool, handed_off: bool) -> i32 {
    if background || (formal_bundle && handed_off) {
        0
    } else {
        1
    }
}

pub(crate) fn finish(handle: &AppHandle, background: bool) {
    let formal_bundle = hiroute_desktop_host_effects::formal_main_app_installation();
    // Only a confirmed formal bundle may hand the reopen to the OS; a debug or incomplete
    // build cannot raise the existing window and reports the duplicate accurately.
    let handed_off = if background {
        false
    } else if formal_bundle {
        hiroute_desktop_host_effects::open_own_bundle_in_workspace()
    } else {
        false
    };
    if handed_off {
        eprintln!("DESKTOP_ALREADY_RUNNING: reopen handed to the running HiRoute bundle");
    } else {
        eprintln!(
            "DESKTOP_ALREADY_RUNNING: another host owns this root's desktop.lock; \
             no daemon started and service state is unverified"
        );
    }
    handle.exit(duplicate_exit(background, formal_bundle, handed_off));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_successful_foreground_handoff_or_background_duplicate_exits_cleanly() {
        assert_eq!(duplicate_exit(true, false, false), 0, "repeated background");
        assert_eq!(
            duplicate_exit(true, true, false),
            0,
            "repeated background in a bundle"
        );
        assert_eq!(duplicate_exit(false, true, true), 0, "handed off");
        assert_eq!(
            duplicate_exit(false, true, false),
            1,
            "bundle handoff failed"
        );
        assert_eq!(
            duplicate_exit(false, false, false),
            1,
            "debug build reports accurately"
        );
    }
}
