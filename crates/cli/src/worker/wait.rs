//! CLI completion waiting composes bounded Local Control observations.

use hiroute_application_api::{
    DelegationWaitV1, MachineEnvelopeV2, MachineStatus, RunStateV1, WorkerCommandDataV1,
    WorkerResultRequestV1,
};
use hiroute_client_core::{Client, ClientFailure, FailureCode};
use tokio::time::Instant;

use super::input::ParsedWait;
use super::render::{CommandData, command_data, render_transport, render_wait};
use super::{call, failure, render_worker_data};
use crate::args::OutputMode;
use crate::{CliExecution, LocalControlClient};

pub(super) fn execute(
    parsed: ParsedWait,
    client: &LocalControlClient,
    output: OutputMode,
    request_id: &str,
) -> CliExecution {
    if parsed.request.after_revision.is_some() {
        // Suggested foreground actions retain their one-observation contract.
        return render_wait(
            call(
                client
                    .shared_client()
                    .worker_wait(request_id, &parsed.request),
            ),
            output,
            "worker.wait",
        );
    }
    let response = call(async {
        let client = client.shared_client();
        let waited = until_terminal(&client, request_id, parsed).await?;
        finish(&client, request_id, waited).await
    });
    match response {
        Ok(mut envelope) => {
            failure::classify_terminal(&mut envelope);
            render_worker_data(envelope, output, false)
        }
        Err(error) => render_transport(
            Err::<MachineEnvelopeV2<WorkerCommandDataV1>, _>(error),
            output,
            "worker.wait",
            None,
        ),
    }
}

async fn until_terminal(
    client: &Client,
    request_id: &str,
    parsed: ParsedWait,
) -> Result<MachineEnvelopeV2<DelegationWaitV1>, ClientFailure> {
    let deadline = parsed.timeout.map(|timeout| Instant::now() + timeout);
    let mut request = parsed.request;
    let mut last: Option<MachineEnvelopeV2<DelegationWaitV1>> = None;
    loop {
        let observation = client.worker_wait(request_id, &request);
        let response = if let Some(deadline) = deadline {
            match tokio::time::timeout_at(deadline, observation).await {
                Ok(response) => response?,
                Err(_) => {
                    // Dropping an observation closes its transport, never the run.
                    // The deadline may fall between the daemon's whole-second checks.
                    let mut response =
                        last.ok_or_else(|| ClientFailure::before_send(FailureCode::Deadline))?;
                    response.data.as_mut().expect("observed run").timed_out = true;
                    return Ok(response);
                }
            }
        } else {
            observation.await?
        };
        if !matches!(
            response.status,
            MachineStatus::Succeeded | MachineStatus::Accepted
        ) {
            return Ok(response);
        }
        let Some(data) = response.data.as_ref() else {
            return Ok(response);
        };
        if matches!(
            data.run.state,
            RunStateV1::Succeeded
                | RunStateV1::Failed
                | RunStateV1::Cancelled
                | RunStateV1::Unknown
        ) {
            return Ok(response);
        }
        request.after_revision = Some(data.run.state_revision);
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let mut response = response;
                response.data.as_mut().expect("observed run").timed_out = true;
                return Ok(response);
            }
            // The wire contract uses seconds; the monotonic deadline above also
            // bounds the fractional remainder without renewing the total budget.
            request.wait_timeout_secs = u32::try_from(remaining.as_secs())
                .unwrap_or(request.wait_timeout_secs)
                .clamp(1, request.wait_timeout_secs);
        }
        last = Some(response);
    }
}

async fn finish(
    client: &Client,
    request_id: &str,
    response: MachineEnvelopeV2<DelegationWaitV1>,
) -> Result<MachineEnvelopeV2<WorkerCommandDataV1>, ClientFailure> {
    if matches!(
        response.status,
        MachineStatus::Succeeded | MachineStatus::Accepted
    ) && let Some(data) = response.data.as_ref()
        && data.run.state == RunStateV1::Succeeded
        && data.run.result_available
    {
        let result = client
            .worker_result(
                request_id,
                &WorkerResultRequestV1 {
                    run_id: data.run.run_id.clone(),
                    offset: None,
                    max_bytes: None,
                },
            )
            .await?;
        return Ok(result.map_data(|result| project(result.run, result.text.as_deref(), false)));
    }
    Ok(response.map_data(|data| project(data.run, None, data.timed_out)))
}

fn project(
    run: hiroute_application_api::DelegationRunViewV1,
    result: Option<&str>,
    timed_out: bool,
) -> WorkerCommandDataV1 {
    command_data(CommandData {
        failure: run.failure,
        operation: "wait",
        submission_key: None,
        task_id: Some(&run.task_id),
        run_id: Some(&run.run_id),
        state: Some(run.state),
        title: None,
        replayed: false,
        result,
        timed_out,
    })
}
