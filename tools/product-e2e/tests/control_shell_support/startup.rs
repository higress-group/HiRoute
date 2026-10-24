//! Private, bounded failure snapshots for the control-shell startup assertion.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const READINESS_TIMEOUT: Duration = Duration::from_secs(10);
const SNAPSHOT_BYTES: u64 = 64 * 1024;

pub(super) struct StartupCapture {
    root: PathBuf,
}

impl StartupCapture {
    pub(super) fn new(fixture: &Path) -> io::Result<Self> {
        let root = fixture.join("control-startup");
        std::fs::DirBuilder::new().mode(0o700).create(&root)?;
        Ok(Self { root })
    }

    pub(super) fn configure(&self, command: &mut Command) -> io::Result<()> {
        // Regular files cannot fill an undrained pipe or wait for a descendant's EOF.
        // They live only with the fixture; failure retention copies bounded tails.
        command
            .stdout(private_file(&self.root.join("stdout"))?)
            .stderr(private_file(&self.root.join("stderr"))?)
            // This fresh root excludes the earlier rejected role=all boot.
            .arg("--diagnostics-root")
            .arg(self.root.join("diagnostics"))
            .args(["--diagnostic-level-override", "debug"]);
        Ok(())
    }

    pub(super) fn wait_for_endpoint(
        &self,
        endpoint: &Path,
        child: &mut Child,
        evidence_root: &Path,
    ) -> Result<(), String> {
        let started = Instant::now();
        let deadline = started + READINESS_TIMEOUT;
        loop {
            if endpoint
                .metadata()
                .is_ok_and(|metadata| metadata.file_type().is_socket())
            {
                return Ok(());
            }
            let (reason, status) = match child.try_wait() {
                Ok(Some(status)) => ("hirouted exited before readiness", status.to_string()),
                Err(error) => (
                    "hirouted readiness status unavailable",
                    format!("{:?}", error.kind()),
                ),
                Ok(None) if Instant::now() >= deadline => {
                    ("hirouted readiness timed out", "running".to_owned())
                }
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(25));
                    continue;
                }
            };
            return Err(self.failure(
                reason,
                &status,
                child.id(),
                started.elapsed(),
                evidence_root,
            ));
        }
    }

    fn failure(
        &self,
        reason: &str,
        status: &str,
        pid: u32,
        elapsed: Duration,
        evidence_root: &Path,
    ) -> String {
        match self.preserve(reason, status, pid, elapsed, evidence_root) {
            Ok(path) => format!(
                "{reason}; child={status}; private evidence: {}",
                path.display()
            ),
            Err(error) => format!(
                "{reason}; child={status}; evidence unavailable: {:?}",
                error.kind()
            ),
        }
    }

    fn preserve(
        &self,
        reason: &str,
        status: &str,
        pid: u32,
        elapsed: Duration,
        evidence_root: &Path,
    ) -> io::Result<PathBuf> {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(evidence_root)?;
        let directory = tempfile::Builder::new()
            .prefix("control-shell-startup-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(evidence_root)?;
        // Keep even an incomplete export if a later write fails.
        let destination = directory.keep();
        let mut files = serde_json::Map::new();
        let mut diagnostics = json!({"state": "unavailable", "current_boot": null});
        for (source, name) in [
            ("stdout", "stdout.tail"),
            ("stderr", "stderr.tail"),
            (
                "diagnostics/daemon/current.jsonl",
                "daemon-current.tail.jsonl",
            ),
        ] {
            let metadata = match read_tail(&self.root.join(source)) {
                Ok((bytes, length, offset)) => {
                    private_file(&destination.join(name))?.write_all(&bytes)?;
                    if name == "daemon-current.tail.jsonl" {
                        diagnostics = diagnostic_summary(&bytes, offset > 0);
                    }
                    json!({
                        "state": "captured", "source_bytes": length,
                        "offset": offset, "captured_bytes": bytes.len(),
                        "truncated": offset > 0 || bytes.len() as u64 != length,
                    })
                }
                Err(error) => {
                    json!({"state": "unavailable", "error": format!("{:?}", error.kind())})
                }
            };
            files.insert(name.into(), metadata);
        }
        let report = json!({
            "reason": reason, "pid": pid, "child_status": status,
            "elapsed_ms": elapsed.as_millis(), "deadline_ms": READINESS_TIMEOUT.as_millis(),
            "snapshot_bytes_per_file": SNAPSHOT_BYTES,
            "files": files, "diagnostics": diagnostics,
            // The writer is asynchronous. A live snapshot never establishes a flush
            // or absence of dropped events, even if every retained line parses.
            "writer_flush": "unknown", "snapshot_before_cleanup": true,
        });
        private_file(&destination.join("report.json"))?
            .write_all(&serde_json::to_vec_pretty(&report)?)?;
        Ok(destination)
    }
}

fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

fn read_tail(path: &Path) -> io::Result<(Vec<u8>, u64, u64)> {
    // Refuse special files: diagnostics collection must never wait on a FIFO.
    if !std::fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let offset = length.saturating_sub(SNAPSHOT_BYTES);
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(length - offset).read_to_end(&mut bytes)?;
    Ok((bytes, length, offset))
}

fn diagnostic_summary(bytes: &[u8], truncated: bool) -> Value {
    let mut boots = std::collections::BTreeSet::new();
    let mut records = 0;
    let mut invalid = 0;
    let mut last_stage = None;
    let mut level_applied = false;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            invalid += 1;
            continue;
        };
        let Some(boot) = record["boot_id"]
            .as_str()
            .filter(|boot| boot.len() == 32 && boot.bytes().all(|byte| byte.is_ascii_hexdigit()))
        else {
            invalid += 1;
            continue;
        };
        boots.insert(boot.to_owned());
        records += 1;
        level_applied |= record["event"]["level_applied"].is_object();
        for kind in ["stage_begin", "stage_end", "stage_progress"] {
            if let Some(stage) = record["event"][kind]["stage"].as_str()
                && [
                    "root_validate",
                    "resident_lock",
                    "artifact_validate",
                    "spawn",
                    "ready_wait",
                    "catalog_load",
                    "gateway_start",
                    "worker_revalidate",
                    "control_bind",
                    "publication_reconcile",
                ]
                .contains(&stage)
            {
                last_stage = Some(stage.to_owned());
            }
        }
    }
    let single_boot = boots.len() == 1;
    json!({
        "state": if records == 0 { "unavailable" } else if truncated || invalid > 0 || !single_boot || !level_applied { "partial" } else { "available" },
        "current_boot": if single_boot { boots.first() } else { None },
        "last_observed_stage": if single_boot { last_stage } else { None },
        "level_applied_observed": level_applied,
        "record_count": records, "invalid_record_count": invalid,
        "truncated": truncated,
    })
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
