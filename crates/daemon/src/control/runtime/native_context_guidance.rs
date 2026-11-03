//! Keep the storage-owned refusal intact while explaining a possible native account switch.
use hiroute_local_storage::LocalStorageError;
use std::path::Path;

pub(crate) const CODEX_HOME_CONFLICT: &str = "CODEX_HOME overlaps HiRoute's independent Codex Desktop directory. \
     Restart HiRoute with the original CODEX_HOME used for its active connections. \
     Then disable the independent connection through Agents > Codex Desktop if needed. \
     Existing configuration, credentials and sessions have not been replaced; \
     do not delete storage or rebind its managed files to resolve this environment conflict.";

pub(super) fn managed_artifact_error(error: &LocalStorageError, qoder_settings: &Path) -> String {
    let original = error.to_string();
    if !matches!(error, LocalStorageError::Locked) {
        return original;
    }
    let Some(config_dir) = qoder_settings.parent() else {
        return original;
    };
    format!(
        "{original}. Qoder configuration directory selected by this daemon: {config_dir:?}. \
         If QODER_CONFIG_DIR was changed while HiRoute-managed model or collaboration Skill \
         configuration was active, restart with the previous directory and use \
         `hiroute agents restore preview` / `hiroute agents restore apply` for that context \
         before changing the daemon environment and restarting again. Existing Worker tasks \
         retain their original native context; start a new task to use the new account. \
         This hint does not establish the cause of a missing key; preserve the original \
         storage, keys and native login files for recovery."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_state_keeps_original_cause_and_conditional_account_switch_guidance() {
        let settings = std::env::temp_dir().join("qoder-second-account/settings.json");
        let message = managed_artifact_error(&LocalStorageError::Locked, &settings);
        assert!(message.starts_with(&LocalStorageError::Locked.to_string()));
        assert!(message.contains("qoder-second-account"));
        assert!(message.contains("If QODER_CONFIG_DIR was changed"));
        assert!(message.contains("hiroute agents restore preview"));
        assert!(message.contains("previous directory"));
        assert!(message.contains("start a new task"));
    }

    #[test]
    fn other_storage_failures_keep_their_existing_message() {
        assert_eq!(
            managed_artifact_error(&LocalStorageError::Permission, Path::new("settings.json")),
            LocalStorageError::Permission.to_string()
        );
    }
}
