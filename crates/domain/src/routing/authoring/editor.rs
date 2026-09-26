//! Reopen published effective configuration; parked mode data belongs exclusively to drafts.
use super::*;
impl AgentPlanAuthoringV2 {
    pub fn editor(
        &self,
        published_alias: Option<String>,
    ) -> Result<PlanEditorStateV2, PlanAuthoringError> {
        self.validate()?;
        let mut editor = PlanEditorStateV2 {
            schema: PLAN_EDITOR_SCHEMA_V2.into(),
            display_name: self.display_name.as_str().into(),
            purpose: self.purpose.as_str().into(),
            custom_alias: published_alias,
            mode: self.mode,
            candidates: vec![],
            smart: SmartEditorV2 {
                economy: vec![],
                primary: vec![],
                primary_fallback: false,
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
            AgentPlanStrategyV2::Custom { candidates } => {
                editor.candidates = candidates.clone();
            }
            AgentPlanStrategyV2::SmartSaving {
                economy,
                primary,
                primary_fallback,
                reselect_on_user_message,
                classifier,
                complex_keywords,
            } => {
                editor.smart = SmartEditorV2 {
                    economy: economy.clone(),
                    primary: primary.clone(),
                    primary_fallback: *primary_fallback,
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
        Ok(editor)
    }
}
