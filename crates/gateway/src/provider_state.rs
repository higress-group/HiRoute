//! Process-local provenance for native ciphertext actually accepted downstream.
//! The store retains only a digest and exact owner; it never rewrites ciphertext.
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::content_ref::ContentRef;
use crate::replay::ReplayStore;

use crate::server::core_runtime::model_ir::{
    ContentPart, ExactProviderPathV1, ModelIrError, ModelRequestIRV1,
};
use crate::server::request_plan::IngressProtocol;

const CAPACITY: usize = 4096;
const IDLE_TTL: Duration = Duration::from_secs(3600);
/// Authorization scope for opaque provider state, never for ordinary tool IDs.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ProviderStateScopeV1 {
    pub authority_id: String,
    pub authority_epoch: u64,
    pub grant_id: String,
    pub grant_generation: u64,
    pub served_model_id: String,
    pub route: hiroute_domain::ModelRequestRouteV2,
}
type Key = (ProviderStateScopeV1, [u8; 32]);

#[derive(Default)]
pub(crate) struct ProviderStateStore(Mutex<BTreeMap<Key, Entry>>);

struct Entry {
    owner: Option<ExactProviderPathV1>,
    expires: Instant,
}

#[derive(Default, Debug, PartialEq)]
pub(crate) struct ResolvedProviderStates {
    owners: BTreeMap<[u8; 32], ExactProviderPathV1>,
}

impl ResolvedProviderStates {
    pub(crate) fn decode_owner(&self, ingress: IngressProtocol) -> Option<&ExactProviderPathV1> {
        self.owners
            .values()
            .find(|owner| owner.upstream_protocol == ingress)
            .or_else(|| self.owners.values().next())
    }

    pub(crate) fn owner_for(&self, value: &str) -> Option<&ExactProviderPathV1> {
        self.owners.get(&digest(value))
    }

    pub(crate) fn bind_request(&self, request: &mut ModelRequestIRV1) -> Result<(), ModelIrError> {
        if self.owners.is_empty() {
            return Ok(());
        }
        for state in request
            .messages
            .iter_mut()
            .flat_map(|message| message.content.iter_mut())
            .filter_map(|part| match part {
                ContentPart::ProviderState { state } => Some(state),
                _ => None,
            })
        {
            let value = match state.kind.as_str() {
                "encrypted_content" => state.value.as_str(),
                "thinking" => state.value.get("signature").and_then(Value::as_str),
                "redacted_thinking" => state.value.get("data").and_then(Value::as_str),
                _ => None,
            };
            let Some(value) = value else {
                continue;
            };
            let owner = self
                .owner_for(value)
                .ok_or(ModelIrError::ProviderStateOwnershipRequired)?;
            if owner.upstream_protocol != request.ingress_protocol
                && !(request.ingress_protocol == IngressProtocol::Messages
                    && owner.upstream_protocol == IngressProtocol::Responses)
            {
                return Err(ModelIrError::ProviderStateNotPortable);
            }
            state.owner = owner.clone();
            if request.ingress_protocol == IngressProtocol::Messages
                && owner.upstream_protocol == IngressProtocol::Responses
                && state.kind == "thinking"
            {
                let thinking = state
                    .value
                    .get("thinking")
                    .and_then(Value::as_str)
                    .ok_or(ModelIrError::InvalidField("thinking"))?
                    .to_owned();
                state.kind = "encrypted_content".into();
                state.messages_thinking = Some(thinking);
                state.value = Value::String(value.to_owned());
            }
        }
        Ok(())
    }
}

