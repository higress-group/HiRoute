//! Bounded observation projections. Reader contexts are internal authenticated
//! inputs, never deserialized from a client request or inferred from its name.

mod api_equivalent;
mod cache_hit;
mod catalog;
mod content;
mod facts;
mod input_selection;
mod plan_quality;
#[cfg(test)]
mod plan_quality_bounds_tests;
mod trust_cost;
pub(crate) use plan_quality::routing_reason;
mod relation;
mod requests;
mod routing_observations;
pub(crate) mod routing_timeline;
mod schema;
mod search;
#[cfg(test)]
pub(crate) mod search_tests;
#[cfg(test)]
mod search_window_tests;
mod session_search;
pub(crate) use session_search::analysis_matching_sessions;
#[cfg(test)]
mod session_search_tests;
mod sessions;
#[cfg(test)]
mod sessions_tests;
#[cfg(test)]
mod tests;
mod totals;
mod trust;
mod turns;
mod valuation;

use hiroute_domain::LogicalRequestId;
#[cfg(test)]
use hiroute_domain::WorkspaceId;

pub use hiroute_domain::{
    ObservationReaderContext, ObservationRequestPage, ObservationRequestQuery,
    ObservedRequestSummary, RunObservationLink,
};
pub(crate) use schema::migrate;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ObservationV2Error {
    #[error("observation scope is too large; narrow the time window")]
    ScopeTooLarge,
    #[error("observation scope is not authorized")]
    Unauthorized,
    #[error("observation query is invalid")]
    Invalid,
    #[error("observation relationship conflicts with prior evidence")]
    RelationshipConflict,
    #[error("observation resource is unavailable")]
    Unavailable,
    #[error("observation cursor is stale")]
    Stale,
    #[error("observation query workers are busy")]
    Busy,
    #[error("observation query reached its deadline")]
    Timeout,
}

impl From<rusqlite::Error> for ObservationV2Error {
    fn from(_error: rusqlite::Error) -> Self {
        #[cfg(test)]
        eprintln!("observation query SQLite failure: {_error:?}");
        match _error.sqlite_error_code() {
            Some(rusqlite::ErrorCode::OperationInterrupted) => Self::Timeout,
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                Self::Busy
            }
            _ => Self::Unavailable,
        }
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

pub(crate) fn invalidate_visibility(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE observation_meta SET value=CAST(value AS INTEGER)+1 WHERE key='query_visibility_generation'",[],
    )?;
    Ok(())
}

pub(crate) struct QueryPermit<'a>(&'a std::sync::atomic::AtomicUsize);

impl crate::LocalObservationStore {
    pub(crate) fn query_permit(&self) -> Result<QueryPermit<'_>, ObservationV2Error> {
        use std::sync::atomic::Ordering;
        self.query_workers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |workers| {
                (workers < 4).then_some(workers + 1)
            })
            .map_err(|_| ObservationV2Error::Busy)?;
        Ok(QueryPermit(&self.query_workers))
    }
}

impl Drop for QueryPermit<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Release);
    }
}

impl From<hiroute_domain::ObservationQueryError> for ObservationV2Error {
    fn from(error: hiroute_domain::ObservationQueryError) -> Self {
        match error {
            hiroute_domain::ObservationQueryError::Unauthorized => Self::Unauthorized,
            hiroute_domain::ObservationQueryError::InvalidQuery => Self::Invalid,
            hiroute_domain::ObservationQueryError::ReadTimeout => Self::Timeout,
            hiroute_domain::ObservationQueryError::ReaderBusy => Self::Busy,
            _ => Self::Unavailable,
        }
    }
}
impl ObservationV2Error {
    pub(crate) fn into_legacy_domain(self) -> hiroute_domain::ObservationQueryError {
        match self {
            Self::Busy | Self::Timeout => hiroute_domain::ObservationQueryError::Unavailable,
            other => other.into_domain(),
        }
    }
    pub(crate) fn into_domain(self) -> hiroute_domain::ObservationQueryError {
        use hiroute_domain::ObservationQueryError as E;
        match self {
            Self::Unauthorized => E::Unauthorized,
            Self::Invalid => E::InvalidQuery,
            Self::ScopeTooLarge => E::ScopeTooLarge,
            Self::Stale => E::StalePreview,
            Self::RelationshipConflict => E::RevisionConflict,
            Self::Busy => E::ReaderBusy,
            Self::Timeout => E::ReadTimeout,
            _ => E::Unavailable,
        }
    }
}

/// SQLite interrupt is independent of the writer and needs no optional SQLite
/// hooks. At most the four admitted read workers can own deadline waiters.
pub(crate) struct QueryDeadline(std::sync::mpsc::Sender<()>);
impl QueryDeadline {
    pub(crate) fn start_for(
        connection: &rusqlite::Connection,
        reader: &ObservationReaderContext,
    ) -> Result<Self, ObservationV2Error> {
        Self::start_inner(connection, Some(reader.clone()))
    }
    pub(crate) fn start(connection: &rusqlite::Connection) -> Result<Self, ObservationV2Error> {
        Self::start_inner(connection, None)
    }
    fn start_inner(
        connection: &rusqlite::Connection,
        reader: Option<ObservationReaderContext>,
    ) -> Result<Self, ObservationV2Error> {
        let now = std::time::Instant::now();
        let deadline = reader
            .as_ref()
            .and_then(ObservationReaderContext::query_deadline)
            .unwrap_or(now + std::time::Duration::from_millis(500));
        if deadline <= now {
            return Err(ObservationV2Error::Timeout);
        }
        connection.busy_timeout((deadline - now).min(std::time::Duration::from_millis(100)))?;
        let interrupt = connection.get_interrupt_handle();
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("observation-query-deadline".into())
            .spawn(move || {
                loop {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if reader.as_ref().is_some_and(|r| r.is_cancelled()) || remaining.is_zero() {
                        // Keep interrupting until the operation drops its guard: a
                        // later statement must not receive a fresh execution budget.
                        interrupt.interrupt();
                    }
                    if !matches!(
                        receive.recv_timeout(if remaining.is_zero() {
                            std::time::Duration::from_millis(20)
                        } else {
                            remaining.min(std::time::Duration::from_millis(20))
                        }),
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            })
            .map_err(|_error| {
                #[cfg(test)]
                eprintln!("observation query deadline thread failure: {_error:?}");
                ObservationV2Error::Unavailable
            })?;
        Ok(Self(send))
    }
}
impl Drop for QueryDeadline {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

pub(super) fn check_visibility(
    connection: &rusqlite::Connection,
    expected: u64,
) -> Result<(), ObservationV2Error> {
    let current: u64 = connection.query_row("SELECT CAST(value AS INTEGER) FROM observation_meta WHERE key='query_visibility_generation'", [], |row| row.get(0))?;
    if current != expected {
        return Err(ObservationV2Error::Stale);
    }
    Ok(())
}
