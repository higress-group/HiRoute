//! Worker-specific machine-envelope projection and terminal rendering.

use hiroute_application_api::{
    DelegationCancelV1, DelegationGetV1, DelegationListV1, DelegationResultV1,
    DelegationSubmissionOperationV1, DelegationWaitV1, ErrorCode, ErrorV1, MachineEnvelopeV2,
    MachineStatus, NextActionV1, RunStateV1, WORKER_COMMAND_DATA_SCHEMA_V1, WorkPlanListV1,
    WorkerActionFactsV1, WorkerCommandDataV1, WorkerDependenciesViewV1,
    WorkerExecutorAvailabilityListV1, WorkerHarnessV1, WorkerReadContentStateV1, WorkerReadDataV1,
    WorkerSubmissionStateV1, worker_next_actions,
};
use hiroute_client_core::{ClientFailure, FailureCode};
use serde::Serialize;
use serde_json::Value;

use crate::CliExecution;
use crate::args::OutputMode;

pub(super) fn render_status(
    response: Result<MachineEnvelopeV2<DelegationGetV1>, ClientFailure>,
    output: OutputMode,
    operation: &str,
) -> CliExecution {
    match response {
        Ok(envelope)
            if matches!(
                envelope.status,
                MachineStatus::Succeeded | MachineStatus::Accepted
            ) =>
        {
            let projected = envelope.map_data(|data| {
                command_data(CommandData {
                    operation: operation.strip_prefix("worker.").unwrap_or(operation),
                    submission_key: None,
                    task_id: Some(&data.task.task_id),
                    run_id: Some(&data.task.run.run_id),
                    state: Some(data.task.run.state),
                    title: data.task.title.as_deref(),
                    replayed: false,
                    result: None,
                    timed_out: false,
                })
            });
            render_worker_data(projected, output, false)
        }
        other => render_transport(other, output, operation, None),
    }
}

pub(super) fn render_wait(
    response: Result<MachineEnvelopeV2<DelegationWaitV1>, ClientFailure>,
    output: OutputMode,
    operation: &str,
) -> CliExecution {
    match response {
        Ok(envelope)
            if matches!(
                envelope.status,
                MachineStatus::Succeeded | MachineStatus::Accepted
            ) =>
        {
            let projected = envelope.map_data(|data| {
                command_data(CommandData {
                    operation: "wait",
                    submission_key: None,
                    task_id: Some(&data.run.task_id),
                    run_id: Some(&data.run.run_id),
                    state: Some(data.run.state),
                    title: None,
                    replayed: false,
                    result: None,
                    timed_out: data.timed_out,
                })
            });
            render_worker_data(projected, output, false)
        }
        other => render_transport(other, output, operation, None),
    }
}

pub(super) fn render_result(
    response: Result<MachineEnvelopeV2<DelegationResultV1>, ClientFailure>,
    output: OutputMode,
) -> CliExecution {
    match response {
        Ok(envelope)
            if matches!(
                envelope.status,
                MachineStatus::Succeeded | MachineStatus::Accepted
            ) =>
        {
            let projected = envelope.map_data(|data| {
                command_data(CommandData {
                    operation: "result",
                    submission_key: None,
                    task_id: Some(&data.run.task_id),
                    run_id: Some(&data.run.run_id),
                    state: Some(data.run.state),
                    title: None,
                    replayed: false,
                    result: data.text.as_deref(),
                    timed_out: false,
                })
            });
            render_worker_data(projected, output, false)
        }
        other => render_transport(other, output, "worker.result", None),
    }
}

pub(super) fn render_cancel(
    response: Result<MachineEnvelopeV2<DelegationCancelV1>, ClientFailure>,
    output: OutputMode,
) -> CliExecution {
    match response {
        Ok(envelope)
            if matches!(
                envelope.status,
                MachineStatus::Succeeded | MachineStatus::Accepted
            ) =>
        {
            let projected = envelope.map_data(|data| {
                command_data(CommandData {
                    operation: "cancel",
                    submission_key: None,
                    task_id: Some(&data.run.task_id),
                    run_id: Some(&data.run.run_id),
                    state: Some(data.run.state),
                    title: None,
                    replayed: false,
                    result: None,
                    timed_out: false,
                })
            });
            render_worker_data(projected, output, false)
        }
        other => render_transport(other, output, "worker.cancel", None),
    }
}

