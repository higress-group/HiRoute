//! One bounded execution over the small launcher. No production fallback for a missing backend.
use super::acp::{AcpRunInput, AcpRunJournal, AcpRunOutcome, run_acp_with_progress};
use super::finalization::DelegationFinalizationLease;
use super::platform::*;
use super::profile::TaskSessionRoot;
use super::progress::{ProgressBatchWriter, ProgressCapture};
use hiroute_domain::delegation::DelegationErrorV1;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{Instant, timeout, timeout_at};

pub const STOP_WAIT_MS: u64 = 10_000;
const CLAUDE_HISTORY_FLUSH_WAIT: Duration = Duration::from_secs(2);
const CLAUDE_HISTORY_QUIET: Duration = Duration::from_millis(200);
const CLAUDE_HISTORY_POLL: Duration = Duration::from_millis(25);

mod startup;

#[derive(Clone, Copy)]
enum HistoryBaseline {
    New,
    Existing(u64),
    Invalid,
}

struct ClaudeHistoryCheckpoint {
    root: PathBuf,
    baseline: HistoryBaseline,
}

impl ClaudeHistoryCheckpoint {
    fn capture(
        profile: &super::profile::CandidateWorkerProfile,
        input: &AcpRunInput,
    ) -> Option<Self> {
        if profile.identity_contract != super::acp::AcpNativeIdentityContract::ClaudeSessionV1 {
            return None;
        }
        let baseline = match &input.session {
            super::acp::AcpSessionStart::New => HistoryBaseline::New,
            super::acp::AcpSessionStart::Load(binding)
            | super::acp::AcpSessionStart::Resume(binding) => binding
                .native_session_id
                .as_deref()
                .and_then(|id| claude_history_len(&profile.session_root, id))
                .map_or(HistoryBaseline::Invalid, HistoryBaseline::Existing),
        };
        Some(Self {
            root: profile.session_root.clone(),
            baseline,
        })
    }

