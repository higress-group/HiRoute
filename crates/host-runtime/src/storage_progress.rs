//! Storage readiness phases shared by daemon startup and the Desktop status projection.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageUpgradePhase {
    SourceCheck,
    Backup,
    Conversion,
    Validation,
    ServiceRecovery,
}

/// One startup may finish an old batch before converting the next one. Share
/// the furthest entered phase across those owners; no frame means ordinary boot.
#[derive(Clone, Debug)]
pub struct StorageUpgradeProgress {
    sender: std::sync::mpsc::Sender<StorageUpgradePhase>,
    phase: std::sync::Arc<std::sync::Mutex<Option<StorageUpgradePhase>>>,
}

impl From<std::sync::mpsc::Sender<StorageUpgradePhase>> for StorageUpgradeProgress {
    fn from(sender: std::sync::mpsc::Sender<StorageUpgradePhase>) -> Self {
        Self {
            sender,
            phase: Default::default(),
        }
    }
}

impl StorageUpgradeProgress {
    pub fn report(&self, phase: StorageUpgradePhase) {
        if let Ok(mut last) = self.phase.lock()
            && last.is_none_or(|previous| phase > previous)
        {
            *last = Some(phase);
            let _ = self.sender.send(phase);
        }
    }

    pub fn started(&self) -> bool {
        self.phase.lock().is_ok_and(|phase| phase.is_some())
    }
}
