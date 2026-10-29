//! Optional installation-wide early retention. One completion-based schedule owns
//! selection, resumable row retirement, orphan objects and SQLite page release.
use super::LocalObservationStore;
#[cfg(test)]
pub(super) mod probe;
#[cfg(test)]
mod quality_tests;
#[cfg(test)]
mod tests;
use hiroute_domain::{ObservationCapacityStatusV1 as Status, ObservationQueryError as Error};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) const INTERVAL: Duration = Duration::from_secs(10);
const DEFAULT_LIMIT: u64 = 20_000_000_000;
const BUDGET: Duration = Duration::from_millis(100);
const ROWS: usize = 128;

pub(crate) const SCHEMA: &str = r#"
CREATE INDEX IF NOT EXISTS observation_capacity_oldest_v5 ON logical_requests(started_at_ms,workspace_id,request_id) WHERE finished_at_ms IS NOT NULL;
CREATE INDEX IF NOT EXISTS observation_capacity_turn_v5 ON logical_requests(workspace_id,turn_id);
CREATE TABLE IF NOT EXISTS observation_capacity_pending_v5(workspace TEXT NOT NULL,request TEXT NOT NULL,session TEXT NOT NULL,step INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(workspace,request));
CREATE TABLE IF NOT EXISTS observation_capacity_blobs_v5(workspace TEXT NOT NULL,digest TEXT NOT NULL,PRIMARY KEY(workspace,digest));
CREATE INDEX IF NOT EXISTS observation_capacity_quality_assessment_v5 ON plan_quality_segments(workspace_id,assessment_trigger_request_id);
CREATE INDEX IF NOT EXISTS observation_capacity_quality_last_v5 ON plan_quality_segments(workspace_id,last_request_id);
CREATE INDEX IF NOT EXISTS observation_capacity_quality_first_v5 ON plan_quality_segments(workspace_id,first_request_id);
CREATE INDEX IF NOT EXISTS content_instances_v2_live_blob_global ON content_instances_v2(content_blob_digest) WHERE state='complete';
"#;

pub(crate) struct State {
    pub next_due: Instant,
    failures: u32,
    inventory: Option<Inventory>,
    pub status: Status,
    draining: bool,
    committed_progress: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            next_due: Instant::now(),
            failures: 0,
            inventory: None,
            status: Status {
                state: "idle".into(),
                reason: "disabled".into(),
                ..Default::default()
            },
            draining: false,
            committed_progress: false,
        }
    }
}

