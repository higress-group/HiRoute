//! Bounded native wire hints. Serde remains the grammar authority; these ranges
//! authorize only raw JSON forwarding, never canonical history/content evidence.
use std::collections::BTreeMap;
use std::io::Read;
use std::ops::Range;
use std::sync::{Arc, Weak};

use hiroute_gateway_core::runtime::body::{ChargedBytesBuilder, MemoryRole, Reservation};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::value::RawValue;

use super::{
    RANGE_METADATA_BYTES, ReplayError, ReplayStore, STREAM_METADATA_BYTES, StoreInner, lock,
};
use crate::content_ref::ContentRef;

const MAX_ROOTS: usize = 256;
const MAX_INPUT: usize = 1024 * 1024;
const MAX_MESSAGES: usize = 1024;

pub(crate) struct RawJsonIndex {
    owner: Weak<StoreInner>,
    source: ContentRef,
    roots: BTreeMap<String, ContentRef>,
    messages: Vec<Option<ContentRef>>,
    _workspace: Reservation,
}

pub(super) struct RawJsonAlias {
    _charge: Arc<Reservation>,
}

impl RawJsonIndex {
    pub(crate) fn capture(store: &ReplayStore, raw: &ContentRef) -> Option<Self> {
        let length = usize::try_from(raw.byte_len()).ok()?;
        if !(32 * 1024..=MAX_INPUT).contains(&length) || store.snapshot().disk_backed {
            return None;
        }
        let workspace = store
            .budget()
            .reserve(
                MemoryRole::ModelIrBacking,
                length.checked_add(MAX_ROOTS * 256 + MAX_MESSAGES * 128)?,
            )
            .ok()?;
        let mut input =
            ChargedBytesBuilder::new(store.budget(), MemoryRole::ModelIrBacking, length).ok()?;
        input.resize_zeroed(length).ok()?;
        let mut reader = store.reader(raw).ok()?;
        reader.read_exact(input.as_mut_slice()).ok()?;
        reader.verify_terminal().ok()?;
        let bytes = input.finish();
        let ranges = root_ranges(bytes.bytes())?;
        let mut messages = Vec::new();
        if let Some(span) = ranges.get("messages") {
            let input = &bytes.bytes()[span.clone()];
            for part in array_ranges(input)? {
                let message = &input[part.clone()];
                // Flat native messages need no nested argument normalization.
                // Duplicate decoded keys or nested shapes use the generic path.
                let flat = root_ranges(message).is_some_and(|fields| {
                    fields
                        .values()
                        .all(|value| !matches!(message[value.start], b'{' | b'['))
                });
                messages.push(if flat {
                    Some(ContentRef::new(
                        raw.stream_ordinal(),
                        raw.byte_offset()
                            .checked_add((span.start + part.start) as u64)?,
                        message.len() as u64,
                        super::stream::json_escaped_len(message).ok()?,
                    ))
                } else {
                    None
                });
            }
        }
        let mut roots = BTreeMap::new();
        for (key, range) in ranges {
            if key == "messages" {
                continue;
            }
            let value = &bytes.bytes()[range.clone()];
            roots.insert(
                key,
                ContentRef::new(
                    raw.stream_ordinal(),
                    raw.byte_offset().checked_add(range.start as u64)?,
                    value.len() as u64,
                    super::stream::json_escaped_len(value).ok()?,
                ),
            );
        }
        Some(Self {
            owner: Arc::downgrade(&store.inner),
            source: raw.clone(),
            roots,
            messages,
            _workspace: workspace,
        })
    }

    pub(crate) fn get(&self, key: &str) -> Option<&ContentRef> {
        self.roots.get(key)
    }

    pub(crate) fn message(&self, index: usize) -> Option<&ContentRef> {
        self.messages.get(index)?.as_ref()
    }

