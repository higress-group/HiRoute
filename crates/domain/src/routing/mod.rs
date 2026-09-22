//! Pure AgentPlan routing values.
//!
//! The Application compiler consumes these values and emits a fully materialized publication.
//! The Gateway runtime consumes only the compiled request and attempt phases; it never evaluates
//! these desired-state templates or consults rating and price inputs.

mod authoring;
mod classifier;
mod context_window;
mod gateway_execution;
mod materialized;
mod ordering;
mod reasoning;
mod strategy;

pub use authoring::*;
pub use classifier::*;
pub use gateway_execution::*;
pub use materialized::*;
pub use reasoning::*;
pub use strategy::*;

pub const AGENT_PLAN_DESIRED_SCHEMA_V1: &str = "hiroute.agent-plan-desired/v1";
pub const AGENT_PLAN_FACTS_SCHEMA_V1: &str = "hiroute.agent-plan-facts/v1";
pub const AGENT_PLAN_COMPILED_SCHEMA_V1: &str = "hiroute.compiled-agent-plan/v1";
pub const AGENT_PLAN_COMPILER_REVISION_V1: &str = "agent-plan-compiler/v1";

pub const AGENT_PLAN_COMPILED_SCHEMA_V2: &str = "hiroute.compiled-agent-plan/v2";
pub const AGENT_PLAN_COMPILER_REVISION_V2: &str = "agent-plan-compiler/v2";