pub(super) struct CommandData<'a> {
    pub(super) operation: &'a str,
    pub(super) submission_key: Option<&'a str>,
    pub(super) task_id: Option<&'a str>,
    pub(super) run_id: Option<&'a str>,
    pub(super) state: Option<RunStateV1>,
    pub(super) title: Option<&'a str>,
    pub(super) replayed: bool,
    pub(super) result: Option<&'a str>,
    pub(super) timed_out: bool,
}

pub(super) fn command_data(data: CommandData<'_>) -> WorkerCommandDataV1 {
    WorkerCommandDataV1 {
        schema: WORKER_COMMAND_DATA_SCHEMA_V1.into(),
        operation: data.operation.into(),
        submission_state: WorkerSubmissionStateV1::Accepted,
        submission_key: data.submission_key.map(str::to_owned),
        task_id: data.task_id.map(str::to_owned),
        run_id: data.run_id.map(str::to_owned),
        run_state: data.state,
        title: data.title.map(str::to_owned),
        replayed: data.replayed,
        result: data.result.map(str::to_owned),
        timed_out: data.timed_out,
    }
}

pub(super) fn uncertain_submission(
    operation: &str,
    key: &str,
    failure: ClientFailure,
    output: OutputMode,
    request_id: String,
) -> CliExecution {
    let mut data = command_data(CommandData {
        operation,
        submission_key: Some(key),
        task_id: None,
        run_id: None,
        state: None,
        title: None,
        replayed: false,
        result: None,
        timed_out: false,
    });
    data.submission_state = WorkerSubmissionStateV1::Unknown;
    let code = match failure.code {
        FailureCode::ProtectedInputUnavailable | FailureCode::PeerRejected => {
            ErrorCode::CapabilityDenied
        }
        _ => ErrorCode::DaemonUnavailable,
    };
    let mut envelope = MachineEnvelopeV2::failed_with_data(
        ErrorV1::new(code),
        serde_json::to_value(data).expect("Worker data serializes"),
        Some(request_id),
    );
    envelope.next_actions = worker_next_actions(&WorkerActionFactsV1::SubmissionRecovery {
        operation: if operation == "continue" {
            DelegationSubmissionOperationV1::Continue
        } else {
            DelegationSubmissionOperationV1::Start
        },
        submission_key: key.to_owned(),
    });
    render_worker_data(envelope, output, true)
}

pub(super) fn render_transport<T: Serialize>(
    response: Result<MachineEnvelopeV2<T>, ClientFailure>,
    output: OutputMode,
    operation: &str,
    submission_key: Option<&str>,
) -> CliExecution {
    match response {
        Ok(response) => render_erased(erase(response), output, operation, submission_key),
        Err(_) => render_erased(
            MachineEnvelopeV2::failed(ErrorV1::new(ErrorCode::DaemonUnavailable), None),
            output,
            operation,
            submission_key,
        ),
    }
}

pub(super) fn erase<T: Serialize>(envelope: MachineEnvelopeV2<T>) -> MachineEnvelopeV2<Value> {
    MachineEnvelopeV2 {
        schema_version: envelope.schema_version,
        request_id: envelope.request_id,
        status: envelope.status,
        data: envelope
            .data
            .map(|data| serde_json::to_value(data).expect("typed response serializes")),
        operation: envelope.operation,
        warnings: envelope.warnings,
        next_actions: envelope.next_actions,
        error: envelope.error,
    }
}

