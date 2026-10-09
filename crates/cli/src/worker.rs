use std::future::Future;
use std::time::Duration;

use hiroute_application_api::{
    DelegationAcceptedV1, DelegationTaskInputV1, ErrorCode, ErrorV1, MachineEnvelopeV2,
    MachineStatus, RunStateV1, WORKER_CONTINUE_SCHEMA_V1, WORKER_EXEC_SCHEMA_V1,
    WorkerContinueRequestV1, WorkerExecRequestV1, WorkerListRequestV1, WorkerPlansRequestV1,
    WorkerResultRequestV1, WorkerWaitRequestV1,
};
use hiroute_client_core::{ClientFailure, FailureCode, SubmissionState};
use serde_json::json;

use crate::args::{Globals, OutputMode};
use crate::{CliExecution, LocalControlClient};

mod input;
mod receipt;
mod render;

use input::{
    parse_cancel, parse_dependencies_discover, parse_dependency_selection, parse_list, parse_read,
    parse_residual_confirm, parse_result, parse_status, parse_submission, parse_wait,
};
use receipt::{ReceiptError, SubmissionReceipt};
use render::{
    CommandData, command_data, erase, render_cancel, render_erased, render_result, render_status,
    render_transport, render_wait, render_worker_data, uncertain_submission,
};

pub(crate) fn input_failure(code: ErrorCode, globals: Globals) -> CliExecution {
    failure(code, globals.output, globals.request_id)
}

fn failure(code: ErrorCode, output: OutputMode, request_id: Option<String>) -> CliExecution {
    render_erased(
        MachineEnvelopeV2::failed(ErrorV1::new(code), request_id),
        output,
        "worker",
        None,
    )
}

