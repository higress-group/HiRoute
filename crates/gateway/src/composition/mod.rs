//! Production composition contracts. Fixtures never enter this module.

mod ports;
mod prepared;

pub use prepared::PreparedPlannerInput;
pub(crate) use prepared::RequestTemplates;

pub use ports::{
    ConversationContentSink, CredentialLease, CredentialResolver, ExecutionFactSink,
    FileCredentialResolver, FilePlannerInputAuthority, LifecycleTelemetrySink,
    PlannerInputAuthority, PortError, ProductionPorts, PublicationPlannerInputAuthority,
    RuntimePublicationFeed, RuntimeStateStore,
};
