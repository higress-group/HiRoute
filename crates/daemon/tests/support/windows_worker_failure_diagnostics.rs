//! Closed facts from a real failed Worker result and the independent oracle.
//! Result bodies, task locators, transcripts and native stderr are never published.
use hiroute_application_api::{
    MACHINE_ENVELOPE_SCHEMA_V2, MachineEnvelopeV2, RunStateV1, WORKER_COMMAND_DATA_SCHEMA_V1,
    WorkerCommandDataV1,
};
use hiroute_domain::delegation::WorkerExecutionFailureV1;
use serde_json::{Value, json};

use super::{dependency_selection, oracle::Ledger};

const MAX_RESULT_BYTES: usize = 1024 * 1024;
const MAX_REQUESTS: usize = 64;

pub(super) fn collaboration_preview(value: &Value) {
    let Ok(preview) = serde_json::from_value::<hiroute_application_api::AgentSettingsPreviewV2>(
        value["data"].clone(),
    ) else {
        return;
    };
    for block in preview.preview.blockers {
        // Typed enums only: never project configuration, paths, model IDs or native output.
        println!(
            "Windows collaboration preview blocker: facet={:?}, reason={:?}, capabilities={:?}",
            block.facet, block.reason, block.capabilities
        );
    }
}

pub(super) fn receipt(
    current: &Value,
    result: &Value,
    task: &str,
    run: &str,
    exit: i32,
    ledger: &Ledger,
    poisoned: bool,
) -> Value {
    let envelope = serde_json::from_value::<MachineEnvelopeV2<Value>>(result.clone()).ok();
    let envelope_valid = envelope
        .as_ref()
        .is_some_and(|envelope| envelope.schema_version == MACHINE_ENVELOPE_SCHEMA_V2);
    let data = envelope
        .as_ref()
        .and_then(|envelope| envelope.data.as_ref())
        .and_then(|data| serde_json::from_value::<WorkerCommandDataV1>(data.clone()).ok())
        .filter(|data| data.schema == WORKER_COMMAND_DATA_SCHEMA_V1 && data.operation == "result");
    let state = serde_json::from_value::<RunStateV1>(current["data"]["run_state"].clone()).ok();
    let failure = data.as_ref().and_then(|data| data.failure).or_else(|| {
        serde_json::from_value::<WorkerExecutionFailureV1>(current["data"]["failure"].clone()).ok()
    });
    let bytes = data
        .as_ref()
        .and_then(|data| data.result.as_ref())
        .map(String::len);
    let counts = [
        ledger.requests,
        ledger.worker_skill_results,
        ledger.worker_write_results,
        ledger.worker_read_results,
        ledger.worker_continuation_requests,
        ledger.continued,
    ];
    json!({
        "schema":"hiroute.windows-worker-terminal-facts/v1",
        "result_exit":exit,
        "error_code":dependency_selection::error_code(result),
        "error_key":dependency_selection::message_key(result),
        "envelope_schema_valid":envelope_valid,
        "result_schema_valid":data.is_some(),
        "observed_run_state":state,
        "result_run_state":data.as_ref().and_then(|data| data.run_state),
        "result_timed_out":data.as_ref().map(|data| data.timed_out),
        "result_present":bytes.is_some(),
        "result_bytes":bytes.map(|bytes| bytes.min(MAX_RESULT_BYTES)),
        "result_bytes_exceeded_bound":bytes.is_some_and(|bytes| bytes>MAX_RESULT_BYTES),
        "task_identity_equal":data.as_ref().is_some_and(|data| data.task_id.as_deref()==Some(task)),
        "run_identity_equal":data.as_ref().is_some_and(|data| data.run_id.as_deref()==Some(run)),
        "typed_failure_reason_available":failure.is_some(),
        "typed_failure_message_key":failure.map(WorkerExecutionFailureV1::message_key),
        "oracle_scope":"oracle-lifetime",
        "per_run_stage_known":false,
        "oracle_poisoned":poisoned,
        "oracle_stage":ledger.worker_stage.map(|stage| stage.label()).unwrap_or("unknown"),
        "oracle_requests":ledger.requests.min(MAX_REQUESTS),
        "oracle_skill_results":ledger.worker_skill_results.min(MAX_REQUESTS),
        "oracle_write_results":ledger.worker_write_results.min(MAX_REQUESTS),
        "oracle_read_results":ledger.worker_read_results.min(MAX_REQUESTS),
        "oracle_continuation_requests":ledger.worker_continuation_requests.min(MAX_REQUESTS),
        "oracle_validated_continues":ledger.continued.min(MAX_REQUESTS),
        "oracle_counts_exceeded_bound":counts.into_iter().any(|count| count>MAX_REQUESTS),
        "oracle_direct_completed":ledger.completed.contains("direct"),
        "oracle_delegated_completed":ledger.completed.contains("delegated")
    })
}

