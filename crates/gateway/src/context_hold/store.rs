use std::collections::{BTreeMap, BTreeSet};
use std::mem::size_of;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hiroute_gateway_core::core::execution_plan::ResolvedTargetBindingId;
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use sha2::Sha256;

use crate::server::core_runtime::profiles::HoldPreferenceV1;
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

use super::{ContextIdentityFacts, HistoryEvidence};

type HmacSha256 = Hmac<Sha256>;

pub(crate) const DEFAULT_MAX_MEMORY_BYTES: usize = 50_000_000;
pub(crate) const DEFAULT_IDLE_TTL: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ContextHoldKey([u8; 32]);

impl ContextHoldKey {
    pub(crate) fn from_request(
        authorized: &AuthorizedRequestPlan,
        ingress_protocol: IngressProtocol,
        identity: &ContextIdentityFacts,
        digest_key: &[u8; 32],
    ) -> Option<Self> {
        let (identity_kind, identity_parts) = identity.hold_identity()?;
        let receipt = authorized.receipt();
        let mut mac = HmacSha256::new_from_slice(digest_key).ok()?;
        frame(&mut mac, b"domain", b"context-hold/v1");
        frame(&mut mac, b"workspace_id", receipt.workspace_id.as_bytes());
        frame(&mut mac, b"authority_id", receipt.authority_id.as_bytes());
        frame_u64(&mut mac, b"authority_epoch", receipt.authority_epoch);
        frame(&mut mac, b"grant_id", receipt.grant_id.as_bytes());
        frame_u64(&mut mac, b"grant_generation", receipt.grant_generation);
        frame(
            &mut mac,
            b"route_identity",
            &serde_json::to_vec(&authorized.planner_policy().identity).ok()?,
        );
        frame(
            &mut mac,
            b"served_model_id",
            receipt.served_model_id.as_bytes(),
        );
        frame(
            &mut mac,
            b"route_provenance",
            &serde_json::to_vec(&receipt.route).ok()?,
        );
        frame_u64(
            &mut mac,
            b"publication_revision",
            receipt.publication_revision,
        );
        frame(
            &mut mac,
            b"publication_digest",
            receipt.publication_digest.as_bytes(),
        );
        frame(
            &mut mac,
            b"ingress_protocol",
            ingress_label(ingress_protocol),
        );
        frame(&mut mac, b"identity_kind", identity_kind.as_bytes());
        frame_u64(
            &mut mac,
            b"identity_parts_len",
            u64::try_from(identity_parts.len()).ok()?,
        );
        for part in identity_parts {
            frame(&mut mac, b"identity-part", part.as_bytes());
        }
        Some(Self(mac.finalize().into_bytes().into()))
    }
}

fn ingress_label(ingress: IngressProtocol) -> &'static [u8] {
    match ingress {
        IngressProtocol::Responses => b"responses",
        IngressProtocol::ChatCompletions => b"chat_completions",
        IngressProtocol::Messages => b"messages",
    }
}

fn frame(mac: &mut HmacSha256, kind: &[u8], value: &[u8]) {
    mac.update(&(kind.len() as u64).to_be_bytes());
    mac.update(kind);
    mac.update(&(value.len() as u64).to_be_bytes());
    mac.update(value);
}

fn frame_u64(mac: &mut HmacSha256, kind: &[u8], value: u64) {
    frame(mac, kind, &value.to_be_bytes());
}

#[derive(Clone, Debug)]
pub(crate) struct HoldTicket {
    key: ContextHoldKey,
    entry_token: u64,
    cycle: u64,
    value_version: u64,
    pub(crate) message_history_continues: bool,
    pub(crate) hint: Option<HoldPreferenceV1>,
    /// Last fully delivered candidate in this exact route scope, even when
    /// rebuilt history invalidates the stronger continuity hint.
    pub(crate) previous_success: Option<HoldPreferenceV1>,
}

#[derive(Clone, Debug)]
pub(crate) struct HoldCompletion {
    pub(crate) ticket: HoldTicket,
    pub(crate) candidates: Arc<[(ResolvedTargetBindingId, HoldPreferenceV1)]>,
}

