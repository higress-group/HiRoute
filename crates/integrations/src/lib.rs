#![forbid(unsafe_code)]

//! Agent and compute integration adapter boundary.
//!
//! The contract slice provides Release facts and managed CPA account materialization. Agent file
//! writes, final composition, and Gateway retry behavior remain owned by later processes.

use serde::{Deserialize, Serialize};

pub mod agents;
pub mod collaboration_artifact;
pub mod compute;
pub mod gateway;
pub mod model_connections;
pub mod model_feed;
pub mod release_facts;

pub use agents::*;
pub use compute::*;
pub use model_connections::*;
pub use release_facts::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationCapability {
    AgentDiscovery,
    AgentConfigTransaction,
    ConnectorRegistryProvider,
    SourceConnectorRegistry,
    CpaSupervisor,
}

pub const IMPLEMENTATION_STATUS: &str = "not_implemented";
pub const CPA_IMPLEMENTATION_STATUS: &str = "managed_bridge_available";