    async fn wait_until_settled(&self, native_session_id: Option<&str>) -> bool {
        let Some(native_session_id) = native_session_id else {
            return false;
        };
        if matches!(self.baseline, HistoryBaseline::Invalid) {
            return false;
        }
        let deadline = Instant::now() + CLAUDE_HISTORY_FLUSH_WAIT;
        let mut observed = None;
        let mut quiet_since = None;
        loop {
            let length = claude_history_len(&self.root, native_session_id);
            let changed = length.is_some_and(|length| match self.baseline {
                HistoryBaseline::New => length > 0,
                HistoryBaseline::Existing(previous) => length > previous,
                HistoryBaseline::Invalid => false,
            });
            if changed {
                if observed == length {
                    let since = quiet_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= CLAUDE_HISTORY_QUIET {
                        return true;
                    }
                } else {
                    observed = length;
                    quiet_since = Some(Instant::now());
                }
            } else {
                observed = None;
                quiet_since = None;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(CLAUDE_HISTORY_POLL).await;
        }
    }
}

fn claude_history_len(root: &std::path::Path, native_session_id: &str) -> Option<u64> {
    let transcript = TaskSessionRoot::native_transcript_path(
        root,
        hiroute_domain::delegation::WorkerHarnessV1::ClaudeCode,
        native_session_id,
    )
    .ok()?;
    fs::symlink_metadata(transcript)
        .ok()
        .filter(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        .map(|metadata| metadata.len())
}

#[derive(Clone)]
pub(crate) struct WorkerProgressWriter(Arc<dyn ProgressBatchWriter>);

impl WorkerProgressWriter {
    pub(crate) fn new(writer: Arc<dyn ProgressBatchWriter>) -> Self {
        Self(writer)
    }

    fn into_inner(self) -> Arc<dyn ProgressBatchWriter> {
        self.0
    }

    /// A prelaunch check has no native output/capture task. Record only closed host facts.
    pub(crate) fn dependency_failure(
        &self,
        failure: hiroute_domain::delegation::NativeDependencyFailureV1,
        now_ms: i64,
    ) {
        let action = if failure.reason.retryable() {
            "Retry the same dependency selection after the temporary check failure."
        } else {
            "Check the selected native executable and its required capabilities."
        };
        let text = format!("[HiRoute] Worker dependency check failed: {failure}. {action}\n");
        let _ = self.0.write_batch(&text, false, now_ms);
    }
}

pub(crate) trait WorkerRunJournal: AcpRunJournal {
    /// Production accepted runs expose an independent bounded progress writer. The default keeps
    /// protocol-only journal fixtures free of persistence concerns.
    fn progress_writer(&self) -> Option<WorkerProgressWriter> {
        None
    }
    /// Production journals acquire the shared owned lease immediately before publishing a
    /// terminal run. Probe journals have no durable continuation state and keep the default.
    fn begin_finalization(&self) -> Option<DelegationFinalizationLease> {
        None
    }
    /// Current authority check immediately before spawning (outside the admission gate).
    fn before_launch(&self) -> Result<(), DelegationErrorV1>;
    /// Persist the held-object locator before initialize/prompt. Failure triggers cleanup.
    fn process_spawned(&self, identity: &WorkerProcessIdentity) -> Result<(), DelegationErrorV1>;
    /// Immediately deny the run token and durably record termination intent before stopping.
    fn stop_intent(&self) -> Result<(), DelegationErrorV1>;
    /// Persist the result body relation and successful ACP completion before the launcher
    /// revokes the run credential and stops its owned process scope.
    fn completed(&self, outcome: &AcpRunOutcome) -> Result<(), DelegationErrorV1>;
    /// Persist an uncertain/failed execution outcome before owned-process cleanup. A prompt is
    /// never replayed merely because this method observes a disconnection.
    fn execution_failed(&self, error: DelegationErrorV1) -> Result<(), DelegationErrorV1>;
    /// A process may have started but the durable held-object binding failed. Record an explicit
    /// unknown rather than claiming that launch never happened before cleanup runs.
    fn spawned_unrecorded(&self) -> Result<(), DelegationErrorV1>;
}

pub struct WorkerRunResult {
    pub execution: Result<AcpRunOutcome, DelegationErrorV1>,
    pub identity: WorkerProcessIdentity,
    pub stop: Option<WorkerStopEvidence>,
    /// A persistence failure is not disguised as a successful cancellation.
    pub stop_intent_error: Option<DelegationErrorV1>,
    /// True when any adapter-specific native tail needed for a later Continue was durably quiet
    /// before the owned process scope was stopped. Non-Claude profiles have no extra tail gate.
    pub native_history_settled: bool,
    /// Held from immediately before terminal publication until the executor has recorded stop
    /// evidence and retained the exact native continuation materials.
    pub(super) finalization: Option<DelegationFinalizationLease>,
}

pub(crate) async fn execute(
    platform: &dyn WorkerPlatformPort,
    request: WorkerLaunchRequest,
    mut input: AcpRunInput,
    journal: Arc<dyn WorkerRunJournal>,
) -> Result<WorkerRunResult, DelegationErrorV1> {
    let platform_facts = match platform.capabilities(&request.profile) {
        Ok(facts) => facts,
        Err(error) => {
            let _ = journal.execution_failed(error);
            return Err(error);
        }
    };
    if let Err(error) = request.profile.require_platform(&platform_facts) {
        let _ = journal.execution_failed(error);
        return Err(error);
    }
    if request.profile.cwd != input.cwd
        || request.profile.identity_contract != input.identity_contract
        || request.profile.session_meta != input.session_meta
    {
        let _ = journal.execution_failed(DelegationErrorV1::InvalidArguments);
        return Err(DelegationErrorV1::InvalidArguments);
    }
    if input.cancellation.is_cancelled() {
        let _ = journal.execution_failed(DelegationErrorV1::Cancelled);
        return Err(DelegationErrorV1::Cancelled);
    }
    if input.deadline <= Instant::now() {
        let _ = journal.execution_failed(DelegationErrorV1::DeadlineExceeded);
        return Err(DelegationErrorV1::DeadlineExceeded);
    }
    let history_checkpoint = ClaudeHistoryCheckpoint::capture(&request.profile, &input);
    let startup = match startup::StartupPermit::acquire(
        request.profile.codex_initialization_root(),
        input.deadline,
        &input.cancellation,
    )
    .await
    {
        Ok(permit) => permit,
        Err(error) => {
            let _ = journal.execution_failed(error);
            return Err(error);
        }
    };
    // Waiting for another native initializer does not preserve stale launch authority.
    journal.before_launch()?;
    // Dropped launch futures must finitely release acquired resources in the platform backend.
    let launch_nonce = request.launch_nonce.clone();
    let worker = tokio::select! {
        biased;
        _ = input.cancellation.cancelled() => {
            let _ = journal.execution_failed(DelegationErrorV1::Cancelled);
            return Err(DelegationErrorV1::Cancelled);
        }
        result = timeout_at(input.deadline, platform.launch(request)) => {
            let result = match result {
                Ok(result) => result,
                Err(_) => {
                    let _ = journal.execution_failed(DelegationErrorV1::DeadlineExceeded);
                    return Err(DelegationErrorV1::DeadlineExceeded);
                }
            };
            match result {
                Ok(worker) => worker,
                Err(error) => {
                    let _ = journal.execution_failed(error);
                    return Err(error);
                }
            }
        }
    };
    if let Some(startup) = &startup {
        startup.bind_process(&worker.identity);
    }
    let binding = if worker.identity.launch_nonce == launch_nonce {
        journal.process_spawned(&worker.identity)
    } else {
        Err(DelegationErrorV1::Conflict)
    };
    let mut early_stop_error = None;
    let mut progress_capture = None;
    let execution = match binding {
        Err(error) => {
            let _ = journal.spawned_unrecorded();
            Err(error)
        }
        Ok(()) => {
            let acp_journal: Arc<dyn AcpRunJournal> = journal.clone();
            let acp_journal = startup.as_ref().map_or_else(
                || acp_journal.clone(),
                |startup| startup.journal(acp_journal.clone()),
            );
            progress_capture = journal
                .progress_writer()
                .map(WorkerProgressWriter::into_inner)
                .map(ProgressCapture::start);
            let progress_sink = progress_capture.as_ref().map(ProgressCapture::sink);
            let outer_cancel = input.cancellation.clone();
            let deadline = input.deadline;
            let acp_cancel = tokio_util::sync::CancellationToken::new();
            input.cancellation = acp_cancel.clone();
            let future = run_acp_with_progress(
                worker.stdout,
                worker.stdin,
                input,
                acp_journal,
                progress_sink,
            );
            tokio::pin!(future);
            tokio::select! {
                biased;
                _ = outer_cancel.cancelled() => {
                    // Failure remains visible through the idempotent stop_intent below.
                    early_stop_error = journal.stop_intent().err();
                    acp_cancel.cancel();
                    future.await
                },
                _ = tokio::time::sleep_until(deadline) => {
                    early_stop_error = journal.stop_intent().err();
                    acp_cancel.cancel();
                    future.await
                },
                result = &mut future => result,
            }
        }
    };
    let finalization = journal.begin_finalization();
    let execution = match execution {
        Ok(outcome) => journal.completed(&outcome).map(|()| outcome),
        Err(error) => {
            let _ = journal.execution_failed(error);
            Err(error)
        }
    };
    let stop_intent_error = early_stop_error.or(journal.stop_intent().err());
    // The terminal result and credential revocation stay authoritative. Claude's ACP prompt
    // response can precede its append-only transcript flush, so keep the owned process alive only
    // long enough to observe the exact file change become quiet. A timeout affects Continue
    // eligibility, never the already-recorded task result.
    let native_history_settled = match (&execution, history_checkpoint.as_ref()) {
        (Ok(outcome), Some(checkpoint)) if outcome.stop_reason != "cancelled" => {
            checkpoint
                .wait_until_settled(outcome.session.native_session_id.as_deref())
                .await
        }
        (Ok(_), None) => true,
        _ => false,
    };
    // Even if persistence fails, do not abandon a held child. Return both pieces of evidence.
    let stop = stop_owned(platform, &worker.identity).await;
    if let Some(capture) = progress_capture {
        capture.stop_and_flush().await;
    }
    Ok(WorkerRunResult {
        execution,
        identity: worker.identity,
        stop,
        stop_intent_error,
        native_history_settled,
        finalization,
    })
}

pub async fn observe_owned(
    platform: &dyn WorkerPlatformPort,
    identity: &WorkerProcessIdentity,
) -> WorkerObservation {
    timeout(
        Duration::from_millis(STOP_WAIT_MS),
        platform.observe(identity),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or(WorkerObservation::Unknown)
}

pub async fn stop_owned(
    platform: &dyn WorkerPlatformPort,
    identity: &WorkerProcessIdentity,
) -> Option<WorkerStopEvidence> {
    let evidence = timeout(
        Duration::from_millis(STOP_WAIT_MS),
        platform.terminate(identity, STOP_WAIT_MS),
    )
    .await
    .ok()
    .and_then(Result::ok)?;
    // Reject contradictory success evidence rather than release a live execution's slot.
    if evidence.scope_stopped && !matches!(evidence.observation, WorkerObservation::Exited { .. }) {
        return None;
    }
    startup::observe_stopped(identity, &evidence);
    Some(evidence)
}

#[cfg(test)]
mod tests;
