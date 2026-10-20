//! Deterministic feed projection from current ReleaseFacts and recorded supported releases.
use hiroute_domain::CanonicalDigest;
use hiroute_integrations::{TrustedReleaseCatalog, model_feed::*};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

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
    // Bootstrap still validates maintained editorial/publication inputs with the client schema.
    FeedPayload {
        schema: MODEL_FEED_SCHEMA.into(),
        sequence: publication.sequence,
        issued_at: publication.issued_at.clone(),
        source_revision: "0".repeat(40),
        records: vec![],
        featured: featured.clone(),
    }
    .validate()
    .map_err(|e| e.to_string())?;
    let index: ReleaseIndex = serde_json::from_slice(&read("contracts/releases/index.v1.json")?)
        .map_err(|_| "invalid release index")?;
    if index.schema != "hiroute.release-contract-index/v1" {
        return Err("unsupported release index".into());
    }
    let current =
        load_catalog(|file| read(&format!("assets/release-facts/current/bundle/{file}")))?;
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
        let resolved = git(root, &["rev-parse", &format!("{}^{{commit}}", release.tag)])?;
        if resolved.trim() != release.revision {
            return Err(format!("recorded release tag moved: {}", release.tag));
        }
        let catalog = load_catalog(|file| {
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
        })?;
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
    let bytes = if clients.is_empty() {
        if previous.is_some() {
            return Err("cannot replace an existing publication with bootstrap".into());
        }
        b"null\n".to_vec()
    } else {
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
        if let Some(previous) = &previous {
            validate_transition(previous, &payload)?;
        }
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
        if bytes.len().div_ceil(3) * 4 + 1024 > MAX_MODEL_FEED_BYTES {
            return Err("generated payload cannot fit the bounded envelope".into());
        }
        bytes
    };
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
            "bootstrap (no feed-capable release)"
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
    for record in &next.records {
        if let Some(old) = previous
            .records
            .iter()
            .find(|r| r.kind() == record.kind() && r.id() == record.id())
            && old.included_on() != record.included_on()
        {
            return Err(format!(
                "first inclusion date changed for {} {}",
                record.kind(),
                record.id()
            ));
        }
    }
    Ok(())
}

