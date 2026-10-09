//! Explicit local-user recovery, separate from Worker submission and self-query.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerResidualConfirmRequestV1 {
    pub run_id: String,
    pub expected_revision: u64,
    pub idempotency_key: String,
    pub user_confirmed: bool,
}

impl WorkerResidualConfirmRequestV1 {
    pub fn valid(&self) -> bool {
        self.user_confirmed
            && self.expected_revision > 0
            && super::worker::reference(&self.run_id)
            && hiroute_domain::validate_idempotency_key(&self.idempotency_key).is_ok()
    }
}
