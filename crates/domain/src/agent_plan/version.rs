//! Complete immutable Plan content and durable ownership identities. No run credentials,
//! task bodies, current grants or follow-latest monetary values belong in these records.
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{AgentPlanId, ModelAlias};
use crate::{AgentPlanAuthoringV2, CanonicalDigest, CompiledAgentPlanV1, WorkspaceId};

pub const PLAN_VERSION_SCHEMA_V1: &str = "hiroute.plan-version/v2";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanExecutionRef {
    pub workspace_id: WorkspaceId,
    pub plan_id: AgentPlanId,
    pub content_revision: u64,
    pub content_digest: CanonicalDigest,
}

impl PlanExecutionRef {
    pub fn validate(&self) -> Result<(), PlanVersionError> {
        WorkspaceId::parse(self.workspace_id.as_str()).map_err(|_| PlanVersionError::Invalid)?;
        AgentPlanId::parse(self.plan_id.as_str()).map_err(|_| PlanVersionError::Invalid)?;
        CanonicalDigest::parse(self.content_digest.as_str())
            .map_err(|_| PlanVersionError::Invalid)?;
        if self.content_revision == 0 {
            return Err(PlanVersionError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanVersionV1 {
    pub schema: String,
    pub reference: PlanExecutionRef,
    pub configuration: AgentPlanAuthoringV2,
    #[serde(with = "crate::plan_codec")]
    pub compiled: CompiledAgentPlanV1,
}

impl PlanVersionV1 {
    pub fn new(
        workspace: WorkspaceId,
        configuration: AgentPlanAuthoringV2,
        compiled: CompiledAgentPlanV1,
    ) -> Result<Self, PlanVersionError> {
        // Authenticated unversioned recovery retains candidate order, then removes the retired
        // rating-order evidence before sealing the current stable representation.
        let compiled = compiled
            .into_current()
            .map_err(|_| PlanVersionError::Invalid)?;
        let compiled = crate::StoredPlanV1::freeze(&compiled)
            .and_then(|p| p.build())
            .map_err(|_| PlanVersionError::Invalid)?;
        let content_digest = CanonicalDigest::of(&(
            PLAN_VERSION_SCHEMA_V1,
            &workspace,
            &configuration,
            &crate::StoredPlanV1::freeze(&compiled).map_err(|_| PlanVersionError::Invalid)?,
        ))
        .map_err(|_| PlanVersionError::Invalid)?;
        let value = Self {
            schema: PLAN_VERSION_SCHEMA_V1.into(),
            reference: PlanExecutionRef {
                workspace_id: workspace,
                plan_id: compiled.agent_plan_id().clone(),
                content_revision: compiled.body.agent_plan_revision,
                content_digest,
            },
            configuration,
            compiled,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PlanVersionError> {
        self.reference.validate()?;
        self.configuration
            .validate()
            .map_err(|_| PlanVersionError::Invalid)?;
        self.compiled
            .validate()
            .map_err(|_| PlanVersionError::Invalid)?;
        let expected = CanonicalDigest::of(&(
            PLAN_VERSION_SCHEMA_V1,
            &self.reference.workspace_id,
            &self.configuration,
            &crate::StoredPlanV1::freeze(&self.compiled).map_err(|_| PlanVersionError::Invalid)?,
        ))
        .map_err(|_| PlanVersionError::Invalid)?;
        if self.schema != PLAN_VERSION_SCHEMA_V1
            || expected != self.reference.content_digest
            || self.reference.plan_id != *self.compiled.agent_plan_id()
            || self.reference.content_revision != self.compiled.body.agent_plan_revision
            || self.configuration.display_name != self.compiled.body.identity.display_name
            || self.configuration.purpose != self.compiled.body.identity.purpose
            || self.configuration.limits != self.compiled.body.materialized.attempt_owned.limits
        {
            return Err(PlanVersionError::Invalid);
        }
        super::version_consistency::validate_execution(&self.configuration, &self.compiled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanLifecycleV1 {
    Enabled,
    Disabled,
    Deleted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlanHeadV1 {
    pub reference: PlanExecutionRef,
    pub head_revision: u64,
    pub model_alias: ModelAlias,
    pub status: PlanLifecycleV1,
}

impl PlanHeadV1 {
    pub fn validate(&self) -> Result<(), PlanVersionError> {
        self.reference.validate()?;
        ModelAlias::parse(self.model_alias.as_str()).map_err(|_| PlanVersionError::Invalid)?;
        if self.head_revision < self.reference.content_revision {
            return Err(PlanVersionError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionOwnerKindV1 {
    Task,
    Run,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionOwnerPurposeV1 {
    Execution,
    Continuation,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VersionOwnerRefV1 {
    pub kind: VersionOwnerKindV1,
    pub owner_id: String,
    pub purpose: VersionOwnerPurposeV1,
}

impl VersionOwnerRefV1 {
    pub fn validate(&self) -> Result<(), PlanVersionError> {
        if self.owner_id.is_empty()
            || self.owner_id.len() > 256
            || !self
                .owner_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':'))
        {
            return Err(PlanVersionError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VersionReservationV1 {
    pub owner: VersionOwnerRefV1,
    pub reference: PlanExecutionRef,
    pub expires_at_unix: i64,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PlanVersionError {
    #[error("plan version record is invalid")]
    Invalid,
    #[error("exact plan version is unavailable")]
    Unavailable,
    #[error("plan or owner identity conflicts with existing content")]
    Conflict,
    #[error("plan is disabled or deleted")]
    Disabled,
    #[error("plan selection is stale")]
    Stale,
    #[error("plan version recovery must complete")]
    RecoveryRequired,
    #[error("plan version is still referenced")]
    Retained,
    #[error("plan version storage is unavailable")]
    StorageUnavailable,
}