    pub(crate) fn register(
        &self,
        store: &ReplayStore,
        references: &[ContentRef],
    ) -> Result<(), ReplayError> {
        if !Weak::ptr_eq(&self.owner, &Arc::downgrade(&store.inner)) {
            return Err(ReplayError::Integrity);
        }
        let source = lock(&store.inner.streams)
            .get(&self.source.stream_ordinal())
            .cloned()
            .ok_or(ReplayError::UnknownContentRef)?;
        if !source.is_raw() || source.reference != self.source {
            return Err(ReplayError::Integrity);
        }
        let end = self
            .source
            .byte_offset()
            .checked_add(self.source.byte_len())
            .ok_or(ReplayError::LengthOverflow)?;
        let references: std::collections::BTreeSet<_> = references.iter().cloned().collect();
        let mut previous_end = self.source.byte_offset();
        if references.iter().any(|range| {
            let known = self.roots.values().any(|value| value == range)
                || self.messages.iter().flatten().any(|value| value == range);
            let overlaps = range.byte_offset() < previous_end;
            previous_end = range.byte_offset().saturating_add(range.byte_len());
            !known
                || overlaps
                || range.byte_offset() < self.source.byte_offset()
                || range
                    .byte_offset()
                    .checked_add(range.byte_len())
                    .is_none_or(|last| last > end)
        }) {
            return Err(ReplayError::Integrity);
        }
        let mut aliases = lock(&store.inner.raw_json_aliases);
        let count = references
            .iter()
            .filter(|range| !aliases.contains_key(*range))
            .count();
        if count == 0 {
            return Ok(());
        }
        let charge = Arc::new(
            store.budget().reserve(
                MemoryRole::ModelIrBacking,
                count
                    .checked_mul(RANGE_METADATA_BYTES)
                    .and_then(|v| v.checked_add(STREAM_METADATA_BYTES))
                    .ok_or(ReplayError::LengthOverflow)?,
            )?,
        );
        for reference in &references {
            aliases
                .entry(reference.clone())
                .or_insert_with(|| RawJsonAlias {
                    _charge: charge.clone(),
                });
        }
        Ok(())
    }
}

pub(crate) fn root_ranges(bytes: &[u8]) -> Option<BTreeMap<String, Range<usize>>> {
    struct Roots<'a>(Vec<(String, &'a RawValue)>);
    impl<'de> Deserialize<'de> for Roots<'de> {
        fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
            struct RootVisitor;
            impl<'de> Visitor<'de> for RootVisitor {
                type Value = Roots<'de>;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("a bounded JSON object")
                }
                fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                    let mut values = BTreeMap::new();
                    while let Some((key, value)) = map.next_entry::<String, &'de RawValue>()? {
                        if values.len() >= MAX_ROOTS || values.insert(key, value).is_some() {
                            return Err(de::Error::custom("ambiguous or excessive root members"));
                        }
                    }
                    Ok(Roots(values.into_iter().collect()))
                }
            }
            decoder.deserialize_map(RootVisitor)
        }
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let values = Roots::deserialize(&mut decoder).ok()?;
    decoder.end().ok()?;
    values
        .0
        .into_iter()
        .map(|(key, value)| {
            let start = (value.get().as_ptr() as usize).checked_sub(bytes.as_ptr() as usize)?;
            let end = start.checked_add(value.get().len())?;
            bytes.get(start..end)?;
            Some((key, start..end))
        })
        .collect()
}

