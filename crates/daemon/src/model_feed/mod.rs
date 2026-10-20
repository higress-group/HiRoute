//! Service-owned HTTPS model catalog state. A catalog and its admission generation travel together.

#[cfg(feature = "integration-test-hooks")]
pub(crate) mod product_fixture;
mod recovery;
mod store;
mod transition;
pub(crate) use store::*;

use arc_swap::ArcSwap;
use hiroute_domain::CanonicalDigest;
use hiroute_integrations::{
    TrustedReleaseCatalog,
    model_feed::{FeedCounts, FeedDownload, ValidatedFeed, admit, parse_feed, retrieve_feed},
};
use serde::Serialize;
use std::{
    ops::Deref,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug)]
pub(crate) struct ReleaseCatalogSnapshot {
    pub catalog: TrustedReleaseCatalog,
    pub generation: u64,
    pub accepted_sequence: u64,
}
impl Deref for ReleaseCatalogSnapshot {
    type Target = TrustedReleaseCatalog;
    fn deref(&self) -> &Self::Target {
        &self.catalog
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ModelFeedStatus {
    pub retrieval_enabled: bool,
    pub recovery_available: bool,
    pub held: Option<HeldPublication>,
    pub highest_accepted_sequence: Option<u64>,
    pub highest_accepted_data_revision: Option<u64>,
    pub last_success_at_ms: Option<u64>,
    pub last_attempt: Option<FeedAttempt>,
    pub verification: String,
    pub counts: FeedCounts,
    pub disclosure_acknowledged: bool,
    pub next_automatic_attempt_at: Option<u64>,
    pub generation: u64,
}

struct MutableFeed {
    state: FeedState,
    counts: FeedCounts,
    refused: Option<String>,
    recovery_available: bool,
    refreshing: bool,
    retrieval_epoch: u64,
    last_refresh_result: Option<Result<(), String>>,
    next_attempt: Option<u64>,
    failures: u32,
}

pub(crate) struct ModelFeedService {
    embedded: Arc<TrustedReleaseCatalog>,
    current: ArcSwap<ReleaseCatalogSnapshot>,
    store: FeedStore,
    mutable: Mutex<MutableFeed>,
    refresh_finished: Condvar,
    active: AtomicBool,
    client_version: String,
}

impl ModelFeedService {
    pub fn open(
        root: &Path,
        embedded: TrustedReleaseCatalog,
        minimum_generation: u64,
        minimum_sequence: u64,
    ) -> Result<Self, String> {
        let store = FeedStore::open(root)?;
        let (mut state, refused, recovery_available) =
            recovery::restore_state(&store, minimum_generation, minimum_sequence)?;
        let client_version = env!("CARGO_PKG_VERSION").to_owned();
        let installed = embedded.official_snapshot();
        if refused.is_none() {
            let floor = DataFloor {
                revision: installed.data_revision,
                digest: installed.data_digest.clone(),
            };
            match &mut state.data_floor {
                Some(current) => current.advance(&floor)?,
                None => state.data_floor = Some(floor),
            }
        }
        let mut effective = embedded.clone();
        let mut counts = FeedCounts::default();
        if let Some(held) = &state.held {
            let restored = store
                .blob(held)
                .and_then(|bytes| parse_feed(&bytes).map_err(|e| e.to_string()))
                .and_then(|validated| {
                    if validated.payload.sequence != held.sequence
                        || validated.payload_digest != held.payload_digest
                    {
                        return Err("feed_cache_identity_mismatch".into());
                    }
                    if state.data_floor.as_ref().is_some_and(|f| {
                        validated.payload.data_revision == f.revision
                            && validated.payload.data_digest != f.digest
                    }) {
                        return Err("same_data_revision_conflicting_digest".into());
                    }
                    if validated.payload.data_revision < installed.data_revision {
                        return Ok((embedded.clone(), FeedCounts::default()));
                    }
                    if state
                        .data_floor
                        .as_ref()
                        .is_some_and(|f| validated.payload.data_revision < f.revision)
                    {
                        return Err("older_data_revision".into());
                    }
                    let admission = admit(&embedded, &validated.payload, &client_version)
                        .map_err(|e| e.to_string())?;
                    Ok((
                        embedded.with_feed(&admission).map_err(|e| e.to_string())?,
                        admission.counts,
                    ))
                });
            match restored {
                Ok((catalog, new_counts)) => {
                    effective = catalog;
                    counts = new_counts;
                }
                Err(_) => {
                    state.held = None;
                    state.etag = None;
                    state.generation = state
                        .generation
                        .max(minimum_generation)
                        .checked_add(1)
                        .ok_or("feed_generation_overflow")?;
                    state.last_attempt = Some(FeedAttempt {
                        at_ms: now_ms(),
                        result: "validation".into(),
                        reason: Some("discarded_invalid_cache".into()),
                    });
                    store.commit(&state)?;
                    // Only after the held pointer is durably cleared may an invalid
                    // blob be retired, allowing the same accepted bytes to be fetched again.
                    store.retire_unreferenced(None);
                }
            }
        }
        let selected = effective.official_snapshot();
        let selected = DataFloor {
            revision: selected.data_revision,
            digest: selected.data_digest,
        };
        if state
            .selected_data
            .as_ref()
            .is_some_and(|old| old != &selected)
        {
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or("feed_generation_overflow")?;
        }
        state.selected_data = Some(selected);
        if refused.is_none() {
            store.commit(&state)?;
        }
        let generation = state.generation;
        let next_attempt = state
            .retrieval_enabled
            .then(|| now_ms() + startup_delay_ms());
        Ok(Self {
            embedded: Arc::new(embedded),
            current: ArcSwap::from_pointee(ReleaseCatalogSnapshot {
                catalog: effective,
                generation,
                accepted_sequence: accepted_sequence(&state),
            }),
            store,
            mutable: Mutex::new(MutableFeed {
                state,
                counts,
                refused,
                recovery_available,
                refreshing: false,
                retrieval_epoch: 0,
                last_refresh_result: None,
                next_attempt,
                failures: 0,
            }),
            refresh_finished: Condvar::new(),
            active: AtomicBool::new(true),
            client_version,
        })
    }
    pub(crate) fn is_embedded_model(&self, id: &str) -> bool {
        self.embedded.model_data().model(id).is_some()
    }
    pub fn current(&self) -> Arc<ReleaseCatalogSnapshot> {
        self.current.load_full()
    }
    pub fn available_catalog(&self) -> Option<Arc<ReleaseCatalogSnapshot>> {
        for _ in 0..3 {
            if self.active.load(Ordering::Acquire) {
                return Some(self.current());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    }
    pub fn automatic_attempt_due(&self) -> bool {
        self.mutable.lock().is_ok_and(|guard| {
            guard.refused.is_none()
                && !guard.refreshing
                && guard.state.retrieval_enabled
                && guard.next_attempt.is_some_and(|at| at <= now_ms())
        })
    }
    pub fn status(&self) -> Result<ModelFeedStatus, String> {
        let guard = self.mutable.lock().map_err(|_| "feed_lock")?;
        let state = &guard.state;
        Ok(ModelFeedStatus {
            retrieval_enabled: state.retrieval_enabled,
            recovery_available: guard.recovery_available,
            held: state.held.clone(),
            highest_accepted_sequence: Some(accepted_sequence(state)).filter(|s| *s > 0),
            highest_accepted_data_revision: state.data_floor.as_ref().map(|f| f.revision),
            last_success_at_ms: state.last_success_at_ms,
            last_attempt: state.last_attempt.clone(),
            verification: guard.refused.clone().unwrap_or_else(|| {
                if state.held.is_some() {
                    "validated".into()
                } else {
                    "none".into()
                }
            }),
            counts: guard.counts.clone(),
            disclosure_acknowledged: state.disclosure_acknowledged,
            next_automatic_attempt_at: guard.next_attempt,
            generation: state.generation,
        })
    }
    pub fn settings(
        &self,
        enabled: Option<bool>,
        acknowledged: Option<bool>,
    ) -> Result<(), String> {
        let mut guard = self.mutable.lock().map_err(|_| "feed_lock")?;
        check_mutable(&guard)?;
        let mut next = guard.state.clone();
        if let Some(value) = enabled {
            next.retrieval_enabled = value;
            guard.retrieval_epoch = guard.retrieval_epoch.wrapping_add(1);
        }
        if let Some(value) = acknowledged {
            next.disclosure_acknowledged = value;
        }
        self.store.commit(&next)?;
        guard.state = next;
        guard.next_attempt = guard
            .state
            .retrieval_enabled
            .then(|| now_ms() + startup_delay_ms());
        Ok(())
    }
    pub fn removal_digest(&self) -> Result<CanonicalDigest, String> {
        let guard = self.mutable.lock().map_err(|_| "feed_lock")?;
        removal_digest(&guard)
    }
    #[cfg(test)]
    pub fn remove(
        &self,
        expected: &CanonicalDigest,
        disable: bool,
        install: impl FnOnce(u64, u64) -> Result<(), String>,
    ) -> Result<(), String> {
        self.remove_prepared(
            expected,
            disable,
            |_| Ok(()),
            |catalog, ()| install(catalog.generation, catalog.accepted_sequence),
        )
    }
    pub fn remove_prepared<P>(
        &self,
        expected: &CanonicalDigest,
        disable: bool,
        prepare: impl FnOnce(&ReleaseCatalogSnapshot) -> Result<P, String>,
        install: impl FnOnce(&ReleaseCatalogSnapshot, P) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut candidate = ReleaseCatalogSnapshot {
            catalog: (*self.embedded).clone(),
            generation: self
                .current()
                .generation
                .checked_add(1)
                .ok_or("feed_generation_overflow")?,
            accepted_sequence: self.current().accepted_sequence,
        };
        let prepared = prepare(&candidate)?;
        let mut guard = self.mutable.lock().map_err(|_| "feed_lock")?;
        if !guard.recovery_available {
            check_mutable(&guard)?;
        }
        if &removal_digest(&guard)? != expected {
            return Err("feed_preview_changed".into());
        }
        let mut next = guard.state.clone();
        next.held = None;
        next.etag = None;
        let embedded = self.embedded.official_snapshot();
        next.selected_data = Some(DataFloor {
            revision: embedded.data_revision,
            digest: embedded.data_digest,
        });
        next.generation = next
            .generation
            .checked_add(1)
            .ok_or("feed_generation_overflow")?;
        if disable {
            next.retrieval_enabled = false;
        }
        if guard.recovery_available {
            self.store.quarantine_state()?;
        }
        if candidate.generation != next.generation {
            return Err("feed_retrieval_superseded".into());
        }
        candidate.accepted_sequence = accepted_sequence(&next);
        self.store.commit(&next)?;
        guard.state = next;
        guard.counts = FeedCounts::default();
        guard.next_attempt = guard
            .state
            .retrieval_enabled
            .then(|| now_ms() + startup_delay_ms());
        self.active.store(false, Ordering::Release);
        if let Err(error) = install(&candidate, prepared) {
            guard.refused = Some("feed_activation_requires_restart".into());
            guard.recovery_available = false;
            // The control transaction failed atomically; its previous snapshot is still valid.
            self.active.store(true, Ordering::Release);
            return Err(error);
        }
        self.current.store(Arc::new(candidate));
        self.active.store(true, Ordering::Release);
        guard.refused = None;
        guard.recovery_available = false;
        self.store.retire_unreferenced(None);
        Ok(())
    }
}

fn accepted_sequence(state: &FeedState) -> u64 {
    state
        .highest_accepted
        .as_ref()
        .map_or(0, |h| h.sequence)
        .max(state.sequence_floor)
}
fn removal_digest(guard: &MutableFeed) -> Result<CanonicalDigest, String> {
    CanonicalDigest::of(&(&guard.state, &guard.refused, guard.recovery_available))
        .map_err(|_| "feed_digest".into())
}
fn check_mutable(guard: &MutableFeed) -> Result<(), String> {
    guard
        .refused
        .as_ref()
        .map_or(Ok(()), |reason| Err(reason.clone()))
}
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn jitter_ms(bound: u64) -> u64 {
    let mut bytes = [0; 8];
    let _ = getrandom::fill(&mut bytes);
    u64::from_le_bytes(bytes) % (bound + 1)
}
fn startup_delay_ms() -> u64 {
    2 * 60 * 1000 + jitter_ms(8 * 60 * 1000)
}

fn failure_code(kind: hiroute_integrations::model_feed::FeedFailureKind) -> &'static str {
    use hiroute_integrations::model_feed::FeedFailureKind::*;
    match kind {
        Network => "network",
        Size => "size",
        Schema => "schema",
        Validation => "validation",
        OlderSequence => "older_sequence",
    }
}
fn attempt_failure_code(reason: &str) -> &'static str {
    match reason {
        "network" => "network",
        "size" => "size",
        "schema" => "schema",
        "older_sequence" => "older_sequence",
        _ => "validation",
    }
}

#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
mod tests;
