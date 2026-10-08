//! Pure AgentPlan routing values.
//!
//! The Application compiler consumes these values and emits a fully materialized publication.
//! The Gateway runtime consumes only the compiled request and attempt phases; it never evaluates
//! these desired-state templates or consults rating and price inputs.

mod authoring;
mod capability_compiler;
mod classifier;
mod decision;
pub use decision::*;
mod decision_definition;
pub use decision_definition::*;
mod context_window;
mod gateway_execution;
mod materialized;
mod native_protocol;
pub use native_protocol::{AgentProtocolAdviceV1, agent_protocol_advice};
mod stored;
mod stored_capability;
pub use stored::*;
pub use stored_capability::*;
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

pub const AGENT_PLAN_COMPILED_SCHEMA_V3: &str = "hiroute.compiled-agent-plan/v3";
pub const AGENT_PLAN_COMPILER_REVISION_V3: &str = "agent-plan-compiler/v3";
