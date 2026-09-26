//! In-memory compatibility is based exclusively on already-published executable arrays.
use crate::*;

impl PlanVersionV1 {
    pub fn from_legacy_compiled(
        workspace: WorkspaceId,
        compiled: CompiledAgentPlanV1,
    ) -> Result<Self, PlanVersionError> {
        compiled.validate().map_err(|_| PlanVersionError::Invalid)?;
        if compiled.body.schema != AGENT_PLAN_COMPILED_SCHEMA_V1 {
            return Err(PlanVersionError::Invalid);
        }
        Self::from_unversioned_compiled(workspace, compiled)
    }

    /// Reconstructs authoring metadata only when a publication predates durable PlanVersion rows.
    /// V2 appears here after a legacy publication was authenticated and currentized by another
    /// publication owner (for example, an AgentConnection grant write).
    pub fn from_unversioned_compiled_recovery(
        workspace: WorkspaceId,
        compiled: CompiledAgentPlanV1,
    ) -> Result<Self, PlanVersionError> {
        compiled.validate().map_err(|_| PlanVersionError::Invalid)?;
        if compiled.body.schema != AGENT_PLAN_COMPILED_SCHEMA_V1
            && compiled.body.schema != AGENT_PLAN_COMPILED_SCHEMA_V2
        {
            return Err(PlanVersionError::Invalid);
        }
        Self::from_unversioned_compiled(workspace, compiled)
    }

    fn from_unversioned_compiled(
        workspace: WorkspaceId,
        compiled: CompiledAgentPlanV1,
    ) -> Result<Self, PlanVersionError> {
        let materialized = &compiled.body.materialized;
        let group = |id| {
            materialized
                .attempt_owned
                .groups
                .iter()
                .find(|g| g.group_id == id)
                .map(|group| {
                    group
                        .candidates
                        .iter()
                        .map(|c| CandidateSelectionV1 {
                            binding_id: c.binding_id.clone(),
                            reasoning: match &c.exact_reasoning {
                                ExactNativeReasoningV1::Fixed { .. } => None,
                                ExactNativeReasoningV1::Toggle { enabled, .. } => {
                                    Some(ReasoningSelectionV1::Toggle { enabled: *enabled })
                                }
                                ExactNativeReasoningV1::Profile { profile, .. } => {
                                    Some(ReasoningSelectionV1::Profile {
                                        profile: profile.clone(),
                                    })
                                }
                                ExactNativeReasoningV1::Budget { tokens, .. } => {
                                    Some(ReasoningSelectionV1::Budget { tokens: *tokens })
                                }
                            },
                        })
                        .collect::<Vec<_>>()
                })
                .ok_or(PlanVersionError::Invalid)
        };
        let (mode, strategy) = match &materialized.request_owned {
            RequestOwnedRouteV1::Classified {
                classifier,
                reselect_on_user_message,
                simple_groups,
                ..
            } => (
                PlanEditorMode::SmartSaving,
                AgentPlanStrategyV2::SmartSaving {
                    economy: group(MaterializedGroupId::Economy)?,
                    primary: group(MaterializedGroupId::Primary)?,
                    primary_fallback: simple_groups.contains(&MaterializedGroupId::Primary),
                    reselect_on_user_message: *reselect_on_user_message,
                    classifier: classifier.mode.clone(),
                    complex_keywords: classifier.user_keywords.clone(),
                },
            ),
            RequestOwnedRouteV1::Ordered { ordered_groups, .. }
                if ordered_groups.first() == Some(&MaterializedGroupId::Free) =>
            {
                let fallback = ordered_groups.contains(&MaterializedGroupId::Primary);
                (
                    PlanEditorMode::FreeFirst,
                    AgentPlanStrategyV2::FreeFirst {
                        candidates: group(MaterializedGroupId::Free)?,
                        primary_fallback: fallback,
                        primary: if fallback {
                            group(MaterializedGroupId::Primary)?
                        } else {
                            vec![]
                        },
                    },
                )
            }
            RequestOwnedRouteV1::Ordered { .. } => {
                let candidates = group(MaterializedGroupId::Custom)?;
                (
                    PlanEditorMode::FixedModel,
                    AgentPlanStrategyV2::Custom { candidates },
                )
            }
        };
        let configuration = AgentPlanAuthoringV2 {
            schema: PLAN_AUTHORING_SCHEMA_V2.into(),
            mode,
            display_name: compiled.body.identity.display_name.clone(),
            purpose: compiled.body.identity.purpose.clone(),
            // Legacy publications do not carry authoring capability preferences. Do not invent
            // them; request-time capability filtering remains in the executable Gateway path.
            requirements: CapabilityRequirementsV1::default(),
            limits: materialized.attempt_owned.limits.clone(),
            strategy,
            delegation_enabled: false,
            work: None,
        };
        Self::new(workspace, configuration, compiled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unversioned_recovery_accepts_currentized_legacy_without_reopening_legacy_authoring() {
        let publication: GatewayPublicationV1 = GatewayPublicationV1::decode_persisted(
            include_bytes!("../../../../e2e/product/golden/routing/compiled-publication.v2.json"),
        )
        .unwrap();
        let legacy = publication.plans[0].clone();
        let legacy_version =
            PlanVersionV1::from_legacy_compiled(publication.workspace_id.clone(), legacy.clone())
                .unwrap();
        let current = legacy.into_current().unwrap();
        let recovered = PlanVersionV1::from_unversioned_compiled_recovery(
            publication.workspace_id,
            current.clone(),
        )
        .unwrap();

        assert_eq!(recovered.configuration, legacy_version.configuration);
        assert_eq!(recovered.compiled, current);
        recovered.validate().unwrap();
        assert!(
            PlanVersionV1::from_legacy_compiled(
                recovered.reference.workspace_id.clone(),
                recovered.compiled,
            )
            .is_err()
        );
    }
}