impl ProviderStateStore {
    fn accept(&self, scope: &ProviderStateScopeV1, pending: &Pending, now: Instant) {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, entry| entry.expires > now);
        let key = (scope.clone(), pending.digest);
        if let Some(entry) = entries.get_mut(&key) {
            if entry.owner.as_ref() != Some(&pending.owner) {
                entry.owner = None;
            }
            return;
        }
        if entries.len() >= CAPACITY
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.expires)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(
            key,
            Entry {
                owner: Some(pending.owner.clone()),
                expires: now + IDLE_TTL,
            },
        );
    }

    #[cfg(test)]
    fn resolve(
        &self,
        scope: &ProviderStateScopeV1,
        document: &Value,
        now: Instant,
    ) -> Result<ResolvedProviderStates, ModelIrError> {
        self.resolve_with_replay(scope, document, None, now)
    }

    pub(crate) fn resolve_with_replay(
        &self,
        scope: &ProviderStateScopeV1,
        document: &Value,
        replay: Option<&ReplayStore>,
        now: Instant,
    ) -> Result<ResolvedProviderStates, ModelIrError> {
        let states = document
            .get("input")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| item["type"] == "reasoning")
            .map(|item| item.get("encrypted_content"))
            .chain(
                document
                    .get("messages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|message| message["role"] == "assistant")
                    .filter_map(|message| message.get("content").and_then(Value::as_array))
                    .flatten()
                    .filter(|block| {
                        matches!(
                            block["type"].as_str(),
                            Some("thinking" | "redacted_thinking")
                        )
                    })
                    .map(|block| match block["type"].as_str() {
                        Some("thinking") => block.get("signature"),
                        _ => block.get("data"),
                    }),
            );
        let mut requested = Vec::new();
        for value in states {
            let state = match value {
                None | Some(Value::Null) => continue,
                Some(Value::String(state)) if state.is_empty() => continue,
                Some(Value::String(state)) => state,
                Some(_) => return Err(ModelIrError::InvalidField("encrypted_content")),
            };
            let state_digest = match ContentRef::from_wire_marker(state) {
                Some(reference)
                    if replay.is_some_and(|replay| replay.has_reference(&reference)) =>
                {
                    let mut reader = replay
                        .expect("checked above")
                        .reader(&reference)
                        .map_err(|_| ModelIrError::ProviderStateOwnershipRequired)?;
                    let mut hash = Sha256::new();
                    let mut buffer = [0_u8; 16 * 1024];
                    loop {
                        let size = reader
                            .read(&mut buffer)
                            .map_err(|_| ModelIrError::ProviderStateOwnershipRequired)?;
                        if size == 0 {
                            break;
                        }
                        hash.update(&buffer[..size]);
                    }
                    reader
                        .verify_terminal()
                        .map_err(|_| ModelIrError::ProviderStateOwnershipRequired)?;
                    hash.finalize().into()
                }
                _ => digest(state),
            };
            requested.push((digest(state), state_digest));
        }
        // Replay may be disk-backed. Never read it while holding the shared
        // provenance lock; only the exact lookup and renew need serialization.
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, entry| entry.expires > now);
        let mut resolved = ResolvedProviderStates::default();
        let mut resolved_keys = Vec::new();
        for (decode_digest, state_digest) in requested {
            let key = (scope.clone(), state_digest);
            let entry = entries
                .get(&key)
                .ok_or(ModelIrError::ProviderStateOwnershipRequired)?;
            let next = entry
                .owner
                .as_ref()
                .ok_or(ModelIrError::ProviderStateNotPortable)?;
            // Decode sees the compact request-local marker, while ownership
            // was recorded for the original bytes accepted downstream.
            resolved.owners.insert(decode_digest, next.clone());
            resolved_keys.push(key);
        }
        // Renew only after the entire replay passes ownership validation.
        for key in resolved_keys {
            entries
                .get_mut(&key)
                .expect("resolved entry remains locked")
                .expires = now + IDLE_TTL;
        }
        Ok(resolved)
    }
}

fn digest(state: &str) -> [u8; 32] {
    Sha256::digest(state.as_bytes()).into()
}

// Counting writer uses the same escaping rules as the actual serializer.
struct JsonLength(usize);

