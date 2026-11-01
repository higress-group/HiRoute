//! Opt-in acceptance over an operator-owned stopped copy, never the live store.
use super::*;
use crate::maintenance::ObservationMaintenance;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

#[test]
#[ignore = "requires an explicitly prepared private stopped-source copy"]
fn current_history_and_real_maintenance() {
    let root = PathBuf::from(std::env::var_os("HIROUTE_STORAGE_FIELD_ROOT").expect("field root"));
    assert_eq!(root.file_name().unwrap(), "observation");
    assert!(
        root.parent()
            .unwrap()
            .join(".hiroute-storage-validation")
            .is_file(),
        "only an explicitly marked validation copy may be changed"
    );
    let key_path = std::env::var_os("HIROUTE_STORAGE_FIELD_KEY_FILE").expect("private key file");
    let key = Zeroizing::new(fs::read(key_path).unwrap());
    let authority = DigestAuthority::new(key.as_slice().try_into().expect("32-byte original key"));
    assert_eq!(
        super::upgrade::version(&root).unwrap().as_deref(),
        Some("6"),
        "archive old history through production startup before running current-store maintenance"
    );
    let start = Instant::now();
    let store = Arc::new(LocalObservationStore::open(&root, authority.clone()).unwrap());
    println!(
        "{}",
        serde_json::json!({"phase":"open", "seconds":start.elapsed().as_secs_f64(), "pid":std::process::id(), "counts":counts(&store, true)})
    );
    drop(store);
    let reopened = Instant::now();
    let store = Arc::new(LocalObservationStore::open(&root, authority).unwrap());
    println!(
        "{}",
        serde_json::json!({"phase":"reopen", "seconds":reopened.elapsed().as_secs_f64(), "counts":counts(&store, true)})
    );
    let seconds: u64 = std::env::var("HIROUTE_STORAGE_FIELD_SECONDS")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .unwrap();
    if seconds == 0 {
        return;
    }
    let limit = std::env::var("HIROUTE_STORAGE_FIELD_LIMIT")
        .ok()
        .map(|s| s.parse::<u64>().unwrap());
    store.set_capacity_limit(limit).unwrap();
    capacity::probe::enable();
    let maintenance = ObservationMaintenance::start(&store).unwrap();
    let readers: usize = std::env::var("HIROUTE_STORAGE_FIELD_READERS")
        .unwrap_or_else(|_| "0".into())
        .parse()
        .unwrap();
    assert!(readers <= 4);
    let stop = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(ReaderStats::default());
    let workspace = store.connection.lock().query_row("SELECT workspace_id FROM logical_requests GROUP BY workspace_id ORDER BY COUNT(*) DESC LIMIT 1",[],|r|r.get::<_,String>(0)).map(|s|WorkspaceId::parse(s).unwrap()).unwrap_or_default();
    let mut workers = Vec::new();
    for i in 0..readers {
        let (store, stop, stats, workspace) = (
            store.clone(),
            stop.clone(),
            stats.clone(),
            workspace.clone(),
        );
        workers.push(
            std::thread::Builder::new()
                .name(format!("field-read-{i}"))
                .spawn(move || {
                    use hiroute_domain::{ObservationReaderContext, ObservationRequestQuery};
                    let reader = ObservationReaderContext::local_user(
                        workspace,
                        format!("field-reader-{i}"),
                        1,
                        now_ms()
                            .saturating_add((seconds as i64).saturating_mul(1000))
                            .saturating_add(60000),
                        true,
                        true,
                    )
                    .unwrap();
                    while !stop.load(Ordering::Relaxed) {
                        let query = ObservationRequestQuery {
                            from_ms: 0,
                            to_ms: now_ms().saturating_add(1),
                            session_id: None,
                            request_id: None,
                            limit: 10,
                            cursor: None,
                            agent_id: None,
                            plan_id: None,
                            native_model: None,
                            outcome: None,
                            only_model_switch: false,
                        };
                        let start = Instant::now();
                        match store.observed_sessions(&reader, &query, now_ms()) {
                            Ok(page) => {
                                stats.passed.fetch_add(1, Ordering::Relaxed);
                                stats
                                    .rows
                                    .fetch_add(page.sessions.len() as u64, Ordering::Relaxed);
                            }
                            Err(error) => {
                                eprintln!(
                                    "field reader failure: {error:?}, elapsed_us={}",
                                    start.elapsed().as_micros()
                                );
                                stats.failed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        stats.max_us.fetch_max(
                            start.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
                            Ordering::Relaxed,
                        );
                        std::thread::sleep(Duration::from_millis(250));
                    }
                })
                .unwrap(),
        );
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        let passes = capacity::probe::samples();
        println!(
            "{}",
            serde_json::json!({"phase":"maintenance", "seconds":start.elapsed().as_secs_f64(), "capacity":store.capacity_status().unwrap(), "capacity_pass_count":passes.len(),"capacity_last_pass":passes.last(),"counts":counts(&store, false), "maintenance":store.maintenance_status()})
        );
        std::thread::sleep(
            Duration::from_secs(10)
                .min(Duration::from_secs(seconds).saturating_sub(start.elapsed())),
        );
    }
    stop.store(true, Ordering::Relaxed);
    for worker in workers {
        worker.join().unwrap();
    }
    drop(maintenance);
    let capacity_passes = capacity::probe::samples();
    println!(
        "{}",
        serde_json::json!({"phase":"finished", "seconds":start.elapsed().as_secs_f64(), "capacity":store.capacity_status().unwrap(), "capacity_passes":capacity_passes,"counts":counts(&store, true), "readers":{"passed":stats.passed.load(Ordering::Relaxed),"failed":stats.failed.load(Ordering::Relaxed),"rows":stats.rows.load(Ordering::Relaxed),"max_us":stats.max_us.load(Ordering::Relaxed)}})
    );
    // A pressure run never leaves an enabled policy in its retained evidence.
    store.set_capacity_limit(None).unwrap();
    if readers > 0 {
        assert!(stats.passed.load(Ordering::Relaxed) > 0 && stats.rows.load(Ordering::Relaxed) > 0);
        assert_eq!(stats.failed.load(Ordering::Relaxed), 0);
    }
}

#[derive(Default)]
struct ReaderStats {
    passed: AtomicU64,
    failed: AtomicU64,
    rows: AtomicU64,
    max_us: AtomicU64,
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}

fn counts(store: &LocalObservationStore, full: bool) -> serde_json::Value {
    let db = store.connection.lock();
    let mut result = serde_json::Map::new();
    let tables: &[&str] = if full {
        &[
            "logical_requests",
            "observation_current_commits_v6",
            "content_blobs_v2",
            "observation_capacity_pending_v5",
            "observation_capacity_blobs_v5",
        ]
    } else {
        &[
            "logical_requests",
            "observation_capacity_pending_v5",
            "observation_capacity_blobs_v5",
        ]
    };
    for &table in tables {
        let count: u64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        result.insert(table.into(), count.into());
    }
    serde_json::Value::Object(result)
}
