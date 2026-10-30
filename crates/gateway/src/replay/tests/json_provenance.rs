use super::*;
use crate::content_ref::ContentRef;

#[test]
fn serialized_ranges_keep_exact_offsets_and_escaped_lengths_across_spill() {
    let text = "路由🦀\n\"\\\u{1}".repeat(4096);
    // Frozen expected syntax is independent of Replay's serializer/counters.
    let expected = format!(
        "{{\"a\":[true,null,-12,1.5],\"z\":\"{}\"}}",
        "路由🦀\\n\\\"\\\\\\u0001".repeat(4096)
    );
    for threshold in [31, 1024 * 1024] {
        let root = TestRoot::new("json-range-lengths");
        let manager = ReplayManager::open(config(&root.0, threshold, 31)).unwrap();
        let budget = budget();
        let store = manager.begin_request(budget.clone()).unwrap();
        let mut pool = store.begin_content_pool().unwrap();
        let before = pool.append(b"left").unwrap();
        let value = serde_json::json!({"z":text,"a":[true,null,-12,1.5]});
        let json = pool.append_canonical_json(&value).unwrap();
        let after = pool.append(b"right").unwrap();
        pool.seal().unwrap();
        assert_eq!(json.byte_offset(), before.byte_len());
        assert_eq!(after.byte_offset(), json.byte_offset() + json.byte_len());
        assert_eq!(read_all(&store, &json), expected.as_bytes());
        let escaped = serde_json::to_string(&expected).unwrap();
        assert_eq!(json.json_escaped_len(), (escaped.len() - 2) as u64);
        assert_eq!(read_all(&store, &before), b"left");
        assert_eq!(read_all(&store, &after), b"right");
        assert_eq!(store.snapshot().disk_backed, threshold == 31);
        drop(store);
        assert_eq!(budget.snapshot().unwrap().live, 0);
    }
}

#[test]
fn concurrent_serialized_ranges_share_the_existing_request_memory_ceiling() {
    let root = TestRoot::new("json-concurrent-ranges");
    let manager = ReplayManager::open(config(&root.0, 64 * 1024, 4 * 1024)).unwrap();
    let budget = budget();
    let store = manager.begin_request(budget.clone()).unwrap();
    let threads = (0..8)
        .map(|_| {
            let store = store.clone();
            std::thread::spawn(move || {
                let text = "escaped\n\"\\🦀".repeat(4096);
                let value = serde_json::json!({"a":text});
                let mut pool = store.begin_content_pool().unwrap();
                let reference = pool.append_canonical_json(&value).unwrap();
                pool.seal().unwrap();
                (reference, serde_json::to_vec(&value).unwrap())
            })
        })
        .collect::<Vec<_>>();
    for thread in threads {
        let (reference, expected) = thread.join().unwrap();
        assert_eq!(read_all(&store, &reference), expected);
    }
    assert!(store.snapshot().disk_backed);
    assert!(store.snapshot().memory_retained_bytes <= 64 * 1024);
    drop(store);
    assert_eq!(budget.snapshot().unwrap().live, 0);
}

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
