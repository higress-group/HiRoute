use std::path::PathBuf;

use hiroute_domain::{CanonicalDigest, CompiledAgentPlanV1};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use toml_edit::Item;

use super::filesystem_config::read_system_config_bytes;
use super::{
    AgentFilesystemScanError as Error, CodexConfigurationScope, CodexDefaultPolicy,
    sample_codex_configuration,
};

/// Raw selected-target metadata, not installation, account or access proof.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexConfiguredCatalog {
    pub metadata_source: CodexCatalogMetadataSourceV1,
    pub path: PathBuf,
    pub original: Value,
    pub content_digest: CanonicalDigest,
    pub context_digest: CanonicalDigest,
    pub dependency_digest: CanonicalDigest,
}

/// Safe projection of the native Codex model directory. It retains the exact client-facing
/// names while omitting the catalog's large prompts and opaque metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexCatalogSummaryV1 {
    pub metadata_source: CodexCatalogMetadataSourceV1,
    pub native_default_model: String,
    pub models: Vec<CodexCatalogModelSummaryV1>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexCatalogMetadataSourceV1 {
    UserConfigured,
    TargetCache,
    TargetBundled,
    HirouteGenerated,
}

/// Exact non-secret producer facts sealed between Preview and Apply. `path` is the source
/// artifact, never the HiRoute-owned merged output.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexCatalogProducerFactsV1 {
    pub metadata_source: CodexCatalogMetadataSourceV1,
    pub path: PathBuf,
    pub content_digest: CanonicalDigest,
    pub context_digest: CanonicalDigest,
    pub dependency_digest: CanonicalDigest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexCatalogModelSummaryV1 {
    pub client_model_id: String,
    pub display_name: String,
    /// Effective native reasoning profile: the selected scope override, otherwise this model's
    /// catalog default. It is metadata only and grants no source access.
    pub native_reasoning_profile: Option<String>,
}

