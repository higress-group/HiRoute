#![forbid(unsafe_code)]
pub mod authority;
pub mod catalog;
pub mod confirmation;
pub mod failure;
pub mod operation_observation;
pub mod session;
pub mod web_confirmation;
pub use authority::{HostAuthority, SessionAuthority};
pub use session::ProductSession;
#[cfg(feature = "test-support")]
pub mod fault;
pub fn random_id() -> Result<String, String> {
    let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| "RANDOM_UNAVAILABLE".to_owned())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod authority_tests;

pub mod commands;
