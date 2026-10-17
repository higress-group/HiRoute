#![forbid(unsafe_code)]
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
fn main() {
    #[cfg(windows)]
    {
        if let Some(exit) = hiroute_shell::runtime::run_helper() {
            std::process::exit(exit);
        }
        hiroute_shell::runtime::run();
    }
    #[cfg(not(windows))]
    {
        eprintln!("UNSUPPORTED_PLATFORM");
        std::process::exit(2);
    }
}
