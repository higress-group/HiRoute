//! The explicit diagnostic reuses production construction, transport and parsing.
use super::*;
use crate::agent_turn_history::{
    AgentTurnStatus, ExecutionAttribution, PlanSnapshot, VisibleAgentTurn, VisibleContentPart,
};

#[derive(Clone, Debug)]
pub struct CClassifierDiagnosticOutcome {
    pub shape_complete: bool,
    pub question_count: u16,
    pub duration_millis: u64,
}

impl ProductionGatewayRuntime {
    pub async fn test_classifier_c(
        &self,
        mode: &hiroute_domain::ComplexityClassifierModeV1,
        protocol: &hiroute_domain::FrozenNativeProtocolV1,
    ) -> Result<CClassifierDiagnosticOutcome, ClassifierDiagnosticError> {
        protocol
            .validate()
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        if !hiroute_domain::native_mode(mode) {
            return Err(ClassifierDiagnosticError::InvalidConfig);
        }
        let mut classifier = crate::server::publication::compile_classifier_mode_authority(mode, 1)
            .map_err(|_| ClassifierDiagnosticError::InvalidConfig)?;
        let value = Arc::make_mut(&mut classifier);
        value.native_protocol = Some(protocol.clone());
        value.decision = protocol.decision.clone();
        let (id, frozen_scope) = protocol
            .scopes
            .first_key_value()
            .ok_or(ClassifierDiagnosticError::InvalidConfig)?;
        let judgment_for = |scope: &hiroute_domain::FrozenAssessmentScopeV1| -> Result<
            hiroute_domain::JudgmentSettingsV1,
            ClassifierDiagnosticError,
        > {
            let mut j = hiroute_domain::JudgmentSettingsV1::default();
            j.competence.instructions = scope.assessment["instructions"]
                .as_str()
                .and_then(|s| {
                    s.strip_prefix(&(protocol.protocol.fixed.assessment_wrapper.clone() + " "))
                })
                .ok_or(ClassifierDiagnosticError::InvalidConfig)?
                .into();
            j.competence.criteria = std::array::from_fn(|i| {
                scope.assessment["criteria"][i]
                    .as_str()
                    .unwrap_or_default()
                    .into()
            });
            j.degree.simple_threshold_millis = scope.simple_threshold_millis;
            j.competence.floor_millis = scope.competence_floor_millis;
            if !j.validate() {
                return Err(ClassifierDiagnosticError::InvalidConfig);
            }
            Ok(j)
        };
        let judgment = judgment_for(frozen_scope)?;
        match &protocol.decision {
            hiroute_domain::DecisionDefinitionV1::Ordinal { .. } => {
                value.smart_judgment = Some(judgment.clone());
            }
            hiroute_domain::DecisionDefinitionV1::Categorical { options, .. } => {
                value.smart_judgment = None;
                value.branch_policies = options
                    .iter()
                    .enumerate()
                    .map(|(i, o)| {
                        Ok(hiroute_domain::MaterializedBranchV1 {
                            id: o.id.clone(),
                            name: o.id.clone(),
                            condition: o.criterion.clone(),
                            group: hiroute_domain::MaterializedGroupId::Branch(i as u16),
                            primary_group: o.refinement.as_ref().map(|_| {
                                hiroute_domain::MaterializedGroupId::BranchPrimary(i as u16)
                            }),
                            judgment: Some(judgment_for(&protocol.scopes[&o.id])?),
                        })
                    })
                    .collect::<Result<_, ClassifierDiagnosticError>>()?;
            }
        }
        let name = if id == hiroute_domain::SMART_SAVING_SCOPE_ID {
            "Smart saving"
        } else {
            id
        };
        let history = AgentTurnHistorySnapshot {
            previous_decision: None,
            history_partial: false,
            assessment_from: Some(0),
            _pin: None,
            visible_conversation: vec![Arc::new(VisibleAgentTurn {
                branch_id: Some(id.clone()),
                executed_branch_id: Some(id.clone()),
                status: AgentTurnStatus::Completed,
                user: vec![VisibleContentPart::Text {
                    text: "Provide a short status note for a synthetic task.".into(),
                }],
                steps: vec![vec![VisibleContentPart::Text {
                    text: "The synthetic status note is ready.".into(),
                }]],
            })],
            assessment_target: Some(AssessmentTarget {
                evaluation_scope: None,
                history_membership: None,
                branch_execution: Some(hiroute_domain::BranchExecutionV1 {
                    policy: protocol.execution_policy(id, &judgment, name),
                    group: hiroute_domain::ExecutionGroupV1::Regular,
                    candidate_index: 0,
                }),
                segment_id: "c-capability-stage".into(),
                first_turn_id: "c-capability-turn".into(),
                through_turn_id: "c-capability-turn".into(),
                first_ordinal: 1,
                through_ordinal: 1,
                target_partial: false,
                plan: PlanSnapshot {
                    plan_id: "c-capability-plan".into(),
                    plan_revision: 1,
                },
                attribution: ExecutionAttribution::Single {
                    selected_branch_id: id.clone(),
                    executed_branch_id: id.clone(),
                    model_configuration_id: "synthetic".into(),
                    profile_digest: "synthetic".into(),
                },
            }),
        };
        let budget = self
            .execution
            .allocate_bound_stream_budget()
            .map_err(|_| ClassifierDiagnosticError::Resource)?;
        let replay = self
            .replay
            .as_ref()
            .ok_or(ClassifierDiagnosticError::Resource)?
            .begin_request(budget)
            .map_err(|_| ClassifierDiagnosticError::Resource)?;
        replay
            .prevalidate(&[])
            .map_err(|_| ClassifierDiagnosticError::Integrity)?;
        let prepared =
            system_one::request_template(&classifier, &diagnostic_request(), &history, false)
                .map_err(CallFailure::diagnostic)?;
        let count = match &protocol.decision {
            hiroute_domain::DecisionDefinitionV1::Ordinal { .. } => 3,
            hiroute_domain::DecisionDefinitionV1::Categorical { options, .. } => {
                3 + options.iter().filter(|o| o.refinement.is_some()).count() as u16
            }
        };
        let started = Instant::now();
        let deadline = started
            .checked_add(classifier.timeout)
            .ok_or(ClassifierDiagnosticError::Resource)?;
        let result = self
            .call_classifier(
                &classifier,
                prepared,
                &mut None,
                &replay,
                1,
                deadline,
                deadline,
                false,
                CancellationToken::new(),
            )
            .await
            .map_err(|e| match e {
                ClassificationError::Cancelled => ClassifierDiagnosticError::Cancelled,
                ClassificationError::Deadline => ClassifierDiagnosticError::Timeout,
                ClassificationError::Integrity => ClassifierDiagnosticError::Integrity,
                ClassificationError::Resource => ClassifierDiagnosticError::Resource,
            })?
            .map_err(CallFailure::diagnostic)?;
        Ok(CClassifierDiagnosticOutcome {
            shape_complete: result.native_shape_complete
                && result.assessment.is_some()
                && result.applicability.is_some(),
            question_count: count,
            duration_millis: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        })
    }
}
