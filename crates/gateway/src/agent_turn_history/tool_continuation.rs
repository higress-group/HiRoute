//! Evidence for a single completed native Responses client-tool batch.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use hiroute_gateway_core::runtime::body::Reservation;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::context_hold::fingerprint::FingerprintBudget;
use crate::replay::ReplayStore;
use crate::runtime_fingerprint::RuntimeHash;
use crate::server::core_runtime::model_ir::{
    ContentPart, MessageRole, ModelRequestIRV1, NativeHandoffItem, NativeHistoryItem, ToolKindV1,
};

use super::boundary::{argument_digest, text_digest, tool_digest};

const MAX_CALLS: usize = 64;
const MAX_SUFFIX_MESSAGES: usize = 256;
const MAX_WIRE_ARGUMENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct CallIdentity {
    id: [u8; 32],
    discovery: bool,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct Invocation {
    identity: CallIdentity,
    digest: [u8; 32],
}

pub(crate) struct DeliveredToolBatch(Box<[Invocation]>);

pub(crate) type ChargedToolBatch = (Arc<DeliveredToolBatch>, Reservation);

/// The accepted encoder prepares the batch before EOS is visible. Only the
/// successful downstream write makes it eligible for the predecessor index.
#[derive(Default)]
pub(crate) struct ToolDeliveryHandoff {
    batch: OnceLock<ChargedToolBatch>,
    delivered: AtomicBool,
    pub(crate) requested: CancellationToken,
    pub(crate) closed: CancellationToken,
}

impl ToolDeliveryHandoff {
    pub(super) const INDEX_BYTES: usize = std::mem::size_of::<Self>() + 512;

    pub(crate) fn prepare(&self, batch: ChargedToolBatch) {
        let _ = self.batch.set(batch);
    }

    pub(super) fn prepared_batch(&self) -> Option<&DeliveredToolBatch> {
        self.batch.get().map(|(batch, _)| batch.as_ref())
    }

    pub(crate) fn confirm(&self) {
        if self.batch.get().is_some() {
            self.delivered.store(true, Ordering::Release);
        }
    }

    pub(crate) fn delivered_batch(&self) -> Option<Arc<DeliveredToolBatch>> {
        if !self.delivered.load(Ordering::Acquire) {
            return None;
        }
        self.batch.get().map(|(batch, _)| Arc::clone(batch))
    }
}

impl DeliveredToolBatch {
    /// The native projector has already reconciled the terminal snapshot with
    /// each streamed done item, or validated the complete JSON response.
    pub(crate) fn from_completed_output(
        output: &[Value],
        has_done: impl Fn(usize) -> bool,
    ) -> Option<Self> {
        if output.len() > MAX_SUFFIX_MESSAGES {
            return None;
        }
        let mut calls = Vec::new();
        let mut remaining = MAX_WIRE_ARGUMENT_BYTES;
        let mut budget = FingerprintBudget::default();
        for (index, item) in output.iter().enumerate() {
            match item.get("type")?.as_str()? {
                "message" | "reasoning" | "web_search_call" => continue,
                "tool_search_call" if has_done(index) && calls.len() < MAX_CALLS => {
                    if item.get("execution")?.as_str()? != "client"
                        || item.get("status").is_some_and(|s| s != "completed")
                        || item.as_object()?.keys().any(|key| {
                            !matches!(
                                key.as_str(),
                                "type"
                                    | "execution"
                                    | "call_id"
                                    | "id"
                                    | "status"
                                    | "arguments"
                                    | "internal_chat_message_metadata_passthrough"
                            )
                        })
                    {
                        return None;
                    }
                    calls.push(discovery_call(item, None, &mut budget)?);
                    continue;
                }
                "function_call" if has_done(index) && calls.len() < MAX_CALLS => {}
                // Other types could be a client tool whose result is also due.
                _ => return None,
            }
            if item
                .get("status")
                .is_some_and(|status| status != "completed")
                || item.get("encrypted_function_args").is_some_and(|args| {
                    !args.is_null() && !args.as_array().is_some_and(Vec::is_empty)
                })
            {
                return None;
            }
            let namespace = match item.get("namespace") {
                None | Some(Value::Null) => None,
                Some(value) => Some(value.as_str()?),
            };
            let raw = item.get("arguments")?.as_str()?;
            remaining = remaining.checked_sub(raw.len())?;
            let arguments = serde_json::from_str(raw).ok()?;
            calls.push(invocation(
                item.get("call_id")?.as_str()?,
                namespace,
                item.get("name")?.as_str()?,
                &arguments,
                None,
                &mut budget,
            )?);
        }
        calls.sort_unstable();
        if calls.is_empty()
            || calls
                .windows(2)
                .any(|pair| pair[0].identity.id == pair[1].identity.id)
        {
            return None;
        }
        Some(Self(calls.into_boxed_slice()))
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.0)
    }

    pub(super) fn matches(&self, suffix: &ToolResultSuffix) -> bool {
        self.0
            .iter()
            .map(|call| call.identity)
            .eq(suffix.results.iter().copied())
            && suffix
                .calls
                .iter()
                .all(|call| self.0.binary_search(call).is_ok())
    }
}

