//! Read the website's one current release catalog; URLs are native-derived, never IPC input.
use semver::Version;
use serde::{Deserialize, Serialize};

pub const RELEASE_FEED: &str = "https://hiroute.ai/releases.json";
pub const MAX_RELEASE_FEED_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebsiteReleasesV2 {
    schema: String,
    releases: Vec<WebsiteReleaseV2>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WebsiteReleaseV2 {
    version: String,
    channel: String,
    published_at: String,
    notes: ReleaseNotes,
    artifacts: Vec<ReleaseArtifactV2>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseNotes {
    pub zh: String,
    pub en: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReleaseArtifactV2 {
    Desktop {
        platform: String,
        architecture: String,
        format: String,
        minimum_os: String,
        distribution: String,
        filename: String,
        sha256: String,
        size: u64,
    },
    Standalone {
        platform: String,
        architecture: String,
        target: String,
        format: String,
        distribution: String,
        filename: String,
        sha256: String,
        size: u64,
        manifest_filename: String,
        manifest_sha256: String,
        manifest_size: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopRelease {
    pub version: String,
    pub notes: ReleaseNotes,
    pub architecture: String,
    pub distribution: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}
impl DesktopRelease {
    pub fn download_url(&self) -> String {
        format!(
            "https://hiroute.ai/releases/{}/{}",
            self.version, self.filename
        )
    }
    pub fn revision_prefix(&self) -> Result<&str, &'static str> {
        let prefix = format!("HiRoute-{}-", self.version);
        let suffix = match self.distribution.as_str() {
            "self-signed" => "trial",
            "developer-id" => "developer-id",
            _ => return Err("UPGRADE_ARTIFACT_INVALID"),
        };
        let tail = format!("-macos-{}-{suffix}.dmg", self.architecture);
        let revision = self
            .filename
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(&tail))
            .ok_or("UPGRADE_ARTIFACT_INVALID")?;
        if semver::Version::parse(&self.version).is_err()
            || !matches!(self.architecture.as_str(), "arm64" | "x86_64")
            || !sized_digest(&self.filename, &self.sha256, self.size)
            || revision.len() != 12
            || !revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("UPGRADE_ARTIFACT_INVALID");
        }
        Ok(revision)
    }
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn filename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 240
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
fn sized_digest(name: &str, sha: &str, size: u64) -> bool {
    filename(name) && digest(sha) && size > 0 && size <= 2 * 1024 * 1024 * 1024
}
impl WebsiteReleasesV2 {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.is_empty() || bytes.len() > MAX_RELEASE_FEED_BYTES {
            return Err("UPGRADE_CATALOG_INVALID");
        }
        let result: Self = serde_json::from_slice(bytes).map_err(|_| "UPGRADE_CATALOG_INVALID")?;
        result.validate()?;
        Ok(result)
    }
    fn validate(&self) -> Result<(), &'static str> {
        let invalid = || "UPGRADE_CATALOG_INVALID";
        if self.schema != "hiroute.website.releases/v2" || self.releases.len() > 64 {
            return Err(invalid());
        }
        let mut versions = std::collections::BTreeSet::new();
        for release in &self.releases {
            let version = Version::parse(&release.version).map_err(|_| invalid())?;
            if !versions.insert(&release.version)
                || release.version.len() > 64
                || !matches!(release.channel.as_str(), "stable" | "preview")
                || (release.channel == "stable" && !version.pre.is_empty())
                || release.published_at.is_empty()
                || release.published_at.len() > 64
                || [&release.notes.zh, &release.notes.en]
                    .iter()
                    .any(|n| n.trim().is_empty() || n.len() > 64 * 1024)
                || release.artifacts.is_empty()
                || release.artifacts.len() > 16
            {
                return Err(invalid());
            }
            let mut names = std::collections::BTreeSet::new();
            for artifact in &release.artifacts {
                match artifact {
                    ReleaseArtifactV2::Desktop {
                        platform,
                        architecture,
                        format,
                        minimum_os,
                        distribution,
                        filename,
                        sha256,
                        size,
                    } => {
                        let suffix = match distribution.as_str() {
                            "self-signed" => "trial",
                            "developer-id" => "developer-id",
                            _ => return Err(invalid()),
                        };
                        let prefix = format!("HiRoute-{}-", release.version);
                        let tail = format!("-macos-{architecture}-{suffix}.dmg");
                        let revision = filename
                            .strip_prefix(&prefix)
                            .and_then(|s| s.strip_suffix(&tail))
                            .ok_or_else(invalid)?;
                        if platform != "macOS"
                            || !matches!(architecture.as_str(), "arm64" | "x86_64")
                            || format != "dmg"
                            || minimum_os != "15.0"
                            || !sized_digest(filename, sha256, *size)
                            || revision.len() != 12
                            || !revision
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                            || !names.insert(filename.as_str())
                        {
                            return Err(invalid());
                        }
                    }
                    ReleaseArtifactV2::Standalone {
                        platform,
                        architecture,
                        target,
                        format,
                        distribution,
                        filename,
                        sha256,
                        size,
                        manifest_filename,
                        manifest_sha256,
                        manifest_size,
                    } => {
                        let expected_target = match architecture.as_str() {
                            "x86_64" => "x86_64-unknown-linux-gnu",
                            "aarch64" => "aarch64-unknown-linux-gnu",
                            _ => return Err(invalid()),
                        };
                        let prefix = format!("hiroute-{}-", release.version);
                        let tail = format!("-{target}.tar.gz");
                        let revision = filename
                            .strip_prefix(&prefix)
                            .and_then(|s| s.strip_suffix(&tail))
                            .ok_or_else(invalid)?;
                        if platform != "Linux"
                            || target != expected_target
                            || format != "tar.gz"
                            || distribution != "unsigned"
                            || !sized_digest(filename, sha256, *size)
                            || !sized_digest(manifest_filename, manifest_sha256, *manifest_size)
                            || manifest_filename != &format!("{filename}.json")
                            || revision.len() != 12
                            || !revision
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                            || !names.insert(filename.as_str())
                            || !names.insert(manifest_filename.as_str())
                        {
                            return Err(invalid());
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub fn desktop_update(
        &self,
        current: &str,
        architecture: &str,
    ) -> Result<Option<DesktopRelease>, &'static str> {
        let current = Version::parse(current).map_err(|_| "UPGRADE_VERSION_INVALID")?;
        let mut selected: Option<(Version, DesktopRelease)> = None;
        for release in self.releases.iter().filter(|r| r.channel == "stable") {
            let version =
                Version::parse(&release.version).map_err(|_| "UPGRADE_CATALOG_INVALID")?;
            if version <= current || selected.as_ref().is_some_and(|(v, _)| v >= &version) {
                continue;
            }
            let matches = release
                .artifacts
                .iter()
                .filter_map(|a| match a {
                    ReleaseArtifactV2::Desktop {
                        architecture: arch,
                        distribution,
                        filename,
                        sha256,
                        size,
                        ..
                    } if arch == architecture => Some(DesktopRelease {
                        version: release.version.clone(),
                        notes: release.notes.clone(),
                        architecture: arch.clone(),
                        distribution: distribution.clone(),
                        filename: filename.clone(),
                        sha256: sha256.clone(),
                        size: *size,
                    }),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if matches.len() > 1 {
                return Err("UPGRADE_ARTIFACT_AMBIGUOUS");
            }
            if let Some(artifact) = matches.into_iter().next() {
                selected = Some((version, artifact));
            }
        }
        Ok(selected.map(|(_, release)| release))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_website_catalog_is_the_upgrade_reader_contract() {
        let bytes = include_bytes!("../../../apps/website/data/releases.json");
        let catalog = WebsiteReleasesV2::parse(bytes).unwrap();
        let original: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        for architecture in ["arm64", "x86_64"] {
            let update = catalog
                .desktop_update("0.0.1", architecture)
                .unwrap()
                .unwrap();
            let release = original["releases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["version"] == update.version && r["channel"] == "stable")
                .unwrap();
            let artifact = release["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|a| a["kind"] == "desktop" && a["architecture"] == architecture)
                .unwrap();
            assert_eq!(update.filename, artifact["filename"].as_str().unwrap());
            assert_eq!(update.sha256, artifact["sha256"].as_str().unwrap());
            assert_eq!(update.size, artifact["size"].as_u64().unwrap());
            assert_eq!(update.architecture, architecture);
            assert_eq!(
                update.download_url(),
                format!(
                    "https://hiroute.ai/releases/{}/{}",
                    release["version"].as_str().unwrap(),
                    artifact["filename"].as_str().unwrap()
                )
            );
            assert!(update.revision_prefix().is_ok());
            assert!(
                catalog
                    .desktop_update(&update.version, architecture)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            catalog
                .desktop_update("0.0.1", "riscv64")
                .unwrap()
                .is_none()
        );
        for (field, bad) in [
            ("filename", "../other.dmg"),
            ("sha256", "unverified"),
            ("platform", "Windows"),
            ("distribution", "unsigned"),
        ] {
            let mut broken = original.clone();
            let desktop = broken["releases"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .flat_map(|r| r["artifacts"].as_array_mut().unwrap())
                .find(|a| a["kind"] == "desktop")
                .unwrap();
            desktop[field] = bad.into();
            assert!(
                WebsiteReleasesV2::parse(&serde_json::to_vec(&broken).unwrap()).is_err(),
                "{field}"
            );
        }
        let mut broken = original;
        broken["download_url"] = "https://other.invalid".into();
        assert!(WebsiteReleasesV2::parse(&serde_json::to_vec(&broken).unwrap()).is_err());
    }

    #[test]
    fn update_selection_uses_latest_stable_version_for_each_architecture() {
        let release = |version: &str, channel: &str, architecture: &str| {
            serde_json::json!({
                "version": version, "channel": channel, "published_at": "2026-10-09",
                "notes": {"zh": "版本说明", "en": "Release notes"},
                "artifacts": [{
                    "kind": "desktop", "platform": "macOS", "architecture": architecture,
                    "format": "dmg", "minimum_os": "15.0", "distribution": "self-signed",
                    "filename": format!("HiRoute-{version}-0123456789ab-macos-{architecture}-trial.dmg"),
                    "sha256": "a".repeat(64), "size": 1024
                }]
            })
        };
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema": "hiroute.website.releases/v2",
            "releases": [
                release("0.10.0", "stable", "arm64"),
                release("1.0.0-rc.1", "preview", "arm64"),
                release("0.11.0", "stable", "x86_64"),
                release("0.2.0", "stable", "arm64")
            ]
        }))
        .unwrap();
        let catalog = WebsiteReleasesV2::parse(&bytes).unwrap();
        for (architecture, expected) in [("arm64", "0.10.0"), ("x86_64", "0.11.0")] {
            let update = catalog
                .desktop_update("0.1.0", architecture)
                .unwrap()
                .unwrap();
            assert_eq!(update.version, expected);
            assert!(
                catalog
                    .desktop_update(expected, architecture)
                    .unwrap()
                    .is_none()
            );
            assert!(
                catalog
                    .desktop_update("2.0.0", architecture)
                    .unwrap()
                    .is_none()
            );
        }
    }
}
