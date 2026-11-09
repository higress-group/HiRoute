#![forbid(unsafe_code)]
fn main() {
    #[cfg(target_os = "macos")]
    if let Some(result) = hiroute_desktop::updates::finish_update_if_requested() {
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--version")) {
            hiroute_desktop::linux_shell::print_version();
            return;
        }
        hiroute_desktop::linux_shell::run();
    }
    #[cfg(not(target_os = "linux"))]
    hiroute_desktop::bridge::run();
}
