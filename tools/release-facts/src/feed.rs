//! Deterministic feed projection from current source, with recorded-reader compatibility checks.
use hiroute_domain::{CanonicalDigest, OfficialModelSnapshotV1};
use hiroute_integrations::{TrustedReleaseCatalog, model_feed::*};
use serde::Deserialize;
use std::{path::Path, process::Command};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    minimum_feed_client: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publication {
    sequence: u64,
    issued_at: String,
}
#[derive(Deserialize)]
struct ReleaseIndex {
    schema: String,
    releases: Vec<Release>,
}
#[derive(Deserialize)]
struct Release {
    channel: String,
    file: String,
    revision: String,
    sha256: String,
    tag: String,
}

pub struct SupportedFeedClient {
    pub version: String,
    pub catalog: TrustedReleaseCatalog,
}

pub fn build(root: &Path, check: bool) -> Result<(), String> {
    build_against(root, check, None)
}

pub fn build_against(root: &Path, check: bool, base: Option<&str>) -> Result<(), String> {
    let read =
        |path: &str| std::fs::read(root.join(path)).map_err(|_| format!("cannot read {path}"));
    let policy: Policy = serde_json::from_slice(&read("assets/model-feed/policy.json")?)
        .map_err(|_| "invalid feed policy")?;
    let minimum = semver::Version::parse(&policy.minimum_feed_client)
        .map_err(|_| "invalid minimum feed client")?;
    let publication: Publication =
        serde_json::from_slice(&read("assets/model-feed/publication.json")?)
            .map_err(|_| "invalid feed publication")?;
    let featured: Vec<FeaturedModel> =
        serde_json::from_slice(&read("assets/model-feed/featured.json")?)
            .map_err(|_| "invalid featured entries")?;
    validate_publication_fields(
        publication.sequence,
        &publication.issued_at,
        &"0".repeat(40),
        &featured,
    )
    .map_err(|e| e.to_string())?;
    let index: ReleaseIndex = serde_json::from_slice(&read("contracts/releases/index.v1.json")?)
        .map_err(|_| "invalid release index")?;
    if index.schema != "hiroute.release-contract-index/v1" {
        return Err("unsupported release index".into());
    }
    let current = load_catalog(
        |file| read(&format!("assets/release-facts/current/bundle/{file}")),
        true,
    )?;
    // Current development source is sufficient publication authority. Package validation
    // and a recorded stable release are not prerequisites; content checks still apply.
    let candidate = FeedPayload::from_snapshot(
        current.official_snapshot(),
        publication.sequence,
        publication.issued_at.clone(),
        "0".repeat(40),
        featured.clone(),
    );
    candidate.validate().map_err(|e| e.to_string())?;
    let candidate_bytes =
        super::canonical_sorted_json_line(&candidate).map_err(|e| e.to_string())?;
    parse_feed(&candidate_bytes).map_err(|e| e.to_string())?;
    validate_embedded_history(root, &current.official_snapshot(), base)?;
    let mut clients = Vec::new();
    for release in index.releases {
        if release.channel != "stable" {
            continue;
        }
        let version = semver::Version::parse(release.tag.trim_start_matches('v'))
            .map_err(|_| "invalid recorded release version")?;
        // The current support policy retains all stable sources from 0.2.0. The feed-capable
        // floor is stricter; do not use latest_stable to retire earlier compatible clients.
        if version < minimum || version < semver::Version::new(0, 2, 0) {
            continue;
        }
        if release.revision.len() != 40
            || !release.revision.bytes().all(|b| b.is_ascii_hexdigit())
            || release.file != format!("{}.json", release.tag)
        {
            return Err("invalid recorded release identity".into());
        }
        let resolved = git(root, &["rev-parse", &format!("{}^{{commit}}", release.tag)])?;
        if resolved.trim() != release.revision {
            return Err(format!("recorded release tag moved: {}", release.tag));
        }
        let reader_path = "crates/integrations/src/model_feed/mod.rs";
        if git(
            root,
            &[
                "ls-tree",
                "--name-only",
                &release.revision,
                "--",
                reader_path,
            ],
        )?
        .trim()
        .is_empty()
        {
            continue;
        }
        let reader = git_bytes(
            root,
            &["show", &format!("{}:{reader_path}", release.revision)],
        )?;
        if !String::from_utf8_lossy(&reader).contains("hiroute.model-feed/v2") {
            continue;
        }
        let snapshot = read(&format!("contracts/releases/{}", release.file))?;
        if CanonicalDigest::of_bytes(&snapshot).as_str() != format!("sha256:{}", release.sha256) {
            return Err("release snapshot digest mismatch".into());
        }
        let record: serde_json::Value =
            serde_json::from_slice(&snapshot).map_err(|_| "invalid release snapshot")?;
        if record["release"]["revision"] != release.revision
            || record["release"]["tag"] != release.tag
        {
            return Err("release snapshot identity mismatch".into());
        }
        let catalog = load_catalog(
            |file| {
                git_bytes(
                    root,
                    &[
                        "show",
                        &format!(
                            "{}:assets/release-facts/current/bundle/{file}",
                            release.revision
                        ),
                    ],
                )
            },
            true,
        )?;
        clients.push(SupportedFeedClient {
            version: version.to_string(),
            catalog,
        });
    }
    clients
        .sort_by_key(|client| semver::Version::parse(&client.version).expect("validated release"));
    let previous: Option<FeedPayload> = serde_json::from_slice(&git_bytes(
        root,
        &["show", "HEAD:assets/model-feed/payload.json"],
    )?)
    .map_err(|_| "invalid previous payload")?;
    if let Some(previous) = &previous {
        previous.validate().map_err(|e| e.to_string())?;
    }
    // A committed bundle tree survives a squash merge while the PR commit ID does not.
    let revision = git(
        root,
        &["rev-parse", "HEAD:assets/release-facts/current/bundle"],
    )?;
    let revision = revision.trim();
    if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("metadata source tree is unavailable".into());
    }
    let unchanged = Command::new("git")
        .current_dir(root)
        .args([
            "diff",
            "--quiet",
            "HEAD",
            "--",
            "assets/model-data/current/inputs/metadata-catalog.json",
            "assets/release-facts/current/bundle",
        ])
        .status()
        .map_err(|_| "cannot compare metadata source")?;
    if !unchanged.success() {
        return Err(
            "commit current metadata and ReleaseFacts before generating a publication".into(),
        );
    }
    let payload = project(
        &current,
        &clients,
        publication.sequence,
        &publication.issued_at,
        revision,
        featured,
        previous.as_ref(),
    )?;
    if let Some(base) = base {
        let reference = git(
            root,
            &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
        )?;
        let reference = reference.trim();
        let exists = git(
            root,
            &[
                "ls-tree",
                "--name-only",
                reference,
                "--",
                "assets/model-feed/payload.json",
            ],
        )?;
        if !exists.trim().is_empty() {
            let baseline: Option<FeedPayload> = serde_json::from_slice(&git_bytes(
                root,
                &[
                    "show",
                    &format!("{reference}:assets/model-feed/payload.json"),
                ],
            )?)
            .map_err(|_| "invalid baseline payload")?;
            if let Some(baseline) = baseline {
                validate_transition(&baseline, &payload)?;
            }
        }
    }
    let bytes = super::canonical_sorted_json_line(&payload).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_MODEL_FEED_BYTES {
        return Err("generated payload cannot fit the bounded envelope".into());
    }
    let output = root.join("assets/model-feed/payload.json");
    if check {
        if std::fs::read(&output).map_err(|_| "missing generated payload")? != bytes {
            return Err("model feed payload is stale; run feed build".into());
        }
    } else {
        std::fs::write(output, &bytes).map_err(|_| "cannot write feed payload")?;
    }
    println!(
        "model feed: {}",
        if clients.is_empty() {
            "current development source (no recorded feed-capable release required)"
        } else {
            "validated against recorded clients"
        }
    );
    Ok(())
}

