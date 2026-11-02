//! Host-authored explanations share ordinary Worker progress and its privacy/retention rules.
use super::WorkerProgressWriter;
use hiroute_domain::delegation::DelegationErrorV1;
use hiroute_gateway::server::dispatch::RunRequestFailure;

impl WorkerProgressWriter {
    pub(crate) fn qoder_context(&self, config_dir: &str, now_ms: i64) {
        let _ = self
            .0
            .write_batch(&qoder_context_text(config_dir), false, now_ms);
    }

    pub(crate) fn execution_failure(
        &self,
        failure: hiroute_domain::delegation::WorkerExecutionFailureV1,
        gateway: Option<RunRequestFailure>,
        config_dir: Option<&str>,
        now_ms: i64,
    ) {
        let mut text = config_dir.map(qoder_context_text).unwrap_or_default();
        text.push_str(&format!(
            "[HiRoute] Worker failure: {}.\n",
            failure.message_key()
        ));
        text.push_str(&format!("[HiRoute] {}\n", failure.next_action_hint()));
        if let DelegationErrorV1::DependencyCheckFailed(check) = failure.error {
            text.push_str(&format!(
                "[HiRoute] Worker dependency check failed: {check}.\n"
            ));
        }
        text.push_str(&execution_failure_text(failure.error, gateway));
        let _ = self.0.write_batch(&text, false, now_ms);
    }
}

fn qoder_context_text(config_dir: &str) -> String {
    // JSON quoting keeps paths with newlines/terminal controls from forging host messages.
    let config_dir = serde_json::to_string(config_dir).unwrap_or_default();
    format!(
        "[HiRoute] Qoder config directory for this run: {config_dir}. \
             This is the task's selected native context, including on Continue. \
             Installation readiness does not verify login or access to custom models. \
             Log in normally using this directory and check that account's model access. \
             To switch accounts, restore any HiRoute-managed model/Skill configuration using \
             the previous directory before changing QODER_CONFIG_DIR in the daemon environment \
             and restarting it. Existing tasks keep their original directory; use a new task \
             for the new account.\n"
    )
}

fn execution_failure_text(error: DelegationErrorV1, gateway: Option<RunRequestFailure>) -> String {
    use DelegationErrorV1 as E;
    let (code, action) = match error {
        E::PromptFailed => (
            "NATIVE_PROMPT_FAILED",
            "The native client reported an error or cancelled without a HiRoute cancel request. Check its selected account's model access and the route.",
        ),
        E::DeadlineExceeded => (
            "RUN_DEADLINE_EXCEEDED",
            "The run reached its execution deadline; this is not a user cancellation.",
        ),
        E::Cancelled => (
            "RUN_CANCELLED",
            "HiRoute requested cancellation of this run.",
        ),
        E::ProtocolFailed => (
            "ACP_PROTOCOL_FAILED",
            "The native connection failed. The prompt may have executed; inspect this run before submitting new work.",
        ),
        E::ResumeUnavailable => (
            "NATIVE_RESUME_UNAVAILABLE",
            "Check the original native context and exact session history; no replacement session was created.",
        ),
        E::CapabilityUnavailable => (
            "WORKER_CAPABILITY_UNAVAILABLE",
            "Check the selected native client's required capabilities and configuration.",
        ),
        E::DependenciesMissing
        | E::DependenciesInvalid
        | E::DependenciesUnavailable
        | E::DependencyCheckFailed(_) => (
            "WORKER_DEPENDENCY_FAILED",
            "Inspect the selected Worker dependency installation.",
        ),
        E::PermissionDenied => (
            "RUN_PERMISSION_DENIED",
            "The run's current execution authority is unavailable; this does not establish a native account entitlement failure.",
        ),
        E::ContentUnavailable => (
            "RUN_CONTENT_UNAVAILABLE",
            "Required task content is unavailable; restore its visibility before continuing.",
        ),
        E::StorageUnavailable => (
            "RUN_STORAGE_UNAVAILABLE",
            "Check local storage availability before retrying this operation.",
        ),
        E::InvalidArguments => (
            "WORKER_CONFIGURATION_INVALID",
            "Check the selected native execution configuration.",
        ),
        E::Busy | E::CapacityExceeded | E::Conflict | E::NotFound => (
            "RUN_STATE_UNAVAILABLE",
            "Inspect this run's current state before attempting another operation.",
        ),
    };
    let mut text = format!("[HiRoute] Worker execution stopped: {code}. {action}\n");
    if let Some(failure) = gateway {
        let detail = failure
            .code
            .unwrap_or("(Gateway error code unavailable; provider body not inspected)");
        text.push_str(&format!(
            "[HiRoute] Last Gateway rejection observed for this run: HTTP {} {}. \
                 This is a model-request observation, not a separate cancellation request.\n",
            failure.http_status, detail,
        ));
    }
    text
}