impl std::io::Write for JsonLength {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("serialized provider state length overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) fn json_string_length(value: &str) -> Result<usize, ModelIrError> {
    let mut length = JsonLength(0);
    serde_json::to_writer(&mut length, value).map_err(|_| ModelIrError::BufferLimit(usize::MAX))?;
    Ok(length.0)
}

struct Pending {
    digest: [u8; 32],
    owner: ExactProviderPathV1,
    pattern: Vec<u8>,
    charge: Arc<Reservation>,
}

#[derive(Clone)]
pub(crate) struct ActiveProviderStates {
    store: Arc<ProviderStateStore>,
    scope: ProviderStateScopeV1,
    pending: Arc<Mutex<Vec<Pending>>>,
    budget: StreamBudget,
    downstream: IngressProtocol,
}

impl ActiveProviderStates {
    #[cfg(test)]
    pub(crate) fn new(
        store: Arc<ProviderStateStore>,
        scope: ProviderStateScopeV1,
        downstream: IngressProtocol,
    ) -> Self {
        let tree = hiroute_gateway_core::runtime::body::BudgetTree::new(
            64 * 1024 * 1024,
            64 * 1024 * 1024,
        )
        .unwrap();
        Self::with_budget(
            store,
            scope,
            downstream,
            tree.stream(64 * 1024 * 1024).unwrap(),
        )
    }

    pub(crate) fn with_budget(
        store: Arc<ProviderStateStore>,
        scope: ProviderStateScopeV1,
        downstream: IngressProtocol,
        budget: StreamBudget,
    ) -> Self {
        Self {
            store,
            scope,
            pending: Arc::new(Mutex::new(Vec::new())),
            budget,
            downstream,
        }
    }

    pub(crate) fn budget(&self) -> &StreamBudget {
        &self.budget
    }

    pub(crate) fn record(
        &self,
        state: &Value,
        owner: &ExactProviderPathV1,
    ) -> Result<(), ModelIrError> {
        let state = state
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or(ModelIrError::InvalidField("encrypted_content"))?;
        let prefix: &[u8] = match self.downstream {
            IngressProtocol::Responses => b"\"encrypted_content\":",
            IngressProtocol::Messages => b"\"signature\":",
            _ => return Err(ModelIrError::ProviderStateNotPortable),
        };
        let length = json_string_length(state)?
            .checked_add(prefix.len())
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        self.record_pattern(state, owner, length, |pattern| {
            pattern.extend_from_slice(prefix);
            serde_json::to_writer(pattern, state)
                .map_err(|_| ModelIrError::InvalidField("encrypted_content"))
        })
    }

    pub(crate) fn record_at_acceptance(
        &self,
        state: &str,
        owner: &ExactProviderPathV1,
        pattern: &[u8],
    ) -> Result<(), ModelIrError> {
        if state.is_empty() || pattern.is_empty() {
            return Err(ModelIrError::InvalidField("provider_state"));
        }
        self.record_pattern(state, owner, pattern.len(), |out| {
            out.extend_from_slice(pattern);
            Ok(())
        })
    }

    fn record_pattern(
        &self,
        state: &str,
        owner: &ExactProviderPathV1,
        length: usize,
        fill: impl FnOnce(&mut Vec<u8>) -> Result<(), ModelIrError>,
    ) -> Result<(), ModelIrError> {
        let digest = digest(state);
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending
            .iter()
            .any(|p| p.digest == digest && &p.owner == owner)
        {
            return Ok(());
        }
        // Charge pattern and scanner carry/boundary copies before allocating.
        let bytes = length
            .checked_mul(8)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Pending>()))
            .ok_or(ModelIrError::BufferLimit(usize::MAX))?;
        let charge = self
            .budget
            .reserve(MemoryRole::SemanticState, bytes)
            .map_err(|_| ModelIrError::BufferLimit(bytes))?;
        let mut pattern = Vec::with_capacity(length);
        fill(&mut pattern)?;
        pending.push(Pending {
            digest,
            owner: owner.clone(),
            pattern,
            charge: Arc::new(charge),
        });
        Ok(())
    }

    pub(crate) fn scanner(&self) -> AcceptedProviderStateScanner {
        AcceptedProviderStateScanner {
            active: self.clone(),
            carry: Vec::new(),
            carry_charge: None,
        }
    }
}