#[cfg(test)]
mod tests {
    use super::super::oracle::WorkerStage;
    use super::*;
    use hiroute_application_api::{ErrorCode, ErrorV1, WorkerSubmissionStateV1};

    #[test]
    fn terminal_facts_publish_typed_states_and_correlations_without_result_or_locator_text() {
        let private = "private-native-text-or-path";
        let data = WorkerCommandDataV1 {
            failure: None,
            schema: WORKER_COMMAND_DATA_SCHEMA_V1.into(),
            operation: "result".into(),
            submission_state: WorkerSubmissionStateV1::Accepted,
            submission_key: None,
            task_id: Some(private.into()),
            run_id: Some(private.into()),
            run_state: Some(RunStateV1::Failed),
            title: Some(private.into()),
            replayed: false,
            result: Some(private.into()),
            timed_out: false,
        };
        let mut envelope = MachineEnvelopeV2::succeeded(data, Some(private.into()));
        let mut error = ErrorV1::new(ErrorCode::CapabilityUnavailable);
        error.message_key = private.into();
        envelope.error = Some(error);
        let mut ledger = Ledger {
            requests: 3,
            worker_skill_results: 1,
            worker_write_results: 1,
            worker_stage: Some(WorkerStage::WriteResult),
            ..Default::default()
        };
        ledger.completed.insert(private.into());
        let facts = receipt(
            &json!({"data":{"run_state":"failed"}}),
            &serde_json::to_value(envelope).unwrap(),
            private,
            private,
            0,
            &ledger,
            false,
        );
        assert!(facts["envelope_schema_valid"].as_bool().unwrap());
        assert_eq!(facts["result_run_state"], "failed");
        assert_eq!(facts["oracle_stage"], "correlated-write-result-observed");
        assert_eq!(facts["oracle_write_results"], 1);
        assert_eq!(facts["task_identity_equal"], true);
        assert_eq!(facts["run_identity_equal"], true);
        assert_eq!(facts["error_code"], "CAPABILITY_UNAVAILABLE");
        assert_eq!(facts["error_key"], "unclassified-key");
        assert_eq!(facts["per_run_stage_known"], false);
        assert!(!facts.to_string().contains(private));
    }

    #[test]
    fn unknown_terminal_shapes_and_excessive_counts_remain_closed_and_unattributed() {
        let private = "native-auth-payload-or-foreign-path";
        let input = json!({"schema_version":{"major":2,"minor":0},"status":"succeeded",
            "data":{"schema":private,"operation":"result","run_state":private,"result":private},
            "error":{"code":private,"message_key":private}});
        let ledger = Ledger {
            requests: usize::MAX,
            worker_read_results: usize::MAX,
            ..Default::default()
        };
        let facts = receipt(
            &json!({"data":{"run_state":private}}),
            &input,
            private,
            private,
            6,
            &ledger,
            true,
        );
        assert_eq!(facts["envelope_schema_valid"], false);
        assert_eq!(facts["result_schema_valid"], false);
        assert!(facts["observed_run_state"].is_null());
        assert_eq!(facts["oracle_poisoned"], true);
        assert_eq!(facts["oracle_requests"], MAX_REQUESTS);
        assert_eq!(facts["oracle_counts_exceeded_bound"], true);
        assert_eq!(facts["oracle_stage"], "unknown");
        assert_eq!(facts["task_identity_equal"], false);
        assert!(!facts.to_string().contains(private));
    }
}
