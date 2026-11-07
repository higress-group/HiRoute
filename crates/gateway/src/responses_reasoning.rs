//! Optional source evidence for a native Responses replay. A cache miss keeps the
//! original payload. The evidence never authorizes a request or a candidate.
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::sync::Arc;

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};
use parking_lot::Mutex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::content_ref::ContentValueExt;
use crate::context_hold::{ContextHoldKey, ContextHoldStore, ContextIdentityFacts};
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::{ContentPart, ModelRequestIRV1};
use crate::server::core_runtime::profiles::CandidateProtocolProfile;
use crate::server::publication::SealedCandidateExecutionV1;
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

const RETAINED_ITEMS: usize = 8192;
const REQUEST_ITEMS: usize = 512;
const RESPONSE_ITEMS: usize = 128;
const HASH_BYTES: usize = 4 * 1024 * 1024;

type Fingerprint = [u8; 32];
type ItemKey = (ContextHoldKey, Fingerprint);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReasoningSource(Fingerprint);

impl ReasoningSource {
    pub(crate) fn for_execution(execution: &SealedCandidateExecutionV1) -> Self {
        Self::for_profile(
            &execution.protocol_profile,
            execution.operational_target.uri(),
            &execution.credential_destination_ref,
        )
    }

    pub(crate) fn for_profile(
        profile: &CandidateProtocolProfile,
        endpoint: &str,
        destination: &str,
    ) -> Self {
        let (endpoint, destination) =
            profile
                .native_target
                .as_ref()
                .map_or((endpoint, destination), |target| {
                    (
                        target.operational_target.uri(),
                        target.credential_destination_ref.as_str(),
                    )
                });
        Self(
            hash_identity(endpoint, destination.as_bytes())
                .finalize()
                .into(),
        )
    }
}

#[derive(Default)]
pub(crate) struct ReasoningStore(Mutex<RetainedItems>);

#[derive(Default)]
struct RetainedItems {
    order: VecDeque<ItemKey>,
    // Conflicting deliveries in the same scope remain ambiguous until evicted.
    sources: HashMap<ItemKey, Option<ReasoningSource>>,
    scopes: HashMap<ContextHoldKey, usize>,
}

#[derive(Clone)]
pub(crate) struct ReasoningOrigin {
    store: Arc<ReasoningStore>,
    scope: ContextHoldKey,
    source: ReasoningSource,
}

impl ReasoningOrigin {
    pub(crate) fn record(&self, fingerprints: &[Fingerprint]) {
        if fingerprints.is_empty() {
            return;
        }
        let mut items = self.store.0.lock();
        for fingerprint in fingerprints {
            let key = (self.scope.clone(), *fingerprint);
            if let Some(known) = items.sources.get_mut(&key) {
                if *known != Some(self.source) {
                    *known = None;
                }
                continue;
            }
            if items.order.len() == RETAINED_ITEMS {
                let oldest = items.order.pop_front().expect("full reasoning store");
                items.sources.remove(&oldest);
                let count = items.scopes.get_mut(&oldest.0).expect("retained scope");
                *count -= 1;
                if *count == 0 {
                    items.scopes.remove(&oldest.0);
                }
            }
            *items.scopes.entry(self.scope.clone()).or_default() += 1;
            items.order.push_back(key.clone());
            items.sources.insert(key, Some(self.source));
        }
    }
}

/// Request-owned candidate projection facts, separate from canonical history.
#[derive(Default)]
pub struct ResponsesReasoningReplay {
    scope: Option<(Arc<ReasoningStore>, ContextHoldKey)>,
    sources: Vec<(usize, ReasoningSource)>,
    _memory: Option<Reservation>,
}

impl ResponsesReasoningReplay {
    pub(crate) fn prepare(
        store: &Arc<ReasoningStore>,
        holds: &ContextHoldStore,
        authorized: &AuthorizedRequestPlan,
        identity: &ContextIdentityFacts,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
    ) -> Self {
        if request.ingress_protocol != IngressProtocol::Responses
            || authorized.candidates().len() < 2
            || !matches!(
                identity.correlation_provenance(),
                "agent_supplied" | "protocol_state"
            )
        {
            return Self::default();
        }
        let Some(scope) = holds.digest_key().and_then(|key| {
            ContextHoldKey::routing_history(authorized, request.ingress_protocol, identity, &key)
        }) else {
            return Self::default();
        };
        let mut result = Self {
            scope: Some((Arc::clone(store), scope.clone())),
            ..Self::default()
        };
        let capacity = request.responses_reasoning_history.len().min(REQUEST_ITEMS);
        if capacity == 0 || !store.0.lock().scopes.contains_key(&scope) {
            return result;
        }
        let Some(layout) = request.native_history_layout() else {
            return result;
        };
        let bytes_per_item = size_of::<(usize, Fingerprint)>()
            + size_of::<(usize, ReasoningSource)>()
            + size_of::<usize>();
        let Ok(memory) = replay
            .budget()
            .reserve(MemoryRole::ModelIrBacking, capacity * bytes_per_item)
        else {
            return result;
        };
        let mut remaining = HASH_BYTES;
        let mut fingerprints = Vec::with_capacity(capacity);
        fingerprints.extend(
            request
                .responses_reasoning_history
                .keys()
                .rev()
                .take(capacity)
                .filter_map(|&index| {
                    if !layout[index].modeled {
                        return None;
                    }
                    let id = request.responses_item_ids.get(&index)?;
                    let encrypted = match request.messages[index].content.as_slice() {
                        [] => None,
                        [ContentPart::ProviderState { state }]
                            if state.kind == "encrypted_content" =>
                        {
                            Some(state.value.as_str()?)
                        }
                        _ => return None,
                    };
                    fingerprint(id, encrypted, Some(replay), &mut remaining)
                        .map(|digest| (index, digest))
                }),
        );
        {
            let items = store.0.lock();
            result.sources.reserve_exact(fingerprints.len());
            result.sources.extend(
                fingerprints
                    .into_iter()
                    .rev()
                    .filter_map(|(index, digest)| {
                        let source = (*items.sources.get(&(scope.clone(), digest))?)?;
                        Some((index, source))
                    }),
            );
        }
        if result.sources.is_empty() {
            result.sources = Vec::new();
        } else {
            result._memory = Some(memory);
        }
        result
    }

