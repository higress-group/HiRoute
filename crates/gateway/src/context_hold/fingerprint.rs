//! Optional evidence work is bounded independently of the forwarded body.
use serde_json::Value;

use crate::content_ref::{ContentValueExt, JsonValueExt};

pub(crate) const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_ITEMS: usize = 65_536;
const MAX_DEPTH: usize = 64;

pub(crate) struct FingerprintBudget {
    bytes: usize,
    items: usize,
}

impl Default for FingerprintBudget {
    fn default() -> Self {
        Self {
            bytes: MAX_BYTES,
            items: MAX_ITEMS,
        }
    }
}

impl FingerprintBudget {
    pub(crate) fn bytes(&mut self, count: usize) -> Option<()> {
        match self.bytes.checked_sub(count) {
            Some(left) => {
                self.bytes = left;
                Some(())
            }
            None => {
                self.bytes = 0;
                None
            }
        }
    }

    pub(crate) fn items(&mut self, count: usize) -> Option<()> {
        match self.items.checked_sub(count) {
            Some(left) => {
                self.items = left;
                Some(())
            }
            None => {
                self.items = 0;
                None
            }
        }
    }

    /// Preflight before opening Replay or sorting JSON keys. Escaped inline
    /// strings use their worst-case wire size; references carry exact lengths.
    pub(crate) fn json(&mut self, value: &Value) -> Option<bool> {
        self.json_at(value, 0)
    }

    fn json_at(&mut self, value: &Value, depth: usize) -> Option<bool> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.items(1)?;
        self.bytes(2)?;
        if let Some(reference) = value.content_ref() {
            self.bytes(usize::try_from(reference.byte_len()).ok()?)?;
            return Some(true);
        }
        let mut referenced = false;
        match value {
            Value::String(text) => {
                if let Some(reference) = text.content_ref() {
                    self.bytes(usize::try_from(reference.json_escaped_len()).ok()?)?;
                    referenced = true;
                } else {
                    self.bytes(text.len().checked_mul(6)?)?;
                }
            }
            Value::Array(values) => {
                if values.len() > self.items {
                    return None;
                }
                for value in values {
                    referenced |= self.json_at(value, depth + 1)?;
                }
            }
            Value::Object(values) => {
                if values.len() > self.items {
                    return None;
                }
                for (key, value) in values {
                    self.bytes(key.len().checked_mul(6)?.checked_add(4)?)?;
                    referenced |= self.json_at(value, depth + 1)?;
                }
            }
            Value::Number(number) => self.bytes(number.to_string().len())?,
            _ => self.bytes(5)?,
        }
        Some(referenced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_ref::ContentRef;

    #[test]
    fn rejects_large_inline_replay_and_empty_collection_work_without_reading_replay() {
        for value in [
            Value::String("x".repeat(MAX_BYTES / 6 + 1)),
            ContentRef::new(1, 0, MAX_BYTES as u64 + 1, MAX_BYTES as u64 + 1).json_marker(),
            Value::Array(vec![Value::Null; MAX_ITEMS + 1]),
        ] {
            assert!(FingerprintBudget::default().json(&value).is_none());
        }
        let mut deep = Value::Null;
        for _ in 0..MAX_DEPTH + 2 {
            deep = Value::Array(vec![deep]);
        }
        assert!(FingerprintBudget::default().json(&deep).is_none());
        assert_eq!(
            FingerprintBudget::default().json(&serde_json::json!({"ok":[1,"text"]})),
            Some(false)
        );
    }
}
