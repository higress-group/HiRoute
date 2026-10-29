//! Bounded native wire hints. Serde remains the grammar authority; these ranges
//! authorize only raw JSON forwarding, never canonical history/content evidence.
use std::collections::BTreeMap;
use std::io::Read;
use std::ops::Range;
use std::sync::{Arc, Weak};

use hiroute_gateway_core::runtime::body::{ChargedBytesBuilder, MemoryRole, Reservation};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};
use serde_json::value::RawValue;

use super::{
    RANGE_METADATA_BYTES, ReplayError, ReplayStore, STREAM_METADATA_BYTES, StoreInner, lock,
};
use crate::content_ref::ContentRef;

const MAX_ROOTS: usize = 256;
const MAX_INPUT: usize = 1024 * 1024;

pub(crate) struct RawJsonIndex {
    owner: Weak<StoreInner>,
    source: ContentRef,
    roots: BTreeMap<String, ContentRef>,
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
                length.checked_add(MAX_ROOTS * 256)?,
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
        let mut roots = BTreeMap::new();
        for (key, range) in ranges {
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
            _workspace: workspace,
        })
    }

    pub(crate) fn get(&self, key: &str) -> Option<&ContentRef> {
        self.roots.get(key)
    }

    pub(crate) fn register(&self, store: &ReplayStore) -> Result<(), ReplayError> {
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
        if self.roots.values().any(|range| {
            range.byte_offset() < self.source.byte_offset()
                || range
                    .byte_offset()
                    .checked_add(range.byte_len())
                    .is_none_or(|last| last > end)
        }) {
            return Err(ReplayError::Integrity);
        }
        let mut aliases = lock(&store.inner.raw_json_aliases);
        let count = self
            .roots
            .values()
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
        for reference in self.roots.values() {
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
    fn raw_ranges_require_their_current_owner_and_never_authorize_semantic_content() {
        let fixture = Fixture::new();
        let value = json!({"messages":[{"role":"user","content":"x".repeat(40000)}]});
        let bytes = serde_json::to_vec(&value).unwrap();
        let raw = fixture.capture(&bytes);
        let index = RawJsonIndex::capture(&fixture.store, &raw).unwrap();
        let reference = index.get("messages").unwrap();
        assert!(fixture.store.reader(reference).is_err());
        let foreign = Fixture::new();
        let _same = foreign.capture(&bytes);
        assert!(index.register(&foreign.store).is_err());
        index.register(&fixture.store).unwrap();
        assert!(!fixture.store.has_reference(reference));
        assert!(!fixture.store.is_canonical_json(reference));
        assert!(fixture.store.runtime_fingerprint(reference).is_err());
        let mut actual = Vec::new();
        let mut reader = fixture.store.reader(reference).unwrap();
        reader.read_to_end(&mut actual).unwrap();
        reader.verify_terminal().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&actual).unwrap(),
            value["messages"]
        );
        fixture
            .store
            .prevalidate(std::slice::from_ref(&raw))
            .unwrap();
        let lease = fixture.store.content_lease().unwrap();
        assert!(lease.reader(reference).is_err());
        fixture.store.release_stream(&raw).unwrap();
        assert!(fixture.store.reader(reference).is_err());
        assert!(index.register(&fixture.store).is_err());
        drop(reader);
    }
}
