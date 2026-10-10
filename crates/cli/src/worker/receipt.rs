use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hiroute_application_api::CanonicalDigest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SubmissionReceiptRecord {
    schema: String,
    operation: String,
    submission_key: String,
    selectors: Value,
    intent_digest: CanonicalDigest,
    prepared_at_ms: u64,
    updated_at_ms: u64,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
}

pub(super) struct SubmissionReceipt {
    path: PathBuf,
    record: SubmissionReceiptRecord,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReceiptError {
    Conflict,
    Unavailable,
}

impl SubmissionReceipt {
    pub(super) fn prepare(
        operation: &str,
        key: &str,
        selectors: Value,
        intent: &impl Serialize,
    ) -> Result<Self, ReceiptError> {
        let root = receipt_root().ok_or(ReceiptError::Unavailable)?;
        Self::prepare_in(&root, operation, key, selectors, intent)
    }

    fn prepare_in(
        root: &Path,
        operation: &str,
        key: &str,
        selectors: Value,
        intent: &impl Serialize,
    ) -> Result<Self, ReceiptError> {
        prepare_private_directory(root)?;
        let name = CanonicalDigest::of(&("hiroute.worker-receipt/v1", operation, key))
            .map_err(|_| ReceiptError::Unavailable)?;
        let path = root.join(format!(
            "{}.json",
            name.as_str()
                .strip_prefix("sha256:")
                .unwrap_or(name.as_str())
        ));
        let now = now_ms()?;
        let record = SubmissionReceiptRecord {
            schema: "hiroute.worker-submission-receipt/v1".into(),
            operation: operation.into(),
            submission_key: key.into(),
            selectors,
            intent_digest: CanonicalDigest::of(intent).map_err(|_| ReceiptError::Unavailable)?,
            prepared_at_ms: now,
            updated_at_ms: now,
            state: "prepared".into(),
            task_id: None,
            run_id: None,
        };
        if path.exists() {
            let existing = read_receipt(&path)?;
            if !same_submission(&existing, &record) {
                return Err(ReceiptError::Conflict);
            }
            return Ok(Self {
                path,
                record: existing,
            });
        }
        match write_receipt(&path, &record, false) {
            Ok(()) => Ok(Self { path, record }),
            Err(ReceiptError::Conflict) => {
                let existing = read_receipt(&path)?;
                if same_submission(&existing, &record) {
                    Ok(Self {
                        path,
                        record: existing,
                    })
                } else {
                    Err(ReceiptError::Conflict)
                }
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn record(
        mut self,
        state: &str,
        task_id: Option<&str>,
        run_id: Option<&str>,
    ) -> Result<(), ReceiptError> {
        self.record.updated_at_ms = now_ms()?;
        self.record.state = state.into();
        self.record.task_id = task_id.map(str::to_owned);
        self.record.run_id = run_id.map(str::to_owned);
        write_receipt(&self.path, &self.record, true)
    }
}

fn same_submission(left: &SubmissionReceiptRecord, right: &SubmissionReceiptRecord) -> bool {
    left.operation == right.operation
        && left.submission_key == right.submission_key
        && left.selectors == right.selectors
        && left.intent_digest == right.intent_digest
}

fn receipt_root() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HIROUTE_WORKER_RECEIPT_DIR") {
        return Some(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Application Support/HiRoute/worker-receipts"))
    }
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("HiRoute/worker-receipts"));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local/share"))
            })
            .map(|root| root.join("hiroute/worker-receipts"))
    }
}

fn prepare_private_directory(path: &Path) -> Result<(), ReceiptError> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|_| ReceiptError::Unavailable)?;
    #[cfg(unix)]
    {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| ReceiptError::Unavailable)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(ReceiptError::Unavailable);
        }
    }
    Ok(())
}

fn read_receipt(path: &Path) -> Result<SubmissionReceiptRecord, ReceiptError> {
    validate_private_file(path)?;
    let bytes = std::fs::read(path).map_err(|_| ReceiptError::Unavailable)?;
    if bytes.len() > 16 * 1024 {
        return Err(ReceiptError::Unavailable);
    }
    serde_json::from_slice(&bytes).map_err(|_| ReceiptError::Unavailable)
}

fn write_receipt(
    path: &Path,
    record: &SubmissionReceiptRecord,
    replace: bool,
) -> Result<(), ReceiptError> {
    let parent = path.parent().ok_or(ReceiptError::Unavailable)?;
    let temporary = parent.join(format!(".receipt-{}-{}", std::process::id(), now_ms()?));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|_| ReceiptError::Unavailable)?;
        serde_json::to_writer(&mut file, record).map_err(|_| ReceiptError::Unavailable)?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|_| ReceiptError::Unavailable)?;
        if replace {
            replace_file(&temporary, path)?;
        } else {
            match std::fs::hard_link(&temporary, path) {
                Ok(()) => {
                    std::fs::remove_file(&temporary).map_err(|_| ReceiptError::Unavailable)?;
                }
                Err(_) if path.exists() => return Err(ReceiptError::Conflict),
                Err(_) => return Err(ReceiptError::Unavailable),
            }
        }
        sync_parent(parent)?;
        validate_private_file(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn replace_file(temporary: &Path, path: &Path) -> Result<(), ReceiptError> {
    match std::fs::rename(temporary, path) {
        Ok(()) => Ok(()),
        // Windows does not replace an existing destination with rename. Keep the already
        // durable prepared receipt rather than introduce a remove/rename visibility gap.
        Err(_) if path.exists() => {
            std::fs::remove_file(temporary).map_err(|_| ReceiptError::Unavailable)
        }
        Err(_) => Err(ReceiptError::Unavailable),
    }
}

fn sync_parent(path: &Path) -> Result<(), ReceiptError> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| ReceiptError::Unavailable)?;
    }
    Ok(())
}

fn validate_private_file(path: &Path) -> Result<(), ReceiptError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ReceiptError::Unavailable)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ReceiptError::Unavailable);
    }
    Ok(())
}

fn now_ms() -> Result<u64, ReceiptError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ReceiptError::Unavailable)
        .and_then(|duration| {
            u64::try_from(duration.as_millis()).map_err(|_| ReceiptError::Unavailable)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn receipt_rejects_same_key_with_changed_intent_and_stores_no_prompt() {
        let root = tempfile::tempdir().unwrap();
        let one = json!({"prompt":"secret one"});
        let receipt = SubmissionReceipt::prepare_in(
            root.path(),
            "exec",
            "same/key",
            json!({"plan":"p"}),
            &one,
        )
        .unwrap();
        let receipt_path = receipt.path.clone();
        receipt
            .record("accepted", Some("task/one"), Some("run/one"))
            .unwrap();
        let stored = read_receipt(&receipt_path).unwrap();
        assert_eq!(stored.task_id.as_deref(), Some("task/one"));
        assert_eq!(stored.run_id.as_deref(), Some("run/one"));
        let bytes = std::fs::read(&receipt_path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("secret one"));
        let two = json!({"prompt":"secret two"});
        assert_eq!(
            SubmissionReceipt::prepare_in(
                root.path(),
                "exec",
                "same/key",
                json!({"plan":"p"}),
                &two,
            )
            .err(),
            Some(ReceiptError::Conflict)
        );
    }
}
