//! The same artifact admission rules apply before a Release build and at native launch.
use serde::Deserialize;

const MANIFEST_SCHEMA: &str = "hiroute.desktop.cpa-artifacts/v1";
const INVALID: &str = "BUNDLED_CPA_MANIFEST_INVALID";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: String,
    development_only: bool,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    target: String,
    os: String,
    arch: String,
    binary_name: String,
    version: String,
    commit: String,
    built_at: String,
    size: u64,
    pub(crate) sha256: String,
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

pub(crate) fn artifact_for(
    json: &str,
    os: &str,
    arch: &str,
    allow_development: bool,
) -> Result<Artifact, String> {
    let manifest: Manifest = serde_json::from_str(json).map_err(|_| INVALID)?;
    if manifest.schema != MANIFEST_SCHEMA || (!allow_development && manifest.development_only) {
        return Err(INVALID.into());
    }
    let mut matches = manifest
        .artifacts
        .into_iter()
        .filter(|artifact| artifact.os == os && artifact.arch == arch);
    let artifact = matches.next().ok_or("BUNDLED_CPA_TARGET_UNAVAILABLE")?;
    if matches.next().is_some() {
        return Err("BUNDLED_CPA_TARGET_UNAVAILABLE".into());
    }
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
        return Err(INVALID.into());
    }
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../development-cpa-artifacts.v1.json")).unwrap()
    }

    #[test]
    fn release_rejects_development_only_before_optional_runtime_launch() {
        let mut manifest = fixture();
        let original = manifest.to_string();
        assert!(artifact_for(&original, "macos", "aarch64", true).is_ok());
        assert_eq!(
            artifact_for(&original, "macos", "aarch64", false)
                .err()
                .unwrap(),
            INVALID
        );
        // A packaging-produced manifest uses the same artifact checks in either profile.
        manifest["development_only"] = false.into();
        let release = manifest.to_string();
        let debug = artifact_for(&release, "macos", "aarch64", true).unwrap();
        let optimized = artifact_for(&release, "macos", "aarch64", false).unwrap();
        assert_eq!(debug.sha256, optimized.sha256);
    }

    #[test]
    fn signed_release_identity_and_invalid_manifest() {
        let mut manifest = fixture();
        manifest["development_only"] = false.into();
        manifest["artifacts"][0]["signature"]["kind"] = "developer-id".into();
        manifest["artifacts"][0]["signature"]["team_identifier"] = "TEAM123456".into();
        assert!(artifact_for(&manifest.to_string(), "macos", "aarch64", false).is_ok());
        manifest["artifacts"][0]["signature"]["team_identifier"] = serde_json::Value::Null;
        assert!(artifact_for(&manifest.to_string(), "macos", "aarch64", false).is_err());
        assert!(artifact_for(&manifest.to_string(), "macos", "x86_64", false).is_err());
    }

    #[test]
    fn duplicate_target_and_unknown_fields_are_rejected() {
        let mut manifest = fixture();
        let duplicate = manifest["artifacts"][0].clone();
        manifest["artifacts"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(artifact_for(&manifest.to_string(), "macos", "aarch64", true).is_err());
        let mut manifest = fixture();
        manifest["unrecognized"] = true.into();
        assert!(artifact_for(&manifest.to_string(), "macos", "aarch64", true).is_err());
    }

    #[test]
    fn pinned_macos_arm64_artifact_is_complete() {
        let manifest = include_str!("../development-cpa-artifacts.v1.json");
        let artifact = artifact_for(manifest, "macos", "aarch64", true).unwrap();
        assert_eq!(artifact.target, "aarch64-apple-darwin");
        assert_eq!(artifact.version, "8.0.4-hiroute.1");
        assert_eq!(
            artifact.sha256,
            "836834077b2d1d0c6b6e39fa836267fc249bddffdcaa6bf403be776ca7579b3f"
        );
    }
}
