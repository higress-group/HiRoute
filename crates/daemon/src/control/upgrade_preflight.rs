//! Candidate-only read-only preflight used by verified installers.
use hiroute_host_runtime::{
    StorageRecovery, StorageRecoveryAction, StorageRecoveryCause, UpgradePreflight,
};
use std::path::PathBuf;
pub(super) fn run_if_requested() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_none_or(|a| a != "--upgrade-preflight") {
        return None;
    }
    if args.len() != 5 || args[1] != "--storage-root" || args[3] != "--recovery-package" {
        return Some(2);
    }
    let root = PathBuf::from(&args[2]);
    let package = PathBuf::from(&args[4]);
    if !root.is_absolute() || !package.is_absolute() {
        return Some(2);
    }
    let result = (|| {
        let seal = hiroute_observation::LocalObservationStore::legacy_history_requires_seal(
            root.join("observation"),
        )
        .map_err(|_| {
            Box::new(StorageRecovery::new(
                StorageRecoveryCause::UnknownState,
                StorageRecoveryAction::PreserveAndDiagnose,
            ))
        })?;
        let required =
            hiroute_local_storage::LocalStorageSet::preflight_upgrade(&root, &package, seal)
                .map_err(|e| Box::new(e.recovery()))?;
        Ok::<_, Box<StorageRecovery>>((seal, required))
    })();
    let available = fs2::available_space(if root.exists() {
        &root
    } else {
        root.parent().unwrap_or(&root)
    })
    .unwrap_or(0);
    let (history_cutover_required, required_bytes, recovery) = match result {
        Ok((seal, required)) => (seal, required, None),
        Err(e) => (
            false,
            e.storage_space
                .as_ref()
                .map_or(0, |space| space.required_bytes),
            Some(*e),
        ),
    };
    let failed = recovery.is_some();
    let report = UpgradePreflight {
        schema: "hiroute.upgrade-preflight/v1".into(),
        history_cutover_required,
        required_bytes,
        available_bytes: available,
        recovery,
    };
    if serde_json::to_writer(std::io::stdout().lock(), &report).is_err() {
        return Some(1);
    }
    Some(if failed { 1 } else { 0 })
}
