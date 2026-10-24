//! Failure-only test diagnostics. Raw snapshot values and native paths are never retained.
use hiroute_domain::CanonicalDigest;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

const MAX_ROWS: usize = 96;

#[cfg(unix)]
mod executable;

#[cfg(unix)]
pub(super) fn record_executable(path: &std::path::Path) {
    // Sampling is diagnostic work, not a discovery dependency or a capability proof.
    // Do not read installed binaries when no failure-only transaction scope is active.
    if TRACE.with(|trace| trace.borrow().is_some()) {
        record("native_executable_content", &executable::snapshot(path));
    }
}

#[derive(Default)]
struct Trace {
    key: String,
    operation: &'static str,
    previous: BTreeMap<&'static str, BTreeMap<String, CanonicalDigest>>,
    rows: Vec<Value>,
    snapshots: usize,
}

impl Trace {
    fn record(&mut self, label: &'static str, value: &Value) {
        // The callers supply objects with fixed field names. Hash values before storage,
        // including nested configuration, login material and filesystem metadata.
        let fields: BTreeMap<_, _> = value
            .as_object()
            .expect("diagnostic snapshot must be an object")
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    CanonicalDigest::of(value).expect("JSON value must be digestible"),
                )
            })
            .collect();
        let previous = self.previous.insert(label, fields.clone());
        let mut changes = Vec::new();
        if let Some(previous) = &previous {
            let keys: BTreeSet<_> = previous.keys().chain(fields.keys()).collect();
            for field in keys {
                if previous.get(field) != fields.get(field) {
                    changes.push(json!({
                        "field": field,
                        "before": previous.get(field),
                        "after": fields.get(field),
                    }));
                }
            }
        }
        self.snapshots += 1;
        if self.rows.len() == MAX_ROWS {
            self.rows.remove(0);
        }
        let mut row = json!({
            "snapshot": self.snapshots,
            "operation": self.operation,
            "label": label,
            "digest": CanonicalDigest::of(value).expect("JSON value must be digestible"),
            "baseline": previous.is_none(),
            "changes": changes,
        });
        if label == "native_executable_content" {
            // Closed sampling outcomes must remain readable: two unavailable hashes are
            // not evidence that the executable bytes or native permissions were equal.
            let state = |pointer| match value.pointer(pointer).and_then(Value::as_str) {
                Some("stable") => "stable",
                Some("changed") => "changed",
                Some("complete") => "complete",
                Some("limit") => "limit",
                Some("invalid") => "invalid",
                Some("not_sampled") | None => "not_sampled",
                _ => "unavailable",
            };
            row["sampling"] = json!({
                "snapshot":state("/state"), "content":state("/content/state"),
                "acl_flags":state("/extended/acl_flags/state"),
                "xattrs":state("/extended/xattrs/state"),
            });
        }
        self.rows.push(row);
    }

    fn report(&self) -> Value {
        json!({
            "key": self.key,
            "snapshots": self.snapshots,
            "omitted_rows": self.snapshots.saturating_sub(self.rows.len()),
            "rows": self.rows,
        })
    }
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

pub(crate) struct Scope;

impl Scope {
    pub(crate) fn begin(key: &str) -> Self {
        TRACE.with(|trace| {
            *trace.borrow_mut() = Some(Trace {
                key: key.to_owned(),
                ..Trace::default()
            });
        });
        Self
    }

    pub(crate) fn operation(&self, operation: &'static str) {
        TRACE.with(|trace| {
            if let Some(trace) = trace.borrow_mut().as_mut() {
                trace.operation = operation;
            }
        });
    }

    pub(crate) fn print_failure(&self) {
        TRACE.with(|trace| {
            if let Some(trace) = trace.borrow().as_ref() {
                eprintln!("SETTINGS_SNAPSHOT_DIAGNOSTIC {}", trace.report());
            }
        });
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        TRACE.with(|trace| *trace.borrow_mut() = None);
    }
}

pub(super) fn record(label: &'static str, value: &Value) {
    TRACE.with(|trace| {
        if let Some(trace) = trace.borrow_mut().as_mut() {
            trace.record(label, value);
        }
    });
}

#[test]
fn snapshot_diagnostics_keep_only_field_changes_and_digests() {
    let mut trace = Trace::default();
    trace.record("model", &json!({"content":"private-before", "stable":42}));
    trace.record("model", &json!({"content":"private-after", "stable":42}));
    let report = trace.report();
    assert_eq!(report["rows"][0]["baseline"], true);
    assert_eq!(report["rows"][1]["changes"].as_array().unwrap().len(), 1);
    assert_eq!(report["rows"][1]["changes"][0]["field"], "content");
    assert!(!report.to_string().contains("private-"));
    assert!(
        report["rows"][1]["changes"][0]["after"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
}

#[test]
fn snapshot_diagnostics_are_bounded_and_discarded_after_the_transaction() {
    {
        let scope = Scope::begin("test-transaction");
        scope.operation("PreviewAgentConnectionChange");
        for index in 0..MAX_ROWS + 1 {
            record("model", &json!({"revision": index}));
        }
        TRACE.with(|trace| {
            let trace = trace.borrow();
            let report = trace.as_ref().unwrap().report();
            assert_eq!(report["rows"].as_array().unwrap().len(), MAX_ROWS);
            assert_eq!(report["omitted_rows"], 1);
        });
    }
    TRACE.with(|trace| assert!(trace.borrow().is_none()));
}