impl HoldCompletion {
    pub(crate) fn preference_for(
        &self,
        binding: ResolvedTargetBindingId,
    ) -> Option<&HoldPreferenceV1> {
        self.candidates
            .iter()
            .find_map(|(candidate_binding, preference)| {
                (*candidate_binding == binding).then_some(preference)
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HoldCompleteOutcome {
    Applied,
    Stale,
    CapacityMiss,
}

pub(crate) struct ContextHoldStore {
    key: Option<[u8; 32]>,
    max_memory_bytes: usize,
    idle_ttl: Duration,
    inner: Mutex<StoreInner>,
}

struct StoreInner {
    entries: BTreeMap<ContextHoldKey, HoldEntry>,
    lru: BTreeSet<LruRecord>,
    next_token: u64,
    access_clock: u64,
    dynamic_bytes: usize,
    peak_accounted_bytes: usize,
    #[cfg(test)]
    maintenance_visits: usize,
}

struct HoldEntry {
    entry_token: u64,
    cycle: u64,
    checkpoint_revision: u64,
    value_version: u64,
    instruction_digest: [u8; 32],
    message_count: usize,
    history_digest: [u8; 32],
    preference: Option<HoldPreferenceV1>,
    last_access: Instant,
    access_order: u64,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct LruRecord {
    last_access: Instant,
    access_order: u64,
    key: ContextHoldKey,
}

#[derive(Clone, Copy)]
struct BeginSnapshot {
    entry_token: u64,
    cycle: u64,
    checkpoint_revision: u64,
    instruction_digest: [u8; 32],
    message_count: usize,
    history_digest: [u8; 32],
}

const MAINTENANCE_BATCH: usize = 16;
const EVICTION_BATCH: usize = 64;
const TREE_INDEX_BASE_BYTES: usize = 2_048;
const TREE_INDEX_ENTRY_BYTES: usize = 1_024;

impl Default for ContextHoldStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_MEMORY_BYTES, DEFAULT_IDLE_TTL)
    }
}

impl ContextHoldStore {
    pub(crate) fn new(max_memory_bytes: usize, idle_ttl: Duration) -> Self {
        let mut key = [0_u8; 32];
        let key = getrandom::fill(&mut key).ok().map(|()| key);
        Self {
            key,
            max_memory_bytes,
            idle_ttl,
            inner: Mutex::new(StoreInner {
                entries: BTreeMap::new(),
                lru: BTreeSet::new(),
                next_token: 1,
                access_clock: 0,
                dynamic_bytes: 0,
                peak_accounted_bytes: store_fixed_bytes(),
                #[cfg(test)]
                maintenance_visits: 0,
            }),
        }
    }

    pub(crate) fn digest_key(&self) -> Option<[u8; 32]> {
        self.key
    }

    pub(crate) fn begin(
        &self,
        key: ContextHoldKey,
        history: &HistoryEvidence<'_>,
        now: Instant,
    ) -> Option<HoldTicket> {
        self.key?;
        let snapshot = {
            let mut inner = self.inner.lock();
            inner.purge_expired_bounded(now, self.idle_ttl);
            inner.remove_if_expired(&key, now, self.idle_ttl);
            inner.entries.get(&key).map(BeginSnapshot::from)
        };

        // Scan visible history outside the mutex, retaining only the stored
        // prefix checkpoint and the final digest.
        let measured = history.measure(snapshot.map(|entry| entry.message_count))?;
        let mut inner = self.inner.lock();
        inner.remove_if_expired(&key, now, self.idle_ttl);
        if let Some(snapshot) = snapshot {
            let (current_cycle, current_value_version) = {
                let entry = inner.entries.get(&key)?;
                if !snapshot.matches_checkpoint(entry) {
                    // A newer entrance won the checkpoint CAS. This request
                    // uses the ordinary route and never overwrites history.
                    return None;
                }
                (entry.cycle, entry.value_version)
            };
            let message_history_continues = measured.message_count >= snapshot.message_count
                && measured.prefix_digest == Some(snapshot.history_digest);
            let continues = snapshot.instruction_digest == history.instruction_digest
                && message_history_continues;
            let checkpoint_revision = snapshot.checkpoint_revision.checked_add(1)?;
            let cycle = if continues {
                current_cycle
            } else {
                current_cycle.checked_add(1)?
            };
            let value_version = if continues {
                current_value_version
            } else {
                current_value_version.checked_add(1)?
            };
            let access_order = inner.allocate_access_order()?;
            let (hint, previous_success, removed_dynamic) = {
                let entry = inner.entries.get_mut(&key)?;
                let previous_success = entry.preference.clone();
                let hint = continues.then(|| entry.preference.clone()).flatten();
                let removed_dynamic = if continues {
                    0
                } else {
                    entry.preference.take().as_ref().map_or(0, preference_bytes)
                };
                entry.cycle = cycle;
                entry.checkpoint_revision = checkpoint_revision;
                entry.value_version = value_version;
                entry.instruction_digest = history.instruction_digest;
                entry.message_count = measured.message_count;
                entry.history_digest = measured.complete_digest;
                (hint, previous_success, removed_dynamic)
            };
            inner.dynamic_bytes = inner.dynamic_bytes.saturating_sub(removed_dynamic);
            inner.touch(&key, now, access_order);
            return Some(HoldTicket {
                key,
                entry_token: snapshot.entry_token,
                cycle,
                value_version,
                message_history_continues,
                hint,
                previous_success,
            });
        }

        if inner.entries.contains_key(&key) {
            // Another request inserted this scope while the history was being
            // hashed. Do not retry the scan under contention.
            return None;
        }
        let entry_token = inner.allocate_entry_token()?;
        let access_order = inner.allocate_access_order()?;
        let entry = HoldEntry {
            entry_token,
            cycle: 1,
            checkpoint_revision: 1,
            value_version: 0,
            instruction_digest: history.instruction_digest,
            message_count: measured.message_count,
            history_digest: measured.complete_digest,
            preference: None,
            last_access: now,
            access_order,
        };
        if !inner.make_room_for_insert(&entry, self.max_memory_bytes) {
            return None;
        }
        inner.entries.insert(key.clone(), entry);
        inner.lru.insert(LruRecord {
            last_access: now,
            access_order,
            key: key.clone(),
        });
        let accounted = inner.accounted_bytes();
        inner.peak_accounted_bytes = inner.peak_accounted_bytes.max(accounted);
        if accounted > self.max_memory_bytes {
            inner.remove_entry(&key);
            return None;
        }
        Some(HoldTicket {
            key,
            entry_token,
            cycle: 1,
            value_version: 0,
            message_history_continues: false,
            hint: None,
            previous_success: None,
        })
    }

    pub(crate) fn complete(
        &self,
        ticket: &HoldTicket,
        preference: HoldPreferenceV1,
        now: Instant,
    ) -> HoldCompleteOutcome {
        let mut inner = self.inner.lock();
        inner.purge_expired_bounded(now, self.idle_ttl);
        let key = &ticket.key;
        inner.remove_if_expired(key, now, self.idle_ttl);
        let Some(entry) = inner.entries.get(key) else {
            return HoldCompleteOutcome::Stale;
        };
        if entry.entry_token != ticket.entry_token
            || entry.cycle != ticket.cycle
            || entry.value_version != ticket.value_version
        {
            return HoldCompleteOutcome::Stale;
        }
        let Some(next_value_version) = entry.value_version.checked_add(1) else {
            return HoldCompleteOutcome::Stale;
        };
        if !inner.make_room_for_value(key, &preference, self.max_memory_bytes) {
            return HoldCompleteOutcome::CapacityMiss;
        }
        let Some(entry) = inner.entries.get_mut(key) else {
            return HoldCompleteOutcome::Stale;
        };
        if entry.entry_token != ticket.entry_token
            || entry.cycle != ticket.cycle
            || entry.value_version != ticket.value_version
        {
            return HoldCompleteOutcome::Stale;
        }
        let previous_dynamic = entry.preference.as_ref().map_or(0, preference_bytes);
        let next_dynamic = preference_bytes(&preference);
        entry.preference = Some(preference);
        entry.value_version = next_value_version;
        inner.dynamic_bytes = inner
            .dynamic_bytes
            .saturating_sub(previous_dynamic)
            .saturating_add(next_dynamic);
        inner.peak_accounted_bytes = inner.peak_accounted_bytes.max(inner.accounted_bytes());
        HoldCompleteOutcome::Applied
    }

    #[cfg(test)]
    pub(crate) fn accounted_bytes(&self) -> usize {
        self.inner.lock().accounted_bytes()
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        self.inner.lock().entries.len()
    }

    #[cfg(test)]
    pub(crate) fn peak_accounted_bytes(&self) -> usize {
        self.inner.lock().peak_accounted_bytes
    }

    #[cfg(test)]
    pub(crate) fn maintenance_visits(&self) -> usize {
        self.inner.lock().maintenance_visits
    }
}

impl StoreInner {
    fn purge_expired_bounded(&mut self, now: Instant, idle_ttl: Duration) {
        for _ in 0..MAINTENANCE_BATCH {
            let Some(oldest) = self.lru.first().cloned() else {
                break;
            };
            #[cfg(test)]
            {
                self.maintenance_visits = self.maintenance_visits.saturating_add(1);
            }
            if !is_expired(oldest.last_access, now, idle_ttl) {
                break;
            }
            self.remove_entry(&oldest.key);
        }
    }

    fn remove_if_expired(
        &mut self,
        key: &ContextHoldKey,
        now: Instant,
        idle_ttl: Duration,
    ) -> bool {
        let expired = self
            .entries
            .get(key)
            .is_some_and(|entry| is_expired(entry.last_access, now, idle_ttl));
        expired && self.remove_entry(key)
    }

    fn make_room_for_insert(&mut self, entry: &HoldEntry, budget: usize) -> bool {
        let dynamic = entry.dynamic_bytes();
        for _ in 0..=EVICTION_BATCH {
            let projected = retained_bytes(
                self.entries.len().saturating_add(1),
                self.dynamic_bytes.saturating_add(dynamic),
            );
            if projected <= budget {
                self.peak_accounted_bytes = self.peak_accounted_bytes.max(projected);
                return true;
            }
            if !self.evict_lru(None) {
                return false;
            }
        }
        false
    }

    fn make_room_for_value(
        &mut self,
        protected: &ContextHoldKey,
        preference: &HoldPreferenceV1,
        budget: usize,
    ) -> bool {
        for _ in 0..=EVICTION_BATCH {
            let current = self
                .entries
                .get(protected)
                .map(HoldEntry::dynamic_bytes)
                .unwrap_or(0);
            let projected = self
                .accounted_bytes()
                .saturating_sub(current)
                .saturating_add(preference_bytes(preference));
            if projected <= budget {
                self.peak_accounted_bytes = self.peak_accounted_bytes.max(projected);
                return true;
            }
            if !self.evict_lru(Some(protected)) {
                return false;
            }
        }
        false
    }

    fn evict_lru(&mut self, protected: Option<&ContextHoldKey>) -> bool {
        let victim = self
            .lru
            .iter()
            .find(|record| protected.is_none_or(|protected| &record.key != protected))
            .map(|record| record.key.clone());
        victim.is_some_and(|victim| self.remove_entry(&victim))
    }

    fn accounted_bytes(&self) -> usize {
        retained_bytes(self.entries.len(), self.dynamic_bytes)
    }

    fn allocate_entry_token(&mut self) -> Option<u64> {
        let token = self.next_token;
        self.next_token = self.next_token.checked_add(1)?;
        Some(token)
    }

    fn allocate_access_order(&mut self) -> Option<u64> {
        self.access_clock = self.access_clock.checked_add(1)?;
        Some(self.access_clock)
    }

    fn touch(&mut self, key: &ContextHoldKey, now: Instant, access_order: u64) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        self.lru.remove(&LruRecord {
            last_access: entry.last_access,
            access_order: entry.access_order,
            key: key.clone(),
        });
        entry.last_access = now;
        entry.access_order = access_order;
        self.lru.insert(LruRecord {
            last_access: now,
            access_order,
            key: key.clone(),
        });
    }

    fn remove_entry(&mut self, key: &ContextHoldKey) -> bool {
        let Some(entry) = self.entries.remove(key) else {
            return false;
        };
        self.lru.remove(&LruRecord {
            last_access: entry.last_access,
            access_order: entry.access_order,
            key: key.clone(),
        });
        self.dynamic_bytes = self.dynamic_bytes.saturating_sub(entry.dynamic_bytes());
        true
    }
}

impl BeginSnapshot {
    fn matches_checkpoint(self, entry: &HoldEntry) -> bool {
        entry.entry_token == self.entry_token
            && entry.cycle == self.cycle
            && entry.checkpoint_revision == self.checkpoint_revision
            && entry.instruction_digest == self.instruction_digest
            && entry.message_count == self.message_count
            && entry.history_digest == self.history_digest
    }
}

impl From<&HoldEntry> for BeginSnapshot {
    fn from(entry: &HoldEntry) -> Self {
        Self {
            entry_token: entry.entry_token,
            cycle: entry.cycle,
            checkpoint_revision: entry.checkpoint_revision,
            instruction_digest: entry.instruction_digest,
            message_count: entry.message_count,
            history_digest: entry.history_digest,
        }
    }
}

impl HoldEntry {
    fn dynamic_bytes(&self) -> usize {
        self.preference.as_ref().map_or(0, preference_bytes)
    }
}

fn preference_bytes(preference: &HoldPreferenceV1) -> usize {
    preference.stable_binding_id.capacity()
        + preference.candidate_id.capacity()
        + preference.profile_digest.capacity()
        + preference.reasoning_profile_id.capacity()
        + preference.origin_group_id.capacity()
}

fn tree_index_bytes(len: usize) -> usize {
    if len == 0 {
        0
    } else {
        // Covers both tree indexes, allocator headers, partially occupied
        // nodes and a node-split peak without relying on private std growth.
        TREE_INDEX_BASE_BYTES.saturating_add(len.saturating_mul(TREE_INDEX_ENTRY_BYTES))
    }
}

fn store_fixed_bytes() -> usize {
    // ContextHoldStore includes the inline Mutex/StoreInner containers. The
    // additional allowance covers allocator headers and implementation slack.
    size_of::<ContextHoldStore>().saturating_add(512)
}

fn retained_bytes(len: usize, dynamic: usize) -> usize {
    store_fixed_bytes()
        .saturating_add(tree_index_bytes(len))
        .saturating_add(dynamic)
}

fn is_expired(last_access: Instant, now: Instant, idle_ttl: Duration) -> bool {
    now.checked_duration_since(last_access)
        .is_some_and(|idle| idle >= idle_ttl)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