struct Inventory {
    pending: VecDeque<PathBuf>,
    entries: Option<fs::ReadDir>,
    bytes: u64,
    files: BTreeMap<PathBuf, u64>,
    complete: bool,
}
impl Inventory {
    fn new(store: &LocalObservationStore) -> Self {
        Self {
            pending: VecDeque::from([store.blob_root.clone(), store.staging_root.clone()]),
            entries: None,
            bytes: 0,
            files: BTreeMap::new(),
            complete: false,
        }
    }
    fn advance(&mut self, deadline: Instant) -> Result<bool, Error> {
        if self.complete {
            return Ok(true);
        }
        let mut visited = 0;
        while visited < 16384 && Instant::now() < deadline {
            if let Some(entries) = &mut self.entries {
                match entries.next() {
                    Some(Ok(entry)) => {
                        let path = entry.path();
                        let meta = fs::symlink_metadata(&path).map_err(|_| Error::Unavailable)?;
                        if meta.file_type().is_symlink() {
                            return Err(Error::Corrupt);
                        }
                        if meta.is_dir() {
                            self.pending.push_back(path);
                        } else if meta.is_file() {
                            #[cfg(unix)]
                            let bytes = {
                                use std::os::unix::fs::MetadataExt;
                                meta.blocks().checked_mul(512).ok_or(Error::Corrupt)?
                            };
                            #[cfg(not(unix))]
                            let bytes = allocated(&path)?;
                            self.record(path, bytes)?;
                        }
                        visited += 1;
                    }
                    Some(Err(_)) => return Err(Error::Unavailable),
                    None => self.entries = None,
                }
            } else if let Some(path) = self.pending.pop_front() {
                self.entries = Some(fs::read_dir(path).map_err(|_| Error::Unavailable)?);
            } else {
                self.complete = true;
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn record(&mut self, path: PathBuf, bytes: u64) -> Result<(), Error> {
        let prior = if bytes == 0 {
            self.files.remove(&path).unwrap_or(0)
        } else {
            self.files.insert(path, bytes).unwrap_or(0)
        };
        self.bytes = self
            .bytes
            .checked_sub(prior)
            .and_then(|n| n.checked_add(bytes))
            .ok_or(Error::Corrupt)?;
        Ok(())
    }
}
#[cfg(unix)]
fn allocated(path: &std::path::Path) -> Result<u64, Error> {
    use std::os::unix::fs::MetadataExt;
    // Opening and closing a separate descriptor cancels this process's POSIX
    // locks on a live SQLite DB/SHM file. Path metadata needs no descriptor.
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => meta.blocks().checked_mul(512).ok_or(Error::Corrupt),
        Ok(_) => Err(Error::Corrupt),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(_) => Err(Error::Unavailable),
    }
}
#[cfg(not(unix))]
fn allocated(path: &std::path::Path) -> Result<u64, Error> {
    match fs::File::open(path) {
        Ok(file) => fs2::FileExt::allocated_size(&file).map_err(|_| Error::Unavailable),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(_) => Err(Error::Unavailable),
    }
}
fn db_bytes(store: &LocalObservationStore) -> Result<u64, Error> {
    let mut n = 0u64;
    for suffix in ["", "-wal", "-shm"] {
        n = n
            .checked_add(allocated(&PathBuf::from(format!(
                "{}{suffix}",
                store.activity_path.display()
            )))?)
            .ok_or(Error::Corrupt)?;
    }
    Ok(n)
}
fn get_limit(db: &Connection) -> Result<Option<u64>, Error> {
    let text: Option<String> = db
        .query_row(
            "SELECT value FROM observation_meta WHERE key='max_active_bytes'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::Unavailable)?;
    match text.as_deref() {
        None => Ok(Some(DEFAULT_LIMIT)),
        Some("null") => Ok(None),
        Some(text) => text
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0)
            .map(Some)
            .ok_or(Error::Corrupt),
    }
}
fn sql<T>(r: rusqlite::Result<T>) -> Result<T, Error> {
    r.map_err(|_| Error::Unavailable)
}

fn vacuum_pages(db: &Connection) -> Result<bool, Error> {
    let mut statement = sql(db.prepare("PRAGMA incremental_vacuum(16)"))?;
    let mut rows = sql(statement.query([]))?;
    let mut released = false;
    // This pragma returns one row per released page. execute_batch only steps
    // its first row, silently reducing a bounded batch to a single page.
    while sql(rows.next())?.is_some() {
        released = true;
    }
    Ok(released)
}

fn checkpoint_pages(db: &Connection, deadline: Instant) -> Result<bool, Error> {
    let checkpoint: (i64, i64, i64) =
        sql(db.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        }))?;
    if checkpoint.0 != 0 || checkpoint.1 != checkpoint.2 || Instant::now() >= deadline {
        return Ok(false);
    }
    let busy: i64 = sql(db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0)))?;
    Ok(busy == 0)
}

impl LocalObservationStore {
    pub(crate) fn note_object_path(&self, path: &std::path::Path) {
        let mut state = self.capacity.lock();
        if let Some(inventory) = &mut state.inventory {
            match allocated(path) {
                Ok(bytes) => {
                    let _ = inventory.record(path.to_owned(), bytes);
                }
                Err(_) => state.inventory = None,
            }
        }
    }
    pub(crate) fn note_removed_directory(&self, path: &std::path::Path) {
        let mut state = self.capacity.lock();
        if let Some(inventory) = &mut state.inventory {
            while let Some(key) = inventory
                .files
                .range(path.to_path_buf()..)
                .next()
                .map(|(k, _)| k.clone())
                .filter(|k| k.starts_with(path))
            {
                let _ = inventory.record(key, 0);
            }
        }
    }

