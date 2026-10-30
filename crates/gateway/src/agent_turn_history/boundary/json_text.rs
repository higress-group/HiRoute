//! Hash a normalized string independently of inline, string or JSON Replay storage.
//! Serde owns syntax; the visitor borrows its scratch instead of cloning the text.
use super::*;
use crate::content_ref::JsonValueExt;
use crate::context_hold::fingerprint::MAX_BYTES;
use hiroute_gateway_core::runtime::body::MemoryRole;
use serde::de::{DeserializeSeed, Visitor};

pub(super) fn digest(
    value: &serde_json::Value,
    replay: &ReplayStore,
    budget: &mut FingerprintBudget,
) -> Option<([u8; 32], bool)> {
    if let Some(reference) = value.content_ref() {
        let bytes = usize::try_from(reference.byte_len()).ok()?;
        // JSON can escape each plaintext byte. Bound syntax work and reserve
        // serde's worst-case scratch before opening the optional backing.
        if bytes > MAX_BYTES.checked_mul(6)?.checked_add(2)? {
            return None;
        }
        let scratch = bytes.checked_mul(2)?.checked_add(4096)?;
        let _scratch = replay
            .budget()
            .reserve(MemoryRole::ModelIrBacking, scratch)
            .ok()?;
        let mut reader = replay.reader(&reference).ok()?;
        let mut decoder = serde_json::Deserializer::from_reader(&mut reader);
        let digest = Seed(budget).deserialize(&mut decoder).ok()?;
        decoder.end().ok()?;
        drop(decoder);
        reader.verify_terminal().ok()?;
        Some(digest)
    } else {
        let text = value.as_str()?;
        let nonempty = !text.is_empty()
            && !text
                .content_ref()
                .is_some_and(|reference| reference.byte_len() == 0);
        Some((text_digest(text, Some(replay), budget)?, nonempty))
    }
}

struct Seed<'a>(&'a mut FingerprintBudget);
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = ([u8; 32], bool);
    fn deserialize<D: serde::Deserializer<'de>>(self, decoder: D) -> Result<Self::Value, D::Error> {
        decoder.deserialize_str(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = ([u8; 32], bool);
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("canonical string evidence")
    }
    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
        plain_digest(text, self.0).ok_or_else(|| E::custom("optional comparison budget exceeded"))
    }
}

fn plain_digest(text: &str, budget: &mut FingerprintBudget) -> Option<([u8; 32], bool)> {
    budget.bytes(text.len())?;
    Some((RuntimeHash::digest(text.as_bytes()), !text.is_empty()))
}