pub(super) struct ToolResultSuffix {
    calls: Vec<Invocation>,
    results: Vec<CallIdentity>,
}

impl ToolResultSuffix {
    pub(super) fn measure(
        request: &ModelRequestIRV1,
        layout: &[NativeHistoryItem],
        start: usize,
        replay: &ReplayStore,
        budget: &mut FingerprintBudget,
    ) -> Option<Self> {
        let messages = request.messages.get(start..)?;
        if messages.len() > MAX_SUFFIX_MESSAGES {
            return None;
        }
        let mut calls = Vec::new();
        let mut results = Vec::new();
        for (offset, (message, native)) in messages.iter().zip(&layout[start..]).enumerate() {
            match native.handoff {
                NativeHandoffItem::GoalContext
                    if message.role == MessageRole::Assistant
                        && message.name.is_none()
                        && message.content.is_empty() =>
                {
                    continue;
                }
                NativeHandoffItem::ToolSearchCall
                    if results.is_empty() && calls.len() < MAX_CALLS =>
                {
                    calls.push(discovery_call(
                        request.native_history_item(native)?,
                        Some(replay),
                        budget,
                    )?);
                    continue;
                }
                NativeHandoffItem::ToolSearchOutput if results.len() < MAX_CALLS => {
                    let id = request
                        .native_history_item(native)?
                        .get("call_id")?
                        .as_str()?;
                    results.push(CallIdentity {
                        id: text_digest(id, Some(replay), budget)?,
                        discovery: true,
                    });
                    continue;
                }
                NativeHandoffItem::None => {}
                _ => return None,
            }
            if !native.modeled {
                return None;
            }
            let index = start + offset;
            match (&message.role, message.content.as_slice()) {
                (
                    MessageRole::User,
                    [
                        ContentPart::ToolResult {
                            logical_id,
                            tool_kind: ToolKindV1::Function,
                            ..
                        },
                    ],
                ) if results.len() < MAX_CALLS
                    && request
                        .responses_item_statuses
                        .get(&index)
                        .is_none_or(|s| s != "in_progress") =>
                {
                    results.push(CallIdentity {
                        id: call_id(logical_id, budget)?,
                        discovery: false,
                    });
                }
                (
                    MessageRole::Assistant,
                    [
                        ContentPart::ToolCall {
                            logical_id,
                            tool_kind: ToolKindV1::Function,
                            namespace,
                            name,
                            arguments,
                            raw_arguments: None,
                            encrypted_function_args,
                        },
                    ],
                ) if results.is_empty()
                    && calls.len() < MAX_CALLS
                    && encrypted_function_args.as_ref().is_none_or(Vec::is_empty) =>
                {
                    calls.push(invocation(
                        logical_id,
                        namespace.as_deref(),
                        name,
                        arguments,
                        Some(replay),
                        budget,
                    )?);
                }
                (MessageRole::Assistant, _)
                    if results.is_empty()
                        && (request.responses_reasoning_history.contains_key(&index)
                            || request.responses_search_history.contains_key(&index)
                            || message
                                .content
                                .iter()
                                .all(|part| matches!(part, ContentPart::Text { .. }))) => {}
                _ => return None,
            }
        }
        if results.is_empty() {
            return None;
        }
        results.sort_unstable();
        calls.sort_unstable();
        if results.windows(2).any(|pair| pair[0].id == pair[1].id)
            || calls
                .windows(2)
                .any(|pair| pair[0].identity.id == pair[1].identity.id)
        {
            return None;
        }
        Some(Self { calls, results })
    }
}

fn call_id(id: &str, budget: &mut FingerprintBudget) -> Option<[u8; 32]> {
    budget.bytes(id.len())?;
    (!id.trim().is_empty()).then(|| RuntimeHash::digest(id.as_bytes()))
}

fn invocation(
    id: &str,
    namespace: Option<&str>,
    name: &str,
    arguments: &Value,
    replay: Option<&ReplayStore>,
    budget: &mut FingerprintBudget,
) -> Option<Invocation> {
    if name.is_empty() {
        return None;
    }
    Some(Invocation {
        identity: CallIdentity {
            id: call_id(id, budget)?,
            discovery: false,
        },
        digest: tool_digest(
            id,
            ToolKindV1::Function,
            namespace,
            name,
            arguments,
            replay,
            budget,
        )?,
    })
}

fn discovery_call(
    item: &Value,
    replay: Option<&ReplayStore>,
    budget: &mut FingerprintBudget,
) -> Option<Invocation> {
    let arguments = item.get("arguments")?;
    if !arguments.is_object() {
        return None;
    }
    let id = item.get("call_id")?.as_str()?;
    if id.trim().is_empty() {
        return None;
    }
    Some(Invocation {
        identity: CallIdentity {
            id: text_digest(id, replay, budget)?,
            discovery: true,
        },
        digest: argument_digest(
            RuntimeHash::new_with_prefix(b"client-tool-search/v1"),
            arguments,
            replay,
            budget,
        )?,
    })
}
