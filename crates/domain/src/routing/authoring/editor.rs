//! Reopen published effective configuration; parked mode data belongs exclusively to drafts.
use super::*;
impl AgentPlanAuthoringV2 {
    pub fn editor(
        &self,
        published_alias: Option<String>,
    ) -> Result<PlanEditorStateV2, PlanAuthoringError> {
        self.validate()?;
        let mut editor = PlanEditorStateV2 {
            follow_request_reasoning: self.follow_request_reasoning,
            schema: PLAN_EDITOR_SCHEMA_V2.into(),
            display_name: self.display_name.as_str().into(),
            purpose: self.purpose.as_str().into(),
            custom_alias: published_alias,
            mode: self.mode,
            branch_routing: None,
            candidates: vec![],
            smart: SmartEditorV2 {
                economy: vec![],
                primary: vec![],
                judgment: Default::default(),
                reselect_on_user_message: false,
                classifier: ComplexityClassifierModeV1::LocalRules,
                complex_keywords: vec![],
            },
            free: FreeEditorV2 {
                candidates: vec![],
                primary: vec![],
                primary_fallback: false,
            },
            delegation_enabled: self.delegation_enabled,
            work: self.work.clone(),
            requirements: self.requirements.clone(),
            limits: self.limits.clone(),
        };
        match &self.strategy {
            AgentPlanStrategyV2::Branches { routing } => {
                editor.branch_routing = Some(routing.into())
            }
            AgentPlanStrategyV2::Custom { candidates } => {
                editor.candidates = candidates.clone();
            }
            AgentPlanStrategyV2::SmartSaving {
                native_protocol: _,
                economy,
                primary,
                judgment,
                reselect_on_user_message,
                classifier,
                complex_keywords,
            } => {
                editor.smart = SmartEditorV2 {
                    economy: economy.clone(),
                    primary: primary.clone(),
                    judgment: judgment.clone(),
                    reselect_on_user_message: *reselect_on_user_message,
                    classifier: classifier.clone(),
                    complex_keywords: complex_keywords.clone(),
                };
            }
            AgentPlanStrategyV2::FreeFirst {
                candidates,
                primary,
                primary_fallback,
            } => {
                editor.free = FreeEditorV2 {
                    candidates: candidates.clone(),
                    primary: primary.clone(),
                    primary_fallback: *primary_fallback,
                };
            }
        }
        // Opening an edit prepares the current defaults without mutating the publication.
        if super::super::native_mode(&editor.smart.classifier) {
            editor.smart.judgment = editor
                .smart
                .judgment
                .resolve_assessment(Some(&super::super::NativeProtocolSelectionV1::default()))
                .map_err(PlanAuthoringError::NativeProtocol)?;
        }
        if let Some(routing) = &mut editor.branch_routing
            && super::super::native_mode(&routing.classifier)
        {
            let protocol = super::super::NativeProtocolSelectionV1::default();
            routing.judgment = routing
                .judgment
                .resolve_assessment(Some(&protocol))
                .map_err(PlanAuthoringError::NativeProtocol)?;
            for branch in &mut routing.branches {
                if let Some(judgment) = &mut branch.judgment {
                    *judgment = judgment
                        .resolve_assessment(Some(&protocol))
                        .map_err(PlanAuthoringError::NativeProtocol)?;
                }
            }
        }
        Ok(editor)
    }
}
