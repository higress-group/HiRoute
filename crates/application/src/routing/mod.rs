//! Stateless routing Preview contract.
//!
//! Preview materializes the complete route and binds exact facts into a digest, but deliberately
//! allocates no AgentPlan ID or model alias and writes no Operation/publication state.

mod drafts;
pub use drafts::*;
mod lifecycle;
pub use lifecycle::*;
mod authoring;
#[cfg(test)]
pub(crate) use authoring::tests::retained_v1_operation_plan;
pub use authoring::*;
mod editor_options;
pub(crate) use editor_options::prepare_authoring;
mod suggestions;
pub use suggestions::*;
