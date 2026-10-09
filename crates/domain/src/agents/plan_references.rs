//! Read-only reference facts owned by Agent settings. No Plan lifecycle policy or version holds.
use crate::{AgentPlanId, CanonicalDigest, PortResult, WorkspaceId};
use serde::Serialize;

/// Identifiers are stable storage identities, never executable names inferred from a path.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentPlanReferenceSubject {
    ModelProfile {
        connection_id: String,
        agent_id: String,
        profile_id: String,
    },
    ModelContext {
        context_id: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPlanReferenceKind {
    DefaultModel,
    ModelAllowed,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct AgentPlanReference {
    pub subject: AgentPlanReferenceSubject,
    pub kind: AgentPlanReferenceKind,
    /// Model connection revision or the settings operation's installed publication revision.
    pub revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentPlanReferences {
    pub workspace_id: WorkspaceId,
    pub plan_id: AgentPlanId,
    pub references: Vec<AgentPlanReference>,
    /// Digest of this exact sorted query result, including workspace and Plan identity.
    pub facts_digest: CanonicalDigest,
}

pub trait AgentPlanReferenceReadPort {
    /// A fresh coherent read of owned settings, not an authorization capability.
    /// Preview may display it. Apply MUST reread under the existing writer and shared Plan gate.
    /// Uncertain Agent operations or corrupt storage return an error, never an empty result.
    fn agent_plan_references(
        &self,
        workspace: &WorkspaceId,
        plan: &AgentPlanId,
    ) -> PortResult<AgentPlanReferences>;
}
