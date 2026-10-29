//! Optimistic native-JSON reuse cost, not an integrated cache or latency claim.
//! Excludes budget admission, publication, compaction, decoding and history work.
#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::hint::black_box;
use std::time::Instant;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};

const ROUNDS: usize = 11;
const ITERATIONS: usize = 20;

struct Fields<'a>(Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Fields<'de> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Root;
        impl<'de> Visitor<'de> for Root {
            type Value = Fields<'de>;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a native request object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut fields = Vec::new();
                while let Some(field) = map.next_entry()? {
                    fields.push(field);
                }
                Ok(Fields(fields))
            }
        }
        deserializer.deserialize_map(Root)
    }
}

struct Snapshot<'a> {
    fields: Fields<'a>,
    messages: Vec<&'a RawValue>,
    value: Value,
}

impl<'a> Snapshot<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        let fields: Fields<'_> = serde_json::from_slice(bytes).unwrap();
        let messages = fields
            .0
            .iter()
            .find(|(key, _)| key == "messages")
            .unwrap()
            .1;
        Self {
            messages: serde_json::from_str(messages.get()).unwrap(),
            value: serde_json::from_slice(bytes).unwrap(),
            fields,
        }
    }

    fn assemble(&self, bytes: &[u8]) -> Option<Value> {
        // The approved first version validates all current syntax with serde;
        // it does not resume parser state or trust a short fingerprint.
        let fields: Fields<'_> = serde_json::from_slice(bytes).ok()?;
        if fields.0.len() > 256 {
            return None;
        }
        let mut seen = HashSet::new();
        let mut output = Map::new();
        for (key, raw) in fields.0 {
            if !seen.insert(key.clone()) {
                return None;
            }
            let value = if key == "messages" {
                let messages: Vec<&RawValue> = serde_json::from_str(raw.get()).ok()?;
                if messages.len() > 1024 {
                    return None;
                }
                let old_values = self.value["messages"].as_array()?;
                let mut prefix = true;
                let mut values = Vec::with_capacity(messages.len());
                for (index, message) in messages.into_iter().enumerate() {
                    prefix &= self
                        .messages
                        .get(index)
                        .is_some_and(|old| old.get().as_bytes() == message.get().as_bytes());
                    values.push(if prefix {
                        old_values.get(index)?.clone()
                    } else {
                        serde_json::from_str(message.get()).ok()?
                    });
                }
                Value::Array(values)
            } else if self.fields.0.iter().any(|(old_key, old)| {
                old_key == &key && old.get().as_bytes() == raw.get().as_bytes()
            }) {
                self.value.get(&key)?.clone()
            } else {
                serde_json::from_str(raw.get()).ok()?
            };
            output.insert(key, value);
        }
        Some(Value::Object(output))
    }
}

fn synthetic() -> Value {
    json!({"model":"alias","stream":true,
        "messages":(0..237).map(|index|json!({
            "role":if index%2==0 {"user"} else {"assistant"},
            "content":format!("{index}: {}", "route 路由🦀 \"\\\n".repeat(128))
        })).collect::<Vec<_>>(),
        "tools":(0..21).map(|index|json!({"type":"function","function":{
            "name":format!("tool_{index}"),"description":"A bounded tool.",
            "parameters":{"type":"object","properties":(0..12).map(|n|
                (format!("field_{n}"),json!({"type":"string","description":"A bounded value."}))
            ).collect::<Map<_,_>>()}
        }})).collect::<Vec<_>>()})
}

fn main() {
    if cfg!(debug_assertions) {
        panic!("run this diagnostic in release mode");
    }
    let bytes = match std::env::var_os("HIROUTE_PARSE_REUSE_INPUT") {
        Some(path) => std::fs::read(path).unwrap(),
        None => serde_json::to_vec(&synthetic()).unwrap(),
    };
    let snapshot = Snapshot::new(&bytes);
    let mut appended = snapshot.value.clone();
    appended["messages"].as_array_mut().unwrap().extend([
        json!({"role":"assistant","content":"42"}),
        json!({"role":"user","content":"Add 8."}),
    ]);
    let appended = serde_json::to_vec(&appended).unwrap();
    // Independent serde value equality checks identity, suffix and old-message
    // edits. This prototype never supplies accepted-output or routing evidence.
    for current in [&bytes, &appended] {
        assert_eq!(
            snapshot.assemble(current).unwrap(),
            serde_json::from_slice::<Value>(current).unwrap()
        );
    }
    let count = snapshot.messages.len();
    for index in [0, count / 2, count - 1] {
        let mut changed = snapshot.value.clone();
        changed["messages"][index]["content"] = json!("changed 路由\n\"\\");
        let raw = serde_json::to_vec(&changed).unwrap();
        assert_eq!(snapshot.assemble(&raw).unwrap(), changed);
    }
    assert!(
        snapshot
            .assemble(br#"{"messages":[],"model":"a","model":"b"}"#)
            .is_none()
    );
    assert!(snapshot.assemble(br#"{"messages":["unfinished"#).is_none());
    println!("variant\tpath\tbytes\tround\tns_per_operation");
    for (variant, current) in [("identical", &bytes), ("appended", &appended)] {
        for round in 0..=ROUNDS {
            for offset in 0..2 {
                let reuse = (round + offset) % 2 == 0;
                let started = Instant::now();
                for _ in 0..ITERATIONS {
                    let value = if reuse {
                        snapshot.assemble(black_box(current)).unwrap()
                    } else {
                        serde_json::from_slice::<Value>(black_box(current)).unwrap()
                    };
                    black_box(value); // The measured scope includes destruction.
                }
                let ns = started.elapsed().as_nanos() / ITERATIONS as u128;
                if round > 0 {
                    let path = if reuse {
                        "validated_reuse"
                    } else {
                        "full_serde"
                    };
                    println!("{variant}\t{path}\t{}\t{round}\t{ns}", current.len());
                }
            }
        }
    }
}