fn load_catalog(
    mut read: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<TrustedReleaseCatalog, String> {
    TrustedReleaseCatalog::load_release_facts(
        &read("manifest.json")?,
        &read("connector-registry.json")?,
        &read("model-data.json")?,
    )
    .map_err(|e| format!("invalid release catalog: {e}"))
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
    let earliest = clients
        .first()
        .ok_or("no supported clients")?
        .version
        .clone();
    let included = issued_at.get(..10).ok_or("invalid issue date")?;
    let mut payload = FeedPayload {
        schema: MODEL_FEED_SCHEMA.into(),
        sequence,
        issued_at: issued_at.into(),
        source_revision: source_revision.into(),
        records: vec![],
        featured,
    };
    let date = |kind: &str, id: &str| {
        previous
            .and_then(|p| p.records.iter().find(|r| r.kind() == kind && r.id() == id))
            .map_or(included, FeedRecord::included_on)
            .to_owned()
    };
    let ids = current
        .model_data()
        .models
        .iter()
        .filter(|model| {
            clients.iter().any(|client| {
                client
                    .catalog
                    .model_data()
                    .model(&model.model_configuration_id)
                    .is_none()
            })
        })
        .map(|model| model.model_configuration_id.as_str())
        .collect::<BTreeSet<_>>();
    for model in &current.model_data().models {
        if !ids.contains(model.model_configuration_id.as_str()) {
            continue;
        }
        let upstream = current
            .model_data()
            .model_endpoint_capabilities
            .iter()
            .filter(|cap| cap.model_configuration_id == model.model_configuration_id)
            .map(|cap| cap.upstream_model_id.as_str())
            .collect::<BTreeSet<_>>();
        let canonical = current
            .model_metadata()
            .canonical_models
            .iter()
            .find(|canonical| {
                canonical
                    .upstream_ids
                    .iter()
                    .any(|id| upstream.contains(id.as_str()))
            })
            .ok_or_else(|| {
                format!(
                    "model has no canonical metadata: {}",
                    model.model_configuration_id
                )
            })?;
        payload.records.push(FeedRecord::Model {
            min_client_version: earliest.clone(),
            included_on: date("model", &model.model_configuration_id),
            record: Box::new(FeedModel {
                definition: model.clone(),
                native_reasoning: current
                    .native_reasoning()
                    .iter()
                    .find(|native| native.model_configuration_id == model.model_configuration_id)
                    .cloned(),
                canonical: canonical.clone(),
            }),
        });
    }
    for record in &current.model_data().model_endpoint_capabilities {
        if ids.contains(record.model_configuration_id.as_str()) {
            payload.records.push(FeedRecord::ModelEndpointCapability {
                min_client_version: earliest.clone(),
                included_on: date("model_endpoint_capability", &record.capability_id),
                record: record.clone(),
            });
        }
    }
    for record in &current.model_data().offers {
        if record.billing_class != hiroute_domain::BillingClass::Free
            && record
                .model_configuration_ids
                .iter()
                .any(|id| ids.contains(id.as_str()))
        {
            payload.records.push(FeedRecord::Offer {
                min_client_version: earliest.clone(),
                included_on: date("offer", &record.offer_id),
                record: record.clone(),
            });
        }
    }
    for record in &current.model_data().price_rates {
        if ids.contains(record.model_configuration_id.as_str())
            && payload
                .records
                .iter()
                .any(|r| r.kind() == "offer" && r.id() == record.offer_ref)
        {
            payload.records.push(FeedRecord::PriceRate {
                min_client_version: earliest.clone(),
                included_on: date("price_rate", &record.price_rate_id),
                record: record.clone(),
            });
        }
    }
    // A model must have a legal capability + paid/subscription offer on at least one client.
    // Registry/adapters are never filled in by the feed to force admission.
    let mut executable_minimums: BTreeMap<String, String> = BTreeMap::new();
    let mut minimums: BTreeMap<(String, String), String> = BTreeMap::new();
    for client in clients {
        let admission = admit(&client.catalog, &payload, &client.version)
            .map_err(|e| format!("feed admission failed for {}: {e}", client.version))?;
        let effective = client
            .catalog
            .with_feed(&admission)
            .map_err(|e| e.to_string())?;
        for id in &ids {
            let executable = effective
                .model_data()
                .model_endpoint_capabilities
                .iter()
                .any(|cap| {
                    cap.model_configuration_id == *id
                        && effective.model_data().offers.iter().any(|offer| {
                            offer.endpoint_profile_id == cap.endpoint_profile_id
                                && offer.endpoint_profile_revision == cap.endpoint_profile_revision
                                && offer.billing_class != hiroute_domain::BillingClass::Free
                                && offer
                                    .model_configuration_ids
                                    .iter()
                                    .any(|member| member.as_str() == *id)
                        })
                });
            if executable && client.catalog.model_data().model(id).is_none() {
                executable_minimums
                    .entry((*id).to_owned())
                    .or_insert_with(|| client.version.clone());
            }
        }
        for record in &admission.records {
            if record.outcome == RecordAdmission::Applied {
                minimums
                    .entry((record.kind.clone(), record.id.clone()))
                    .or_insert_with(|| client.version.clone());
            }
        }
    }
    let eligible = executable_minimums.keys().cloned().collect::<BTreeSet<_>>();
    for id in &ids {
        if !eligible.contains(*id) {
            println!("model feed: {id} waits for a compatible supported release");
        }
    }
    for (id, minimum) in executable_minimums {
        minimums.insert(("model".into(), id), minimum);
    }
    // Dependent rows advertise the same usable floor as their model/offer, even when
    // their own scalar facts could be parsed by an earlier registry.
    for record in &payload.records {
        let key = (record.kind().to_owned(), record.id().to_owned());
        let mut dependencies = Vec::new();
        if let Some(id) = record.model_id() {
            dependencies.push(("model".to_owned(), id.to_owned()));
        }
        if let FeedRecord::PriceRate { record, .. } = record {
            dependencies.push(("offer".to_owned(), record.offer_ref.clone()));
        }
        let required = dependencies
            .iter()
            .filter_map(|key| minimums.get(key))
            .filter_map(|v| semver::Version::parse(v).ok())
            .max();
        if let (Some(minimum), Some(required)) = (minimums.get_mut(&key), required)
            && semver::Version::parse(minimum).map_err(|_| "invalid client minimum")? < required
        {
            *minimum = required.to_string();
        }
    }
    payload.records.retain_mut(|record| {
        let Some(minimum) = minimums.get(&(record.kind().into(), record.id().into())) else {
            return false;
        };
        if record.model_id().is_some_and(|id| !eligible.contains(id)) {
            return false;
        }
        match record {
            FeedRecord::Model {
                min_client_version, ..
            }
            | FeedRecord::ModelEndpointCapability {
                min_client_version, ..
            }
            | FeedRecord::PriceRate {
                min_client_version, ..
            } => *min_client_version = minimum.clone(),
            FeedRecord::Offer {
                min_client_version,
                record,
                ..
            } => {
                *min_client_version = minimum.clone();
                if !record
                    .model_configuration_ids
                    .iter()
                    .any(|id| eligible.contains(id))
                {
                    return false;
                }
            }
        }
        true
    });
    payload
        .records
        .sort_by(|a, b| (a.kind(), a.id()).cmp(&(b.kind(), b.id())));
    payload.featured.sort_by(|a, b| a.id.cmp(&b.id));
    payload.validate().map_err(|e| e.to_string())?;
    for client in clients {
        let admission =
            admit(&client.catalog, &payload, &client.version).map_err(|e| e.to_string())?;
        if admission.counts.unresolved > 0 {
            return Err(format!(
                "unresolved feed record at declared minimum for {}",
                client.version
            ));
        }
    }
    Ok(payload)
}

#[cfg(test)]
mod tests;
