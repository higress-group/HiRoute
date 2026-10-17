//! Windows consumes V3; released macOS readers retain the V2 projection unchanged.
use crate::{MAX_RELEASE_FEED_BYTES, ReleaseNotes, WebsiteReleasesV2};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub const WINDOWS_RELEASE_FEED: &str = "https://hiroute.ai/releases-v3.json";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebsiteReleasesV3 {
    schema: String,
    releases: Vec<ReleaseV3>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseV3 {
    version: String,
    channel: String,
    published_at: String,
    notes: ReleaseNotes,
    artifacts: Vec<Value>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsDesktopRelease {
    pub version: String,
    pub notes: ReleaseNotes,
    pub architecture: String,
    pub distribution: String,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
}
impl WindowsDesktopRelease {
    pub fn revision_prefix(&self) -> Result<&str, &'static str> {
        let prefix = format!("HiRoute-{}-", self.version);
        let revision = self
            .filename
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix("-windows-x86_64-setup.exe"))
            .ok_or("UPGRADE_ARTIFACT_INVALID")?;
        if Version::parse(&self.version).is_err()
            || self.version.len() > 64
            || self.architecture != "x86_64"
            || self.distribution != "unsigned"
            || !facts(&self.filename, &self.sha256, self.size)
            || !revision_id(revision)
        {
            return Err("UPGRADE_ARTIFACT_INVALID");
        }
        Ok(revision)
    }
    pub fn download_url(&self) -> String {
        format!(
            "https://hiroute.ai/releases/{}/{}",
            self.version, self.filename
        )
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WindowsArtifact {
    Desktop {
        platform: String,
        architecture: String,
        format: String,
        distribution: String,
        filename: String,
        sha256: String,
        size: u64,
    },
}
fn facts(name: &str, sha: &str, size: u64) -> bool {
    !name.is_empty()
        && name.len() <= 240
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && sha.len() == 64
        && sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && size > 0
        && size <= 2 * 1024 * 1024 * 1024
}
fn revision_id(value: &str) -> bool {
    value.len() == 12
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl WebsiteReleasesV3 {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let invalid = || "UPGRADE_CATALOG_INVALID";
        if bytes.is_empty() || bytes.len() > MAX_RELEASE_FEED_BYTES {
            return Err(invalid());
        }
        let catalog: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        if catalog.schema != "hiroute.website.releases/v3" || catalog.releases.len() > 64 {
            return Err(invalid());
        }
        let mut versions = BTreeSet::new();
        for release in &catalog.releases {
            let version = Version::parse(&release.version).map_err(|_| invalid())?;
            if release.version.len() > 64
                || !versions.insert(&release.version)
                || !matches!(release.channel.as_str(), "stable" | "preview")
                || (release.channel == "stable" && !version.pre.is_empty())
                || release.published_at.is_empty()
                || release.published_at.len() > 64
                || [&release.notes.zh, &release.notes.en]
                    .iter()
                    .any(|s| s.trim().is_empty() || s.len() > 64 * 1024)
                || release.artifacts.is_empty()
                || release.artifacts.len() > 16
            {
                return Err(invalid());
            }
            let mut names = BTreeSet::new();
            for value in &release.artifacts {
                let filename = value["filename"].as_str().ok_or_else(invalid)?;
                if !names.insert(filename) {
                    return Err(invalid());
                }
                if let Some(name) = value.get("manifest_filename")
                    && !names.insert(name.as_str().ok_or_else(invalid)?)
                {
                    return Err(invalid());
                }
                if value["platform"] != "Windows" {
                    continue;
                }
                match serde_json::from_value::<WindowsArtifact>(value.clone())
                    .map_err(|_| invalid())?
                {
                    WindowsArtifact::Desktop {
                        platform,
                        architecture,
                        format,
                        distribution,
                        filename,
                        sha256,
                        size,
                    } => {
                        if platform != "Windows" || format != "nsis" {
                            return Err(invalid());
                        }
                        WindowsDesktopRelease {
                            version: release.version.clone(),
                            notes: release.notes.clone(),
                            architecture,
                            distribution,
                            filename,
                            sha256,
                            size,
                        }
                        .revision_prefix()
                        .map_err(|_| invalid())?;
                    }
                }
            }
        }
        WebsiteReleasesV2::parse(&catalog.v2_projection()?)?;
        Ok(catalog)
    }
    pub fn v2_projection(&self) -> Result<Vec<u8>, &'static str> {
        let releases: Vec<_> = self
            .releases
            .iter()
            .filter_map(|release| {
                let mut release = release.clone();
                release.artifacts.retain(|a| a["platform"] != "Windows");
                (!release.artifacts.is_empty()).then_some(release)
            })
            .collect();
        serde_json::to_vec(&json!({"schema":"hiroute.website.releases/v2","releases":releases}))
            .map_err(|_| "UPGRADE_CATALOG_INVALID")
    }
    pub fn windows_update(
        &self,
        current: &str,
    ) -> Result<Option<WindowsDesktopRelease>, &'static str> {
        let current = Version::parse(current).map_err(|_| "UPGRADE_VERSION_INVALID")?;
        let mut selected: Option<(Version, WindowsDesktopRelease)> = None;
        for release in &self.releases {
            let version =
                Version::parse(&release.version).map_err(|_| "UPGRADE_CATALOG_INVALID")?;
            if release.channel != "stable"
                || version <= current
                || selected.as_ref().is_some_and(|(v, _)| v >= &version)
            {
                continue;
            }
            let candidates: Vec<_> = release
                .artifacts
                .iter()
                .filter(|a| a["kind"] == "desktop" && a["platform"] == "Windows")
                .collect();
            if candidates.len() > 1 {
                return Err("UPGRADE_ARTIFACT_AMBIGUOUS");
            }
            if let Some(value) = candidates.first() {
                let artifact: WindowsArtifact = serde_json::from_value((*value).clone())
                    .map_err(|_| "UPGRADE_CATALOG_INVALID")?;
                let WindowsArtifact::Desktop {
                    architecture,
                    distribution,
                    filename,
                    sha256,
                    size,
                    ..
                } = artifact;
                selected = Some((
                    version,
                    WindowsDesktopRelease {
                        version: release.version.clone(),
                        notes: release.notes.clone(),
                        architecture,
                        distribution,
                        filename,
                        sha256,
                        size,
                    },
                ));
            }
        }
        Ok(selected.map(|(_, value)| value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Value {
        json!({"schema":"hiroute.website.releases/v3","releases":[{"version":"0.4.0","channel":"stable","published_at":"2026-10-01T00:00:00Z","notes":{"zh":"测试","en":"Test"},"artifacts":[{"kind":"desktop","platform":"Windows","architecture":"x86_64","format":"nsis","distribution":"unsigned","filename":"HiRoute-0.4.0-aaaaaaaaaaaa-windows-x86_64-setup.exe","sha256":"a".repeat(64),"size":100}]}]})
    }
    #[test]
    fn windows_selection_and_legacy_projection_remain_separate() {
        let catalog = WebsiteReleasesV3::parse(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
        assert!(WebsiteReleasesV2::parse(&catalog.v2_projection().unwrap()).is_ok());
        let update = catalog.windows_update("0.3.1").unwrap().unwrap();
        assert_eq!(update.revision_prefix().unwrap(), "aaaaaaaaaaaa");
        assert!(
            update
                .download_url()
                .starts_with("https://hiroute.ai/releases/0.4.0/")
        );
        assert!(catalog.windows_update("0.4.0").unwrap().is_none());
    }
    #[test]
    fn windows_standalone_archives_of_either_architecture_are_never_public() {
        for architecture in ["x86_64", "aarch64"] {
            let mut value = fixture();
            let target = format!("{architecture}-pc-windows-msvc");
            let name = format!("hiroute-0.4.0-aaaaaaaaaaaa-{target}.tar.gz");
            value["releases"][0]["artifacts"][0] = json!({"kind":"standalone","platform":"Windows",
                "architecture":architecture,"target":target,"format":"tar.gz","distribution":"unsigned",
                "filename":name,"sha256":"a".repeat(64),"size":100,
                "manifest_filename":format!("{name}.json"),"manifest_sha256":"b".repeat(64),"manifest_size":200});
            assert!(WebsiteReleasesV3::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }
    #[test]
    fn invalid_windows_artifacts_cannot_be_selected_or_smuggled_into_a_catalog() {
        for (field, value) in [
            ("architecture", json!("aarch64")),
            ("distribution", json!("self-signed")),
            ("format", json!("exe")),
            ("filename", json!("../setup.exe")),
            ("sha256", json!("bad")),
            ("size", json!(0)),
            ("minimum_os", json!("any")),
        ] {
            let mut fixture = fixture();
            fixture["releases"][0]["artifacts"][0][field] = value;
            assert!(
                WebsiteReleasesV3::parse(&serde_json::to_vec(&fixture).unwrap()).is_err(),
                "{field}"
            );
        }
    }
}
