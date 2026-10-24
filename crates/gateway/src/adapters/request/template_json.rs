//! Locate value strings in serde-generated JSON without interpreting literal keys.
use std::ops::Range;

/// This consumes only valid compact JSON just emitted by serde_json, not incoming
/// syntax. Quotes escaped within a string are not token boundaries. A complete
/// string followed by ':' is an object key and cannot contain a content reference.
/// Ranges exclude quotes and borrow the original bytes; no second body or key table
/// is allocated, and the JSON wire representation stays unchanged.
pub(super) fn value_strings(bytes: &[u8]) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut cursor = 0;
    std::iter::from_fn(move || {
        loop {
            while bytes.get(cursor).is_some_and(|byte| *byte != b'"') {
                cursor += 1;
            }
            if cursor == bytes.len() {
                return None;
            }
            cursor += 1;
            let start = cursor;
            loop {
                match bytes.get(cursor) {
                    Some(b'\\') => cursor += 2,
                    Some(b'"') => break,
                    Some(_) => cursor += 1,
                    None => return None, // unreachable for serde-generated input
                }
            }
            let end = cursor;
            cursor += 1;
            if bytes.get(cursor) != Some(&b':') {
                return Some(start..end);
            }
        }
    })
}

#[cfg(test)]
#[path = "template_json_tests.rs"]
mod tests;
