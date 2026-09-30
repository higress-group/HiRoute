use serde::Deserialize;
use std::path::{Path, PathBuf};

const MANIFEST_SCHEMA: &str = "hiroute.desktop.cpa-artifacts/v1";
const MANIFEST_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/cpa-artifacts.json"));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    development_only: bool,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    target: String,
    os: String,
    arch: String,
    binary_name: String,
    version: String,
    commit: String,
    built_at: String,
    size: u64,
    sha256: String,
    file_description: String,
    dynamic_dependencies: Vec<String>,
    signature: Signature,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Signature {
    kind: String,
    team_identifier: Option<String>,
}

fn artifact_for<'a>(manifest: &'a Manifest, os: &str, arch: &str) -> Result<&'a Artifact, String> {
    if manifest.schema != MANIFEST_SCHEMA || (!cfg!(debug_assertions) && manifest.development_only)
    {
        return Err("BUNDLED_CPA_MANIFEST_INVALID".into());
    }
    let matches = manifest
        .artifacts
        .iter()
        .filter(|artifact| artifact.os == os && artifact.arch == arch)
        .collect::<Vec<_>>();
    let [artifact] = matches.as_slice() else {
        return Err("BUNDLED_CPA_TARGET_UNAVAILABLE".into());
    };
    let sha_valid = artifact.sha256.len() == 64
        && artifact
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    let commit_valid = artifact.commit.len() == 40
        && artifact
            .commit
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
    if artifact.target.is_empty()
        || artifact.binary_name != "cliproxyapi"
        || artifact.version.is_empty()
        || !commit_valid
        || !artifact.built_at.ends_with('Z')
        || artifact.size == 0
        || !sha_valid
        || artifact.file_description.is_empty()
        || artifact.dynamic_dependencies.is_empty()
        || !match artifact.signature.kind.as_str() {
            "adhoc" => artifact.signature.team_identifier.is_none(),
            "developer-id" => artifact
                .signature
                .team_identifier
                .as_ref()
                .is_some_and(|team| !team.is_empty()),
            _ => false,
        }
    {
        return Err("BUNDLED_CPA_MANIFEST_INVALID".into());
    }
    Ok(artifact)
}

pub(crate) fn adjacent_to(daemon: &Path) -> Result<Option<(PathBuf, String)>, String> {
    let manifest: Manifest =
        serde_json::from_str(MANIFEST_JSON).map_err(|_| "BUNDLED_CPA_MANIFEST_INVALID")?;
    let path = daemon.with_file_name("cliproxyapi");
    let artifact = artifact_for(&manifest, std::env::consts::OS, std::env::consts::ARCH)?;
    // The managed CPA locator owns file, digest and executable validation. Always pass
    // the expected artifact so missing/tampered CPA remains a subscription-local error.
    Ok(Some((path, artifact.sha256.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_release_identity_and_invalid_manifest() {
        let mut manifest: Manifest =
            serde_json::from_str(include_str!("../development-cpa-artifacts.v1.json")).unwrap();
        manifest.development_only = false;
        manifest.artifacts[0].signature.kind = "developer-id".into();
        manifest.artifacts[0].signature.team_identifier = Some("TEAM123456".into());
        assert!(artifact_for(&manifest, "macos", "aarch64").is_ok());
        manifest.artifacts[0].signature.team_identifier = None;
        assert!(artifact_for(&manifest, "macos", "aarch64").is_err());
        assert!(artifact_for(&manifest, "macos", "x86_64").is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn missing_and_replaced_cpa_still_reach_managed_locator() {
        let root = tempfile::tempdir().unwrap();
        let daemon = root.path().join("hirouted");
        let (cpa, pin) = adjacent_to(&daemon).unwrap().unwrap();
        assert!(!cpa.exists());
        std::fs::write(&cpa, "replacement").unwrap();
        assert_eq!(adjacent_to(&daemon).unwrap().unwrap(), (cpa, pin));
    }

    #[test]
    fn pinned_macos_arm64_artifact_is_complete() {
        let manifest: Manifest =
            serde_json::from_str(include_str!("../development-cpa-artifacts.v1.json")).unwrap();
        let artifact = artifact_for(&manifest, "macos", "aarch64").unwrap();
        assert_eq!(artifact.target, "aarch64-apple-darwin");
        assert_eq!(artifact.version, "8.0.4-hiroute.1");
        assert_eq!(
            artifact.sha256,
            "836834077b2d1d0c6b6e39fa836267fc249bddffdcaa6bf403be776ca7579b3f"
        );
    }
}
