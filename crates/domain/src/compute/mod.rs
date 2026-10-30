//! Trusted compute-catalog, source, pricing, pool, and runtime-state values.
//!
//! These types deliberately contain no network client and no Secret locator. Endpoint authority
//! exists only in a verified [\`ConnectorRegistryBundleV1\`]; model facts and observed inventory
//! can reference that authority but cannot create it.

mod common;
mod control_projection;
mod credential_runtime;
mod credential_selection;
mod decision_credential;
mod inventory;
mod managed_identity;
mod management;
mod model_data;
mod model_metadata;
mod model_metadata_records;
mod official_data;
mod price;
mod ratings;
mod registry;
mod release_facts;
mod runtime;
mod source;
mod source_price;

pub use common::*;
pub use control_projection::*;
pub use credential_runtime::*;
pub use credential_selection::*;
pub use decision_credential::*;
pub use inventory::*;
pub use managed_identity::*;
pub use management::*;
pub use model_data::*;
pub use model_metadata::*;
pub use model_metadata_records::*;
pub use official_data::*;
pub use price::*;
pub use ratings::*;
pub use registry::*;
pub use release_facts::*;
pub use runtime::*;
pub use source::*;
pub use source_price::*;

pub const CONNECTOR_REGISTRY_SCHEMA_V1: &str = "hiroute.connector-registry/v1";
pub const MODEL_DATA_SCHEMA_V1: &str = "hiroute.model-data/v1";
pub const RELEASE_FACTS_SCHEMA_V2: &str = "hiroute.release-facts/v2";
pub const AGENT_PROFILES_ARTIFACT_SCHEMA_V1: &str = "hiroute.agent-profiles-artifact/v1";
pub const COMPUTE_STATE_SCHEMA_V1: &str = "hiroute.compute-state/v1";
pub const MAX_RELEASE_BUNDLE_BYTES: usize = 2 * 1024 * 1024;

#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod state_tests;
