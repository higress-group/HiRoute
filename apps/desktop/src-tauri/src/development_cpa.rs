use std::path::{Path, PathBuf};

const MANIFEST_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cpa-artifacts.json"));

pub(crate) fn adjacent_to(daemon: &Path) -> Result<Option<(PathBuf, String)>, String> {
    let artifact = crate::cpa_manifest::artifact_for(
        MANIFEST_JSON,
        std::env::consts::OS,
        std::env::consts::ARCH,
        cfg!(debug_assertions),
    )?;
    // The managed CPA locator owns file, digest and executable validation. Always pass
    // the expected artifact so missing/tampered CPA remains a subscription-local error.
    Ok(Some((
        daemon.with_file_name("cliproxyapi"),
        artifact.sha256,
    )))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn missing_and_replaced_cpa_still_reach_managed_locator() {
        let root = tempfile::tempdir().unwrap();
        let daemon = root.path().join("hirouted");
        let (cpa, pin) = adjacent_to(&daemon).unwrap().unwrap();
        assert!(!cpa.exists());
        std::fs::write(&cpa, "replacement").unwrap();
        assert_eq!(adjacent_to(&daemon).unwrap().unwrap(), (cpa, pin));
    }
}
