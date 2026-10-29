//! Request-local reuse of an already validated candidate projection. Neither
//! runtime credentials nor mutable per-attempt cursors belong in this cache.
use std::collections::BTreeMap;

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};

use crate::server::core_runtime::adapters::PreparedNativeTemplate;
use crate::server::core_runtime::profiles::PlannerInputV1;
use crate::server::publication::PreparedCandidateExecution;
use std::sync::Arc;

pub struct PreparedPlannerInput {
    pub input: PlannerInputV1,
    pub(crate) templates: RequestTemplates,
}

pub(crate) struct RequestTemplates {
    entries: BTreeMap<String, Entry>,
    executions: BTreeMap<u32, (Arc<PreparedCandidateExecution>, Reservation)>,
    budget: StreamBudget,
    retained: usize,
}

struct Entry {
    profile: String,
    template: PreparedNativeTemplate,
    reservation: Reservation,
}

impl RequestTemplates {
    pub(crate) fn new(budget: StreamBudget) -> Self {
        Self {
            entries: BTreeMap::new(),
            executions: BTreeMap::new(),
            budget,
            retained: 0,
        }
    }

    pub(crate) fn insert(
        &mut self,
        candidate: &str,
        profile: &str,
        template: PreparedNativeTemplate,
    ) {
        let Some(bytes) = template
            .retained_bytes()
            .and_then(|size| size.checked_add(candidate.len() + profile.len() + 256))
        else {
            return;
        };
        if self.entries.contains_key(candidate)
            || self.retained.saturating_add(bytes) > 256 * 1024
            || self.budget.available_bytes().unwrap_or(0)
                < bytes.saturating_mul(2).saturating_add(64 * 1024)
        {
            return;
        }
        let Ok(reservation) = self.budget.reserve(MemoryRole::ModelIrBacking, bytes) else {
            return;
        };
        self.retained += bytes;
        self.entries.insert(
            candidate.into(),
            Entry {
                profile: profile.into(),
                template,
                reservation,
            },
        );
    }

    pub(crate) fn take(
        &mut self,
        candidate: &str,
        profile: &str,
        cleanup: Option<usize>,
    ) -> Option<PreparedNativeTemplate> {
        let entry = self.entries.remove(candidate)?;
        self.retained = self.retained.saturating_sub(entry.reservation.bytes());
        // Transfer the structural bytes to the existing sequential body owner,
        // which performs its normal admission. No cache reservation stays live.
        (cleanup.is_none() && entry.profile == profile).then_some(entry.template)
    }

    pub(crate) fn insert_execution(
        &mut self,
        binding: u32,
        execution: Arc<PreparedCandidateExecution>,
    ) {
        let Some(bytes) = execution.retained_bytes() else {
            return;
        };
        if self.executions.contains_key(&binding)
            || self.retained.saturating_add(bytes) > 256 * 1024
            || self.budget.available_bytes().unwrap_or(0)
                < bytes.saturating_mul(2).saturating_add(64 * 1024)
        {
            return;
        }
        let Ok(reservation) = self.budget.reserve(MemoryRole::ModelIrBacking, bytes) else {
            return;
        };
        self.retained += bytes;
        self.executions.insert(binding, (execution, reservation));
    }

    pub(crate) fn execution(
        &self,
        binding: u32,
        bytes: &Arc<[u8]>,
    ) -> Option<Arc<PreparedCandidateExecution>> {
        self.executions
            .get(&binding)
            .filter(|(execution, _)| Arc::ptr_eq(execution.bytes(), bytes))
            .map(|(execution, _)| execution.clone())
    }
}