fn validate_transition(previous: &FeedPayload, next: &FeedPayload) -> Result<(), String> {
    previous.validate().map_err(|e| e.to_string())?;
    if next.sequence < previous.sequence || (next.sequence == previous.sequence && next != previous)
    {
        return Err("changed publication requires a higher sequence".into());
    }
    validate_data_identity(
        previous.data_revision,
        &previous.data_digest,
        next.data_revision,
        &next.data_digest,
    )?;
    for entry in &next.featured {
        if previous
            .featured
            .iter()
            .find(|old| old.id == entry.id)
            .is_some_and(|old| old.included_on != entry.included_on)
        {
            return Err("first inclusion date changed".into());
        }
    }
    Ok(())
}

fn validate_data_identity(
    previous_revision: u64,
    previous_digest: &CanonicalDigest,
    revision: u64,
    digest: &CanonicalDigest,
) -> Result<(), String> {
    if revision < previous_revision || (revision == previous_revision && digest != previous_digest)
    {
        return Err("changed semantic data requires a greater data revision".into());
    }
    Ok(())
}

fn validate_embedded_history(
    root: &Path,
    current: &OfficialModelSnapshotV1,
    base: Option<&str>,
) -> Result<(), String> {
    const FILE: &str = "assets/release-facts/current/bundle/official-model-data.json";
    let mut references = vec!["HEAD".to_owned()];
    if let Some(base) = base {
        references.push(
            git(
                root,
                &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
            )?
            .trim()
            .into(),
        );
    }
    // Installed readers retain an embedded data floor even before the first website
    // publication. Check both local regeneration and committed PR/push changes.
    for reference in references {
        if git(root, &["ls-tree", "--name-only", &reference, "--", FILE])?
            .trim()
            .is_empty()
        {
            continue;
        }
        let bytes = git_bytes(root, &["show", &format!("{reference}:{FILE}")])?;
        let previous: OfficialModelSnapshotV1 =
            serde_json::from_slice(&bytes).map_err(|_| "invalid previous embedded snapshot")?;
        previous
            .validate()
            .map_err(|_| "invalid previous embedded snapshot")?;
        validate_data_identity(
            previous.data_revision,
            &previous.data_digest,
            current.data_revision,
            &current.data_digest,
        )?;
    }
    Ok(())
}

