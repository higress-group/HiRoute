//! Observe the process launch before AppKit dispatches it, then present only the
//! foreground first host. Tauri invokes user setup at Runtime Ready, which is too
//! late to install an observer for the initial open-application Apple event.
use tauri::{App, AppHandle, Manager};

struct LaunchObserverInstalled(bool);

pub(super) struct LaunchPresentation {
    background: bool,
    duplicate: bool,
    fallback_at_ready: bool,
}

/// Called before running the event loop. The host observer registers again at
/// AppKit's WillFinishLaunching boundary, after its native handlers are installed.
/// The callback can precede user setup: the
/// host-effects owner records the reason before forwarding the native event,
/// and Ready consumes that record once the presentation state/window exist.
pub(super) fn observe_before_run(app: &App, background: bool) {
    #[cfg(target_os = "macos")]
    let installed = !background && {
        let handle = app.handle().clone();
        hiroute_desktop_host_effects::observe_launch_reason(move |login_item| {
            if !login_item && let Some(presentation) = handle.try_state::<LaunchPresentation>() {
                presentation.present_main_window(&handle);
            }
        })
    };
    #[cfg(not(target_os = "macos"))]
    let installed = {
        let _ = background;
        false
    };
    app.manage(LaunchObserverInstalled(installed));
}

impl LaunchPresentation {
    pub(super) fn new(handle: &AppHandle, background: bool, duplicate: bool) -> Self {
        Self {
            background,
            duplicate,
            // A bare isolated Pilot executable receives no normal open event.
            // Ordinary macOS launches instead use the observed native reason.
            fallback_at_ready: !handle.state::<LaunchObserverInstalled>().0
                || cfg!(all(feature = "desktop-pilot", debug_assertions)),
        }
    }

    fn headless(&self) -> bool {
        self.background || self.duplicate
    }

    fn should_present_at_ready(&self, login_item: Option<bool>) -> bool {
        !self.headless()
            && match login_item {
                Some(login_item) => !login_item,
                None => self.fallback_at_ready,
            }
    }

    pub(super) fn on_ready(&self, handle: &AppHandle) {
        #[cfg(target_os = "macos")]
        let reason = hiroute_desktop_host_effects::launch_reason_recorded();
        #[cfg(not(target_os = "macos"))]
        let reason = None;
        if self.should_present_at_ready(reason) {
            self.present_main_window(handle);
        } else if !self.headless() && reason.is_none() {
            // A framework may replace its handler during setup. Rechain only
            // while the launch event is still pending, never after losing it.
            #[cfg(target_os = "macos")]
            hiroute_desktop_host_effects::reinstate_launch_reason_observer();
        }
    }

    fn present_main_window(&self, handle: &AppHandle) {
        if self.headless() {
            return;
        }
        if let Some(window) = handle.get_webview_window("main")
            && !window.is_visible().unwrap_or(false)
        {
            let _ = window.show();
            #[cfg(all(feature = "desktop-pilot", debug_assertions))]
            if std::env::var_os("HIROUTE_DESKTOP_TEST_ROOT").is_some() {
                let _ = window.set_focus();
            }
        }
    }
}

#[cfg(test)]
#[path = "../bridge_launch_presentation_tests.rs"]
mod tests;