pub(crate) struct AcceptedProviderStateScanner {
    active: ActiveProviderStates,
    carry: Vec<u8>,
    carry_charge: Option<Arc<Reservation>>,
}

impl AcceptedProviderStateScanner {
    pub(crate) fn accept_bytes(&mut self, bytes: &[u8], now: Instant) {
        let mut pending = self
            .active
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let keep = pending
            .iter()
            .map(|p| p.pattern.len().saturating_sub(1))
            .max()
            .unwrap_or(0);
        let next_charge = pending
            .iter()
            .max_by_key(|p| p.pattern.len())
            .map(|p| Arc::clone(&p.charge));
        let mut boundary = self.carry.clone();
        boundary.extend_from_slice(&bytes[..bytes.len().min(keep)]);
        pending.retain(|p| {
            let found = bytes.windows(p.pattern.len()).any(|w| w == p.pattern)
                || boundary.windows(p.pattern.len()).any(|w| w == p.pattern);
            if found {
                self.active.store.accept(&self.active.scope, p, now);
            }
            !found
        });
        // Keep only enough bytes to recognize an accepted, split key/value unit.
        if bytes.len() >= keep {
            self.carry = bytes[bytes.len() - keep..].to_vec();
        } else {
            let start = boundary.len().saturating_sub(keep);
            self.carry = boundary[start..].to_vec();
        }
        self.carry_charge = next_charge;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::core_runtime::profiles::{
        CandidateProtocolProfile, Fidelity, NativeProviderStateEmission, StateAffinity,
        fixed_reasoning,
    };
    use crate::server::request_plan::IngressProtocol;
    use serde_json::json;

    fn scope() -> ProviderStateScopeV1 {
        ProviderStateScopeV1 {
            authority_id: "a".into(),
            authority_epoch: 1,
            grant_id: "g".into(),
            grant_generation: 1,
            served_model_id: "route".into(),
            route: hiroute_domain::ModelRequestRouteV2::Plan {
                revision: 1,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"route"),
            },
        }
    }

