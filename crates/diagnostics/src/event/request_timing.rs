use serde::{Deserialize, Serialize};

use super::{IngressProtocol, RequestOutcome};
use crate::identity::CorrelationToken;

/// All fields are microseconds unless named otherwise. `local` and `wait` form
/// an exclusive partition of `elapsed_us`. Async ingestion is separate work,
/// potentially concurrent with the response and with other ingestion channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTimingSummary {
    pub request_token: Option<CorrelationToken>,
    pub protocol: Option<IngressProtocol>,
    pub outcome: RequestOutcome,
    pub http_status: Option<u16>,
    pub elapsed_us: u64,
    pub local: RequestLocalTiming,
    pub wait: RequestWaitTiming,
    pub asynchronous: RequestAsyncTiming,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestLocalTiming {
    pub ingress_us: u64,
    pub replay_us: u64,
    pub parse_us: u64,
    pub externalize_us: u64,
    pub prevalidate_us: u64,
    pub history_us: u64,
    pub decision_us: u64,
    pub plan_us: u64,
    pub prepare_us: u64,
    pub stream_us: u64,
    pub other_us: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestWaitTiming {
    pub client_read_us: u64,
    pub decision_us: u64,
    pub connect_us: u64,
    pub upstream_write_us: u64,
    pub upstream_read_us: u64,
    pub downstream_write_us: u64,
    pub local_work_us: u64,
    pub other_us: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAsyncTiming {
    #[serde(default)]
    pub capture_jobs: u64,
    #[serde(default)]
    pub capture_dropped_jobs: u64,
    #[serde(default)]
    pub capture_queue_wait_max_us: u64,
    #[serde(default)]
    pub capture_work_us: u64,
    pub records: u64,
    pub delivered: u64,
    pub dropped: u64,
    pub queue_wait_max_us: u64,
    pub sink_work_us: u64,
    pub after_response_us: u64,
}
