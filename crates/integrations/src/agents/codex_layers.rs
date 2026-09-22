//! Explicit Codex launch scope. No recursive project discovery or inferred project trust.
use super::filesystem_config::{read_system_config_bytes, read_validated_config_bytes};
use super::{AgentFilesystemScanError as Error, CodexSelectionTarget, ConfigObservationV1};
use hiroute_domain::{CanonicalDigest, ConfigLayerV1};
use std::collections::BTreeMap;
use std::path::PathBuf;
use toml_edit::{DocumentMut, Item};
use zeroize::Zeroizing;

/// Populated by trusted backend launch-context selection, never by discovery guesses.
/// Project paths are ordered from outermost to innermost and must already be trusted.
pub struct CodexConfigurationScope {
    /// Exact native home for the selected Codex target. Catalog fallback is resolved only below
    /// this directory; it never consults the HiRoute process environment after scope creation.
    pub codex_home: PathBuf,
    pub user_file: PathBuf,
    pub system_file: Option<PathBuf>,
    pub trusted_project_files: Vec<PathBuf>,
    pub selection: CodexSelectionTarget,
    pub selected_profile_file: Option<PathBuf>,
    pub cli_overrides: Zeroizing<Vec<String>>,
}
impl CodexConfigurationScope {
    pub fn user_file(path: PathBuf) -> Self {
        let codex_home = path.parent().map(PathBuf::from).unwrap_or_default();
        Self {
            codex_home,
            user_file: path,
            system_file: None,
            trusted_project_files: Vec::new(),
            selection: CodexSelectionTarget::Root,
            selected_profile_file: None,
            cli_overrides: Zeroizing::new(Vec::new()),
        }
    }
}

/// Contains only the owned non-credential fields. Dependency identity covers every sampled byte,
/// selected path and launch override, including absence. It does not prove another launch scope.
pub struct CodexEffectiveConfiguration {
    pub observations: Vec<ConfigObservationV1>,
    pub dependency_digest: CanonicalDigest,
    pub writable_selection: bool,
    pub context_digest: CanonicalDigest,
    pub(super) source_document: DocumentMut,
}

