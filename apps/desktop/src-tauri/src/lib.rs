#![forbid(unsafe_code)]
#[cfg(unix)]
pub mod bootstrap;
#[cfg(all(feature = "desktop-runtime", not(target_os = "linux")))]
pub mod bridge;
#[cfg(all(unix, not(target_os = "linux")))]
mod cli_entry;
#[cfg(unix)]
mod cpa_manifest;
pub use hiroute_product_session::{confirmation, failure, operation_observation, session};
#[cfg(unix)]
mod development_cpa;
#[cfg(all(feature = "desktop-runtime", not(target_os = "linux")))]
mod duplicate_host;
#[cfg(all(feature = "desktop-runtime", target_os = "linux"))]
pub mod linux_shell;

#[cfg(all(test, target_os = "macos"))]
mod fault_tests;
#[cfg(unix)]
mod gateway_address;
mod login_item;
#[cfg(unix)]
mod native_diagnostics;

#[cfg(unix)]
mod resident_ownership;

#[cfg(all(target_os = "macos", feature = "desktop-runtime"))]
pub mod updates;

pub fn random_id() -> Result<String, String> {
    let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| "RANDOM_UNAVAILABLE".to_owned())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(unix)]
pub fn session_from_resident(resident: bootstrap::Resident) -> session::Session {
    let client = resident.client.clone();
    session::Session::new(
        client,
        std::sync::Arc::new(bootstrap::NativeAuthority(std::sync::Mutex::new(resident))),
    )
}
#[cfg(all(test, target_os = "macos"))]
mod price_native_tests;
