#![forbid(unsafe_code)]
fn main() {
    #[cfg(target_os = "macos")]
    if let Some(result) = hiroute_desktop::updates::finish_update_if_requested() {
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    hiroute_desktop::bridge::run();
}
