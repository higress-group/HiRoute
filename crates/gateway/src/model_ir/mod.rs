//! Protocol-neutral request and response semantics.
//!
//! The IR deliberately carries Tool, reasoning, usage, error and opaque
//! provider-state values as typed data. Adapters must reject a value they
//! cannot represent; text conversion and silent omission are not available.

mod error;
mod native_history;
mod request;
mod response;
mod search;

pub use error::ModelIrError;
pub use request::*;
pub use response::*;
pub use search::*;
