//! A helper's readiness or a parent's disappearance does not authorize replacement.
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WindowsUpdateCommitV1 {
    schema: String,
    generation: String,
    installer_sha256: String,
}
impl WindowsUpdateCommitV1 {
    pub(crate) fn new(generation: String, installer_sha256: String) -> Self {
        Self {
            schema: "hiroute.windows-shell-update-commit/v1".into(),
            generation,
            installer_sha256,
        }
    }
    pub(crate) fn matches(&self, generation: &str, installer_sha256: &str) -> bool {
        self.schema == "hiroute.windows-shell-update-commit/v1"
            && self.generation == generation
            && self.installer_sha256 == installer_sha256
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_parent_loss_and_another_installer_cannot_authorize_replacement() {
        for record in [b"{}".as_slice(), br#"{"ready":true,"generation":"abc"}"#] {
            assert!(serde_json::from_slice::<WindowsUpdateCommitV1>(record).is_err());
        }
        let commit:WindowsUpdateCommitV1=serde_json::from_value(serde_json::json!({"schema":"hiroute.windows-shell-update-commit/v1","generation":"abc","installer_sha256":"a".repeat(64)})).unwrap();
        assert!(commit.matches("abc", &"a".repeat(64)));
        assert!(!commit.matches("next-attempt", &"a".repeat(64)));
        assert!(!commit.matches("abc", &"b".repeat(64)));
        let mut unknown =
            serde_json::to_value(WindowsUpdateCommitV1::new("abc".into(), "a".repeat(64))).unwrap();
        unknown["executable"] = serde_json::json!("arbitrary.exe");
        assert!(serde_json::from_value::<WindowsUpdateCommitV1>(unknown).is_err());
    }
}
