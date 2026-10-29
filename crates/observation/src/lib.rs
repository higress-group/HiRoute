#![forbid(unsafe_code)]

//! Loss-aware local observation writer and query adapter.
//!
//! Ordinary logs are intentionally absent from this crate. The only truth inputs are the
//! adapter-facing execution-fact contract and the independent versioned content contract. The
//! writer owns activity/content mutations and never has access to routing/runtime correctness
//! state.

mod analysis;
pub mod content;
pub mod maintenance;
pub mod managed_text;
pub mod query;
pub mod query_v2;
pub mod receipt;
mod request_purpose;
pub mod store;
pub mod text_index;
pub mod valuation;
pub mod value;
pub mod writer;

pub use content::DigestAuthority;
pub use store::LocalObservationStore;

#[cfg(feature = "test-support")]
mod test_support;
pub use writer::{
    BoundedLifecycleReceiverV2, ConversationContentChannel, FactChannel, LocalObservationWriter,
    OfferOutcome, WriterCycleOutcome,
};

pub const IMPLEMENTATION_STATUS: &str = "local-writer-query-content-v2";