pub fn sample_codex_configuration(
    scope: &CodexConfigurationScope,
) -> Result<CodexEffectiveConfiguration, Error> {
    if scope.trusted_project_files.len() > 32
        || scope.cli_overrides.len() > 64
        || (scope.selected_profile_file.is_some() && scope.selection != CodexSelectionTarget::Root)
        || !scope.codex_home.is_absolute()
        || scope.user_file.parent() != Some(scope.codex_home.as_path())
    {
        return Err(Error::InvalidConfig);
    }
    let mut dependencies = Vec::new();
    let mut source_document = DocumentMut::new();
    let mut fields = BTreeMap::<String, (ConfigLayerV1, String, CanonicalDigest)>::new();
    let mut paths = std::collections::BTreeSet::new();
    let mut files = Vec::new();
    if let Some(path) = &scope.system_file {
        files.push((path, ConfigLayerV1::Managed));
    }
    files.push((&scope.user_file, ConfigLayerV1::User));
    for path in &scope.trusted_project_files {
        files.push((path, ConfigLayerV1::Project));
    }
    if let Some(path) = &scope.selected_profile_file {
        files.push((path, ConfigLayerV1::Launch));
    }
    let mut legacy_profile = None;
    for (path, layer) in files {
        if !path.is_absolute() || !paths.insert(path) {
            return Err(Error::InvalidConfig);
        }
        let observed = if layer == ConfigLayerV1::Managed {
            read_system_config_bytes(path)?
        } else {
            read_validated_config_bytes(path)?
        };
        let digest = match &observed {
            Some((bytes, _)) => CanonicalDigest::of_bytes(bytes),
            None => CanonicalDigest::of_bytes(b"absent-codex-layer"),
        };
        dependencies.push(CanonicalDigest::of_bytes(
            format!("{}\0{}\0{layer:?}", path.display(), digest.as_str()).as_bytes(),
        ));
        let Some((bytes, _)) = observed else {
            if scope.selected_profile_file.as_ref() == Some(path) {
                return Err(Error::InvalidConfig);
            }
            continue;
        };
        let text = std::str::from_utf8(&bytes).map_err(|_| Error::InvalidConfig)?;
        let document = text
            .parse::<DocumentMut>()
            .map_err(|_| Error::InvalidConfig)?;
        collect(document.as_table(), layer, &digest, &mut fields)?;
        merge_source(source_document.as_table_mut(), document.as_table(), 0)?;
        if path == &scope.user_file
            && let CodexSelectionTarget::LegacyProfile(name) = &scope.selection
        {
            if name.is_empty() || name.len() > 128 {
                return Err(Error::InvalidConfig);
            }
            let table = document
                .get("profiles")
                .and_then(Item::as_table)
                .and_then(|profiles| profiles.get(name))
                .and_then(Item::as_table)
                .ok_or(Error::InvalidConfig)?;
            let mut selected = BTreeMap::new();
            if table.contains_key("model_providers") {
                return Err(Error::InvalidConfig);
            }
            collect(table, ConfigLayerV1::Launch, &digest, &mut selected)?;
            legacy_profile = Some((selected, table.clone()));
            dependencies.push(CanonicalDigest::of_bytes(
                format!("profile\0{name}").as_bytes(),
            ));
        }
    }
    if let Some((profile, table)) = legacy_profile {
        fields.extend(profile);
        merge_source(source_document.as_table_mut(), &table, 0)?;
    } else if matches!(scope.selection, CodexSelectionTarget::LegacyProfile(_)) {
        return Err(Error::InvalidConfig);
    }
    for value in scope.cli_overrides.iter() {
        if value.len() > 16 * 1024 || value.contains('\0') {
            return Err(Error::InvalidConfig);
        }
        let (key, raw) = value.split_once('=').ok_or(Error::InvalidConfig)?;
        let key = key.trim();
        if key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return Err(Error::InvalidConfig);
        }
        // Native -c parses the value as TOML and falls back to a literal string.
        let mut document = DocumentMut::new();
        let parsed = raw
            .trim()
            .parse::<toml_edit::Value>()
            .unwrap_or_else(|_| toml_edit::Value::from(raw.to_owned()));
        let parts = key.split('.').collect::<Vec<_>>();
        if parts.len() > 8 || parts.iter().any(|part| part.is_empty()) {
            return Err(Error::InvalidConfig);
        }
        let mut destination = document.as_table_mut();
        for part in &parts[..parts.len() - 1] {
            destination.insert(part, Item::Table(toml_edit::Table::new()));
            destination = destination
                .get_mut(part)
                .and_then(Item::as_table_mut)
                .ok_or(Error::InvalidConfig)?;
        }
        destination.insert(parts[parts.len() - 1], Item::Value(parsed));
        let digest = CanonicalDigest::of_bytes(value.as_bytes());
        collect(
            document.as_table(),
            ConfigLayerV1::Process,
            &digest,
            &mut fields,
        )?;
        dependencies.push(digest);
        merge_source(source_document.as_table_mut(), document.as_table(), 0)?;
    }
    let target_layer =
        if scope.selection != CodexSelectionTarget::Root || scope.selected_profile_file.is_some() {
            ConfigLayerV1::Launch
        } else {
            ConfigLayerV1::User
        };
    let writable_selection = ["model", "model_provider", "model_catalog_json"]
        .iter()
        .all(|key| {
            fields
                .get(*key)
                .is_none_or(|(layer, _, _)| layer.precedence() >= target_layer.precedence())
        });
    let observations = fields
        .into_iter()
        .map(
            |(path, (layer, value, source_digest))| ConfigObservationV1 {
                path,
                layer,
                value: serde_json::Value::String(value),
                source_digest,
            },
        )
        .collect();
    Ok(CodexEffectiveConfiguration {
        observations,
        dependency_digest: CanonicalDigest::of(&dependencies).map_err(|_| Error::InvalidConfig)?,
        writable_selection,
        context_digest: CanonicalDigest::of(&(
            "codex-context/v3",
            &scope.codex_home,
            &scope.user_file,
            &scope.system_file,
            &scope.trusted_project_files,
            &scope.selected_profile_file,
            format!("{:?}", scope.selection),
            CanonicalDigest::of(&*scope.cli_overrides).map_err(|_| Error::InvalidConfig)?,
        ))
        .map_err(|_| Error::InvalidConfig)?,
        source_document,
    })
}