pub(crate) fn execute(command: &str, options: &[String], globals: Globals) -> CliExecution {
    let request_id = globals
        .request_id
        .clone()
        .unwrap_or_else(super::next_request_id);
    if globals.agent.is_some() || globals.capability_fd.is_some() {
        return failure(
            ErrorCode::InvalidArguments,
            globals.output,
            Some(request_id),
        );
    }
    let client = match LocalControlClient::cli_from_environment() {
        Ok(client) => {
            client.with_timeout(globals.timeout.unwrap_or_else(|| Duration::from_secs(31)))
        }
        Err(_) => {
            return failure(
                ErrorCode::DaemonUnavailable,
                globals.output,
                Some(request_id),
            );
        }
    };

    match command {
        "worker.executors" => {
            if !options.is_empty() {
                return failure(
                    ErrorCode::InvalidArguments,
                    globals.output,
                    Some(request_id),
                );
            }
            let response = call(
                client
                    .shared_client()
                    .worker_executor_availability(&request_id),
            );
            render_transport(response, globals.output, "worker.executors", None)
        }
        "worker.plans" => {
            if !options.is_empty() {
                return failure(
                    ErrorCode::InvalidArguments,
                    globals.output,
                    Some(request_id),
                );
            }
            let response = call(
                client
                    .shared_client()
                    .worker_plans(&request_id, &WorkerPlansRequestV1 {}),
            );
            render_transport(response, globals.output, "worker.plans", None)
        }
        "worker.dependencies.discover" => {
            let request = match parse_dependencies_discover(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(
                client
                    .shared_client()
                    .worker_dependencies_discover(&request_id, &request),
            );
            render_transport(
                response,
                globals.output,
                "worker.dependencies.discover",
                None,
            )
        }
        "worker.dependencies.select" => {
            let request = match parse_dependency_selection(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(
                client
                    .shared_client()
                    .select_worker_dependencies(&request_id, &request),
            );
            render_transport(response, globals.output, "worker.dependencies.select", None)
        }
        "worker.list" => {
            let request: WorkerListRequestV1 = match parse_list(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(client.shared_client().worker_list(&request_id, &request));
            render_transport(response, globals.output, "worker.list", None)
        }
        "worker.exec" => exec(options, &globals, &client, request_id),
        "worker.status" => {
            let request = match parse_status(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(client.shared_client().worker_status(&request_id, &request));
            render_status(response, globals.output, "worker.status")
        }
        "worker.wait" => {
            let request = match parse_wait(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(client.shared_client().worker_wait(&request_id, &request));
            render_wait(response, globals.output, "worker.wait")
        }
        "worker.result" => {
            let request = match parse_result(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(client.shared_client().worker_result(&request_id, &request));
            render_result(response, globals.output)
        }
        "worker.read" => {
            let request = match parse_read(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(client.shared_client().worker_read(&request_id, &request));
            render_transport(response, globals.output, "worker.read", None)
        }
        "worker.cleanup.confirm" => {
            let request = match parse_residual_confirm(options) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(
                client
                    .shared_client()
                    .confirm_worker_residual(&request_id, &request),
            );
            render_transport(response, globals.output, "worker.cleanup.confirm", None)
        }
        "worker.cancel" => {
            let request = match parse_cancel(options, &request_id) {
                Ok(request) => request,
                Err(code) => return failure(code, globals.output, Some(request_id)),
            };
            let response = call(client.shared_client().worker_cancel(&request_id, &request));
            render_cancel(response, globals.output)
        }
        "worker.continue" => continue_task(options, &globals, &client, request_id),
        _ => failure(ErrorCode::UnknownCommand, globals.output, Some(request_id)),
    }
}

fn exec(
    options: &[String],
    globals: &Globals,
    client: &LocalControlClient,
    request_id: String,
) -> CliExecution {
    let parsed = match parse_submission(options, false, &request_id) {
        Ok(parsed) => parsed,
        Err(code) => return failure(code, globals.output, Some(request_id)),
    };
    let request = WorkerExecRequestV1 {
        schema: WORKER_EXEC_SCHEMA_V1.into(),
        plan_id: parsed.plan.expect("exec parser requires Plan"),
        cwd: parsed.cwd.expect("exec parser requires cwd"),
        permission_policy: parsed.permission_policy,
        run_timeout_secs: parsed.run_timeout_secs,
        input: DelegationTaskInputV1 {
            goal: parsed.prompt,
            context: String::new(),
            constraints: String::new(),
            acceptance_criteria: String::new(),
        },
        submission_key: parsed.submission_key,
        title: parsed.title,
        parent_task_ref: parsed.parent_task_ref,
    };
    if !request.valid() {
        return failure(
            ErrorCode::InvalidArguments,
            globals.output,
            Some(request_id),
        );
    }
    let selectors = json!({"plan_id":request.plan_id,"cwd":request.cwd});
    let receipt =
        match SubmissionReceipt::prepare("exec", &request.submission_key, selectors, &request) {
            Ok(receipt) => receipt,
            Err(ReceiptError::Conflict) => {
                return failure(
                    ErrorCode::RevisionConflict,
                    globals.output,
                    Some(request_id),
                );
            }
            Err(ReceiptError::Unavailable) => {
                return failure(
                    ErrorCode::CapabilityUnavailable,
                    globals.output,
                    Some(request_id),
                );
            }
        };
    let response = call(client.shared_client().worker_exec(&request_id, &request));
    let response = match response {
        Ok(response) => {
            let locator = response
                .data
                .as_ref()
                .map(|accepted| (accepted.task_id.as_str(), accepted.run_id.as_str()));
            let _ = receipt.record(
                if response.status == MachineStatus::Succeeded
                    || response.status == MachineStatus::Accepted
                {
                    "accepted"
                } else {
                    "rejected"
                },
                locator.map(|(task, _)| task),
                locator.map(|(_, run)| run),
            );
            response
        }
        Err(error) => {
            let _ = receipt.record(
                if error.submission == SubmissionState::MayHaveReachedServer {
                    "delivery_uncertain"
                } else {
                    "not_sent"
                },
                None,
                None,
            );
            return uncertain_submission(
                "exec",
                &request.submission_key,
                error,
                globals.output,
                request_id,
            );
        }
    };
    if !matches!(
        response.status,
        MachineStatus::Succeeded | MachineStatus::Accepted
    ) || response.data.is_none()
    {
        return render_erased(
            erase(response),
            globals.output,
            "worker.exec",
            Some(&request.submission_key),
        );
    }
    finish_submission(
        "exec",
        &request.submission_key,
        response,
        parsed.no_wait,
        parsed.wait_timeout_secs,
        client,
        globals.output,
        request_id,
    )
}

fn continue_task(
    options: &[String],
    globals: &Globals,
    client: &LocalControlClient,
    request_id: String,
) -> CliExecution {
    let parsed = match parse_submission(options, true, &request_id) {
        Ok(parsed) => parsed,
        Err(code) => return failure(code, globals.output, Some(request_id)),
    };
    let request = WorkerContinueRequestV1 {
        schema: WORKER_CONTINUE_SCHEMA_V1.into(),
        task_id: parsed.task.expect("continue parser requires task"),
        expected_latest_run_id: parsed
            .expected_latest_run
            .expect("continue parser requires latest run"),
        cwd: parsed.cwd,
        permission_policy: parsed.permission_policy,
        run_timeout_secs: parsed.run_timeout_secs,
        input: DelegationTaskInputV1 {
            goal: parsed.prompt,
            context: String::new(),
            constraints: String::new(),
            acceptance_criteria: String::new(),
        },
        submission_key: parsed.submission_key,
    };
    if !request.valid() {
        return failure(
            ErrorCode::InvalidArguments,
            globals.output,
            Some(request_id),
        );
    }
    let selectors = json!({
        "task_id":request.task_id,
        "expected_latest_run_id":request.expected_latest_run_id,
        "cwd":request.cwd,
    });
    let receipt = match SubmissionReceipt::prepare(
        "continue",
        &request.submission_key,
        selectors,
        &request,
    ) {
        Ok(receipt) => receipt,
        Err(ReceiptError::Conflict) => {
            return failure(
                ErrorCode::RevisionConflict,
                globals.output,
                Some(request_id),
            );
        }
        Err(ReceiptError::Unavailable) => {
            return failure(
                ErrorCode::CapabilityUnavailable,
                globals.output,
                Some(request_id),
            );
        }
    };
    let response = call(
        client
            .shared_client()
            .worker_continue(&request_id, &request),
    );
    let response = match response {
        Ok(response) => {
            let locator = response
                .data
                .as_ref()
                .map(|accepted| (accepted.task_id.as_str(), accepted.run_id.as_str()));
            let _ = receipt.record(
                if matches!(
                    response.status,
                    MachineStatus::Succeeded | MachineStatus::Accepted
                ) {
                    "accepted"
                } else {
                    "rejected"
                },
                locator.map(|(task, _)| task),
                locator.map(|(_, run)| run),
            );
            response
        }
        Err(error) => {
            let _ = receipt.record(
                if error.submission == SubmissionState::MayHaveReachedServer {
                    "delivery_uncertain"
                } else {
                    "not_sent"
                },
                None,
                None,
            );
            return uncertain_submission(
                "continue",
                &request.submission_key,
                error,
                globals.output,
                request_id,
            );
        }
    };
    if !matches!(
        response.status,
        MachineStatus::Succeeded | MachineStatus::Accepted
    ) || response.data.is_none()
    {
        return render_erased(
            erase(response),
            globals.output,
            "worker.continue",
            Some(&request.submission_key),
        );
    }
    finish_submission(
        "continue",
        &request.submission_key,
        response,
        parsed.no_wait,
        parsed.wait_timeout_secs,
        client,
        globals.output,
        request_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_submission(
    operation: &str,
    submission_key: &str,
    accepted_response: MachineEnvelopeV2<DelegationAcceptedV1>,
    no_wait: bool,
    wait_timeout_secs: u32,
    client: &LocalControlClient,
    output: OutputMode,
    request_id: String,
) -> CliExecution {
    let accepted = accepted_response
        .data
        .as_ref()
        .expect("submission response was checked")
        .clone();
    let mut initial = Some(accepted_response.map_data(|accepted| {
        command_data(CommandData {
            operation,
            submission_key: Some(submission_key),
            task_id: Some(&accepted.task_id),
            run_id: Some(&accepted.run_id),
            state: Some(accepted.state),
            title: accepted.title.as_deref(),
            replayed: accepted.replayed,
            result: None,
            timed_out: false,
        })
    }));
    if no_wait {
        let mut response = initial.take().expect("initial response is present");
        if matches!(
            accepted.state,
            RunStateV1::Failed | RunStateV1::Cancelled | RunStateV1::Unknown
        ) {
            response.status = MachineStatus::InternalError;
            response.error = Some(ErrorV1::new(ErrorCode::Internal));
        }
        return render_worker_data(response, output, true);
    }
    let wait_request = WorkerWaitRequestV1 {
        run_id: accepted.run_id.clone(),
        after_revision: Some(accepted.state_revision),
        wait_timeout_secs,
    };
    let waited_response = match call(
        client
            .shared_client()
            .worker_wait(&request_id, &wait_request),
    ) {
        Ok(waited)
            if matches!(
                waited.status,
                MachineStatus::Succeeded | MachineStatus::Accepted
            ) && waited.data.is_some() =>
        {
            Some(waited)
        }
        _ => None,
    };
    let Some(waited_response) = waited_response else {
        return render_worker_data(
            initial.take().expect("initial response is present"),
            output,
            true,
        );
    };
    let waited = waited_response
        .data
        .as_ref()
        .expect("wait response was checked")
        .clone();
    if waited.run.state == RunStateV1::Succeeded && waited.run.result_available {
        let result = call(client.shared_client().worker_result(
            &request_id,
            &WorkerResultRequestV1 {
                run_id: accepted.run_id.clone(),
                offset: None,
                max_bytes: None,
            },
        ));
        if let Ok(result_response) = result
            && matches!(
                result_response.status,
                MachineStatus::Succeeded | MachineStatus::Accepted
            )
            && result_response.data.is_some()
        {
            let projected = result_response.map_data(|result| {
                command_data(CommandData {
                    operation,
                    submission_key: Some(submission_key),
                    task_id: Some(&accepted.task_id),
                    run_id: Some(&accepted.run_id),
                    state: Some(result.run.state),
                    title: accepted.title.as_deref(),
                    replayed: accepted.replayed,
                    result: result.text.as_deref(),
                    timed_out: false,
                })
            });
            return render_worker_data(projected, output, true);
        }
    }
    let mut projected = waited_response.map_data(|waited| {
        command_data(CommandData {
            operation,
            submission_key: Some(submission_key),
            task_id: Some(&accepted.task_id),
            run_id: Some(&accepted.run_id),
            state: Some(waited.run.state),
            title: accepted.title.as_deref(),
            replayed: accepted.replayed,
            result: None,
            timed_out: waited.timed_out,
        })
    });
    if matches!(
        waited.run.state,
        RunStateV1::Failed | RunStateV1::Cancelled | RunStateV1::Unknown
    ) {
        projected.status = MachineStatus::InternalError;
        projected.error = Some(ErrorV1::new(ErrorCode::Internal));
    }
    render_worker_data(projected, output, true)
}

fn call<F, T>(future: F) -> Result<T, ClientFailure>
where
    F: Future<Output = Result<T, ClientFailure>> + Send,
    T: Send,
{
    std::thread::scope(|scope| {
        scope
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| ClientFailure::before_send(FailureCode::TransportUnavailable))?
                    .block_on(future)
            })
            .join()
            .unwrap_or_else(|_| {
                Err(ClientFailure::before_send(
                    FailureCode::TransportUnavailable,
                ))
            })
    })
}
