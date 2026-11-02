use hiroute_application_api::{
    ErrorCode, ErrorV1, MachineEnvelopeV2, MachineStatus, NextActionV1, WorkerCommandDataV1,
};
use hiroute_domain::delegation::RunStateV1;

pub(super) fn classify_terminal(response: &mut MachineEnvelopeV2<WorkerCommandDataV1>) {
    let Some(data) = response.data.as_ref() else {
        return;
    };
    let fallback = match data.run_state {
        Some(RunStateV1::Failed) => "worker.execution.failure_not_recorded",
        Some(RunStateV1::Cancelled) => "worker.execution.cancelled",
        Some(RunStateV1::Unknown) => "worker.execution.outcome_unknown",
        _ => return,
    };
    let mut error = ErrorV1::new(ErrorCode::WorkerExecutionFailed);
    error.message_key = data
        .failure
        .map_or_else(|| fallback.into(), |failure| failure.message_key());
    // A failed native connection may have executed the prompt. Never advertise automatic replay.
    error.retryable = false;
    response.status = MachineStatus::InternalError;
    response.error = Some(error);
    if let Some(run) = data.run_id.as_deref() {
        response.next_actions.push(NextActionV1 {
            command_id: "worker.read".into(),
            input: serde_json::json!({"run_id":run}),
            reason_code: "worker.failure.inspect_saved_progress_before_retry".into(),
        });
    }
}
