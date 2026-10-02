//! Bounded evidence for the gateway's own completion claim, not a wire schema.

use hiroute_gateway_core::runtime::body::MemoryRole;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::{
    IngressProtocol, ModelIrError, NativeTerminalOutcome, ProjectionState, ProtocolAdapterError,
    ResponseItemEvidence, digest_item, response_item_identity,
};

pub(super) type ResponseDeltaKey = (u32, u32, &'static str);

pub(super) struct ResponseDeltaEvidence {
    item_id: [u8; 32],
    hash: Sha256,
    prefix_length: usize,
    saw_delta: bool,
    done_digest: Option<[u8; 32]>,
}

pub(super) struct AddedTextPrefix {
    position: usize,
    kind: &'static str,
    length: usize,
    hash: [u8; 32],
}

impl ProjectionState {
    pub(super) fn chat_terminal(&self) -> NativeTerminalOutcome {
        if self.chat_uncertain || !self.chat_seen {
            NativeTerminalOutcome::Unknown
        } else {
            self.chat_finish.unwrap_or(NativeTerminalOutcome::Unknown)
        }
    }

    pub(super) fn set_terminal(
        &mut self,
        outcome: NativeTerminalOutcome,
    ) -> Result<(), ProtocolAdapterError> {
        if self.terminal.replace(outcome).is_some() {
            return Err(ModelIrError::InvalidResponseLifecycle(
                "native response emitted more than one terminal".into(),
            )
            .into());
        }
        Ok(())
    }

    pub(super) fn mark_unowned_uncertain(&mut self) {
        match self.protocol {
            IngressProtocol::Responses => self.response_items_uncertain = true,
            IngressProtocol::Messages => self.messages_stop = Some(NativeTerminalOutcome::Unknown),
            IngressProtocol::ChatCompletions => self.chat_uncertain = true,
        }
    }

    pub(in crate::server::core_runtime::adapters::response) fn withhold_terminal_completion(
        &mut self,
    ) {
        self.terminal = Some(NativeTerminalOutcome::Unknown);
    }

    pub(super) fn observe_response_item(
        &mut self,
        index: u32,
        item: &Map<String, Value>,
        done: bool,
    ) {
        if !self.response_items.contains_key(&index) {
            // Failure to retain evidence must not reject otherwise legal wire.
            let Ok(charge) = self.budget.reserve(MemoryRole::SemanticState, 128) else {
                self.response_items_uncertain = true;
                return;
            };
            self.retained.push(charge);
            self.response_items
                .insert(index, ResponseItemEvidence::default());
        }
        let identity = response_item_identity(item);
        let added_text = if done {
            Vec::new()
        } else {
            added_text_prefixes(item)
        };
        if !added_text.is_empty() {
            let charge = added_text
                .len()
                .checked_mul(std::mem::size_of::<AddedTextPrefix>() + 16)
                .and_then(|size| self.budget.reserve(MemoryRole::SemanticState, size).ok());
            if let Some(charge) = charge {
                self.retained.push(charge);
                self.response_items
                    .get_mut(&index)
                    .expect("tracked index exists")
                    .added_text
                    .extend(added_text);
            } else {
                self.response_items_uncertain = true;
            }
        }
        let known = self
            .response_items
            .get_mut(&index)
            .expect("tracked index exists");
        if identity.is_none()
            || known
                .identity
                .is_some_and(|previous| Some(previous) != identity)
        {
            self.response_items_uncertain = true;
        }
        known.identity = identity;
        if done {
            let digest = digest_item(item);
            if known.done.is_some_and(|previous| previous != digest) {
                self.response_items_uncertain = true;
            }
            known.done = Some(digest);
        }
    }

    pub(super) fn reconcile_response_items(&mut self, response: &Map<String, Value>) {
        if self.response_items.is_empty() {
            return;
        }
        let Some(output) = response.get("output").and_then(Value::as_array) else {
            self.response_items_uncertain = true;
            return;
        };
        for (&index, known) in &self.response_items {
            let Some(item) = output.get(index as usize).and_then(Value::as_object) else {
                self.response_items_uncertain = true;
                return;
            };
            if known.identity != response_item_identity(item)
                || known.done.is_some_and(|digest| digest != digest_item(item))
                || known.added_text.iter().any(|prefix| {
                    final_text(item, prefix.position, prefix.kind)
                        .and_then(|text| text.as_bytes().get(..prefix.length))
                        .is_none_or(|text| {
                            let digest: [u8; 32] = Sha256::digest(text).into();
                            digest != prefix.hash
                        })
                })
            {
                self.response_items_uncertain = true;
            }
        }
    }

    pub(super) fn observe_response_payload(
        &mut self,
        event: &str,
        object: &Map<String, Value>,
    ) -> bool {
        if event.ends_with(".delta") && !object.get("delta").is_some_and(Value::is_string) {
            self.response_items_uncertain = true;
            return false;
        }
        let delta = event
            .ends_with(".delta")
            .then(|| object.get("delta").and_then(Value::as_str))
            .flatten()
            .filter(|delta| !delta.is_empty());
        let Some((kind, complete_field, position_field)) = delta_kind(event) else {
            return delta.is_some();
        };
        let completed = event
            .ends_with(".done")
            .then(|| object.get(complete_field).and_then(Value::as_str))
            .flatten();
        if delta.is_none() && completed.is_none() {
            return false;
        }
        let identity = object.get("item_id").and_then(Value::as_str);
        let index = object
            .get("output_index")
            .and_then(Value::as_u64)
            .and_then(|index| u32::try_from(index).ok());
        let position = position_field.map_or(Some(0), |field| {
            object
                .get(field)
                .and_then(Value::as_u64)
                .and_then(|index| u32::try_from(index).ok())
        });
        let (Some(identity), Some(index), Some(position)) = (identity, index, position) else {
            self.response_items_uncertain = true;
            return delta.is_some();
        };
        let key = (index, position, kind);
        let had_prior = self.response_deltas.contains_key(&key);
        if !had_prior {
            let prefix_length = self
                .response_items
                .get(&index)
                .and_then(|item| {
                    item.added_text
                        .iter()
                        .find(|prefix| prefix.position == position as usize && prefix.kind == kind)
                })
                .map_or(0, |prefix| prefix.length);
            let charge_size = std::mem::size_of::<ResponseDeltaEvidence>() + 128;
            let Ok(charge) = self.budget.reserve(MemoryRole::SemanticState, charge_size) else {
                self.response_items_uncertain = true;
                return delta.is_some();
            };
            self.retained.push(charge);
            self.response_deltas.insert(
                key,
                ResponseDeltaEvidence {
                    item_id: Sha256::digest(identity.as_bytes()).into(),
                    hash: Sha256::new(),
                    prefix_length,
                    saw_delta: false,
                    done_digest: None,
                },
            );
        }
        let known = self
            .response_deltas
            .get_mut(&key)
            .expect("tracked delta exists");
        let item_id: [u8; 32] = Sha256::digest(identity.as_bytes()).into();
        if known.item_id != item_id {
            self.response_items_uncertain = true;
        }
        if let Some(delta) = delta {
            if known.done_digest.is_some() {
                self.response_items_uncertain = true;
            }
            known.hash.update(delta.as_bytes());
            known.saw_delta = true;
        }
        if let Some(completed) = completed {
            let digest: [u8; 32] = Sha256::digest(completed.as_bytes()).into();
            if known.done_digest.replace(digest).is_some() {
                self.response_items_uncertain = true;
            }
            if !had_prior && delta.is_none() {
                // A done-only event can establish complete textual evidence.
                known.hash.update(completed.as_bytes());
            }
        }
        delta.is_some()
    }

    pub(super) fn reconcile_response_deltas(&mut self, response: &Map<String, Value>) {
        if self.response_deltas.is_empty() {
            return;
        }
        let output = response.get("output").and_then(Value::as_array);
        for (&(index, position, kind), known) in &self.response_deltas {
            let item = output.and_then(|output| output.get(index as usize));
            let id = item.and_then(|item| item.get("id")).and_then(Value::as_str);
            let item_type = item
                .and_then(|item| item.get("type"))
                .and_then(Value::as_str);
            let expected_type = match kind {
                "output_text" | "refusal" => "message",
                "reasoning_summary_text" | "reasoning_text" => "reasoning",
                "function_call_arguments" => "function_call",
                "custom_tool_call_input" => "custom_tool_call",
                _ => unreachable!("tracked delta kind"),
            };
            let text = item
                .and_then(Value::as_object)
                .and_then(|item| final_text(item, position as usize, kind));
            let observed: [u8; 32] = known.hash.clone().finalize().into();
            if item_type != Some(expected_type)
                || id.is_none_or(|id| {
                    let digest: [u8; 32] = Sha256::digest(id.as_bytes()).into();
                    known.item_id != digest
                })
                || text.is_none_or(|text| {
                    let prefix = if known.saw_delta {
                        known.prefix_length
                    } else {
                        0
                    };
                    text.as_bytes().get(prefix..).is_none_or(|suffix| {
                        let digest: [u8; 32] = Sha256::digest(suffix).into();
                        observed != digest
                    }) || known.done_digest.is_some_and(|digest| {
                        let final_digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
                        digest != final_digest
                    })
                })
            {
                self.response_items_uncertain = true;
            }
        }
    }
}

fn added_text_prefixes(item: &Map<String, Value>) -> Vec<AddedTextPrefix> {
    let mut prefixes = Vec::new();
    let mut capture = |position, kind, text: &str| {
        if !text.is_empty() {
            prefixes.push(AddedTextPrefix {
                position,
                kind,
                length: text.len(),
                hash: Sha256::digest(text.as_bytes()).into(),
            });
        }
    };
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for (position, block) in content.iter().enumerate() {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                let kind = if item.get("type").and_then(Value::as_str) == Some("reasoning") {
                    "reasoning_text"
                } else {
                    "output_text"
                };
                capture(position, kind, text);
            }
            if let Some(refusal) = block.get("refusal").and_then(Value::as_str) {
                capture(position, "refusal", refusal);
            }
        }
    }
    if let Some(summary) = item.get("summary").and_then(Value::as_array) {
        for (position, block) in summary.iter().enumerate() {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                capture(position, "reasoning_summary_text", text);
            }
        }
    }
    for (kind, field) in [
        ("function_call_arguments", "arguments"),
        ("custom_tool_call_input", "input"),
    ] {
        if let Some(text) = item.get(field).and_then(Value::as_str) {
            capture(0, kind, text);
        }
    }
    prefixes
}