pub fn codex_has_context_override(scope: &CodexConfigurationScope) -> Result<bool, Error> {
    let config = sample_codex_configuration(scope)?;
    Ok(["model_context_window", "model_auto_compact_token_limit"]
        .iter()
        .any(|key| config.source_document.contains_key(key)))
}

fn merge_source(
    target: &mut toml_edit::Table,
    source: &toml_edit::Table,
    depth: usize,
) -> Result<(), Error> {
    if depth > 16 {
        return Err(Error::InvalidConfig);
    }
    for (key, value) in source.iter() {
        if depth == 0
            && !matches!(
                key,
                "model"
                    | "model_provider"
                    | "model_providers"
                    | "model_catalog_json"
                    | "model_context_window"
                    | "model_auto_compact_token_limit"
            )
        {
            continue;
        }
        if let (Some(existing), Some(overlay)) = (
            target.get_mut(key).and_then(Item::as_table_mut),
            value.as_table(),
        ) {
            merge_source(existing, overlay, depth + 1)?;
        } else {
            target.insert(key, value.clone());
        }
    }
    Ok(())
}

fn collect(
    table: &toml_edit::Table,
    layer: ConfigLayerV1,
    digest: &CanonicalDigest,
    output: &mut BTreeMap<String, (ConfigLayerV1, String, CanonicalDigest)>,
) -> Result<(), Error> {
    for key in ["model_context_window", "model_auto_compact_token_limit"] {
        if let Some(item) = table.get(key) {
            let value = item
                .as_integer()
                .filter(|value| *value > 0)
                .ok_or(Error::InvalidConfig)?;
            output.insert(key.to_owned(), (layer, value.to_string(), digest.clone()));
        }
    }
    for key in ["model", "model_provider", "model_catalog_json"] {
        if let Some(item) = table.get(key) {
            let value = item.as_str().ok_or(Error::InvalidConfig)?;
            if value.contains('\0') || value.len() > 4096 {
                return Err(Error::InvalidConfig);
            }
            output.insert(key.to_owned(), (layer, value.to_owned(), digest.clone()));
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn write(path: &std::path::Path, content: &str) {
        std::fs::write(path, content).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    #[test]
    fn codex_catalog_project_override_blocks_user_pointer_application() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("config.toml");
        let project = root.path().join("project.toml");
        write(&user, "model_catalog_json = '/user/catalog.json'\n");
        write(&project, "model_catalog_json = '/project/catalog.json'\n");
        let mut scope = CodexConfigurationScope::user_file(user);
        let before = sample_codex_configuration(&scope).unwrap();
        assert!(before.writable_selection);
        scope.trusted_project_files.push(project);
        let after = sample_codex_configuration(&scope).unwrap();
        assert!(!after.writable_selection);
        assert_eq!(
            after.source_document["model_catalog_json"].as_str(),
            Some("/project/catalog.json")
        );
        assert_eq!(
            after
                .observations
                .iter()
                .find(|field| field.path == "model_catalog_json")
                .unwrap()
                .value,
            "/project/catalog.json"
        );
        assert_ne!(before.dependency_digest, after.dependency_digest);
    }

    #[test]
    fn codex_catalog_profile_then_cli_override_preserves_scope_identity() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("config.toml");
        write(
            &user,
            "model_catalog_json = '/root.json'\n[profiles.work]\nmodel_catalog_json = '/profile.json'\n",
        );
        let mut scope = CodexConfigurationScope::user_file(user);
        scope.selection = CodexSelectionTarget::LegacyProfile("work".into());
        let profile = sample_codex_configuration(&scope).unwrap();
        assert!(profile.writable_selection);
        assert_eq!(
            profile.source_document["model_catalog_json"].as_str(),
            Some("/profile.json")
        );
        scope
            .cli_overrides
            .push("model_catalog_json='/cli.json'".into());
        let cli = sample_codex_configuration(&scope).unwrap();
        assert!(!cli.writable_selection);
        assert_eq!(
            cli.source_document["model_catalog_json"].as_str(),
            Some("/cli.json")
        );
        assert_ne!(profile.dependency_digest, cli.dependency_digest);
        assert_ne!(profile.context_digest, cli.context_digest);
    }

    #[test]
    fn codex_catalog_non_string_override_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("config.toml");
        write(&user, "model_catalog_json = 17\n");
        assert!(matches!(
            sample_codex_configuration(&CodexConfigurationScope::user_file(user)),
            Err(Error::InvalidConfig)
        ));
    }

    #[test]
    fn codex_layers_follow_explicit_scope_and_cli_wins_without_claiming_writability() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("config.toml");
        let system = root.path().join("system.toml");
        let project = root.path().join("project.toml");
        write(&system, "model = 'system'\nmodel_provider = 'system'\n");
        write(
            &user,
            "model = 'user'\n[profiles.work]\nmodel = 'profile'\n",
        );
        write(&project, "model = 'project'\n");
        let mut scope = CodexConfigurationScope::user_file(user.clone());
        scope.system_file = Some(system);
        scope.trusted_project_files.push(project);
        let project_result = sample_codex_configuration(&scope).unwrap();
        assert_eq!(
            project_result
                .observations
                .iter()
                .find(|v| v.path == "model")
                .unwrap()
                .value,
            "project"
        );
        assert!(!project_result.writable_selection);
        scope.selection = CodexSelectionTarget::LegacyProfile("work".into());
        let profile = sample_codex_configuration(&scope).unwrap();
        assert_eq!(
            profile
                .observations
                .iter()
                .find(|v| v.path == "model")
                .unwrap()
                .value,
            "profile"
        );
        assert!(profile.writable_selection);
        scope.cli_overrides.push("model = 'cli'".into());
        let cli = sample_codex_configuration(&scope).unwrap();
        assert_eq!(
            cli.observations
                .iter()
                .find(|v| v.path == "model")
                .unwrap()
                .value,
            "cli"
        );
        assert!(!cli.writable_selection);
        assert_ne!(profile.dependency_digest, cli.dependency_digest);
        scope.cli_overrides.clear();
        write(
            &user,
            "model = 'user'\nother = true\n[profiles.work]\nmodel = 'profile'\n",
        );
        assert_ne!(
            sample_codex_configuration(&scope)
                .unwrap()
                .dependency_digest,
            profile.dependency_digest
        );
    }
    #[test]
    fn codex_layers_reject_unknown_profile_duplicate_keys_and_untrusted_file_shape() {
        let root = tempfile::tempdir().unwrap();
        let user = root.path().join("config.toml");
        write(&user, "model = 'a'\nmodel = 'b'\n");
        let mut scope = CodexConfigurationScope::user_file(user.clone());
        assert!(sample_codex_configuration(&scope).is_err());
        write(&user, "model = 'a'\n");
        scope.selection = CodexSelectionTarget::LegacyProfile("missing".into());
        assert!(sample_codex_configuration(&scope).is_err());
        scope.selection = CodexSelectionTarget::Root;
        std::fs::set_permissions(&user, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(matches!(
            sample_codex_configuration(&scope),
            Err(Error::UnsafePermissions)
        ));
    }

    #[test]
    fn codex_system_layer_is_read_only_and_may_be_public_readable() {
        let root = tempfile::tempdir().unwrap();
        let system = root.path().join("system.toml");
        write(&system, "model = 'system-default'\n");
        std::fs::set_permissions(&system, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut scope = CodexConfigurationScope::user_file(root.path().join("absent-user.toml"));
        scope.system_file = Some(system.clone());
        let observed = sample_codex_configuration(&scope).unwrap();
        assert_eq!(observed.observations[0].value, "system-default");
        assert_eq!(observed.observations[0].layer, ConfigLayerV1::Managed);
        assert_eq!(
            std::fs::metadata(&system).unwrap().permissions().mode() & 0o777,
            0o644
        );
        let user_view =
            sample_codex_configuration(&CodexConfigurationScope::user_file(system.clone()))
                .unwrap();
        assert_eq!(user_view.observations[0].value, "system-default");
        assert_eq!(user_view.observations[0].layer, ConfigLayerV1::User);
        assert_eq!(
            std::fs::metadata(&system).unwrap().permissions().mode() & 0o777,
            0o644
        );
        std::fs::set_permissions(&system, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(sample_codex_configuration(&scope).is_err());
    }
}
