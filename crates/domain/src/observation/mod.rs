//! Product-owned observation contracts.
//!
//! Execution types describe the versioned adapter-facing fact boundary consumed by the local
//! writer; content remains an independent versioned channel. They are not a dependency on a
//! Gateway implementation DTO and contain no telemetry mapping. Observation is evidence-only:
//! none of these ports may mutate routing/runtime state.

mod agent_turn_admission;
pub use agent_turn_admission::*;
mod analysis;
mod analysis_results;
mod assessment_record;
mod envelope;
mod execution;
mod execution_attempt;
#[cfg(test)]
mod execution_tests;
mod execution_validation;
mod feedback;
mod identity;
mod input_selection;
mod lifecycle;
mod native_continuation;
mod plan_quality;
mod routing_timeline;
pub use routing_timeline::*;
mod routing_observations;
mod stage_evaluation;
pub use routing_observations::*;
mod pricing;
mod projection;
mod query;
mod query_v2;

pub use analysis::*;
pub use analysis_results::*;
pub use assessment_record::*;
pub use envelope::*;
pub use execution::*;
pub use execution_attempt::*;
pub use feedback::*;
pub use identity::*;
pub use input_selection::*;
pub use lifecycle::*;
pub use native_continuation::*;
pub use plan_quality::*;
pub use pricing::*;
pub use projection::*;
pub use query::*;
pub use query_v2::*;
pub use stage_evaluation::*;

mod content_catalog;
pub use content_catalog::*;

mod retention_v2;
pub use retention_v2::*;

mod value_summary;
pub use value_summary::*;

mod trust;
pub use trust::*;
