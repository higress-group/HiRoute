//! Exact candidate capability and connector facts.
//!
//! These DTOs are deliberately independent of ranking and runtime state. A
//! future planner may order profiles, but only this module decides whether a
//! candidate is representable before credential lookup, DNS or connect.

mod capability;
mod connector;
mod context;
mod follow_reasoning;
mod reasoning;
#[path = "fixed_reasoning.rs"]
mod request_reasoning;

// Keep the pure planner seam next to the immutable capability profiles it
// consumes, without granting it access to runtime ports.
#[path = "../planner.rs"]
pub mod planner;

pub use capability::*;
pub use connector::*;
pub use context::*;
pub use follow_reasoning::RequestedNativeReasoning;
pub use planner::*;
pub use reasoning::*;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum CriticalFact<T> {
    Exact(T),
    Unknown,
}

impl<T> CriticalFact<T> {
    pub fn exact(&self) -> Option<&T> {
        match self {
            Self::Exact(value) => Some(value),
            Self::Unknown => None,
        }
    }
}