pub(super) fn render_erased(
    envelope: MachineEnvelopeV2<Value>,
    output: OutputMode,
    operation: &str,
    submission_key: Option<&str>,
) -> CliExecution {
    if output == OutputMode::Json {
        return crate::render(envelope);
    }
    let actions = envelope.next_actions.clone();
    if !matches!(
        envelope.status,
        MachineStatus::Succeeded | MachineStatus::Accepted
    ) {
        let error = envelope.error.as_ref();
        let code = error
            .map(|error| format!("{:?}", error.code))
            .unwrap_or_else(|| "WORKER_FAILED".into());
        let key = error
            .map(|error| error.message_key.as_str())
            .unwrap_or("worker.error.unknown");
        let mut stderr = format!("{code} ({key})\n");
        if error.is_some_and(|error| error.code == ErrorCode::DaemonUnavailable) {
            stderr.push_str(crate::DAEMON_UNAVAILABLE_HINT);
        }
        if output == OutputMode::Text {
            stderr.push_str(&render_action_section(&actions));
        }
        return CliExecution {
            exit_code: envelope.status.exit_code(),
            stdout: String::new(),
            stderr,
        };
    }
    if operation == "worker.plans" {
        let plans = envelope
            .data
            .and_then(|value| serde_json::from_value::<WorkPlanListV1>(value).ok())
            .map(|value| {
                value
                    .plans
                    .into_iter()
                    .map(|plan| {
                        if output == OutputMode::Quiet {
                            plan.agent_plan_id.as_str().to_owned()
                        } else {
                            format!(
                                "{}\t{:?}\t{}",
                                plan.agent_plan_id.as_str(),
                                plan.availability,
                                plan.display_name
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let mut stdout = if plans.is_empty() {
            String::new()
        } else {
            format!("{plans}\n")
        };
        if output == OutputMode::Text {
            stdout.push_str(&render_action_section(&actions));
        }
        return CliExecution {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        };
    }
    if operation == "worker.executors" {
        let executors = envelope
            .data
            .and_then(|value| {
                serde_json::from_value::<WorkerExecutorAvailabilityListV1>(value).ok()
            })
            .filter(WorkerExecutorAvailabilityListV1::valid)
            .map(|value| {
                value
                    .executors
                    .into_iter()
                    .map(|executor| {
                        if output == OutputMode::Quiet {
                            worker_harness_wire_name(executor.harness).to_owned()
                        } else {
                            format!(
                                "{:?}\t{:?}\treason={:?}\tstart={:?}/{:?}\tcancel={:?}/{:?}\tcontinue={:?}/{:?}\trestricted={:?}/{:?}",
                                executor.harness,
                                executor.state,
                                executor.reason,
                                executor.start_approve_all.state,
                                executor.start_approve_all.reason,
                                executor.cancel.state,
                                executor.cancel.reason,
                                executor.continue_session.state,
                                executor.continue_session.reason,
                                executor.restricted_policy.state,
                                executor.restricted_policy.reason,
                            )
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let mut stdout = if executors.is_empty() {
            String::new()
        } else {
            format!("{executors}\n")
        };
        if output == OutputMode::Text {
            stdout.push_str(&render_action_section(&actions));
        }
        return CliExecution {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        };
    }
    if operation == "worker.dependencies.discover" || operation == "worker.dependencies.select" {
        let dependencies = envelope
            .data
            .and_then(|value| serde_json::from_value::<WorkerDependenciesViewV1>(value).ok())
            .filter(WorkerDependenciesViewV1::valid);
        let Some(dependencies) = dependencies else {
            return invalid_response();
        };
        let mut lines = Vec::new();
        for selected in dependencies.selected {
            if output == OutputMode::Quiet {
                lines.push(worker_harness_wire_name(selected.harness).to_owned());
            } else if selected.harness == WorkerHarnessV1::QoderCli {
                lines.push(format!(
                    "selected\t{:?}\t{}",
                    selected.harness, selected.cli_path
                ));
            } else {
                let Some(adapter_path) = selected.adapter_path.as_deref() else {
                    return invalid_response();
                };
                lines.push(format!(
                    "selected\t{:?}\t{}\t{}\t{}",
                    selected.harness,
                    selected.cli_path,
                    adapter_path,
                    selected.node_path.as_deref().unwrap_or("-")
                ));
            }
        }
        if output == OutputMode::Text {
            for candidate in dependencies.candidates {
                lines.push(format!(
                    "candidate\t{:?}\t{:?}\t{:?}\t{}",
                    candidate.harness, candidate.component, candidate.state, candidate.path
                ));
            }
        }
        let mut stdout = if lines.is_empty() {
            String::new()
        } else {
            format!("{}\n", lines.join("\n"))
        };
        if output == OutputMode::Text {
            stdout.push_str(&render_action_section(&actions));
        }
        return CliExecution {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        };
    }
    if operation == "worker.list" {
        let list = envelope
            .data
            .and_then(|value| serde_json::from_value::<DelegationListV1>(value).ok());
        let Some(list) = list else {
            return invalid_response();
        };
        let mut lines = list
            .tasks
            .into_iter()
            .map(|task| {
                if output == OutputMode::Quiet {
                    task.task_id
                } else {
                    let title = task.title.as_deref().unwrap_or(&task.task_id);
                    format!(
                        "{}\t{:?}\t{}\t{}",
                        task.task_id, task.run.state, title, task.run.scope.canonical_cwd
                    )
                }
            })
            .collect::<Vec<_>>();
        if output == OutputMode::Text
            && let Some(cursor) = list.next_cursor
        {
            lines.push(format!("Next cursor: {cursor}"));
        }
        if output == OutputMode::Text {
            let section = render_action_section(&actions);
            if !section.is_empty() {
                lines.push(section.trim_end().to_owned());
            }
        }
        return CliExecution {
            exit_code: 0,
            stdout: if lines.is_empty() {
                String::new()
            } else {
                format!("{}\n", lines.join("\n"))
            },
            stderr: String::new(),
        };
    }
    if operation == "worker.read" {
        let read = envelope
            .data
            .and_then(|value| serde_json::from_value::<WorkerReadDataV1>(value).ok())
            .filter(WorkerReadDataV1::valid);
        let Some(read) = read else {
            return invalid_response();
        };
        if output == OutputMode::Quiet {
            return CliExecution {
                exit_code: 0,
                stdout: read
                    .text
                    .map(|text| display_text(&text))
                    .unwrap_or_default(),
                stderr: String::new(),
            };
        }
        let mut stdout = format!(
            "Summary: {} | {} | {}\n",
            read.task_id,
            read.run_id,
            format!("{:?}", read.run_state).to_ascii_lowercase()
        );
        match read.content_state {
            WorkerReadContentStateV1::Pending => {
                stdout.push_str("Progress: no saved public text yet.\n");
            }
            WorkerReadContentStateV1::Available => {
                stdout.push_str("Body (public progress, this page):\n");
                let text = display_text(read.text.as_deref().unwrap_or_default());
                stdout.push_str(&text);
                if !text.ends_with('\n') {
                    stdout.push('\n');
                }
                if !read.has_more {
                    stdout.push_str("No more saved text is currently available; this does not prove completion.\n");
                }
            }
            WorkerReadContentStateV1::Deleted => stdout.push_str("Progress: deleted.\n"),
            WorkerReadContentStateV1::Expired => stdout.push_str("Progress: expired.\n"),
        }
        stdout.push_str(&render_action_section(&actions));
        return CliExecution {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        };
    }
    CliExecution {
        exit_code: 0,
        stdout: String::new(),
        stderr: submission_key
            .map(|key| format!("submission_key={key}\n"))
            .unwrap_or_default(),
    }
}

pub(super) fn render_worker_data<T: Serialize>(
    envelope: MachineEnvelopeV2<T>,
    output: OutputMode,
    submission: bool,
) -> CliExecution {
    let envelope = erase(envelope);
    if output == OutputMode::Json {
        return crate::render(envelope);
    }
    let exit_code = envelope.status.exit_code();
    let actions = envelope.next_actions.clone();
    let data = envelope
        .data
        .as_ref()
        .and_then(|value| serde_json::from_value::<WorkerCommandDataV1>(value.clone()).ok());
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(data) = data {
        if let Some(result) = data.result {
            stdout.push_str(&display_text(&result));
            if !result.ends_with('\n') {
                stdout.push('\n');
            }
        } else if output == OutputMode::Text {
            for (name, value) in [
                ("submission_key", data.submission_key.as_deref()),
                ("title", data.title.as_deref()),
                ("task_id", data.task_id.as_deref()),
                ("run_id", data.run_id.as_deref()),
            ] {
                if let Some(value) = value {
                    stdout.push_str(&format!("{name}: {value}\n"));
                }
            }
            if data.submission_state == WorkerSubmissionStateV1::Unknown {
                stdout.push_str("submission_state: unknown\n");
            }
            if let Some(state) = data.run_state {
                stdout.push_str(&format!("state: {state:?}\n").to_ascii_lowercase());
            }
            if data.timed_out {
                stdout.push_str("pending: true\n");
            }
        } else if let Some(run) = data.run_id.as_deref() {
            stderr.push_str(&format!("run_id={run}"));
            if let Some(key) = data.submission_key.as_deref() {
                stderr.push_str(&format!(" submission_key={key}"));
            }
            stderr.push('\n');
        } else if submission && let Some(key) = data.submission_key.as_deref() {
            stderr.push_str(&format!("submission_key={key}\n"));
        }
    }
    if !matches!(
        envelope.status,
        MachineStatus::Succeeded | MachineStatus::Accepted
    ) && let Some(error) = envelope.error
    {
        stderr.push_str(&format!("{:?} ({})\n", error.code, error.message_key));
    }
    if output == OutputMode::Text {
        let section = render_action_section(&actions);
        if exit_code == 0 {
            stdout.push_str(&section);
        } else {
            stderr.push_str(&section);
        }
    }
    CliExecution {
        exit_code,
        stdout,
        stderr,
    }
}

fn invalid_response() -> CliExecution {
    CliExecution {
        exit_code: 1,
        stdout: String::new(),
        stderr: "INTERNAL_ERROR (worker.error.invalid_response)\n".into(),
    }
}

fn render_action_section(actions: &[NextActionV1]) -> String {
    if actions.is_empty() {
        return String::new();
    }
    let mut output = String::from("Follow-up actions:\n");
    for action in actions {
        let (command, template) = render_action(action);
        if template {
            output.push_str("Template (fill required values before running): ");
        }
        output.push_str(&command);
        output.push_str("  # ");
        output.push_str(&display_text(&action.reason_code));
        output.push('\n');
    }
    output
}

fn render_action(action: &NextActionV1) -> (String, bool) {
    let input = action.input.as_object();
    let field = |name: &str| input.and_then(|input| input.get(name));
    let mut template = false;
    let mut required = |name: &str, placeholder: &str| match field(name) {
        Some(Value::String(value)) => shell_quote(value),
        Some(value) if !value.is_null() => shell_quote(&value.to_string()),
        _ => {
            template = true;
            shell_quote(placeholder)
        }
    };
    let optional = |name: &str, flag: &str| {
        field(name)
            .filter(|value| !value.is_null())
            .map(|value| {
                let value = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                format!(" {flag} {}", shell_quote(&value))
            })
            .unwrap_or_default()
    };
    let command = match action.command_id.as_str() {
        "worker.plans" => "hiroute worker plans".to_owned(),
        "worker.dependencies.discover" => format!(
            "hiroute worker dependencies discover{}",
            optional("harness", "--harness")
        ),
        "worker.status" if field("submission_key").is_some() => format!(
            "hiroute worker status --submission-key {} --operation {}",
            required("submission_key", "<ORIGINAL_KEY>"),
            required("operation", "<START_OR_CONTINUE>")
        ),
        "worker.status" => format!(
            "hiroute worker status --run {}",
            required("run_id", "<RUN_ID>")
        ),
        "worker.read" => format!(
            "hiroute worker read --run {}{}{}",
            required("run_id", "<RUN_ID>"),
            optional("cursor", "--cursor"),
            optional("max_bytes", "--max-bytes")
        ),
        "worker.wait" => format!(
            "hiroute worker wait --run {}{}{}",
            required("run_id", "<RUN_ID>"),
            optional("after_revision", "--after-revision"),
            optional("wait_timeout_secs", "--wait-timeout")
        ),
        "worker.result" => format!(
            "hiroute worker result --run {}{}{}",
            required("run_id", "<RUN_ID>"),
            optional("offset", "--offset"),
            optional("max_bytes", "--max-bytes")
        ),
        "worker.list" => format!(
            "hiroute worker list{} --cursor {}{}",
            optional("title", "--title"),
            required("cursor", "<CURSOR>"),
            optional("limit", "--limit")
        ),
        "worker.cancel" => format!(
            "hiroute worker cancel --run {} --idempotency-key {} --reason {}",
            required("run_id", "<RUN_ID>"),
            required("idempotency_key", "<NEW_KEY>"),
            required("reason", "<REASON>")
        ),
        "worker.continue" => format!(
            "hiroute worker continue --task {} --expected-latest-run {} --submission-key {} --permission-policy {} -- {}",
            required("task_id", "<TASK_ID>"),
            required("expected_latest_run_id", "<LATEST_RUN_ID>"),
            required("submission_key", "<NEW_KEY>"),
            required("permission_policy", "<CHOSEN_POLICY>"),
            required("input", "<NEW_INSTRUCTION>")
        ),
        _ => {
            template = true;
            format!(
                "{} {}",
                display_text(&action.command_id),
                serde_json::to_string(&action.input).unwrap_or_else(|_| "{}".into())
            )
        }
    };
    (command, template)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", display_text(value).replace('\'', "'\"'\"'"))
}

fn worker_harness_wire_name(harness: WorkerHarnessV1) -> &'static str {
    match harness {
        WorkerHarnessV1::CodexCli => "codex_cli",
        WorkerHarnessV1::ClaudeCode => "claude_code",
        WorkerHarnessV1::QoderCli => "qoder_cli",
        WorkerHarnessV1::Pi => "pi",
    }
}

fn display_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        let hidden = (character.is_control() && !matches!(character, '\n' | '\t'))
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            );
        if hidden {
            output.push_str(&format!("\\u{{{:X}}}", character as u32));
        } else {
            output.push(character);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_application_api::{
        WorkerDependencySelectionRevisionV1, WorkerDependencySelectionV1,
    };
    use serde_json::json;

    #[test]
    fn action_rendering_posix_quotes_values_and_never_hides_controls() {
        let action = NextActionV1 {
            command_id: "worker.status".into(),
            input: json!({"run_id": "run/it's\u{202e}safe"}),
            reason_code: "worker.status.inspect".into(),
        };
        let (command, template) = render_action(&action);
        assert!(!template);
        assert_eq!(
            command,
            "hiroute worker status --run 'run/it'\"'\"'s\\u{202E}safe'"
        );
    }

    #[test]
    fn null_human_choices_render_as_an_explicit_non_executable_template() {
        let action = NextActionV1 {
            command_id: "worker.cancel".into(),
            input: json!({
                "run_id": "run/one",
                "idempotency_key": null,
                "reason": null
            }),
            reason_code: "worker.cancel.confirm_required".into(),
        };
        let (command, template) = render_action(&action);
        assert!(template);
        assert_eq!(
            command,
            "hiroute worker cancel --run 'run/one' --idempotency-key '<NEW_KEY>' --reason '<REASON>'"
        );
        assert!(
            render_action_section(&[action]).starts_with(
                "Follow-up actions:\nTemplate (fill required values before running): "
            )
        );
    }

    fn dependency_view() -> WorkerDependenciesViewV1 {
        let root = std::env::current_dir().unwrap();
        let selection = |harness, name: &str| WorkerDependencySelectionV1 {
            harness,
            adapter_path: (harness != WorkerHarnessV1::QoderCli)
                .then(|| root.join(format!("{name}-adapter")).display().to_string()),
            cli_path: root.join(format!("{name}-cli")).display().to_string(),
            node_path: (harness == WorkerHarnessV1::CodexCli)
                .then(|| root.join("node").display().to_string()),
        };
        WorkerDependenciesViewV1 {
            schema: "hiroute.worker-dependencies-view/v1".into(),
            selection_revisions: vec![
                WorkerDependencySelectionRevisionV1 {
                    harness: WorkerHarnessV1::CodexCli,
                    revision: 1,
                },
                WorkerDependencySelectionRevisionV1 {
                    harness: WorkerHarnessV1::ClaudeCode,
                    revision: 2,
                },
                WorkerDependencySelectionRevisionV1 {
                    harness: WorkerHarnessV1::QoderCli,
                    revision: 3,
                },
            ],
            candidates: Vec::new(),
            selected: vec![
                selection(WorkerHarnessV1::CodexCli, "codex"),
                selection(WorkerHarnessV1::ClaudeCode, "claude"),
                selection(WorkerHarnessV1::QoderCli, "qoder"),
            ],
            install_hints: Vec::new(),
        }
    }

    #[test]
    fn dependency_quiet_output_contains_only_stable_selected_harness_names() {
        let rendered = render_transport(
            Ok::<_, ClientFailure>(MachineEnvelopeV2::succeeded(dependency_view(), None)),
            OutputMode::Quiet,
            "worker.dependencies.discover",
            None,
        );
        assert_eq!(rendered.exit_code, 0);
        assert_eq!(rendered.stdout, "codex_cli\nclaude_code\nqoder_cli\n");
        assert!(rendered.stderr.is_empty());
    }

    #[test]
    fn dependency_text_preserves_adapter_columns_only_for_adapter_harnesses() {
        let view = dependency_view();
        let codex = &view.selected[0];
        let claude = &view.selected[1];
        let qoder = &view.selected[2];
        let expected = format!(
            "selected\tCodexCli\t{}\t{}\t{}\nselected\tClaudeCode\t{}\t{}\t-\nselected\tQoderCli\t{}\n",
            codex.cli_path,
            codex.adapter_path.as_deref().unwrap(),
            codex.node_path.as_deref().unwrap(),
            claude.cli_path,
            claude.adapter_path.as_deref().unwrap(),
            qoder.cli_path,
        );
        let rendered = render_transport(
            Ok::<_, ClientFailure>(MachineEnvelopeV2::succeeded(view, None)),
            OutputMode::Text,
            "worker.dependencies.discover",
            None,
        );
        assert_eq!(rendered.exit_code, 0);
        assert_eq!(rendered.stdout, expected);
        assert!(rendered.stderr.is_empty());
    }
}