    pub fn capacity_status(&self) -> Result<Status, Error> {
        let limit = get_limit(&self.connection.lock())?;
        let mut value = self.capacity.lock().status.clone();
        value.max_active_bytes = limit;
        if limit.is_some() && value.reason == "disabled" {
            value.state = "pending".into();
            value.reason = "measuring".into();
        }
        Ok(value)
    }
    pub fn set_capacity_limit(&self, limit: Option<u64>) -> Result<Status, Error> {
        if limit == Some(0) {
            return Err(Error::InvalidQuery);
        }
        let mut db = self.connection.lock();
        let tx = sql(db.transaction())?;
        // Absence means the product default. A deliberate disable must remain
        // a stored choice, including when this database is reopened.
        let stored = limit
            .map(|n| n.to_string())
            .unwrap_or_else(|| "null".into());
        sql(tx.execute("INSERT INTO observation_meta(key,value) VALUES('max_active_bytes',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[stored]))?;
        sql(tx.commit())?;
        drop(db);
        let mut state = self.capacity.lock();
        state.next_due = Instant::now();
        state.status.max_active_bytes = limit;
        state.status.state = if limit.is_some() { "pending" } else { "idle" }.into();
        state.status.reason = if limit.is_some() {
            "measuring"
        } else {
            "disabled"
        }
        .into();
        state.draining = false;
        if limit.is_none() {
            state.inventory = None;
        }
        Ok(state.status.clone())
    }

    /// False means not due. The mutex is never waited on by maintenance.
    pub(crate) fn capacity_cycle(&self, now_ms: i64) -> Result<bool, Error> {
        let Some(mut state) = self.capacity.try_lock() else {
            return Ok(false);
        };
        if Instant::now() < state.next_due {
            return Ok(false);
        }
        let started = Instant::now();
        let deadline = started + BUDGET;
        let Some(mut db) = self.connection.try_lock() else {
            state.next_due = Instant::now() + INTERVAL;
            state.status.state = "pending".into();
            state.status.reason = "writer_busy".into();
            return Ok(true);
        };
        #[cfg(test)]
        let probe = probe::begin();
        let prior_timeout: i64 = sql(db.query_row("PRAGMA busy_timeout", [], |r| r.get(0)))?;
        sql(db.busy_timeout(Duration::ZERO))?;
        db.progress_handler(1000, Some(move || Instant::now() >= deadline));
        state.committed_progress = false;
        let mut result = self.capacity_work(&mut db, &mut state, now_ms, deadline);
        db.progress_handler(0, None::<fn() -> bool>);
        sql(db.busy_timeout(Duration::from_millis(prior_timeout.max(0) as u64)))?;
        if result.is_err() && Instant::now() >= deadline && state.committed_progress {
            // Earlier bounded transactions remain committed when the next unit
            // reaches the deadline. Yield normally rather than backing off as
            // though the whole pass had made no progress.
            result = Ok(true);
            state.status.state = "pending".into();
            state.status.reason = "work_budget".into();
        }
        match &result {
            Ok(progress) => {
                state.failures = if *progress {
                    0
                } else {
                    state.failures.saturating_add(1).min(4)
                };
            }
            Err(_) => {
                state.failures = state.failures.saturating_add(1).min(4);
                state.status.state = "blocked".into();
                state.status.reason = if Instant::now() >= deadline {
                    "work_budget"
                } else {
                    "storage_unavailable"
                }
                .into();
            }
        }
        let completed = Instant::now();
        // SQLite checkpoint/fsync is not preempted by SQL progress callbacks.
        // Charge its whole elapsed cost to the existing cooldown too: even an
        // overrun gets at most a 1% duty cycle, without a new thread or timer.
        let cooldown = INTERVAL
            .saturating_mul(1 << state.failures)
            .max(completed.duration_since(started).saturating_mul(100));
        state.next_due = completed + cooldown;
        #[cfg(test)]
        probe::record(probe, now_ms, &state.status.reason);
        // Budget exhaustion is a yielding pending pass, not an unbounded retry.
        if result.is_err() && Instant::now() >= deadline {
            return Ok(true);
        }
        result.map(|_| true)
    }
    fn capacity_work(
        &self,
        db: &mut Connection,
        state: &mut State,
        now_ms: i64,
        deadline: Instant,
    ) -> Result<bool, Error> {
        let mut progress = false;
        while Instant::now() < deadline {
            let made = self.capacity_step(db, state, now_ms, deadline)?;
            progress |= made;
            // Selection, cleanup and page release share this one budget. Idle
            // and blocked states stop immediately; inventory retains its file
            // batch bound. No phase incurs another full scheduling interval.
            if !made || state.status.state != "pending" || state.status.reason == "measuring" {
                break;
            }
        }
        Ok(progress)
    }

    fn capacity_step(
        &self,
        db: &mut Connection,
        state: &mut State,
        now_ms: i64,
        deadline: Instant,
    ) -> Result<bool, Error> {
        let limit = get_limit(db)?;
        state.status.max_active_bytes = limit;
        let pending:bool=sql(db.query_row("SELECT EXISTS(SELECT 1 FROM observation_capacity_pending_v5) OR EXISTS(SELECT 1 FROM observation_capacity_blobs_v5)",[],|r|r.get(0)))?;
        // Disabling cancels future selection, but finishes already committed
        // visibility retirement so no partial erase becomes stranded forever.
        if pending {
            let progress = self.capacity_cleanup(db, state, deadline)?;
            state.status.state = "pending".into();
            state.status.reason = "physical_release".into();
            return Ok(progress);
        }
        let Some(limit) = limit else {
            state.status.state = "idle".into();
            state.status.reason = "disabled".into();
            return Ok(true);
        };
        let inventory = state.inventory.get_or_insert_with(|| Inventory::new(self));
        if !inventory.advance(deadline)? {
            state.status.state = "pending".into();
            state.status.reason = "measuring".into();
            return Ok(true);
        }
        let bytes = inventory
            .bytes
            .checked_add(db_bytes(self)?)
            .ok_or(Error::Corrupt)?;
        state.status.active_bytes = Some(bytes);
        state.status.measured_at_ms = Some(now_ms);
        let goal = if state.draining {
            limit / 10 * 9 + limit % 10 * 9 / 10
        } else {
            limit
        };
        if bytes <= goal {
            state.draining = false;
            state.status.state = "idle".into();
            state.status.reason = "within_limit".into();
            return Ok(true);
        }
        state.draining = true;
        let wal = PathBuf::from(format!("{}-wal", self.activity_path.display()));
        let wal_bytes = fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        if wal_bytes > 0 {
            // Complete physical release before retiring another request. A
            // reader-pinned WAL yields without waiting or deleting more data.
            // Physical length retains the WAL high-water mark even after all
            // frames are checkpointed, so it cannot prohibit this attempt.
            if !checkpoint_pages(db, deadline)? {
                state.status.state = "blocked".into();
                state.status.reason = "page_release_pending".into();
                return Ok(false);
            }
            state.committed_progress = true;
            state.status.state = "pending".into();
            state.status.reason = "physical_release".into();
            return Ok(true);
        }
        let free: u64 = sql(db.query_row("PRAGMA freelist_count", [], |r| r.get(0)))?;
        if free > 0 {
            // Each step releases at most 16 pages. No live VACUUM or
            // full checkpoint waits on readers; busy yields to the next due pass.
            let released = vacuum_pages(db)?;
            state.committed_progress |= released;
            state.status.state = "pending".into();
            state.status.reason = "reusable_pages".into();
            return Ok(released);
        }
        let selected:Option<(String,String,String)>=sql(db.query_row("SELECT workspace_id,session_id,request_id FROM logical_requests r WHERE finished_at_ms IS NOT NULL AND NOT EXISTS(SELECT 1 FROM content_instances_v2 c WHERE c.workspace_id=r.workspace_id AND c.request_id=r.request_id AND c.state='installing') AND NOT EXISTS(SELECT 1 FROM conversation_content_streams_v2 c WHERE c.workspace_id=r.workspace_id AND c.request_id=r.request_id AND (c.state='open' OR c.active_content_id IS NOT NULL)) ORDER BY started_at_ms,workspace_id,request_id LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional())?;
        let Some((workspace, session, request)) = selected else {
            state.status.state = "blocked".into();
            state.status.reason = "protected_floor".into();
            return Ok(false);
        };
        self.capacity_retire(db, &workspace, &session, &request, now_ms)?;
        state.committed_progress = true;
        state.status.state = "pending".into();
        state.status.reason = "retiring_request".into();
        Ok(true)
    }
    fn capacity_retire(
        &self,
        db: &mut Connection,
        w: &str,
        s: &str,
        r: &str,
        now: i64,
    ) -> Result<(), Error> {
        use hiroute_domain::{DeletionDataClass, SessionDeletionSpecV1, SessionId, WorkspaceId};
        let tx = sql(db.transaction())?;
        sql(tx.execute("INSERT INTO observation_capacity_pending_v5(workspace,request,session) VALUES(?1,?2,?3)",params![w,r,s]))?;
        let spec = SessionDeletionSpecV1 {
            workspace_id: WorkspaceId::parse(w).map_err(|_| Error::Corrupt)?,
            session_id: SessionId::parse(s).map_err(|_| Error::Corrupt)?,
            data_class: DeletionDataClass::FactsAndContent,
            delete_rollups: false,
        };
        let archive = self
            .authority
            .content_blob_digest("retention-archive/v2", format!("{w}\0{r}").as_bytes());
        super::value_rollup::archive_request_value(
            &tx,
            &spec,
            r,
            SessionId::parse(format!("archive-{archive}")).map_err(|_| Error::Corrupt)?,
        )?;
        crate::valuation::archive_request(&tx, w, s, r).map_err(|_| Error::Unavailable)?;
        sql(tx.execute("INSERT OR IGNORE INTO observation_request_tombstones_v2(workspace_id,request_id,deleted_ms) VALUES(?1,?2,?3)",params![w,r,now]))?;
        // Every content/analysis read joins current request membership. Remove
        // that membership atomically with the archive and signed-scope barrier.
        sql(tx.execute(
            "DELETE FROM logical_requests WHERE workspace_id=?1 AND request_id=?2",
            params![w, r],
        ))?;
        sql(tx.execute(
            "DELETE FROM observation_analysis_dirty_v1 WHERE workspace_id=?1 AND request_id=?2",
            params![w, r],
        ))?;
        sql(crate::query_v2::invalidate_visibility(&tx))?;
        // Do not call increment_store_revision: its 200-request analysis drain
        // would transfer capacity work to the unrelated accelerated catch-up loop.
        // Quality pages and trust generations also change at this logical
        // barrier, rather than waiting for physical projection cleanup.
        sql(tx.execute(
            "UPDATE observation_meta SET value=CAST(value AS INTEGER)+1 WHERE key IN('store_revision','plan_quality_generation')",
            [],
        ))?;
        sql(tx.commit())
    }

    fn capacity_cleanup(
        &self,
        db: &mut Connection,
        state: &mut State,
        deadline: Instant,
    ) -> Result<bool, Error> {
        let mut progress = false;
        while Instant::now() < deadline {
            let next:Option<(String,String,String,i64)>=sql(db.query_row("SELECT workspace,request,session,step FROM observation_capacity_pending_v5 ORDER BY workspace,request LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional())?;
            if let Some((w, r, s, step)) = next {
                let tx = sql(db.transaction())?;
                if step == 0 {
                    let chunks: Vec<(i64, String)> = {
                        // Keep the small selected request as the outer loop;
                        // SQLite otherwise scans the whole workspace's chunks.
                        let mut stmt=sql(tx.prepare("SELECT k.rowid,k.object_path FROM content_instances_v2 c INDEXED BY content_instances_v2_request CROSS JOIN content_chunks_v2 k ON c.workspace_id=k.workspace_id AND c.content_id=k.content_id WHERE c.workspace_id=?1 AND c.request_id=?2 LIMIT 128"))?;
                        sql(stmt
                            .query_map(params![w, r], |r| Ok((r.get(0)?, r.get(1)?)))
                            .map_err(|_| Error::Unavailable)?
                            .collect())?
                    };
                    for (id, path) in &chunks {
                        let path = self.object_path(path).map_err(|_| Error::Corrupt)?;
                        if path.starts_with(&self.staging_root) {
                            match fs::remove_file(&path) {
                                Ok(()) => {}
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                                Err(_) => return Err(Error::Unavailable),
                            }
                        }
                        if path.starts_with(&self.staging_root)
                            && let Some(inventory) = &mut state.inventory
                        {
                            inventory.record(path, 0)?;
                        }
                        sql(tx.execute("DELETE FROM content_chunks_v2 WHERE rowid=?1", [id]))?;
                    }
                    if chunks.is_empty() {
                        sql(tx.execute("UPDATE observation_capacity_pending_v5 SET step=1 WHERE workspace=?1 AND request=?2",params![w,r]))?;
                    }
                } else if step == 1 {
                    let objects: Vec<(i64, Option<String>)> = {
                        let mut stmt=sql(tx.prepare("SELECT rowid,content_blob_digest FROM content_instances_v2 WHERE workspace_id=?1 AND request_id=?2 LIMIT 128"))?;
                        sql(stmt
                            .query_map(params![w, r], |r| Ok((r.get(0)?, r.get(1)?)))
                            .map_err(|_| Error::Unavailable)?
                            .collect())?
                    };
                    for (id, digest) in &objects {
                        if let Some(digest) = digest {
                            sql(tx.execute(
                                "INSERT OR IGNORE INTO observation_capacity_blobs_v5 VALUES(?1,?2)",
                                params![w, digest],
                            ))?;
                        }
                        sql(tx.execute("DELETE FROM content_instances_v2 WHERE rowid=?1", [id]))?;
                    }
                    if objects.is_empty() {
                        sql(tx.execute("UPDATE observation_capacity_pending_v5 SET step=2 WHERE workspace=?1 AND request=?2",params![w,r]))?;
                    }
                } else if let Some(table) = CLEANUP.get((step - 2) as usize) {
                    let feedback = sql(super::records::retire_request_feedback(
                        &tx, &w, &r, table, true,
                    ))?;
                    if feedback == 0 {
                        let order = super::records::retirement_order(table)
                            .map(|order| format!(" ORDER BY {order}"))
                            .unwrap_or_default();
                        let n=sql(tx.execute(&format!("DELETE FROM {table} WHERE rowid IN(SELECT e.rowid FROM {table} e WHERE e.workspace_id=?1 AND e.request_id=?2{order} LIMIT {ROWS})"),params![w,r]))?;
                        if n == 0 {
                            sql(tx.execute("UPDATE observation_capacity_pending_v5 SET step=step+1 WHERE workspace=?1 AND request=?2",params![w,r]))?;
                        }
                    }
                } else {
                    sql(tx.execute("DELETE FROM plan_quality_segments WHERE rowid IN(SELECT rowid FROM plan_quality_segments WHERE workspace_id=?1 AND last_request_id=?2 LIMIT 128)",params![w,r]))?;
                    // Quality annotations are updated only for this selected
                    // request, never through a broad age cutoff.
                    sql(tx.execute("UPDATE plan_quality_segments SET assessment_event_id=NULL,assessment_sequence=NULL,assessment_trigger_request_id=NULL,assessed_at_ms=NULL,target_from_turn_id=NULL,target_through_turn_id=NULL,target_from_ordinal=NULL,target_through_ordinal=NULL,score=NULL,assessment_partial=NULL,reason_present=0 WHERE rowid IN(SELECT rowid FROM plan_quality_segments WHERE workspace_id=?1 AND assessment_trigger_request_id=?2 LIMIT 128)",params![w,r]))?;
                    sql(tx.execute("UPDATE plan_quality_segments SET first_request_id=NULL,history_partial=1 WHERE rowid IN(SELECT rowid FROM plan_quality_segments WHERE workspace_id=?1 AND first_request_id=?2 LIMIT 128)",params![w,r]))?;
                    let remaining:bool=sql(tx.query_row("SELECT EXISTS(SELECT 1 FROM plan_quality_segments WHERE workspace_id=?1 AND (first_request_id=?2 OR last_request_id=?2 OR assessment_trigger_request_id=?2))",params![w,r],|r|r.get(0)))?;
                    if !remaining {
                        sql(tx.execute("UPDATE observation_meta SET value=CAST(value AS INTEGER)+1 WHERE key='plan_quality_generation'",[]))?;
                        let turns = sql(tx.execute("DELETE FROM turns WHERE workspace_id=?1 AND session_id=?2 AND rowid IN(SELECT rowid FROM turns WHERE workspace_id=?1 AND session_id=?2 AND NOT EXISTS(SELECT 1 FROM logical_requests r WHERE r.workspace_id=?1 AND r.turn_id=turns.turn_id) LIMIT 128)",params![w,s]))?;
                        sql(tx.execute("UPDATE sessions SET content_completeness='expired',facts_completeness='unknown',tombstone_reason='retention_expired',agent_id='',correlation='unproven' WHERE workspace_id=?1 AND session_id=?2 AND NOT EXISTS(SELECT 1 FROM logical_requests WHERE workspace_id=?1 AND session_id=?2)",params![w,s]))?;
                        sql(tx.execute("DELETE FROM observation_analysis_dirty_v1 WHERE workspace_id=?1 AND request_id=?2",params![w,r]))?;
                        if turns < ROWS {
                            sql(tx.execute("DELETE FROM observation_capacity_pending_v5 WHERE workspace=?1 AND request=?2",params![w,r]))?;
                        }
                    }
                }
                // Capacity-caused analysis notifications never escape this budget.
                sql(tx.execute("DELETE FROM observation_analysis_dirty_v1 WHERE workspace_id=?1 AND request_id=?2",params![w,r]))?;
                sql(tx.commit())?;
                state.committed_progress = true;
                progress = true;
                continue;
            }
            let blob:Option<(String,String)>=sql(db.query_row("SELECT workspace,digest FROM observation_capacity_blobs_v5 ORDER BY workspace,digest LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).optional())?;
            let Some((w, d)) = blob else {
                break;
            };
            let tx = sql(db.transaction())?;
            let live:bool=sql(tx.query_row("SELECT EXISTS(SELECT 1 FROM content_instances_v2 WHERE workspace_id=?1 AND content_blob_digest=?2 AND state='complete')",params![w,d],|r|r.get(0)))?;
            if !live {
                let n=sql(tx.execute("DELETE FROM observation_text_blocks_v2 WHERE rowid IN(SELECT rowid FROM observation_text_blocks_v2 WHERE workspace=?1 AND digest=?2 LIMIT 128)",params![w,d]))?;
                if n > 0 {
                    sql(tx.commit())?;
                    state.committed_progress = true;
                    progress = true;
                    continue;
                }
                sql(tx.execute(
                    "DELETE FROM observation_text_index_v2 WHERE workspace=?1 AND digest=?2",
                    params![w, d],
                ))?;
                let path:Option<String>=sql(tx.query_row("SELECT object_path FROM content_blobs_v2 WHERE workspace_id=?1 AND blob_digest=?2",params![w,d],|r|r.get(0)).optional())?;
                if let Some(path) = path {
                    let path = self.object_path(&path).map_err(|_| Error::Corrupt)?;
                    // Blob files use an installation-wide digest. Workspace
                    // catalog retirement cannot remove another workspace's bytes.
                    let shared: bool = sql(tx.query_row("SELECT EXISTS(SELECT 1 FROM content_instances_v2 WHERE content_blob_digest=?1 AND state='complete')", [&d], |r| r.get(0)))?;
                    if !shared {
                        match fs::remove_file(&path) {
                            Ok(()) => {}
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                            Err(_) => return Err(Error::Unavailable),
                        }
                        if let Some(inventory) = &mut state.inventory {
                            inventory.record(path, 0)?;
                        }
                    }
                }
                sql(tx.execute(
                    "DELETE FROM content_blobs_v2 WHERE workspace_id=?1 AND blob_digest=?2",
                    params![w, d],
                ))?;
            }
            sql(tx.execute(
                "DELETE FROM observation_capacity_blobs_v5 WHERE workspace=?1 AND digest=?2",
                params![w, d],
            ))?;
            sql(tx.commit())?;
            state.committed_progress = true;
            progress = true;
        }
        Ok(progress)
    }
}
const CLEANUP: &[&str] = &[
    "observation_sensitive_payloads_v2",
    "content_message_instances_v2",
    "request_content_refs_v2",
    "conversation_content_streams_v2",
    "conversation_content_events_v2",
    "transcript_roots_v2",
    "observation_run_links",
    "observation_link_events",
    "observation_attempt_models_v2",
    "observation_safe_facts_v2",
    "execution_fact_events",
    "routing_receipts",
    "attempts",
    "value_ledger_entries",
    "observation_native_roots_v1",
    "observation_analysis_attempts_v1",
    "observation_analysis_recorded_v1",
    "observation_analysis_requests_v1",
    "observation_analysis_models_v1",
    "observation_analysis_reasoning_v1",
    "observation_analysis_turns_v1",
];
