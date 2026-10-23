//! Production Provider and selection adapters for the gateway-core lifecycle.
//!
//! There is deliberately no independent runtime loop here: an authorized
//! request is admitted into gateway-core once, and all fallback, memory and
//! commit ownership remains in that lifecycle.

mod driver;
mod state;
#[cfg(all(unix, debug_assertions))]
pub mod stream_capture;

pub(crate) use driver::response_diagnostics::category as response_failure_category;
pub use driver::*;
pub use state::StateAccessError;
pub(crate) use state::native_endpoint_state_key;

#[cfg(test)]
mod tests;