/// Read the selected complete catalog and project only the native names needed by settings UI.
/// A higher-priority invalid/unreadable source fails closed. The target cache is considered only
/// when the effective scope has no explicit pointer; HiRoute's own bundled fixture is never a
/// production fallback.
pub fn sample_codex_catalog_summary(
    scope: &CodexConfigurationScope,
    explicit_model: Option<&str>,
) -> Result<CodexCatalogSummaryV1, super::CodexCatalogError> {
    let configured = sample_codex_catalog_source(scope).map_err(catalog_source_error)?;
    let metadata_source = configured.metadata_source;
    let original = configured.original;
    let selection = super::CodexCatalogSelection::for_current_adapter(original)?;
    let native_default_model = selection
        .effective_default(CodexDefaultPolicy {
            explicit_model,
            uses_codex_backend: true,
            allow_provider_model_fallback: false,
        })?
        .to_owned();
    let configured_reasoning = sample_codex_configuration(scope)
        .map_err(catalog_source_error)?
        .source_document
        .get("model_reasoning_effort")
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or(super::CodexCatalogError::InvalidCatalog)
        })
        .transpose()?;
    let models = selection.original()["models"]
        .as_array()
        .ok_or(super::CodexCatalogError::InvalidCatalog)?
        .iter()
        .map(|model| {
            Ok(CodexCatalogModelSummaryV1 {
                client_model_id: model["slug"]
                    .as_str()
                    .ok_or(super::CodexCatalogError::InvalidCatalog)?
                    .to_owned(),
                display_name: model["display_name"]
                    .as_str()
                    .ok_or(super::CodexCatalogError::InvalidCatalog)?
                    .to_owned(),
                native_reasoning_profile: configured_reasoning
                    .clone()
                    .or_else(|| model["default_reasoning_level"].as_str().map(str::to_owned)),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CodexCatalogSummaryV1 {
        metadata_source,
        native_default_model,
        models,
    })
}

pub fn sample_codex_configured_catalog(
    scope: &CodexConfigurationScope,
) -> Result<Option<CodexConfiguredCatalog>, Error> {
    let before = sample_codex_configuration(scope)?;
    let Some(path) = before.source_document.get("model_catalog_json") else {
        return Ok(None);
    };
    let path = absolute_catalog_path(path)?;
    read_catalog_source(
        scope,
        before,
        path,
        CodexCatalogMetadataSourceV1::UserConfigured,
        PointerRevalidation::Exact,
    )
    .map(Some)
}

/// Resolve the selected target's complete native catalog with strict source priority.
pub fn sample_codex_catalog_source(
    scope: &CodexConfigurationScope,
) -> Result<CodexConfiguredCatalog, Error> {
    let before = sample_codex_configuration(scope)?;
    if let Some(item) = before.source_document.get("model_catalog_json") {
        let path = absolute_catalog_path(item)?;
        return read_catalog_source(
            scope,
            before,
            path,
            CodexCatalogMetadataSourceV1::UserConfigured,
            PointerRevalidation::Exact,
        );
    }
    read_catalog_source(
        scope,
        before,
        scope.codex_home.join("models_cache.json"),
        CodexCatalogMetadataSourceV1::TargetCache,
        PointerRevalidation::Absent,
    )
}

pub fn revalidate_codex_configured_catalog(
    scope: &CodexConfigurationScope,
    expected: &CodexConfiguredCatalog,
) -> Result<(), Error> {
    if sample_codex_configured_catalog(scope)?.as_ref() != Some(expected) {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

pub fn revalidate_codex_catalog_source(
    scope: &CodexConfigurationScope,
    expected: &CodexConfiguredCatalog,
) -> Result<(), Error> {
    if &sample_codex_catalog_source(scope)? != expected {
        return Err(Error::SourceChanged);
    }
    Ok(())
}

/// The catalog a managed update rebases onto: never a previous HiRoute artifact, whose plan
/// entries would collide with the plans being appended now.
pub enum CodexCatalogBaseline {
    /// No managed edit has claimed the pointer; the scope's own configuration still carries
    /// the user's pointer state.
    Scope,
    /// The active managed edit replaced the user's pointer; the baseline is what its protected
    /// restore record preserved (None means the user had no catalog of their own).
    Original(Option<PathBuf>),
}

/// Interpret a previous managed configuration's protected restore record for catalog rebasing.
pub fn codex_catalog_baseline(
    record: &[u8],
) -> Result<CodexCatalogBaseline, super::CodexCatalogError> {
    if record.len() < 3 || record[0] != 1 || record[1] > 1 {
        return Err(super::CodexCatalogError::InvalidCatalog);
    }
    let restore = super::CodexNativeRestore::decode_protected(&record[2..])
        .map_err(|_| super::CodexCatalogError::InvalidCatalog)?;
    Ok(match restore.catalog_pointer_before() {
        None => CodexCatalogBaseline::Scope,
        Some(pointer) => CodexCatalogBaseline::Original(pointer.map(PathBuf::from)),
    })
}

fn read_catalog_value(bytes: &[u8]) -> Result<Value, Error> {
    let original: Value = serde_json::from_slice(bytes).map_err(|_| Error::InvalidConfig)?;
    if !original
        .get("models")
        .and_then(Value::as_array)
        .is_some_and(|models| !models.is_empty() && models.iter().all(Value::is_object))
    {
        return Err(Error::InvalidConfig);
    }
    Ok(original)
}

#[derive(Clone, Copy)]
enum PointerRevalidation {
    Exact,
    Absent,
    Ignore,
}

fn absolute_catalog_path(item: &Item) -> Result<PathBuf, Error> {
    let path = PathBuf::from(item.as_str().ok_or(Error::InvalidConfig)?);
    validate_catalog_path(&path)?;
    Ok(path)
}

fn validate_catalog_path(path: &std::path::Path) -> Result<(), Error> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(Error::InvalidConfig);
    }
    Ok(())
}

fn read_catalog_source(
    scope: &CodexConfigurationScope,
    before: super::CodexEffectiveConfiguration,
    path: PathBuf,
    metadata_source: CodexCatalogMetadataSourceV1,
    pointer: PointerRevalidation,
) -> Result<CodexConfiguredCatalog, Error> {
    validate_catalog_path(&path)?;
    // Catalog metadata can be public-readable; never chmod a target-owned source file.
    let (bytes, _) = read_system_config_bytes(&path)?.ok_or(Error::SourceUnavailable)?;
    let original = read_catalog_value(&bytes)?;
    let after = sample_codex_configuration(scope)?;
    let selected = after
        .source_document
        .get("model_catalog_json")
        .and_then(Item::as_str);
    let pointer_stable = match pointer {
        PointerRevalidation::Exact => selected == path.to_str(),
        PointerRevalidation::Absent => selected.is_none(),
        PointerRevalidation::Ignore => true,
    };
    if before.context_digest != after.context_digest
        || before.dependency_digest != after.dependency_digest
        || !pointer_stable
    {
        return Err(Error::SourceChanged);
    }
    let content_digest = CanonicalDigest::of_bytes(&bytes);
    let dependency_digest = CanonicalDigest::of(&(
        "codex-catalog-producer/v2",
        metadata_source,
        &before.context_digest,
        &before.dependency_digest,
        &path,
        &content_digest,
    ))
    .map_err(|_| Error::InvalidConfig)?;
    Ok(CodexConfiguredCatalog {
        metadata_source,
        path,
        original,
        content_digest,
        context_digest: before.context_digest,
        dependency_digest,
    })
}

fn catalog_source_error(_: Error) -> super::CodexCatalogError {
    super::CodexCatalogError::CapabilityUnproven
}

/// The merged catalog a plan-carrying Codex selection publishes: the exact selected-target
/// baseline plus one immutable entry per permitted plan. The selected client surface is not
/// version- or digest-gated here: this adapter checks only the complete catalog structure and the
/// semantics needed to preserve its default.
pub fn sample_codex_catalog_plan(
    scope: &CodexConfigurationScope,
    plans: &[hiroute_domain::CompiledAgentPlanV1],
    policy: CodexDefaultPolicy<'_>,
    baseline: &CodexCatalogBaseline,
    retained_models: Option<&std::collections::BTreeSet<String>>,
) -> Result<CodexCatalogPlan, super::CodexCatalogError> {
    use super::CodexCatalogSelection;
    let configured = match baseline {
        CodexCatalogBaseline::Original(Some(path)) => {
            let before = sample_codex_configuration(scope).map_err(catalog_source_error)?;
            read_catalog_source(
                scope,
                before,
                path.clone(),
                CodexCatalogMetadataSourceV1::UserConfigured,
                PointerRevalidation::Ignore,
            )
            .map_err(catalog_source_error)?
        }
        CodexCatalogBaseline::Original(None) => {
            let before = sample_codex_configuration(scope).map_err(catalog_source_error)?;
            read_catalog_source(
                scope,
                before,
                scope.codex_home.join("models_cache.json"),
                CodexCatalogMetadataSourceV1::TargetCache,
                PointerRevalidation::Ignore,
            )
            .map_err(catalog_source_error)?
        }
        CodexCatalogBaseline::Scope => {
            sample_codex_catalog_source(scope).map_err(catalog_source_error)?
        }
    };
    let producer = CodexCatalogProducerFactsV1 {
        metadata_source: configured.metadata_source,
        path: configured.path,
        content_digest: configured.content_digest,
        context_digest: configured.context_digest,
        dependency_digest: configured.dependency_digest,
    };
    let original = configured.original;
    let selection = CodexCatalogSelection::for_current_adapter(original)?;
    let merged =
        selection.append_plans(plans, policy, producer.metadata_source, retained_models)?;
    let bytes =
        serde_json::to_vec(&merged).map_err(|_| super::CodexCatalogError::InvalidCatalog)?;
    Ok(CodexCatalogPlan {
        selection: CodexCatalogSelection::for_current_adapter(merged)?,
        content_digest: CanonicalDigest::of_bytes(&bytes),
        producer,
    })
}

/// A HiRoute-only selection has no native models to preserve. Its exact client directory is
/// derived from the selected published plans, so a fresh CODEX_HOME needs no native cache or
/// account. The observed config scope still binds Preview to Apply.
pub fn sample_codex_hiroute_only_catalog_plan(
    scope: &CodexConfigurationScope,
    plans: &[CompiledAgentPlanV1],
    explicit_model: &str,
) -> Result<CodexCatalogPlan, super::CodexCatalogError> {
    if plans.is_empty() || plans.len() > i32::MAX as usize {
        return Err(super::CodexCatalogError::InvalidCatalog);
    }
    let models = plans
        .iter()
        .enumerate()
        .map(|(index, plan)| super::codex_catalog_plan::plan_entry(plan, index as i32))
        .collect::<Result<Vec<_>, _>>()?;
    let catalog = json!({"models": models});
    let selection = super::CodexCatalogSelection::for_current_adapter(catalog.clone())?;
    if !models
        .iter()
        .any(|model| model["slug"].as_str() == Some(explicit_model))
    {
        return Err(super::CodexCatalogError::MissingDefault);
    }
    let bytes =
        serde_json::to_vec(&catalog).map_err(|_| super::CodexCatalogError::InvalidCatalog)?;
    let content_digest = CanonicalDigest::of_bytes(&bytes);
    let observed = sample_codex_configuration(scope).map_err(catalog_source_error)?;
    let dependency_digest = CanonicalDigest::of(&(
        "hiroute-only-codex-catalog/v1",
        &scope.user_file,
        &observed.dependency_digest,
        &content_digest,
    ))
    .map_err(|_| super::CodexCatalogError::InvalidCatalog)?;
    Ok(CodexCatalogPlan {
        selection,
        content_digest: content_digest.clone(),
        producer: CodexCatalogProducerFactsV1 {
            metadata_source: CodexCatalogMetadataSourceV1::HirouteGenerated,
            path: scope.user_file.clone(),
            content_digest,
            context_digest: observed.context_digest,
            dependency_digest,
        },
    })
}

pub struct CodexCatalogPlan {
    pub selection: super::CodexCatalogSelection,
    pub content_digest: CanonicalDigest,
    pub producer: CodexCatalogProducerFactsV1,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn write(path: &std::path::Path, bytes: &[u8], mode: u32) {
        std::fs::write(path, bytes).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, CodexConfigurationScope, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let catalog = root.path().join("catalog.json");
        write(
            &catalog,
            include_bytes!("codex_bundled_catalog.json"),
            0o644,
        );
        let config = root.path().join("config.toml");
        write(
            &config,
            format!("model_catalog_json = '{}'\n", catalog.display()).as_bytes(),
            0o600,
        );
        (root, CodexConfigurationScope::user_file(config), catalog)
    }

    fn cache_fixture() -> (tempfile::TempDir, CodexConfigurationScope, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("models_cache.json");
        write(&cache, include_bytes!("codex_bundled_catalog.json"), 0o644);
        let config = root.path().join("config.toml");
        write(&config, b"model = 'gpt-5.4'\n", 0o600);
        (root, CodexConfigurationScope::user_file(config), cache)
    }

    #[test]
    fn configured_catalog_preserves_full_values_without_writes() {
        let (_root, scope, path) = fixture();
        let original = std::fs::read(&path).unwrap();
        let sampled = sample_codex_configured_catalog(&scope).unwrap().unwrap();
        assert_eq!(
            sampled.original,
            serde_json::from_slice::<Value>(&original).unwrap()
        );
        assert_eq!(sampled.content_digest, CanonicalDigest::of_bytes(&original));
        revalidate_codex_configured_catalog(&scope, &sampled).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn summary_keeps_native_names_and_distinguishes_metadata_origin() {
        let (_root, scope, _path) = fixture();
        let configured = sample_codex_catalog_summary(&scope, None).unwrap();
        assert_eq!(
            configured.metadata_source,
            CodexCatalogMetadataSourceV1::UserConfigured
        );
        assert!(!configured.models.is_empty());
        assert!(configured.models.iter().any(|model| {
            model.client_model_id == configured.native_default_model
                && !model.display_name.is_empty()
        }));

        let (_root, cache_scope, _path) = cache_fixture();
        let cached = sample_codex_catalog_summary(&cache_scope, None).unwrap();
        assert_eq!(
            cached.metadata_source,
            CodexCatalogMetadataSourceV1::TargetCache
        );
        assert!(!cached.models.is_empty());
    }

    #[test]
    fn no_configured_catalog_does_not_invent_bundled_source() {
        let root = tempfile::tempdir().unwrap();
        let scope = CodexConfigurationScope::user_file(root.path().join("missing.toml"));
        assert!(sample_codex_configured_catalog(&scope).unwrap().is_none());
        assert!(matches!(
            sample_codex_catalog_source(&scope),
            Err(Error::SourceUnavailable)
        ));
    }

    #[test]
    fn target_cache_is_bound_to_the_selected_codex_home() {
        let (selected_root, selected_scope, selected_cache) = cache_fixture();
        let other_root = tempfile::tempdir().unwrap();
        write(
            &other_root.path().join("models_cache.json"),
            include_bytes!("codex_bundled_catalog.json"),
            0o644,
        );
        let sampled = sample_codex_catalog_source(&selected_scope).unwrap();
        assert_eq!(
            sampled.metadata_source,
            CodexCatalogMetadataSourceV1::TargetCache
        );
        assert_eq!(sampled.path, selected_cache);
        assert!(sampled.path.starts_with(selected_root.path()));
        assert!(!sampled.path.starts_with(other_root.path()));
    }

    #[test]
    fn invalid_or_missing_target_cache_never_uses_hiroute_bundled_fixture() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("config.toml");
        write(&config, b"model = 'gpt-5.4'\n", 0o600);
        let scope = CodexConfigurationScope::user_file(config);
        assert!(matches!(
            sample_codex_catalog_source(&scope),
            Err(Error::SourceUnavailable)
        ));
        write(
            &root.path().join("models_cache.json"),
            br#"{"models":[]}"#,
            0o644,
        );
        assert!(matches!(
            sample_codex_catalog_source(&scope),
            Err(Error::InvalidConfig)
        ));
    }

    #[test]
    fn invalid_explicit_catalog_does_not_fall_back_to_valid_target_cache() {
        let (root, scope, explicit) = fixture();
        write(
            &root.path().join("models_cache.json"),
            include_bytes!("codex_bundled_catalog.json"),
            0o644,
        );
        std::fs::remove_file(explicit).unwrap();
        assert!(matches!(
            sample_codex_catalog_source(&scope),
            Err(Error::SourceUnavailable)
        ));
    }

    #[test]
    fn missing_selected_file_is_not_fallback_eligible() {
        let (_root, scope, path) = fixture();
        std::fs::remove_file(path).unwrap();
        assert!(matches!(
            sample_codex_configured_catalog(&scope),
            Err(Error::SourceUnavailable)
        ));
    }

    #[test]
    fn changed_catalog_rejects_old_observation() {
        let (_root, scope, path) = fixture();
        let sampled = sample_codex_configured_catalog(&scope).unwrap().unwrap();
        let mut changed = sampled.original.clone();
        changed["opaque"] = serde_json::json!({"new": true});
        write(&path, &serde_json::to_vec(&changed).unwrap(), 0o644);
        assert!(matches!(
            revalidate_codex_configured_catalog(&scope, &sampled),
            Err(Error::SourceChanged)
        ));
    }

    #[test]
    fn changed_scope_rejects_even_identical_catalog_bytes() {
        let (root, scope, _path) = fixture();
        let sampled = sample_codex_configured_catalog(&scope).unwrap().unwrap();
        let other = root.path().join("other.toml");
        std::fs::copy(&scope.user_file, &other).unwrap();
        let other = CodexConfigurationScope::user_file(other);
        assert!(matches!(
            revalidate_codex_configured_catalog(&other, &sampled),
            Err(Error::SourceChanged)
        ));
    }

    #[test]
    fn symlink_catalog_is_rejected_without_touching_target() {
        let (root, scope, path) = fixture();
        let target = root.path().join("target.json");
        std::fs::rename(&path, &target).unwrap();
        symlink(&target, &path).unwrap();
        assert!(sample_codex_configured_catalog(&scope).is_err());
        assert_eq!(
            std::fs::read(target).unwrap(),
            include_bytes!("codex_bundled_catalog.json")
        );
    }

    #[test]
    fn writable_by_others_catalog_is_rejected() {
        let (_root, scope, path) = fixture();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(matches!(
            sample_codex_configured_catalog(&scope),
            Err(Error::UnsafePermissions)
        ));
    }

    #[test]
    fn hardlinked_catalog_is_rejected() {
        let (root, scope, path) = fixture();
        std::fs::hard_link(path, root.path().join("alias.json")).unwrap();
        assert!(matches!(
            sample_codex_configured_catalog(&scope),
            Err(Error::UnsafePermissions)
        ));
    }

    #[test]
    fn oversized_catalog_is_rejected_before_parsing() {
        let (_root, scope, path) = fixture();
        write(&path, &vec![b' '; 1024 * 1024 + 1], 0o644);
        assert!(matches!(
            sample_codex_configured_catalog(&scope),
            Err(Error::ConfigTooLarge)
        ));
    }

    #[test]
    fn unresolved_relative_catalog_is_not_read_from_process_directory() {
        let (_root, scope, _path) = fixture();
        write(
            &scope.user_file,
            b"model_catalog_json = 'catalog.json'\n",
            0o600,
        );
        assert!(matches!(
            sample_codex_configured_catalog(&scope),
            Err(Error::InvalidConfig)
        ));
    }

    #[test]
    fn empty_catalog_is_not_fallback_eligible() {
        let (_root, scope, path) = fixture();
        write(&path, br#"{"models":[]}"#, 0o644);
        assert!(matches!(
            sample_codex_configured_catalog(&scope),
            Err(Error::InvalidConfig)
        ));
    }

    #[test]
    fn model_list_is_not_accepted_as_configured_catalog() {
        let (_root, scope, path) = fixture();
        write(&path, br#"{"data":[{"id":"model"}]}"#, 0o644);
        assert!(matches!(
            sample_codex_configured_catalog(&scope),
            Err(Error::InvalidConfig)
        ));
    }

    #[test]
    fn selected_profile_catalog_wins_without_reading_root_catalog() {
        let (_root, mut scope, path) = fixture();
        write(&scope.user_file, format!("model_catalog_json = '/missing-root.json'\n[profiles.work]\nmodel_catalog_json = '{}'\n", path.display()).as_bytes(), 0o600);
        scope.selection = super::super::CodexSelectionTarget::LegacyProfile("work".into());
        assert_eq!(
            sample_codex_configured_catalog(&scope)
                .unwrap()
                .unwrap()
                .path,
            path
        );
    }
}
