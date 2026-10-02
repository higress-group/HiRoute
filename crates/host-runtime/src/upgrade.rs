//! Host maintenance admission; no persistent authority, cancellation or task resubmission.
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Default)]
pub struct UpgradeDrain {
    state: AtomicU64,
}
pub struct UpgradeCall {
    owner: Arc<UpgradeDrain>,
}
impl UpgradeDrain {
    pub fn pause(&self) {
        self.state.fetch_or(1, Ordering::AcqRel);
    }
    pub fn resume(&self) {
        self.state.fetch_and(!1, Ordering::AcqRel);
    }
    pub fn paused(&self) -> bool {
        self.state.load(Ordering::Acquire) & 1 != 0
    }
    pub fn active(&self) -> u64 {
        self.state.load(Ordering::Acquire) >> 1
    }
    pub fn enter(self: &Arc<Self>, existing_task: bool) -> Option<UpgradeCall> {
        self.state
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                if state & 1 != 0 && !existing_task {
                    None
                } else {
                    state.checked_add(2)
                }
            })
            .ok()
            .map(|_| UpgradeCall {
                owner: self.clone(),
            })
    }
}
impl Drop for UpgradeCall {
    fn drop(&mut self) {
        self.owner.state.fetch_sub(2, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpgradeAction {
    Prepare,
    Status,
    Cancel,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherUpgradeRequest {
    pub schema: String,
    pub registration_id: String,
    pub action: UpgradeAction,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherUpgradeStatus {
    pub schema: String,
    pub registration_id: String,
    pub paused: bool,
    pub active_calls: u64,
    pub active_tasks: u64,
}
impl LauncherUpgradeStatus {
    pub fn drained(&self) -> bool {
        self.paused && self.active_calls == 0 && self.active_tasks == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrade_wait_retains_existing_calls_and_allows_task_completion_then_cancel_resumes() {
        let gate = Arc::new(UpgradeDrain::default());
        let call = gate.enter(false).unwrap();
        gate.pause();
        assert!(gate.enter(false).is_none());
        let task = gate.enter(true).unwrap();
        assert_eq!(gate.active(), 2);
        drop(call);
        drop(task);
        assert_eq!(gate.active(), 0);
        assert!(gate.paused());
        gate.resume();
        assert!(gate.enter(false).is_some());
    }
}
