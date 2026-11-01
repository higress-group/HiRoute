//! Authentication, bounded selector extraction and alias/grant authorization.

mod authority;
mod run;
mod run_diagnostics;
mod selector;

pub use authority::{AuthenticatedRequest, DispatchError, GatewayRequestAuthority};
pub use run::{
    RunObservationMetadata, RunRequestAuthorityError, RunRequestAuthorityPort, RunRequestLocator,
    RunRequestSafetyPort, VerifiedRunRequestAuthority,
};
pub use run_diagnostics::RunRequestFailure;
pub(crate) use run_diagnostics::{
    note_run_http_failure, note_run_request_failure, with_run_diagnostics,
};
pub use selector::{ModelSelector, SelectorError};

#[cfg(test)]
mod tests;
