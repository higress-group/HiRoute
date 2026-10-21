//! Prepare attested artifacts before reserving slots for runtime checks.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Result, build, nonce, process::Process, require, run::lock_smoke};

/// Build-only evidence: this function does not run or declare green any scenario.
pub fn prepare(root: &Path, cancel: Arc<AtomicBool>) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    let (smoke, _lock) = lock_smoke(&root)?;
    let source = build::Source::capture(&root)?;
    let started = Instant::now();
    let directory = smoke.join(format!("prepare-{}", nonce()?));
    std::fs::create_dir(&directory)?;
    let mut artifacts = Vec::new();
    for (package, binary) in [
        ("hiroute-e2e", "hiroute-e2e"),
        ("hiroute-cli", "hiroute"),
        ("hiroute-daemon", "hirouted"),
    ] {
        artifacts.push(build::get_or_build(
            &source,
            package,
            binary,
            &directory.join(package),
            cancel.clone(),
        )?);
    }
    let tool = &artifacts[0];
    tool.verify(&source)?;
    let mut command = Command::new(&tool.path);
    command.current_dir(&root).arg("prepare-current");
    let mut process = Process::spawn(&mut command, &directory.join("gateway"), cancel)?;
    let status = process.wait(Instant::now() + Duration::from_secs(3600), 64 * 1024 * 1024)?;
    require(status.success(), "gateway_preparation_failed")?;
    let (stdout, _) = process.output(64 * 1024 * 1024)?;
    let gateway: Value = serde_json::from_slice(&stdout)?;
    require(
        gateway["schema"] == "hiroute.e2e.current-preparation/v1"
            && gateway["prepared"] == true
            && gateway["source_revision"] == source.revision,
        "gateway_preparation_identity_changed",
    )?;
    tool.verify(&source)?;
    // Bind the exact receipts selected by get_or_build, including their recipes.
    // Distinct environments may legitimately produce identical executable bytes.
    let mut records = Vec::new();
    for artifact in &artifacts {
        let receipt = artifact.path.with_file_name("receipt.json");
        let mut record = serde_json::to_value(artifact)?;
        record["receipt"] = json!({
            "path":receipt, "sha256":super::digest(&std::fs::read(&receipt)?),
        });
        records.push(record);
    }
    let report = directory.join("preparation.json");
    std::fs::write(
        &report,
        serde_json::to_vec_pretty(&json!({
            "schema":"hiroute.smoke.preparation/v1", "source_revision":source.revision,
            "build_ms":started.elapsed().as_millis(), "artifacts":records, "gateway":gateway,
            "scenarios_executed":0,
        }))?,
    )?;
    Ok(report)
}
