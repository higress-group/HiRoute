#![forbid(unsafe_code)]
#[cfg(unix)]
pub mod bootstrap;
#[cfg(feature = "desktop-runtime")]
pub mod bridge;
#[cfg(unix)]
mod cli_entry;
mod confirmation;
#[cfg(unix)]
mod development_cpa;
#[cfg(feature = "desktop-runtime")]
mod duplicate_host;
pub mod failure;
#[cfg(all(test, target_os = "macos"))]
mod fault_tests;
#[cfg(unix)]
mod gateway_address;
#[cfg(unix)]
mod login_item;
#[cfg(unix)]
mod native_diagnostics;
pub mod operation_observation;
#[cfg(unix)]
mod resident_ownership;
pub mod session;
#[cfg(all(target_os = "macos", feature = "desktop-runtime"))]
pub mod updates;

pub fn random_id() -> Result<String, String> {
    let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| "RANDOM_UNAVAILABLE".to_owned())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