    pub(crate) fn has_matched_items(&self) -> bool {
        !self.sources.is_empty()
    }

    pub(crate) fn foreign_positions(&self, target: impl FnOnce() -> ReasoningSource) -> Vec<usize> {
        if self.sources.is_empty() {
            return Vec::new();
        }
        let target = target();
        self.sources
            .iter()
            .filter_map(|(index, source)| (*source != target).then_some(*index))
            .collect()
    }

    pub(crate) fn origin(
        &self,
        source: impl FnOnce() -> ReasoningSource,
    ) -> Option<ReasoningOrigin> {
        let (store, scope) = self.scope.as_ref()?;
        Some(ReasoningOrigin {
            store: Arc::clone(store),
            scope: scope.clone(),
            source: source(),
        })
    }
}

/// Capture only bounded fingerprints from the actual native response. A
/// completed, delivered snapshot is required before publishing any evidence.
pub(crate) struct ReasoningCapture {
    items: Vec<Fingerprint>,
    remaining: usize,
    completed: bool,
    _memory: Reservation,
}

impl ReasoningCapture {
    pub(crate) fn new(budget: &StreamBudget) -> Option<Self> {
        let memory = budget
            .reserve(
                MemoryRole::SemanticState,
                RESPONSE_ITEMS * size_of::<Fingerprint>(),
            )
            .ok()?;
        Some(Self {
            items: Vec::with_capacity(RESPONSE_ITEMS),
            remaining: HASH_BYTES,
            completed: false,
            _memory: memory,
        })
    }

    pub(crate) fn observe_item(&mut self, item: &Map<String, Value>) {
        if self.items.len() == RESPONSE_ITEMS
            || item.get("type").and_then(Value::as_str) != Some("reasoning")
        {
            return;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            return;
        };
        let encrypted = match item.get("encrypted_content") {
            None | Some(Value::Null) => None,
            Some(Value::String(ciphertext)) => Some(ciphertext.as_str()),
            _ => return,
        };
        if let Some(digest) = fingerprint(id, encrypted, None, &mut self.remaining)
            && !self.items.contains(&digest)
        {
            self.items.push(digest);
        }
    }

    pub(crate) fn observe_completed(&mut self, response: &Map<String, Value>) {
        if response.get("status").and_then(Value::as_str) != Some("completed") {
            return;
        }
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            for item in output.iter().filter_map(Value::as_object) {
                self.observe_item(item);
            }
        }
        self.completed = true;
    }

    pub(crate) fn delivered(&self) -> &[Fingerprint] {
        if self.completed { &self.items } else { &[] }
    }
}

fn hash_identity(id: &str, suffix: &[u8]) -> Sha256 {
    let mut hash = Sha256::new();
    hash.update((id.len() as u64).to_be_bytes());
    hash.update(id.as_bytes());
    hash.update(suffix);
    hash
}

fn fingerprint(
    id: &str,
    encrypted: Option<&str>,
    replay: Option<&ReplayStore>,
    remaining: &mut usize,
) -> Option<Fingerprint> {
    if id.is_empty() || id.len() > 256 || (replay.is_some() && id.content_ref().is_some()) {
        return None;
    }
    let mut hash = hash_identity(id, &[]);
    let text = encrypted.unwrap_or_default();
    if let Some(reference) = replay.and_then(|_| text.content_ref()) {
        *remaining = remaining.checked_sub(usize::try_from(reference.byte_len()).ok()?)?;
        let mut reader = replay?.reader(&reference).ok()?;
        let mut chunk = [0; 8192];
        loop {
            let count = reader.read(&mut chunk).ok()?;
            if count == 0 {
                break;
            }
            hash.update(&chunk[..count]);
        }
        reader.verify_terminal().ok()?;
    } else {
        *remaining = remaining.checked_sub(text.len())?;
        hash.update(text.as_bytes());
    }
    Some(hash.finalize().into())
}
