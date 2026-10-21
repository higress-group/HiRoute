mod history;
mod identity;
mod routing;
mod store;

use std::time::Instant;

use http::HeaderMap;
use serde_json::Value;

use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use crate::server::request_plan::{AuthorizedRequestPlan, IngressProtocol};

#[cfg(test)]
pub(crate) use history::visible_history;
pub(crate) use history::{HistoryEvidence, visible_history_with_replay};
pub(crate) use identity::{ContextIdentityFacts, identity_facts};
#[cfg(test)]
pub(crate) use routing::{NativeCheckpoint, NativeUserOrigin};
pub(crate) use routing::{NativeUserTurn, dsh_user_turns};
pub(crate) use store::{
    ContextHoldKey, ContextHoldStore, HistoryContinuity, HoldCompletion, HoldTicket, PreparedHold,
};

pub(crate) struct ContextRequest<'a> {
    pub(crate) ingress: IngressProtocol,
    pub(crate) headers: &'a HeaderMap,
    pub(crate) document: &'a Value,
    pub(crate) request: &'a ModelRequestIRV1,
    pub(crate) semantic_request: Option<&'a ModelRequestIRV1>,
    pub(crate) user_origin_unverified: bool,
    pub(crate) replay: &'a ReplayStore,
}

pub(crate) fn prepare_context(
    store: &ContextHoldStore,
    observation_key: &[u8; 32],
    authorized: &AuthorizedRequestPlan,
    input: ContextRequest<'_>,
    now: Instant,
) -> (ContextIdentityFacts, Option<PreparedHold>, bool) {
    let ContextRequest {
        ingress,
        headers,
        document,
        request,
        semantic_request,
        user_origin_unverified,
        replay,
    } = input;
    let mut identity =
        store
            .digest_key()
            .map_or_else(ContextIdentityFacts::request_scoped, |hold_key| {
                identity_facts(
                    authorized,
                    ingress,
                    headers,
                    document,
                    request,
                    &hold_key,
                    observation_key,
                )
            });
    if user_origin_unverified {
        identity.mark_routing_origin_unverified();
    }
    let native = identity.native_checkpoint();
    // Known continuations/auxiliary calls must not reinterpret a synthetic User.
    // Unknown origin alone does not make a usable request unavailable.
    let mut native_user_unavailable = native.user_origin == routing::NativeUserOrigin::ExistingTurn;
    let ticket = store.digest_key().and_then(|hold_key| {
        let history = visible_history_with_replay(request, &hold_key, Some(replay))?
            .with_semantic_request(semantic_request)?;
        let key = ContextHoldKey::from_request(authorized, ingress, &identity, &hold_key)?;
        let mut prepared = store.prepare(key, &history, now)?;
        prepared.set_native(native);
        native_user_unavailable = prepared.native_user_unavailable();
        if !prepared.history_can_capture() {
            return None;
        }
        prepared.set_history_key(ContextHoldKey::routing_history(
            authorized, ingress, &identity, &hold_key,
        )?);
        Some(prepared)
    });
    (identity, ticket, native_user_unavailable)
}