    fn owner(model: &str) -> ExactProviderPathV1 {
        CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            model,
            fixed_reasoning("fixed"),
        )
        .exact_provider_path()
        .unwrap()
    }

    fn request(state: &str) -> Value {
        json!({"input":[{"type":"reasoning","encrypted_content":state}]})
    }

    #[test]
    fn replay_backed_ciphertext_resolves_against_the_original_accepted_bytes() {
        let root = std::env::temp_dir().join(format!(
            "hiroute-state-replay-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = crate::replay::ReplayManager::open(crate::replay::ReplayConfig {
            root: root.clone(),
            memory_threshold_bytes: 1024 * 1024,
            record_bytes: 16 * 1024,
            orphan_ttl: Duration::from_secs(60),
        })
        .unwrap();
        let budget =
            hiroute_gateway_core::runtime::body::BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
                .unwrap()
                .stream(8 * 1024 * 1024)
                .unwrap();
        let replay = manager.begin_request(budget.clone()).unwrap();
        let ciphertext = "opaque".repeat(750_000);
        let state_store = Arc::new(ProviderStateStore::default());
        let now = Instant::now();
        let active =
            ActiveProviderStates::new(state_store.clone(), scope(), IngressProtocol::Responses);
        active.record(&json!(ciphertext), &owner("luna")).unwrap();
        active.scanner().accept_bytes(
            json!({"encrypted_content":ciphertext})
                .to_string()
                .as_bytes(),
            now,
        );
        let marker = replay
            .store_content(ciphertext.as_bytes())
            .unwrap()
            .wire_marker();
        let resolved = state_store
            .resolve_with_replay(&scope(), &request(&marker), Some(&replay), now)
            .unwrap();
        assert_eq!(resolved.owner_for(&marker), Some(&owner("luna")));
        assert!(budget.snapshot().unwrap().peak <= 8 * 1024 * 1024);
        drop((active, resolved, replay, manager));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn provider_state_admission_counts_escaping_and_deduplicates() {
        let state = "\"\\\n\u{0000}雪".repeat(128);
        let mut length = JsonLength(0);
        serde_json::to_writer(&mut length, &state).unwrap();
        assert_eq!(length.0, serde_json::to_vec(&state).unwrap().len());
        let bytes =
            (b"\"encrypted_content\":".len() + length.0) * 8 + std::mem::size_of::<Pending>();
        let tree =
            hiroute_gateway_core::runtime::body::BudgetTree::new(bytes * 2, bytes * 2).unwrap();
        let store = Arc::new(ProviderStateStore::default());
        let rejected = ActiveProviderStates::with_budget(
            store.clone(),
            scope(),
            IngressProtocol::Responses,
            tree.stream(bytes - 1).unwrap(),
        );
        assert!(matches!(
            rejected.record(&json!(state), &owner("luna")),
            Err(ModelIrError::BufferLimit(_))
        ));
        assert!(rejected.pending.lock().unwrap().is_empty());
        let accepted = ActiveProviderStates::with_budget(
            store,
            scope(),
            IngressProtocol::Responses,
            tree.stream(bytes).unwrap(),
        );
        accepted.record(&json!(state), &owner("luna")).unwrap();
        accepted.record(&json!(state), &owner("luna")).unwrap();
        assert_eq!(accepted.pending.lock().unwrap().len(), 1);
    }

    #[test]
    fn native_signature_acceptance_has_no_fixed_state_or_pending_count_limit() {
        let store = Arc::new(ProviderStateStore::default());
        let active = ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Messages);
        let now = Instant::now();
        let large = "s".repeat(300 * 1024);
        for index in 0..129 {
            let state = format!("{large}{index}");
            let closing = format!(
                "event: content_block_stop\ndata: {{\"index\":{index},\"type\":\"content_block_stop\"}}\n\n"
            );
            active
                .record_at_acceptance(&state, &owner("luna"), closing.as_bytes())
                .unwrap();
        }
        assert_eq!(active.pending.lock().unwrap().len(), 129);
        let state = format!("{large}128");
        let replay = json!({"messages":[{"role":"assistant","content":[
            {"type":"thinking","thinking":"","signature":state}
        ]}]});
        assert!(store.resolve(&scope(), &replay, now).is_err());
        active.scanner().accept_bytes(
            b"event: content_block_stop\ndata: {\"index\":128,\"type\":\"content_block_stop\"}\n\n",
            now,
        );
        assert_eq!(
            store
                .resolve(&scope(), &replay, now)
                .unwrap()
                .owner_for(&state),
            Some(&owner("luna"))
        );
    }

    #[test]
    fn fragmented_native_signature_requires_accepted_matching_block_close() {
        let store = Arc::new(ProviderStateStore::default());
        let active = ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Messages);
        let now = Instant::now();
        let replay = json!({"messages":[{"role":"assistant","content":[
            {"type":"thinking","thinking":"","signature":"combined-signature"}
        ]}]});
        let closing = br#"{ "index": 0, "type": "content_block_stop" }"#;
        active
            .record_at_acceptance("combined-signature", &owner("luna"), closing)
            .unwrap();
        let mut scanner = active.scanner();
        scanner.accept_bytes(br#"{"signature":"combined-signature"}"#, now);
        scanner.accept_bytes(br#"{ "index": 1, "type": "content_block_stop" }"#, now);
        assert!(store.resolve(&scope(), &replay, now).is_err());
        for part in closing.chunks(2) {
            scanner.accept_bytes(part, now);
        }
        assert_eq!(
            store
                .resolve(&scope(), &replay, now)
                .unwrap()
                .owner_for("combined-signature"),
            Some(&owner("luna"))
        );
    }

    #[test]
    fn messages_signature_binds_only_accepted_responses_ciphertext() {
        let store = Arc::new(ProviderStateStore::default());
        let active = ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Messages);
        let now = Instant::now();
        let replay = json!({"messages":[{"role":"assistant","content":[
            {"type":"thinking","thinking":"","signature":"luna-state"},
            {"type":"tool_use","id":"tool-one","name":"Bash","input":{"command":"pwd"}}
        ]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-one","content":"/work"}]}]});
        active.record(&json!("luna-state"), &owner("luna")).unwrap();
        assert!(store.resolve(&scope(), &replay, now).is_err());
        let mut scanner = active.scanner();
        scanner.accept_bytes(br#"{"encrypted_content":"luna-state"}"#, now);
        assert!(store.resolve(&scope(), &replay, now).is_err());
        let escaped = json!({"text":"\"signature\":\"luna-state\""}).to_string();
        scanner.accept_bytes(escaped.as_bytes(), now);
        assert!(store.resolve(&scope(), &replay, now).is_err());
        for part in br#"data: {"type":"content_block_delta","delta":{"type":"signature_delta","signature":"luna-state"}}"#.chunks(3) {
            scanner.accept_bytes(part, now);
        }
        assert_eq!(
            store
                .resolve(&scope(), &replay, now)
                .unwrap()
                .owner_for("luna-state"),
            Some(&owner("luna"))
        );
        let mut foreign = scope();
        foreign.grant_generation += 1;
        assert!(store.resolve(&foreign, &replay, now).is_err());
        assert!(store.resolve(&scope(), &replay, now + IDLE_TTL).is_err());
    }

    #[test]
    fn only_accepted_ciphertext_resolves_and_scope_expiry_are_enforced() {
        let store = Arc::new(ProviderStateStore::default());
        let active = ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Responses);
        let now = Instant::now();
        let state = "fixture-\\\"ciphertext";
        active.record(&json!(state), &owner("luna")).unwrap();
        assert!(store.resolve(&scope(), &request(state), now).is_err());
        let mut scanner = active.scanner();
        // A user text containing an escaped key/value must not activate anything.
        let fake = json!({"text":format!("\"encrypted_content\":{}", json!(state))}).to_string();
        scanner.accept_bytes(fake.as_bytes(), now);
        assert!(store.resolve(&scope(), &request(state), now).is_err());
        let wire = json!({"item":{"encrypted_content":state}}).to_string();
        for byte in wire.as_bytes().chunks(1) {
            scanner.accept_bytes(byte, now);
        }
        assert_eq!(
            store
                .resolve(&scope(), &request(state), now)
                .unwrap()
                .owner_for(state),
            Some(&owner("luna"))
        );
        let mut foreign = scope();
        foreign.grant_generation += 1;
        assert!(store.resolve(&foreign, &request(state), now).is_err());
        assert!(store.resolve(&scope(), &request("altered"), now).is_err());
        assert!(
            store
                .resolve(&scope(), &request(state), now + IDLE_TTL)
                .is_err()
        );
    }

    #[test]
    fn mixed_verified_owners_are_distinct_and_conflicting_producers_do_not_gain_authority() {
        let store = Arc::new(ProviderStateStore::default());
        let now = Instant::now();
        for (value, model) in [("one", "luna"), ("two", "terra")] {
            let active =
                ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Responses);
            active.record(&json!(value), &owner(model)).unwrap();
            active.scanner().accept_bytes(
                json!({"encrypted_content":value}).to_string().as_bytes(),
                now,
            );
        }
        let mixed = json!({"input":[{"type":"reasoning","encrypted_content":"one"},{"type":"reasoning","encrypted_content":"two"}]});
        let resolved = store.resolve(&scope(), &mixed, now).unwrap();
        assert_eq!(resolved.owner_for("one"), Some(&owner("luna")));
        assert_eq!(resolved.owner_for("two"), Some(&owner("terra")));
        let mut decoded = crate::server::core_runtime::adapters::decode_ingress_request_with_bindings(
            IngressProtocol::Responses,
            &json!({"model":"route","input":[
                {"type":"reasoning","summary":[],"encrypted_content":"one"},
                {"type":"reasoning","summary":[],"encrypted_content":"two"},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"continue"}]}
            ]}),
            &crate::server::core_runtime::adapters::IngressRequestBindings {
                provider_state_owner: resolved.decode_owner(IngressProtocol::Responses).cloned(),
            },
        )
        .unwrap();
        resolved.bind_request(&mut decoded).unwrap();
        let owners = decoded
            .messages
            .iter()
            .flat_map(|message| &message.content)
            .filter_map(|part| match part {
                ContentPart::ProviderState { state } => Some(state.owner.native_model.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(owners, vec!["luna", "terra"]);
        let active = ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Responses);
        active.record(&json!("one"), &owner("terra")).unwrap();
        active
            .scanner()
            .accept_bytes(br#"{"encrypted_content":"one"}"#, now);
        assert_eq!(
            store.resolve(&scope(), &request("one"), now),
            Err(ModelIrError::ProviderStateNotPortable)
        );
        let large = "x".repeat(300 * 1024);
        let active = ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Responses);
        active.record(&json!(large), &owner("luna")).unwrap();
        let wire = serde_json::to_vec(&json!({"encrypted_content":large})).unwrap();
        let mut scanner = active.scanner();
        for chunk in wire.chunks(8191) {
            scanner.accept_bytes(chunk, now);
        }
        assert_eq!(
            store
                .resolve(&scope(), &request(&large), now)
                .unwrap()
                .owner_for(&large),
            Some(&owner("luna"))
        );
    }

    #[test]
    fn mixed_messages_and_responses_signatures_keep_each_original_protocol() {
        let store = Arc::new(ProviderStateStore::default());
        let now = Instant::now();
        let messages_owner = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            IngressProtocol::Messages,
            "glm",
            fixed_reasoning("fixed"),
        )
        .exact_provider_path()
        .unwrap();
        for (signature, source) in [
            ("responses-state", owner("luna")),
            ("messages-state", messages_owner.clone()),
        ] {
            let active =
                ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Messages);
            active.record(&json!(signature), &source).unwrap();
            active
                .scanner()
                .accept_bytes(json!({"signature":signature}).to_string().as_bytes(), now);
        }
        let document = json!({"model":"route","max_tokens":128,"messages":[
            {"role":"assistant","content":[
                {"type":"thinking","thinking":"luna summary","signature":"responses-state"},
                {"type":"thinking","thinking":"model notes","signature":"messages-state"}
            ]},
            {"role":"user","content":"continue"}
        ]});
        let resolved = store.resolve(&scope(), &document, now).unwrap();
        let mut decoded =
            crate::server::core_runtime::adapters::decode_ingress_request_with_bindings(
                IngressProtocol::Messages,
                &document,
                &crate::server::core_runtime::adapters::IngressRequestBindings {
                    provider_state_owner: resolved.decode_owner(IngressProtocol::Messages).cloned(),
                },
            )
            .unwrap();
        resolved.bind_request(&mut decoded).unwrap();
        let states = decoded.messages[0]
            .content
            .iter()
            .filter_map(|part| match part {
                ContentPart::ProviderState { state } => Some(state),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(states[0].owner, owner("luna"));
        assert_eq!(states[0].kind, "encrypted_content");
        assert_eq!(states[0].messages_thinking.as_deref(), Some("luna summary"));
        assert_eq!(states[1].owner, messages_owner);
        assert_eq!(states[1].kind, "thinking");
        let mut target = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Messages,
            IngressProtocol::Messages,
            "new-glm",
            fixed_reasoning("fixed"),
        );
        target.capability.native_provider_state = NativeProviderStateEmission::ExactOwnerAffine;
        target.capability.request.provider_state = Fidelity::Exact;
        target.capability.request.state_affinity = StateAffinity::ExactOwner;
        target.capability.response.provider_state = Fidelity::Exact;
        target.capability.response.state_affinity = StateAffinity::ExactOwner;
        let projected =
            crate::server::core_runtime::adapters::project_candidate_request(&decoded, &target)
                .unwrap();
        assert_eq!(
            projected.body["messages"][0]["content"],
            json!([
                {"type":"thinking","thinking":"luna summary","signature":"responses-state"},
                {"type":"thinking","thinking":"model notes","signature":"messages-state"}
            ])
        );
    }

    #[test]
    fn active_replay_renews_idle_deadline_but_eventually_expires() {
        let store = Arc::new(ProviderStateStore::default());
        let active = ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Responses);
        let now = Instant::now();
        active.record(&json!("state"), &owner("luna")).unwrap();
        active
            .scanner()
            .accept_bytes(br#"{"encrypted_content":"state"}"#, now);
        for minutes in [30, 60, 90, 120] {
            assert_eq!(
                store
                    .resolve(
                        &scope(),
                        &request("state"),
                        now + Duration::from_secs(minutes * 60)
                    )
                    .unwrap()
                    .owner_for("state"),
                Some(&owner("luna"))
            );
        }
        assert_eq!(
            store.resolve(
                &scope(),
                &request("state"),
                now + Duration::from_secs(180 * 60)
            ),
            Err(ModelIrError::ProviderStateOwnershipRequired),
        );
    }

    #[test]
    fn invalid_replay_never_renews_a_valid_prefix() {
        for invalid in [
            json!({"type":"reasoning","encrypted_content":"unknown"}),
            json!({"type":"reasoning","encrypted_content":42}),
        ] {
            let store = Arc::new(ProviderStateStore::default());
            let now = Instant::now();
            let active =
                ActiveProviderStates::new(store.clone(), scope(), IngressProtocol::Responses);
            active.record(&json!("state"), &owner("luna")).unwrap();
            active.scanner().accept_bytes(
                json!({"encrypted_content":"state"}).to_string().as_bytes(),
                now,
            );
            let replay =
                json!({"input":[{"type":"reasoning","encrypted_content":"state"}, invalid]});
            assert!(
                store
                    .resolve(&scope(), &replay, now + Duration::from_secs(1800))
                    .is_err()
            );
            assert_eq!(
                store.resolve(&scope(), &request("state"), now + IDLE_TTL),
                Err(ModelIrError::ProviderStateOwnershipRequired)
            );
        }
    }

    #[test]
    fn summary_only_reasoning_does_not_require_provider_state_authority() {
        let store = ProviderStateStore::default();
        let now = Instant::now();
        for item in [
            json!({"type":"reasoning","summary":[]}),
            json!({"type":"reasoning","summary":[],"encrypted_content":null}),
            json!({"type":"reasoning","summary":[],"encrypted_content":""}),
        ] {
            let document = json!({"input":[item]});
            assert_eq!(
                store.resolve(&scope(), &document, now).unwrap(),
                ResolvedProviderStates::default()
            );
        }
        assert_eq!(
            store.resolve(
                &scope(),
                &json!({"input":[{"type":"reasoning","encrypted_content":7}]}),
                now
            ),
            Err(ModelIrError::InvalidField("encrypted_content"))
        );
    }
}