fn load_catalog(
    mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
    require_complete: bool,
) -> Result<TrustedReleaseCatalog, String> {
    let catalog = TrustedReleaseCatalog::load_release_facts(
        &read("manifest.json")?,
        &read("connector-registry.json")?,
        &read("model-data.json")?,
    )
    .map_err(|e| format!("invalid release catalog: {e}"))?;
    match read("official-model-data.json") {
        Ok(bytes) => catalog
            .with_embedded_official_data(
                serde_json::from_slice(&bytes).map_err(|_| "invalid official model snapshot")?,
            )
            .map_err(|e| e.to_string()),
        Err(error) if require_complete => Err(error),
        Err(_) => Ok(catalog),
    }
}
fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|_| "Git source read failed")?;
    if !output.status.success() {
        return Err("recorded Git source is unavailable; fetch release tags and history".into());
    }
    Ok(output.stdout)
}
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(git_bytes(root, args)?).map_err(|_| "non-text Git source".into())
}

pub fn project(
    current: &TrustedReleaseCatalog,
    clients: &[SupportedFeedClient],
    sequence: u64,
    issued_at: &str,
    source_revision: &str,
    featured: Vec<FeaturedModel>,
    previous: Option<&FeedPayload>,
) -> Result<FeedPayload, String> {
    let payload = FeedPayload::from_snapshot(
        current.official_snapshot(),
        sequence,
        issued_at.into(),
        source_revision.into(),
        featured,
    );
    payload.validate().map_err(|e| e.to_string())?;
    if let Some(previous) = previous {
        validate_transition(previous, &payload)?;
    }
    for client in clients {
        // Unsupported external execution references remain descriptive, never omitted.
        admit(&client.catalog, &payload, &client.version).map_err(|e| e.to_string())?;
    }
    Ok(payload)
}

#[cfg(test)]
mod tests;