pub(crate) fn array_ranges(bytes: &[u8]) -> Option<Vec<Range<usize>>> {
    struct Items<'a>(Vec<&'a RawValue>);
    impl<'de> Deserialize<'de> for Items<'de> {
        fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
            struct ItemVisitor;
            impl<'de> Visitor<'de> for ItemVisitor {
                type Value = Items<'de>;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("a bounded JSON array")
                }
                fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
                    let mut values = Vec::new();
                    while let Some(value) = seq.next_element::<&'de RawValue>()? {
                        if values.len() == MAX_MESSAGES {
                            return Err(de::Error::custom("too many native message ranges"));
                        }
                        values.push(value);
                    }
                    Ok(Items(values))
                }
            }
            decoder.deserialize_seq(ItemVisitor)
        }
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let values = Items::deserialize(&mut decoder).ok()?;
    decoder.end().ok()?;
    values
        .0
        .into_iter()
        .map(|value| {
            let start = (value.get().as_ptr() as usize).checked_sub(bytes.as_ptr() as usize)?;
            let end = start.checked_add(value.get().len())?;
            bytes.get(start..end)?;
            Some(start..end)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::{ReplayConfig, ReplayManager};
    use hiroute_gateway_core::runtime::body::BudgetTree;
    use serde_json::json;
    struct Fixture {
        store: ReplayStore,
        root: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "hiroute-raw-ranges-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let manager = ReplayManager::open(ReplayConfig {
                root: root.clone(),
                ..ReplayConfig::production_default()
            })
            .unwrap();
            let store = manager
                .begin_request(
                    BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024)
                        .unwrap()
                        .stream(8 * 1024 * 1024)
                        .unwrap(),
                )
                .unwrap();
            Self { store, root }
        }
        fn capture(&self, bytes: &[u8]) -> ContentRef {
            let mut writer = self.store.begin_raw().unwrap();
            writer.append(bytes).unwrap();
            writer.seal().unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn raw_root_index_rejects_ambiguous_invalid_and_unbounded_input() {
        for bytes in [
            br#"{"a":1,"a":2}"#.as_slice(),
            br#"{"a":1,"\u0061":2}"#,
            br#"{"a":1} false"#,
            br#"{"a":[}"#,
            br#"[]"#,
        ] {
            assert!(root_ranges(bytes).is_none(), "{bytes:?}");
        }
        let many = json!(
            (0..257)
                .map(|i| (format!("k{i}"), json!(i)))
                .collect::<serde_json::Map<_, _>>()
        );
        assert!(root_ranges(&serde_json::to_vec(&many).unwrap()).is_none());
        let bytes = br#"{ "a": ["x\\\"", {"b":true}], "\u0063": "\ud83e\udd80" }"#;
        let ranges = root_ranges(bytes).unwrap();
        assert_eq!(&bytes[ranges["c"].clone()], br#""\ud83e\udd80""#);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes[ranges["a"].clone()]).unwrap(),
            serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["a"]
        );
    }

    #[test]
    fn message_range_index_is_bounded_and_declines_duplicate_or_nested_message_shapes() {
        assert_eq!(array_ranges(b"[null,1,{},[]]").unwrap().len(), 4);
        for invalid in [b"[1,]".as_slice(), b"[1] false", b"{\"n\":1}"] {
            assert!(array_ranges(invalid).is_none());
        }
        let full = serde_json::to_vec(&vec![json!({}); MAX_MESSAGES]).unwrap();
        assert_eq!(array_ranges(&full).unwrap().len(), MAX_MESSAGES);
        let over = serde_json::to_vec(&vec![json!({}); MAX_MESSAGES + 1]).unwrap();
        assert!(array_ranges(&over).is_none());
        let fixture = Fixture::new();
        let text = "x".repeat(40000);
        let bytes = format!(r#"{{"messages":[{{"role":"user","content":"{text}"}},{{"role":"assistant","content":"first","content":"second"}},{{"role":"assistant","content":null,"tool_calls":[]}}]}}"#).into_bytes();
        let raw = fixture.capture(&bytes);
        let index = RawJsonIndex::capture(&fixture.store, &raw).unwrap();
        assert!(index.message(0).is_some());
        assert!(index.message(1).is_none());
        assert!(index.message(2).is_none());
        let valid = index.message(0).unwrap();
        let forged = ContentRef::new(
            valid.stream_ordinal(),
            valid.byte_offset(),
            valid.byte_len() - 1,
            valid.json_escaped_len(),
        );
        assert!(index.register(&fixture.store, &[forged]).is_err());
    }

    #[test]
    fn raw_ranges_require_their_current_owner_and_never_authorize_semantic_content() {
        let fixture = Fixture::new();
        let value = json!({"messages":[{"role":"user","content":"x".repeat(40000)}]});
        let bytes = serde_json::to_vec(&value).unwrap();
        let raw = fixture.capture(&bytes);
        let index = RawJsonIndex::capture(&fixture.store, &raw).unwrap();
        let reference = index.message(0).unwrap();
        assert!(fixture.store.reader(reference).is_err());
        let foreign = Fixture::new();
        let _same = foreign.capture(&bytes);
        assert!(
            index
                .register(&foreign.store, std::slice::from_ref(reference))
                .is_err()
        );
        index
            .register(&fixture.store, std::slice::from_ref(reference))
            .unwrap();
        assert!(!fixture.store.has_reference(reference));
        assert!(!fixture.store.is_canonical_json(reference));
        assert!(fixture.store.runtime_fingerprint(reference).is_err());
        let mut actual = Vec::new();
        let mut reader = fixture.store.reader(reference).unwrap();
        reader.read_to_end(&mut actual).unwrap();
        reader.verify_terminal().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&actual).unwrap(),
            value["messages"][0]
        );
        fixture
            .store
            .prevalidate(std::slice::from_ref(&raw))
            .unwrap();
        let lease = fixture.store.content_lease().unwrap();
        assert!(lease.reader(reference).is_err());
        fixture.store.release_stream(&raw).unwrap();
        assert!(fixture.store.reader(reference).is_err());
        assert!(
            index
                .register(&fixture.store, std::slice::from_ref(reference))
                .is_err()
        );
        drop(reader);
    }
}
