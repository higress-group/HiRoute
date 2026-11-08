//! Production composition contracts. Fixtures never enter this module.

mod ports;
mod prepared;

pub(crate) use prepared::RequestTemplates;
pub use prepared::{PreparedPlannerInput, ResponsesReasoningReplay};

pub(crate) use ports::BoundRequestReasoning;

pub use ports::{
    ConversationContentSink, CredentialLease, CredentialResolver, ExecutionFactSink,
    FileCredentialResolver, FilePlannerInputAuthority, LifecycleTelemetrySink,
    PlannerInputAuthority, PortError, ProductionPorts, PublicationPlannerInputAuthority,
    RuntimePublicationFeed, RuntimeStateStore,
};
