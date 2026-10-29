use super::*;
use crate::content_ref::ContentRef;

#[test]
fn canonical_json_provenance_belongs_only_to_the_exact_registered_range() {
    for threshold in [64, 64 * 1024] {
        let root = TestRoot::new("json-provenance");
        let manager = ReplayManager::open(config(&root.0, threshold, 64)).unwrap();
        let store = manager.begin_request(budget()).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(r#"{"z":[2,1],"a":{"y":2,"x":1}}"#).unwrap();
        let mut pool = store.begin_content_pool().unwrap();
        let raw = pool.append(br#"{"a":{"x":1,"y":2},"z":[2,1]}"#).unwrap();
        let canonical = pool.append_canonical_json(&value).unwrap();
        let padding = pool.append(&[b'x'; 128]).unwrap();
        pool.seal().unwrap();
        assert_eq!(store.snapshot().disk_backed, threshold == 64);
        assert_eq!(read_all(&store, &raw), read_all(&store, &canonical));
        assert!(
            !store.is_canonical_json(&raw),
            "equal bytes are not producer provenance"
        );
        assert!(store.is_canonical_json(&canonical));
        assert!(!store.is_canonical_json(&padding));
        let subrange = ContentRef::new(
            canonical.stream_ordinal(),
            canonical.byte_offset() + 1,
            canonical.byte_len() - 2,
            canonical.json_escaped_len() - 2,
        );
        assert!(!store.is_canonical_json(&subrange));
        let whole = ContentRef::new(
            raw.stream_ordinal(),
            0,
            raw.byte_len() + canonical.byte_len() + padding.byte_len(),
            raw.json_escaped_len() + canonical.json_escaped_len() + padding.json_escaped_len(),
        );
        assert!(
            !store.is_canonical_json(&whole),
            "a mixed pool cannot inherit one child's provenance"
        );
        assert!(matches!(
            store.release_stream(&canonical),
            Err(ReplayError::Integrity)
        ));
        store.release_stream(&whole).unwrap();
        assert!(!store.is_canonical_json(&canonical));
    }
}
