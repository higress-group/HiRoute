//! Best-effort rejection facts for an already authenticated Worker request.
//! No request/provider text or credential enters this channel, and it grants no authority.
use std::future::Future;

use super::VerifiedRunRequestAuthority;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunRequestFailure {
    pub http_status: u16,
    /// Present only for a fixed Gateway typed-error code. HTTP-only failures do not
    /// authorize copying or guessing a provider's code from its response body.
    pub code: Option<&'static str>,
}

tokio::task_local! {
    static RUN: Option<VerifiedRunRequestAuthority>;
}

pub(crate) async fn with_run_diagnostics<F: Future>(
    run: Option<VerifiedRunRequestAuthority>,
    future: F,
) -> F::Output {
    RUN.scope(run, future).await
}

pub(crate) fn note_run_request_failure(http_status: u16, code: &'static str) {
    report(RunRequestFailure {
        http_status,
        code: Some(code),
    });
}

pub(crate) fn note_run_http_failure(http_status: u16) {
    report(RunRequestFailure {
        http_status,
        code: None,
    });
}

fn report(failure: RunRequestFailure) {
    if !(400..=599).contains(&failure.http_status) {
        return;
    }
    let _ = RUN.try_with(|run| {
        if let Some(run) = run {
            run.note_request_failure(failure);
        }
    });
}
