//! Request-local reuse of an already validated candidate projection. Neither
//! runtime credentials nor mutable per-attempt cursors belong in this cache.
use std::collections::BTreeMap;

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};

use crate::server::core_runtime::adapters::PreparedNativeTemplate;
use crate::server::core_runtime::profiles::PlannerInputV1;

pub struct PreparedPlannerInput {
    pub input: PlannerInputV1,
    pub(crate) templates: RequestTemplates,
}

pub(crate) struct RequestTemplates {
    entries: BTreeMap<String, Entry>,
    budget: StreamBudget,
    retained: usize,
}

struct Entry {
    profile: String,
    template: PreparedNativeTemplate,
    reservation: Reservation,
}

impl RequestTemplates {
    pub(crate) fn prepare_native_ranges(
        &mut self,
        replay: &crate::replay::ReplayStore,
        raw: &crate::content_ref::ContentRef,
    ) {
        if !self
            .entries
            .values()
            .any(|entry| !entry.template.preserved_native_roots.is_empty())
        {
            return;
        }
        let Ok(_scratch) = self
            .budget
            .reserve(MemoryRole::ModelIrBacking, self.retained)
        else {
            return;
        };
        let Some(index) = crate::replay::native_json::RawJsonIndex::capture(replay, raw) else {
            return;
        };
        for entry in self.entries.values_mut() {
            entry.template.use_native_ranges(replay, &index);
        }
    }

    pub(crate) fn new(budget: StreamBudget) -> Self {
        Self {
            entries: BTreeMap::new(),
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
}