fn delta_kind(event: &str) -> Option<(&'static str, &'static str, Option<&'static str>)> {
    if event.starts_with("response.output_text.") {
        Some(("output_text", "text", Some("content_index")))
    } else if event.starts_with("response.refusal.") {
        Some(("refusal", "refusal", Some("content_index")))
    } else if event.starts_with("response.reasoning_summary_text.") {
        Some(("reasoning_summary_text", "text", Some("summary_index")))
    } else if event.starts_with("response.reasoning_text.") {
        Some(("reasoning_text", "text", Some("content_index")))
    } else if event.starts_with("response.function_call_arguments.") {
        Some(("function_call_arguments", "arguments", None))
    } else if event.starts_with("response.custom_tool_call_input.") {
        Some(("custom_tool_call_input", "input", None))
    } else {
        None
    }
}

fn final_text<'a>(item: &'a Map<String, Value>, position: usize, kind: &str) -> Option<&'a str> {
    match kind {
        "output_text" | "refusal" | "reasoning_text" => item
            .get("content")?
            .as_array()?
            .get(position)?
            .get(if kind != "refusal" { "text" } else { "refusal" })?
            .as_str(),
        "reasoning_summary_text" => item
            .get("summary")?
            .as_array()?
            .get(position)?
            .get("text")?
            .as_str(),
        "function_call_arguments" => item.get("arguments")?.as_str(),
        "custom_tool_call_input" => item.get("input")?.as_str(),
        _ => None,
    }
}
