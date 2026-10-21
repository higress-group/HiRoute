//! Bounded CPA discovery facts. No account IDs, model IDs, paths or response text.

use serde::{Deserialize, Serialize};

use super::CpaStageOutcome;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpaControl {
    pub phase: CpaControlPhase,
    pub outcome: CpaStageOutcome,
    pub elapsed_ms: u64,
    pub total_elapsed_ms: u64,
    pub budget_ms: u64,
    pub refresh_models: bool,
    /// Ordinal within this discovery span, never an account identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ordinal: Option<u32>,
    pub polls: u32,
    pub checks: CpaPinChecks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CpaControlPhase {
    InventoryRead,
    AccountRead,
    ControlsRead,
    ControlsPatch,
    AccountPin,
    CatalogPin,
}

/// Absent means this check was not reached. False and unobserved are distinct.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpaPinChecks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_index_matches: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_retry_zero: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persisted_prefix_matches: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persisted_retry_zero: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persisted_cooling_disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_models_nonempty: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_models_prefixed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_model_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_missing_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{CpaFailureCode, DiagnosticEvent};
    use crate::level::DiagnosticLevel;

    #[test]
    fn old_stage_records_remain_readable_and_pin_facts_are_closed() {
        let old: DiagnosticEvent = serde_json::from_str(
            r#"{"cpa_stage":{"stage":"control_call","elapsed_ms":20000,"restart_generation":0}}"#,
        )
        .unwrap();
        assert!(matches!(old, DiagnosticEvent::CpaStage(_)));
        let checks: CpaPinChecks =
            serde_json::from_str(r#"{"account_models_nonempty":false}"#).unwrap();
        assert_eq!(checks.account_models_nonempty, Some(false));
        assert_eq!(checks.account_models_prefixed, None);
        assert!(serde_json::from_str::<CpaPinChecks>(r#"{"account":"private"}"#).is_err());
        let event = DiagnosticEvent::CpaControl(CpaControl {
            phase: CpaControlPhase::AccountPin,
            outcome: CpaStageOutcome::Failed {
                code: CpaFailureCode::ControlPinNotApplied,
            },
            elapsed_ms: 20000,
            total_elapsed_ms: 20000,
            budget_ms: 20000,
            refresh_models: false,
            account_ordinal: Some(0),
            polls: 900,
            checks,
        });
        assert_eq!(event.level(), DiagnosticLevel::Warn);
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.len() < 1024);
        assert_eq!(
            serde_json::from_str::<DiagnosticEvent>(&json).unwrap(),
            event
        );
    }
}
