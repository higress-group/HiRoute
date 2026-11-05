//! Preserve client literals after decoding identities, before trusting native references.
use std::collections::HashSet;

use serde_json::Value;

use super::{JSON_MARKER_KEY, MARKER_PREFIX};
use crate::replay::{ReplayContentWriter, ReplayError, ReplayStore};

struct LiteralNode {
    start: usize,
    end: usize,
    replacement: super::ContentRef,
    string: bool,
}

/// Preorder occurrences in the unchanged ingress tree. A clone retains these
/// occurrences; equality with another field's locator grants no authority.
pub(crate) struct NativeLiteralNodes {
    nodes: Vec<LiteralNode>,
    tree_nodes: usize,
}

impl NativeLiteralNodes {
    pub(super) fn capture(
        document: &Value,
        generated: &HashSet<usize>,
        replay: &ReplayStore,
    ) -> Result<Self, ReplayError> {
        let mut captured = Self {
            nodes: Vec::new(),
            tree_nodes: 0,
        };
        let mut pool = None;
        capture(document, generated, replay, &mut captured, false, &mut pool)?;
        if let Some(pool) = pool {
            pool.seal()?;
        }
        Ok(captured)
    }

    pub(crate) fn externalize(
        self,
        request: &mut super::ModelRequestIRV1,
        replay: &ReplayStore,
    ) -> Result<(), ReplayError> {
        let mut pool = None;
        let mut next = 0;
        let mut literals = self.nodes.iter().peekable();
        let body = request.native_body.as_mut().ok_or(ReplayError::Integrity)?;
        externalize(body, &mut next, &mut literals)?;
        if next != self.tree_nodes || literals.peek().is_some() {
            return Err(ReplayError::Integrity);
        }
        // Message names are imported as literal identities, unlike semantic
        // text leaves. Back them before the generic ContentValue shortcut.
        for message in &mut request.messages {
            if let Some(name) = &mut message.name
                && name.contains(MARKER_PREFIX)
            {
                if pool.is_none() {
                    pool = Some(replay.begin_content_pool()?);
                }
                *name = pool
                    .as_mut()
                    .ok_or(ReplayError::Integrity)?
                    .append(name.as_bytes())?
                    .wire_marker();
            }
        }
        if let Some(pool) = pool {
            pool.seal()?;
        }
        Ok(())
    }
}

fn capture(
    value: &Value,
    generated: &HashSet<usize>,
    replay: &ReplayStore,
    captured: &mut NativeLiteralNodes,
    parent_literal: bool,
    pool: &mut Option<ReplayContentWriter>,
) -> Result<(), ReplayError> {
    let start = captured.tree_nodes;
    captured.tree_nodes = start.checked_add(1).ok_or(ReplayError::LengthOverflow)?;
    let literal = !parent_literal
        && match value {
            Value::String(text) => {
                text.contains(MARKER_PREFIX) && !generated.contains(&(text.as_ptr() as usize))
            }
            Value::Object(fields) if fields.len() == 1 => {
                if let Some(marker) = fields.get(JSON_MARKER_KEY) {
                    !marker
                        .as_str()
                        .is_some_and(|text| generated.contains(&(text.as_ptr() as usize)))
                } else {
                    fields.contains_key(super::LITERAL_WIRE_KEY)
                        || fields.contains_key(super::WIRE_STRING_PARTS_KEY)
                }
            }
            _ => false,
        };
    let slot = if literal {
        replay.charge_metadata(2 * std::mem::size_of::<LiteralNode>())?;
        let (replacement, string) = match value {
            Value::String(text) => {
                if pool.is_none() {
                    *pool = Some(replay.begin_content_pool()?);
                }
                (
                    pool.as_mut()
                        .ok_or(ReplayError::Integrity)?
                        .append(text.as_bytes())?,
                    true,
                )
            }
            _ => (
                super::literal_json::append_literal_json(value, generated, replay, pool)?,
                false,
            ),
        };
        let slot = captured.nodes.len();
        captured.nodes.push(LiteralNode {
            start,
            end: start,
            replacement,
            string,
        });
        Some(slot)
    } else {
        None
    };
    match value {
        Value::Array(values) => {
            for value in values {
                capture(
                    value,
                    generated,
                    replay,
                    captured,
                    parent_literal || literal,
                    pool,
                )?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                capture(
                    value,
                    generated,
                    replay,
                    captured,
                    parent_literal || literal,
                    pool,
                )?;
            }
        }
        _ => {}
    }
    if let Some(slot) = slot {
        captured.nodes[slot].end = captured.tree_nodes;
    }
    Ok(())
}

fn externalize<'a>(
    value: &mut Value,
    next: &mut usize,
    literals: &mut std::iter::Peekable<std::slice::Iter<'a, LiteralNode>>,
) -> Result<(), ReplayError> {
    if let Some(literal) = literals.peek().filter(|literal| literal.start == *next) {
        *value = if literal.string {
            Value::String(literal.replacement.wire_marker())
        } else {
            literal.replacement.json_marker()
        };
        *next = literal.end;
        literals.next();
        return Ok(());
    }
    *next = next.checked_add(1).ok_or(ReplayError::LengthOverflow)?;
    match value {
        Value::Array(values) => {
            for value in values {
                externalize(value, next, literals)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                externalize(value, next, literals)?;
            }
        }
        _ => {}
    }
    Ok(())
}
