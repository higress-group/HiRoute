//! Optional source evidence and stateless summary replay for native Responses.
//! These facts never authorize a request or a candidate.
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
pub(crate) struct ReasoningSource {
    identity: Fingerprint,
    openai_responses: bool,
}

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
        Self {
            identity: hash_identity(endpoint, destination.as_bytes())
                .finalize()
                .into(),
            openai_responses: endpoint.parse::<http::Uri>().is_ok_and(|uri| {
                uri.scheme_str() == Some("https")
                    && uri
                        .host()
                        .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"))
                    && uri.port_u16().unwrap_or(443) == 443
                    && uri.path_and_query().map(http::uri::PathAndQuery::as_str)
                        == Some("/v1/responses")
            }),
        }
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
    stateless_mixed_source: bool,
    _memory: Option<Reservation>,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ReasoningReplayProjection<'a> {
    sources: &'a [(usize, ReasoningSource)],
    target: Option<ReasoningSource>,
    untracked_summary_ids: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum ReasoningReplayAction {
    Preserve,
    Foreign,
    PortableSummary,
}

impl ReasoningReplayProjection<'_> {
    pub(crate) fn is_empty(self) -> bool {
        self.sources.is_empty() && !self.untracked_summary_ids
    }

    pub(crate) fn untracked_summary_ids(self) -> bool {
        self.untracked_summary_ids
    }

    pub(crate) fn action(self, index: usize) -> ReasoningReplayAction {
        match self
            .sources
            .binary_search_by_key(&index, |(index, _)| *index)
        {
            Ok(position) if Some(self.sources[position].1) != self.target => {
                ReasoningReplayAction::Foreign
            }
            Err(_) if self.untracked_summary_ids => ReasoningReplayAction::PortableSummary,
            _ => ReasoningReplayAction::Preserve,
        }
    }
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
        {
            return Self::default();
        }
        let Some(scope) = holds.digest_key().and_then(|key| {
            ContextHoldKey::responses_reasoning(
                authorized,
                request.ingress_protocol,
                identity,
                &key,
            )
        }) else {
            return Self::default();
        };
        let capacity = request.responses_reasoning_history.len().min(REQUEST_ITEMS);
        let mut result = Self {
            scope: Some((Arc::clone(store), scope.clone())),
            stateless_mixed_source: capacity != 0
                && request
                    .responses_options
                    .as_ref()
                    .is_some_and(|options| options.store == Some(false))
                && matches!(
                    authorized.planner_policy().identity,
                    crate::server::core_runtime::profiles::PlannerRouteIdentityV2::Plan { .. }
                )
                && authorized
                    .candidates()
                    .windows(2)
                    .any(|pair| pair[0].endpoint != pair[1].endpoint),
            ..Self::default()
        };
        if capacity == 0 || !store.0.lock().scopes.contains_key(&scope) {
            return result;
        }
        let Some(layout) = request.native_history_layout() else {
            return result;
        };
        let bytes_per_item =
            size_of::<(usize, Fingerprint)>() + size_of::<(usize, ReasoningSource)>();
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

    pub(crate) fn can_project(&self) -> bool {
        !self.sources.is_empty() || self.stateless_mixed_source
    }

    pub(crate) fn project(
        &self,
        target: impl FnOnce() -> ReasoningSource,
    ) -> ReasoningReplayProjection<'_> {
        if !self.can_project() {
            return ReasoningReplayProjection::default();
        }
        let target = target();
        ReasoningReplayProjection {
            sources: &self.sources,
            target: Some(target),
            untracked_summary_ids: self.stateless_mixed_source && target.openai_responses,
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::core_runtime::adapters::{
        decode_ingress_request, project_candidate_request_template_with_history,
    };
    use crate::server::core_runtime::profiles::{
        Fidelity, NativeProviderStateEmission, fixed_reasoning,
    };
    use serde_json::json;

    #[test]
    fn stateless_openai_replay_keeps_old_summaries_without_resolving_old_ids() {
        let body = json!({
            "model": "route", "store": false, "stream": true,
            "input": [
                {"role":"user","content":"Earlier task"},
                {"type":"reasoning","id":"rs_old_absent","summary":[
                    {"type":"summary_text","text":"An earlier portable summary"}]},
                {"type":"reasoning","id":"rs_old_null","encrypted_content":null,"summary":[
                    {"type":"summary_text","text":"Another earlier summary"}]},
                {"type":"reasoning","id":"rs_same_source","summary":[
                    {"type":"summary_text","text":"Same-source history"}]},
                {"type":"reasoning","id":"rs_unknown_ciphertext","summary":[],"encrypted_content":"opaque"},
                {"type":"reasoning","id":"rs_reference_only","summary":[]},
                {"type":"reasoning","id":"rs_native_extension","summary":[
                    {"type":"summary_text","text":"Unknown history"}],"provider_extension":true},
                {"type":"reasoning","id":"rs_incomplete","status":"incomplete","summary":[
                    {"type":"summary_text","text":"Incomplete history"}]},
                {"type":"function_call","id":"fc_original","call_id":"call_a","name":"lookup","arguments":"{}"},
                {"type":"function_call_output","call_id":"call_a","output":"result"},
                {"role":"user","content":"Current task"}
            ]
        });
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        let original = request.clone();
        let mut profile = CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "fixture",
            fixed_reasoning("fixture"),
        );
        profile.capability.native_provider_state = NativeProviderStateEmission::Native;
        profile.capability.request.provider_state = Fidelity::Exact;
        profile.capability.response.provider_state = Fidelity::Exact;
        let openai = "https://api.openai.com/v1/responses";
        let openai_source = ReasoningSource::for_profile(&profile, openai, "openai-key");

        for (endpoint, mixed_sources, expected_removed) in [
            (openai, true, 2),
            (openai, false, 0),
            ("http://127.0.0.1:28001/v1/responses", true, 0),
            ("https://api.openai.com.example/v1/responses", true, 0),
        ] {
            // An upgraded process has no delivery evidence for the older IDs.
            let replay = ResponsesReasoningReplay {
                sources: if endpoint == openai {
                    vec![(3, openai_source)]
                } else {
                    Vec::new()
                },
                stateless_mixed_source: mixed_sources,
                ..Default::default()
            };
            let reasoning =
                replay.project(|| ReasoningSource::for_profile(&profile, endpoint, "openai-key"));
            let template = project_candidate_request_template_with_history(
                &request, &profile, None, reasoning,
            )
            .unwrap();
            let rendered: Value = serde_json::from_slice(&template.bytes).unwrap();
            let mut expected = body["input"].clone();
            if expected_removed != 0 {
                for index in [1, 2] {
                    expected[index].as_object_mut().unwrap().remove("id");
                }
            }
            assert_eq!(rendered["input"], expected, "{endpoint}");
            assert_eq!(template.reasoning_history_removed, expected_removed);
            assert_eq!(request, original);
        }
    }
}
