use hiroute_domain::CanonicalDigest;
use hiroute_integrations::model_feed::MAX_MODEL_FEED_BYTES;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub const STATE_SCHEMA: &str = "hiroute.model-feed.state/v2";
const MAX_STATE_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HighestAccepted {
    pub sequence: u64,
    pub payload_digest: CanonicalDigest,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AcceptanceCheckpoint {
    schema: String,
    highest: Option<HighestAccepted>,
    #[serde(default)]
    data_floor: Option<DataFloor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DataFloor {
    pub revision: u64,
    pub digest: CanonicalDigest,
}
impl DataFloor {
    pub fn advance(&mut self, next: &Self) -> Result<(), String> {
        if self.revision == next.revision && self.digest != next.digest {
            return Err("same_data_revision_conflicting_digest".into());
        }
        if next.revision > self.revision {
            *self = next.clone();
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HeldPublication {
    pub sequence: u64,
    pub payload_digest: CanonicalDigest,
    pub retrieved_at_ms: u64,
    pub issued_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeedAttempt {
    pub at_ms: u64,
    pub result: String,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeedState {
    pub schema: String,
    #[serde(default)]
    pub data_floor: Option<DataFloor>,
    #[serde(default)]
    pub selected_data: Option<DataFloor>,
    #[serde(default)]
    pub qualification_digest: Option<CanonicalDigest>,
    #[serde(default = "enabled")]
    pub retrieval_enabled: bool,
    pub disclosure_acknowledged: bool,
    pub highest_accepted: Option<HighestAccepted>,
    #[serde(default)]
    pub sequence_floor: u64,
    pub generation: u64,
    pub held: Option<HeldPublication>,
    pub last_attempt: Option<FeedAttempt>,
    pub last_success_at_ms: Option<u64>,
    pub etag: Option<String>,
}

fn enabled() -> bool {
    true
}
impl Default for FeedState {
    fn default() -> Self {
        Self {
            schema: STATE_SCHEMA.into(),
            data_floor: None,
            selected_data: None,
            qualification_digest: None,
            retrieval_enabled: true,
            disclosure_acknowledged: false,
            highest_accepted: None,
            sequence_floor: 0,
            generation: 0,
            held: None,
            last_attempt: None,
            last_success_at_ms: None,
            etag: None,
        }
    }
}

pub struct FeedStore {
    root: PathBuf,
    acceptance: PathBuf,
}
impl FeedStore {
    pub fn open(storage_root: &Path) -> Result<Self, String> {
        let root = storage_root.join("model-feed");
        private_directory(&root)?;
        private_directory(&root.join("publications"))?;
        Ok(Self {
            root,
            acceptance: storage_root.join("model-feed-acceptance-v2.json"),
        })
    }
    pub fn read_state(&self) -> Result<FeedState, String> {
        let bytes = match bounded_read(&self.root.join("state.json"), MAX_STATE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(FeedState::default());
            }
            Err(_) => return Err("feed_state_unreadable".into()),
        };
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "feed_state_invalid")?;
        if value.get("schema").and_then(|v| v.as_str()) == Some("hiroute.model-feed.state/v1") {
            return self.convert_v1(&bytes, value);
        }
        if value
            .get("schema")
            .and_then(|v| v.as_str())
            .is_some_and(|schema| schema != STATE_SCHEMA)
        {
            return Err("feed_state_newer_schema".into());
        }
        let state: FeedState = serde_json::from_value(value).map_err(|_| "feed_state_invalid")?;
        if state.schema != STATE_SCHEMA {
            return Err("feed_state_newer_schema".into());
        }
        if state
            .data_floor
            .as_ref()
            .is_some_and(|f| f.revision == 0 || CanonicalDigest::parse(f.digest.as_str()).is_err())
            || state.selected_data.as_ref().is_some_and(|f| {
                f.revision == 0 || CanonicalDigest::parse(f.digest.as_str()).is_err()
            })
            || state
                .qualification_digest
                .as_ref()
                .is_some_and(|d| CanonicalDigest::parse(d.as_str()).is_err())
        {
            return Err("feed_state_invalid".into());
        }
        if let Some(highest) = &state.highest_accepted
            && (highest.sequence == 0
                || CanonicalDigest::parse(highest.payload_digest.as_str()).is_err())
        {
            return Err("feed_state_invalid".into());
        }
        if let Some(held) = &state.held
            && (CanonicalDigest::parse(held.payload_digest.as_str()).is_err()
                || !state.highest_accepted.as_ref().is_some_and(|high| {
                    high.sequence == held.sequence && high.payload_digest == held.payload_digest
                }))
        {
            return Err("feed_state_invalid".into());
        }
        Ok(state)
    }
    pub fn commit(&self, state: &FeedState) -> Result<(), String> {
        let bytes = serde_json::to_vec(state).map_err(|_| "feed_state_encoding")?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err("feed_state_too_large".into());
        }
        self.retain_checkpoint(state.highest_accepted.as_ref(), state.data_floor.as_ref())?;
        hiroute_platform::durable::write_private_atomic(&self.root.join("state.json"), &bytes)
            .map_err(|_| "feed_state_commit_failed".into())
    }
    fn read_checkpoint(&self) -> Result<Option<AcceptanceCheckpoint>, String> {
        let bytes = match bounded_read(&self.acceptance, MAX_STATE_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("feed_acceptance_unreadable".into()),
        };
        let record: AcceptanceCheckpoint =
            serde_json::from_slice(&bytes).map_err(|_| "feed_acceptance_invalid")?;
        if record.schema != "hiroute.model-feed.acceptance/v2"
            || record.highest.as_ref().is_some_and(|h| {
                h.sequence == 0 || CanonicalDigest::parse(h.payload_digest.as_str()).is_err()
            })
            || record.data_floor.as_ref().is_some_and(|f| {
                f.revision == 0 || CanonicalDigest::parse(f.digest.as_str()).is_err()
            })
        {
            return Err("feed_acceptance_invalid".into());
        }
        Ok(Some(record))
    }
    pub fn read_acceptance(&self) -> Result<Option<HighestAccepted>, String> {
        Ok(self.read_checkpoint()?.and_then(|c| c.highest))
    }
    pub fn read_data_floor(&self) -> Result<Option<DataFloor>, String> {
        Ok(self.read_checkpoint()?.and_then(|c| c.data_floor))
    }
    #[cfg(test)]
    pub(super) fn retain_acceptance(&self, next: &HighestAccepted) -> Result<(), String> {
        self.retain_checkpoint(Some(next), None)
    }
    fn retain_checkpoint(
        &self,
        highest: Option<&HighestAccepted>,
        floor: Option<&DataFloor>,
    ) -> Result<(), String> {
        let old = self.read_checkpoint()?;
        let mut highest = highest.cloned();
        let mut floor = floor.cloned();
        if let Some(old) = old {
            if let Some(previous) = old.highest {
                if highest.as_ref().is_some_and(|next| {
                    previous.sequence > next.sequence
                        || previous.sequence == next.sequence
                            && previous.payload_digest != next.payload_digest
                }) {
                    return Err("feed_acceptance_conflict".into());
                }
                if highest.is_none() {
                    highest = Some(previous);
                }
            }
            if let Some(previous) = old.data_floor {
                match &mut floor {
                    Some(next) => {
                        next.advance(&previous)?;
                    }
                    None => floor = Some(previous),
                }
            }
        }
        if highest.is_none() && floor.is_none() {
            return Ok(());
        }
        let record = AcceptanceCheckpoint {
            schema: "hiroute.model-feed.acceptance/v2".into(),
            highest,
            data_floor: floor,
        };
        let bytes = serde_json::to_vec(&record).map_err(|_| "feed_acceptance_encoding")?;
        hiroute_platform::durable::write_private_atomic(&self.acceptance, &bytes)
            .map_err(|_| "feed_acceptance_commit_failed".into())
    }
    fn convert_v1(&self, bytes: &[u8], value: serde_json::Value) -> Result<FeedState, String> {
        let mut old: FeedState = serde_json::from_value(value).map_err(|_| "feed_state_invalid")?;
        // Validate before retaining. A prior namespace never supplies a v2 acceptance floor.
        if old.highest_accepted.as_ref().is_some_and(|h| {
            h.sequence == 0 || CanonicalDigest::parse(h.payload_digest.as_str()).is_err()
        }) {
            return Err("feed_state_invalid".into());
        }
        let legacy_checkpoint = self
            .root
            .parent()
            .ok_or("feed_state_invalid")?
            .join("model-feed-acceptance.json");
        match bounded_read(&legacy_checkpoint, MAX_STATE_BYTES) {
            Ok(bytes) => {
                let record: AcceptanceCheckpoint =
                    serde_json::from_slice(&bytes).map_err(|_| "feed_acceptance_invalid")?;
                if record.schema != "hiroute.model-feed.acceptance/v1"
                    || record.highest.as_ref().is_none_or(|h| {
                        h.sequence == 0
                            || CanonicalDigest::parse(h.payload_digest.as_str()).is_err()
                    })
                {
                    return Err("feed_acceptance_invalid".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("feed_acceptance_unreadable".into()),
        }
        let retained = self.root.join("retained-v1-state.json");
        match bounded_read(&retained, MAX_STATE_BYTES) {
            Ok(previous) if previous == bytes => {}
            Ok(_) => return Err("feed_legacy_state_conflict".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                hiroute_platform::durable::write_private_atomic(&retained, bytes)
                    .map_err(|_| "feed_legacy_state_retention_failed")?;
            }
            Err(_) => return Err("feed_legacy_state_retention_failed".into()),
        }
        old.schema = STATE_SCHEMA.into();
        old.highest_accepted = None;
        old.sequence_floor = 0;
        old.data_floor = None;
        old.held = None;
        old.etag = None;
        old.last_success_at_ms = None;
        old.last_attempt = Some(FeedAttempt {
            at_ms: super::now_ms(),
            result: "fallback".into(),
            reason: Some("retained_v1_settings_converted".into()),
        });
        self.commit(&old)?;
        Ok(old)
    }
    pub fn quarantine_state(&self) -> Result<(), String> {
        let path = self.root.join("state.json");
        bounded_read(&path, MAX_STATE_BYTES).map_err(|_| "feed_state_unreadable")?;
        let mut nonce = [0u8; 8];
        getrandom::fill(&mut nonce).map_err(|_| "feed_quarantine_unavailable")?;
        let target = self.root.join(format!(
            "state.quarantined-{}.json",
            u64::from_le_bytes(nonce)
        ));
        hiroute_platform::durable::rename(path, target).map_err(|_| "feed_quarantine_failed".into())
    }
    fn blob_path(&self, digest: &CanonicalDigest) -> Result<PathBuf, String> {
        CanonicalDigest::parse(digest.as_str()).map_err(|_| "feed_blob_invalid_digest")?;
        Ok(self
            .root
            .join("publications")
            .join(format!("{}.json", &digest.as_str()[7..])))
    }
    pub fn blob(&self, held: &HeldPublication) -> Result<Vec<u8>, String> {
        let bytes = bounded_read(&self.blob_path(&held.payload_digest)?, MAX_MODEL_FEED_BYTES)
            .map_err(|_| "feed_blob_unreadable")?;
        if CanonicalDigest::of_bytes(&bytes) != held.payload_digest {
            return Err("feed_blob_digest_mismatch".into());
        }
        Ok(bytes)
    }
    pub fn persist_blob(&self, bytes: &[u8], digest: &CanonicalDigest) -> Result<(), String> {
        if bytes.len() > MAX_MODEL_FEED_BYTES || &CanonicalDigest::of_bytes(bytes) != digest {
            return Err("feed_blob_invalid".into());
        }
        let path = self.blob_path(digest)?;
        match bounded_read(&path, MAX_MODEL_FEED_BYTES) {
            Ok(existing) if existing == bytes => return Ok(()),
            Ok(_) => return Err("feed_blob_conflict".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("feed_blob_unreadable".into()),
        }
        hiroute_platform::durable::write_private_atomic(&path, bytes)
            .map_err(|_| "feed_blob_write_failed".into())
    }
    pub fn retire_unreferenced(&self, held: Option<&HeldPublication>) {
        let keep = held.and_then(|held| self.blob_path(&held.payload_digest).ok());
        if let Ok(entries) = std::fs::read_dir(self.root.join("publications")) {
            for entry in entries.flatten().take(8192) {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.is_ascii()
                    && name.len() == 69
                    && name.ends_with(".json")
                    && name[..64].bytes().all(|b| b.is_ascii_hexdigit())
                    && Some(entry.path()) != keep
                    && entry.file_type().is_ok_and(|t| t.is_file())
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    hiroute_platform::private_fs::prepare_storage_directory(path)
        .map_err(|_| "feed_directory_unavailable".into())
}

fn bounded_read(path: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut file = hiroute_platform::private_fs::open_private_file(path)?;
    hiroute_platform::private_fs::verify_single_link(&file)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err(std::io::Error::other("feed file exceeds bound"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(std::io::Error::other("feed file exceeds bound"));
    }
    Ok(bytes)
}
