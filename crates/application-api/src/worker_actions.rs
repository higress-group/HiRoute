//! One deterministic producer for Worker recovery and follow-up actions.
//!
//! These facts contain only already-authorized identifiers and public state. The builder performs
//! no I/O, grants no authority, and never invents submission keys or permission choices.

use hiroute_domain::delegation::{RunStateV1, WorkerHarnessV1};
use serde_json::{Value, json};

use crate::{DelegationSubmissionOperationV1, NextActionV1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerObservedOperationV1 {
    Exec,
    Continue,
    Status,
    Wait,
    Read,
    Result,
    Cancel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerReadActionStateV1 {
    /// No read observation was made; a normal query may still be useful.
    Unknown,
    /// A valid cursor from the current visible segment. It remains useful at the current end.
    Available {
        next_cursor: String,
        max_bytes: Option<u32>,
    },
    Deleted,
    Expired,
    CursorStale,
    CursorEvicted {
        recovery_cursor: String,
        max_bytes: Option<u32>,
    },
    CursorGap {
        recovery_cursor: String,
        max_bytes: Option<u32>,
    },
    Unavailable,
    StorageUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerRunActionFactsV1 {
    pub observed_operation: WorkerObservedOperationV1,
    pub task_id: String,
    pub run_id: String,
    pub run_state: RunStateV1,
    pub state_revision: u64,
    pub read: WorkerReadActionStateV1,
    pub result_available: bool,
    pub result_next_offset: Option<u32>,
    /// Set only when the backend has a current, visible resume hint for this exact run.
    pub resume_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerActionFactsV1 {
    SubmissionRecovery {
        operation: DelegationSubmissionOperationV1,
        submission_key: String,
    },
    Run(WorkerRunActionFactsV1),
    ListPage {
        title: Option<String>,
        cursor: String,
        limit: u16,
    },
    DependencyBlocked {
        harness: Option<WorkerHarnessV1>,
    },
    PlanBlocked,
}

pub fn worker_next_actions(facts: &WorkerActionFactsV1) -> Vec<NextActionV1> {
    let mut actions = Vec::new();
    match facts {
        WorkerActionFactsV1::SubmissionRecovery {
            operation,
            submission_key,
        } => push(
            &mut actions,
            "worker.status",
            json!({"submission_key": submission_key, "operation": operation}),
            "worker.submission.recover",
        ),
        WorkerActionFactsV1::ListPage {
            title,
            cursor,
            limit,
        } => {
            let mut input = serde_json::Map::from_iter([
                ("cursor".to_owned(), Value::String(cursor.clone())),
                ("limit".to_owned(), json!(limit)),
            ]);
            if let Some(title) = title {
                input.insert("title".to_owned(), Value::String(title.clone()));
            }
            push(
                &mut actions,
                "worker.list",
                Value::Object(input),
                "worker.list.page",
            );
        }
        WorkerActionFactsV1::DependencyBlocked { harness } => {
            let input = harness.map_or_else(|| json!({}), |harness| json!({"harness": harness}));
            push(
                &mut actions,
                "worker.dependencies.discover",
                input,
                "worker.dependencies.inspect",
            );
        }
        WorkerActionFactsV1::PlanBlocked => push(
            &mut actions,
            "worker.plans",
            json!({}),
            "worker.plans.inspect",
        ),
        WorkerActionFactsV1::Run(run) => run_actions(run, &mut actions),
    }
    debug_assert!(actions.len() <= 8);
    actions
}

fn run_actions(run: &WorkerRunActionFactsV1, actions: &mut Vec<NextActionV1>) {
    match &run.read {
        WorkerReadActionStateV1::CursorEvicted {
            recovery_cursor,
            max_bytes,
        } => {
            read(
                actions,
                &run.run_id,
                Some(recovery_cursor),
                *max_bytes,
                "worker.read.resume_evicted",
            );
            status(actions, &run.run_id);
            return;
        }
        WorkerReadActionStateV1::CursorGap {
            recovery_cursor,
            max_bytes,
        } => {
            read(
                actions,
                &run.run_id,
                Some(recovery_cursor),
                *max_bytes,
                "worker.read.resume_gap",
            );
            status(actions, &run.run_id);
            return;
        }
        _ => {}
    }

    let submitted = matches!(
        run.observed_operation,
        WorkerObservedOperationV1::Exec | WorkerObservedOperationV1::Continue
    );
    match run.run_state {
        RunStateV1::Accepted | RunStateV1::Preparing | RunStateV1::Running => {
            if submitted {
                status(actions, &run.run_id);
            }
            maybe_read(actions, run);
            wait(actions, &run.run_id, run.state_revision);
            if !submitted {
                status(actions, &run.run_id);
            }
            cancel(actions, &run.run_id);
        }
        RunStateV1::Cancelling => {
            maybe_read(actions, run);
            wait(actions, &run.run_id, run.state_revision);
            status(actions, &run.run_id);
        }
        RunStateV1::Succeeded => {
            maybe_result(actions, run);
            maybe_read(actions, run);
            status(actions, &run.run_id);
            if run.resume_available {
                continue_template(actions, &run.task_id, &run.run_id);
            }
        }
        RunStateV1::Failed | RunStateV1::Cancelled | RunStateV1::Unknown => {
            status(actions, &run.run_id);
            maybe_read(actions, run);
            maybe_result(actions, run);
        }
    }
}

fn maybe_read(actions: &mut Vec<NextActionV1>, run: &WorkerRunActionFactsV1) {
    match &run.read {
        WorkerReadActionStateV1::Unknown => {
            read(actions, &run.run_id, None, None, "worker.progress.read")
        }
        WorkerReadActionStateV1::Available {
            next_cursor,
            max_bytes,
        } => read(
            actions,
            &run.run_id,
            Some(next_cursor),
            *max_bytes,
            "worker.progress.read",
        ),
        WorkerReadActionStateV1::Deleted
        | WorkerReadActionStateV1::Expired
        | WorkerReadActionStateV1::CursorStale
        | WorkerReadActionStateV1::Unavailable
        | WorkerReadActionStateV1::StorageUnavailable
        | WorkerReadActionStateV1::CursorEvicted { .. }
        | WorkerReadActionStateV1::CursorGap { .. } => {}
    }
}

fn maybe_result(actions: &mut Vec<NextActionV1>, run: &WorkerRunActionFactsV1) {
    if !run.result_available {
        return;
    }
    let mut input =
        serde_json::Map::from_iter([("run_id".to_owned(), Value::String(run.run_id.clone()))]);
    if let Some(offset) = run.result_next_offset {
        input.insert("offset".to_owned(), json!(offset));
    }
    push(
        actions,
        "worker.result",
        Value::Object(input),
        if run.result_next_offset.is_some() {
            "worker.result.page"
        } else {
            "worker.result.read"
        },
    );
}

fn status(actions: &mut Vec<NextActionV1>, run_id: &str) {
    push(
        actions,
        "worker.status",
        json!({"run_id": run_id}),
        "worker.status.inspect",
    );
}

fn read(
    actions: &mut Vec<NextActionV1>,
    run_id: &str,
    cursor: Option<&str>,
    max_bytes: Option<u32>,
    reason: &str,
) {
    let mut input =
        serde_json::Map::from_iter([("run_id".to_owned(), Value::String(run_id.to_owned()))]);
    if let Some(cursor) = cursor {
        input.insert("cursor".to_owned(), Value::String(cursor.to_owned()));
    }
    if let Some(max_bytes) = max_bytes {
        input.insert("max_bytes".to_owned(), json!(max_bytes));
    }
    push(actions, "worker.read", Value::Object(input), reason);
}

fn wait(actions: &mut Vec<NextActionV1>, run_id: &str, revision: u64) {
    push(
        actions,
        "worker.wait",
        json!({"run_id": run_id, "after_revision": revision}),
        "worker.progress.wait",
    );
}

fn cancel(actions: &mut Vec<NextActionV1>, run_id: &str) {
    push(
        actions,
        "worker.cancel",
        json!({"run_id": run_id, "idempotency_key": null, "reason": null}),
        "worker.cancel.confirm_required",
    );
}

fn continue_template(actions: &mut Vec<NextActionV1>, task_id: &str, run_id: &str) {
    push(
        actions,
        "worker.continue",
        json!({
            "task_id": task_id,
            "expected_latest_run_id": run_id,
            "input": null,
            "submission_key": null
        }),
        "worker.continue.optional_template",
    );
}

fn push(actions: &mut Vec<NextActionV1>, command: &str, input: Value, reason: &str) {
    if actions.len() < 8 {
        actions.push(NextActionV1 {
            command_id: command.to_owned(),
            input,
            reason_code: reason.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(state: RunStateV1) -> WorkerRunActionFactsV1 {
        WorkerRunActionFactsV1 {
            observed_operation: WorkerObservedOperationV1::Status,
            task_id: "task/1".into(),
            run_id: "run/1".into(),
            run_state: state,
            state_revision: 7,
            read: WorkerReadActionStateV1::Unknown,
            result_available: false,
            result_next_offset: None,
            resume_available: false,
        }
    }

    #[test]
    fn uncertain_submission_only_recovers_the_original_key() {
        let actions = worker_next_actions(&WorkerActionFactsV1::SubmissionRecovery {
            operation: DelegationSubmissionOperationV1::Continue,
            submission_key: "continue/one".into(),
        });
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].command_id, "worker.status");
        assert_eq!(actions[0].input["submission_key"], "continue/one");
        assert_eq!(actions[0].input["operation"], "continue");
    }

    #[test]
    fn active_query_actions_have_one_stable_order_and_human_templates() {
        let actions = worker_next_actions(&WorkerActionFactsV1::Run(run(RunStateV1::Running)));
        assert_eq!(
            actions
                .iter()
                .map(|action| action.command_id.as_str())
                .collect::<Vec<_>>(),
            [
                "worker.read",
                "worker.wait",
                "worker.status",
                "worker.cancel"
            ]
        );
        assert!(actions[3].input["idempotency_key"].is_null());
        assert!(actions[3].input["reason"].is_null());
    }

    #[test]
    fn a_read_gap_has_an_explicit_recovery_cursor_and_no_guessed_actions() {
        let mut facts = run(RunStateV1::Running);
        facts.read = WorkerReadActionStateV1::CursorGap {
            recovery_cursor: "signed/head".into(),
            max_bytes: Some(4096),
        };
        let actions = worker_next_actions(&WorkerActionFactsV1::Run(facts));
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].reason_code, "worker.read.resume_gap");
        assert_eq!(actions[0].input["cursor"], "signed/head");
        assert_eq!(actions[1].command_id, "worker.status");
    }

    #[test]
    fn succeeded_actions_require_real_result_and_resume_facts() {
        let mut facts = run(RunStateV1::Succeeded);
        facts.read = WorkerReadActionStateV1::Deleted;
        facts.result_available = true;
        facts.result_next_offset = Some(42);
        facts.resume_available = true;
        let actions = worker_next_actions(&WorkerActionFactsV1::Run(facts));
        assert_eq!(
            actions
                .iter()
                .map(|action| action.command_id.as_str())
                .collect::<Vec<_>>(),
            ["worker.result", "worker.status", "worker.continue"]
        );
        assert_eq!(actions[0].input["offset"], 42);
        assert!(actions[2].input["submission_key"].is_null());
        assert!(actions[2].input.get("permission_policy").is_none());
    }
}
