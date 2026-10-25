use super::LaunchPresentation;

fn presentation(background: bool, duplicate: bool, fallback_at_ready: bool) -> LaunchPresentation {
    LaunchPresentation {
        background,
        duplicate,
        fallback_at_ready,
    }
}

#[test]
fn only_a_foreground_first_host_may_present_the_window() {
    assert!(!presentation(false, false, true).headless());
    assert!(presentation(true, false, true).headless(), "background");
    assert!(
        presentation(false, true, false).headless(),
        "duplicate host"
    );
}

#[test]
fn the_launch_window_starts_hidden_in_the_bundle_configuration() {
    let config = include_str!("../tauri.conf.json");
    assert!(
        config.contains("\"visible\": false"),
        "the main window must be created hidden so the native launch reason decides first"
    );
}

#[test]
fn an_early_or_late_launch_reason_keeps_foreground_and_login_starts_distinct() {
    let foreground = presentation(false, false, false);
    assert!(!foreground.should_present_at_ready(None));
    assert!(foreground.should_present_at_ready(Some(false)));
    assert!(!foreground.should_present_at_ready(Some(true)));
    // An unavailable observer may use the existing foreground fallback, but a
    // proven login-item reason must never be turned into a foreground launch.
    let fallback = presentation(false, false, true);
    assert!(fallback.should_present_at_ready(None));
    assert!(!fallback.should_present_at_ready(Some(true)));
    for blocked in [
        presentation(true, false, true),
        presentation(false, true, true),
    ] {
        for reason in [None, Some(false), Some(true)] {
            assert!(!blocked.should_present_at_ready(reason));
        }
    }
}
