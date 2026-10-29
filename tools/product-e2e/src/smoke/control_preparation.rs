//! Cold storage setup precedes the ordinary control/CLI scenario watchdogs.
use super::*;

const STEP: &str = "control_storage_prepare";
// Match the existing real-process publication fixture's cold-start allowance.
// This is a bounded fixture prerequisite, not a product startup SLA.
const STORAGE_PREPARATION_TIMEOUT: Duration = Duration::from_secs(45);

pub(super) fn initialize_storage(ctx: &mut Context<'_>, daemon: &Artifact) -> Result<()> {
    ctx.enter(STEP);
    ctx.verify_artifact(daemon)?;
    let storage = ctx.runtime.join("storage");
    let logs = ctx.private.join("storage-initialize");
    let cancel = ctx.cancel.clone();
    prepare_storage(ctx, &daemon.sha256, Instant::now, |deadline| {
        fixtures::initialize_current_storage(&daemon.path, &storage, &logs, cancel, deadline)
    })?;
    ctx.verify_artifact(daemon)
}

fn prepare_storage(
    ctx: &mut Context<'_>,
    daemon_sha256: &str,
    mut now: impl FnMut() -> Instant,
    initialize: impl FnOnce(Instant) -> Result<()>,
) -> Result<()> {
    ctx.enter(STEP);
    // Create the evidence destination before starting the owned daemon. Both a
    // failed prerequisite and a completed one retain their actual measured cost.
    let mut evidence_file = private_file(&ctx.private.join("control-storage-prepare.json"))?;
    let started = now();
    let deadline = started + STORAGE_PREPARATION_TIMEOUT;
    let outcome = initialize(deadline);
    let finished = now();
    let outcome =
        outcome.and_then(|()| require(finished < deadline, "initial_storage_ready_timeout"));
    let evidence = json!({
        "role":"control",
        "executable_sha256":daemon_sha256,
        "elapsed_ms":finished.duration_since(started).as_millis(),
        "timeout_ms":STORAGE_PREPARATION_TIMEOUT.as_millis(),
        "outcome":if outcome.is_ok() { "completed" } else { "failed" },
        "reason":outcome.as_ref().err().map(|error| error.0),
    });
    evidence_file.write_all(&serde_json::to_vec(&evidence)?)?;
    evidence_file.sync_all()?;
    ctx.record(STEP, None, None, &evidence)?;
    // The caller starts its unchanged product deadlines only after this real
    // prerequisite, owned-child cleanup and its synthetic inputs all succeed.
    outcome
}

#[cfg(test)]
#[path = "control_preparation_tests.rs"]
mod tests;
