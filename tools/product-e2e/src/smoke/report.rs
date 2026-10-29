use super::{Result, require};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Green,
    ExpectedRed,
    Red,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Execution {
    Executed,
    NotExecuted,
    Interrupted,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub process_exit: Option<i32>,
    pub signal: Option<i32>,
    pub evidence_digest: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CaseReport {
    pub id: String,
    pub domain: String,
    pub scope: String,
    pub owner: String,
    pub execution: Execution,
    pub state: Option<State>,
    pub reason: Option<String>,
    pub steps: Vec<Step>,
    pub build_ms: u128,
    /// Non-build case wall time, including prerequisite setup and integrity checks.
    pub execution_ms: u128,
    /// Measured outer integrity checks; a subset of execution_ms, not additive.
    #[serde(default)]
    pub integrity_ms: u128,
    pub unclassified_ms: u128,
    pub timing_complete: bool,
    pub cleanup: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema: String,
    pub run_id: String,
    pub tool_sha256: String,
    pub started_unix_ms: u128,
    pub finished_unix_ms: u128,
    pub scenario_timeout_ms: u64,
    pub step_timeout_ms: u64,
    pub source_revision: Option<String>,
    pub source_tree: Option<String>,
    pub build_input_digest: Option<String>,
    pub registry_digest: String,
    pub platform: String,
    pub requested_domains: Vec<String>,
    pub requested_cases: Vec<String>,
    pub expected_cases: Vec<String>,
    pub actual_case_count: usize,
    pub cases: Vec<CaseReport>,
    pub artifacts: Vec<super::build::Artifact>,
    pub tool_process_exit: i32,
    pub rust_test_process_exit: Option<i32>,
    pub capability_gaps: Vec<String>,
}
impl Report {
    /// Structural consistency only: imported JSON cannot attest a live product run.
    /// The private executors supply observed evidence before producing this report.
    pub fn verify(&self) -> Result<()> {
        require(
            self.schema == "hiroute.smoke.result/v1",
            "unsupported_result",
        )?;
        let selected = super::select(&self.requested_domains, &self.requested_cases)?;
        require(
            self.expected_cases == selected.iter().map(|c| c.id.to_owned()).collect::<Vec<_>>(),
            "required_cases_missing",
        )?;
        require(
            self.registry_digest == super::digest(&serde_json::to_vec(&selected)?),
            "registry_changed",
        )?;
        require(self.cases.len() == selected.len(), "case_terminal_missing")?;
        for (result, case) in self.cases.iter().zip(selected) {
            require(
                result.id == case.id
                    && result.domain == case.domain
                    && result.scope == case.scope
                    && result.owner == case.owner,
                "case_identity_mismatch",
            )?;
            require(
                if result.timing_complete {
                    result.unclassified_ms == 0
                } else {
                    result.execution_ms == 0
                },
                "ambiguous_execution_timing",
            )?;
            match result.execution {
                Execution::NotExecuted => require(
                    result.state.is_none() && result.reason.is_some(),
                    "unexecuted_has_verdict",
                )?,
                Execution::Interrupted => require(
                    result.state == Some(State::Red) && result.reason.is_some(),
                    "interrupted_has_success",
                )?,
                Execution::Executed => {
                    require(result.state.is_some(), "case_terminal_missing")?;
                    if result.state == Some(State::ExpectedRed) {
                        require(
                            case.expected_red.is_some()
                                && result.reason.as_deref() == case.expected_red,
                            "unregistered_expected_red",
                        )?;
                    }
                    if result.state == Some(State::Green) {
                        require(
                            case.execute.is_some()
                                && result.cleanup == "complete"
                                && result.reason.is_none(),
                            "false_green",
                        )?;
                        require(
                            result
                                .steps
                                .iter()
                                .map(|s| s.id.as_str())
                                .collect::<Vec<_>>()
                                == case.steps,
                            "required_step_missing",
                        )?;
                        require(
                            result.steps.iter().all(|s| {
                                s.evidence_digest.starts_with("sha256:")
                                    && s.evidence_digest.len() == 71
                            }),
                            "missing_evidence",
                        )?;
                    }
                }
            }
        }
        require(
            self.actual_case_count
                == self
                    .cases
                    .iter()
                    .filter(|c| c.execution != Execution::NotExecuted)
                    .count(),
            "case_count_mismatch",
        )?;
        let green = self.cases.iter().all(|c| {
            c.execution == Execution::Executed
                && c.state == Some(State::Green)
                && c.cleanup == "complete"
        });
        require(
            self.tool_process_exit == if green { 0 } else { 1 },
            "exit_verdict_mismatch",
        )?;
        if green {
            require(
                self.source_revision.as_ref().is_some_and(|s| s.len() == 40)
                    && self.source_tree.is_some()
                    && self.build_input_digest.is_some()
                    && !self.artifacts.is_empty(),
                "source_evidence_missing",
            )?;
        }
        Ok(())
    }
}
