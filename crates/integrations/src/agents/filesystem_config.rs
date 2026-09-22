use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use hiroute_domain::{AgentConfigChangeV1, AgentConfigDocumentV1, CanonicalDigest, ConfigLayerV1};
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::filesystem::{
    AgentFilesystemScanError, PermissionHardeningOutcomeV1, PermissionHardeningRequiredV1,
};
#[cfg(test)]
use super::filesystem::{FILESYSTEM_AGENT_SCANNER_ID_V1, FILESYSTEM_AGENT_SCANNER_VERSION_V1};

mod native;
pub(super) use native::{
    claude_change_bytes_are_applied, claude_user_change_is_applied, rebase_claude_change_bytes,
    render_claude_change_bytes, render_claude_user_change, restore_claude_change_bytes,
};
mod auth;
use auth::read_settings_auth_field;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
type ValidatedConfigBytes = (Zeroizing<Vec<u8>>, std::fs::Metadata);

pub(super) const CLAUDE_AUTH_ENVIRONMENT_FIELDS: [&str; 6] = [
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_CUSTOM_HEADERS",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_VERTEX",
];

#[derive(Clone)]
pub(super) enum ClaudeSource {
    Process,
    File(ConfigLayerV1, PathBuf),
}

impl ClaudeSource {
    #[cfg(test)]
    pub(super) fn permission_hardening_required(
        &self,
    ) -> Result<Option<PermissionHardeningRequiredV1>, AgentFilesystemScanError> {
        let Self::File(layer, path) = self else {
            return Ok(None);
        };
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        validate_file_kind_and_owner(&metadata)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.nlink() != 1 {
                return Err(AgentFilesystemScanError::InvalidConfig);
            }
            if metadata.permissions().mode() & 0o077 != 0 {
                let identity = stable_identity_from_metadata(&metadata);
                return Ok(Some(PermissionHardeningRequiredV1 {
                    scanner_id: FILESYSTEM_AGENT_SCANNER_ID_V1.to_owned(),
                    scanner_version: FILESYSTEM_AGENT_SCANNER_VERSION_V1.to_owned(),
                    discovered_source_ref: source_ref(path, *layer),
                    observed_identity: identity.clone(),
                    observed_revision: permission_revision(
                        &identity,
                        metadata.permissions().mode(),
                    ),
                    display_path: display_path(*layer, path),
                    required_mode: 0o600,
                }));
            }
        }
        Ok(None)
    }

    pub(super) fn harden_permissions(
        &self,
        finding: &PermissionHardeningRequiredV1,
    ) -> Result<PermissionHardeningOutcomeV1, AgentFilesystemScanError> {
        let Self::File(layer, path) = self else {
            return Err(AgentFilesystemScanError::InvalidDescriptor);
        };
        if source_ref(path, *layer) != finding.discovered_source_ref {
            return Err(AgentFilesystemScanError::InvalidDescriptor);
        }
        #[cfg(unix)]
        {
            use nix::fcntl::{OFlag, open};
            use nix::sys::stat::{Mode, fchmod};
            use std::os::unix::fs::{MetadataExt, PermissionsExt};

            let descriptor = open(
                path.as_path(),
                OFlag::O_RDONLY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK,
                Mode::empty(),
            )
            .map_err(|_| AgentFilesystemScanError::SourceChanged)?;
            let file = File::from(descriptor);
            let before = file
                .metadata()
                .map_err(|_| AgentFilesystemScanError::SourceChanged)?;
            if !before.is_file()
                || before.uid() != nix::unistd::geteuid().as_raw()
                || before.nlink() != 1
            {
                return Err(AgentFilesystemScanError::SourceChanged);
            }
            let identity = stable_identity_from_metadata(&before);
            if identity != finding.observed_identity {
                return Err(AgentFilesystemScanError::SourceChanged);
            }
            let current_mode = before.permissions().mode() & 0o777;
            if current_mode == 0o600 {
                return Ok(PermissionHardeningOutcomeV1::AlreadyHardened);
            }
            if permission_revision(&identity, before.permissions().mode())
                != finding.observed_revision
            {
                return Err(AgentFilesystemScanError::SourceChanged);
            }
            fchmod(&file, Mode::S_IRUSR | Mode::S_IWUSR)
                .map_err(|_| AgentFilesystemScanError::SourceUnavailable)?;
            let after = file
                .metadata()
                .map_err(|_| AgentFilesystemScanError::SourceChanged)?;
            if stable_identity_from_metadata(&after) != identity
                || after.permissions().mode() & 0o777 != 0o600
            {
                return Err(AgentFilesystemScanError::SourceChanged);
            }
            Ok(PermissionHardeningOutcomeV1::Hardened)
        }
        #[cfg(not(unix))]
        {
            let _ = (path, finding);
            Err(AgentFilesystemScanError::SourceUnavailable)
        }
    }

    pub(super) fn read(
        &self,
        process: &BTreeMap<String, Zeroizing<String>>,
        process_presence: &BTreeSet<String>,
    ) -> Result<ObservedClaudeSettings, AgentFilesystemScanError> {
        match self {
            Self::Process => {
                let settings = ClaudeSettingsSubset {
                    model: None,
                    env: ClaudeEnvironmentSubset {
                        base_url: process
                            .get("ANTHROPIC_BASE_URL")
                            .map(|value| value.as_str().to_owned()),
                        model: process
                            .get("ANTHROPIC_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        default_opus_model: process
                            .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        default_sonnet_model: process
                            .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        default_haiku_model: process
                            .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        small_fast_model: process
                            .get("ANTHROPIC_SMALL_FAST_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        context_environment: hiroute_domain::CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT
                            .into_iter()
                            .filter_map(|key| {
                                process
                                    .get(key)
                                    .map(|value| (key.to_owned(), value.as_str().to_owned()))
                            })
                            .collect(),
                        present_environment_fields: process_presence.clone(),
                    },
                    api_key_helper_present: false,
                };
                let digest = safe_settings_digest(&settings);
                Ok(ObservedClaudeSettings {
                    layer: ConfigLayerV1::Process,
                    revision: revision_from_digest(&digest),
                    digest,
                    source_ref: "claude/process-environment".to_owned(),
                    settings,
                })
            }
            Self::File(layer, path) => {
                let Some((settings, digest, revision)) = read_settings_file(path)? else {
                    return Ok(ObservedClaudeSettings {
                        layer: *layer,
                        revision: 0,
                        digest: CanonicalDigest::of_bytes(b"absent-claude-settings"),
                        source_ref: source_ref(path, *layer),
                        settings: ClaudeSettingsSubset::default(),
                    });
                };
                Ok(ObservedClaudeSettings {
                    layer: *layer,
                    revision,
                    digest,
                    source_ref: source_ref(path, *layer),
                    settings,
                })
            }
        }
    }

    pub(super) fn read_auth_field(
        &self,
        field: &str,
        process: &BTreeMap<String, Zeroizing<String>>,
        process_presence: &BTreeSet<String>,
    ) -> Result<(Zeroizing<String>, u64), AgentFilesystemScanError> {
        if !matches!(field, "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_API_KEY") {
            return Err(AgentFilesystemScanError::InvalidDescriptor);
        }
        match self {
            Self::Process => {
                let value = process
                    .get(field)
                    .cloned()
                    .or_else(|| std::env::var(field).ok().map(Zeroizing::new))
                    .ok_or(AgentFilesystemScanError::SourceUnavailable)?;
                let settings = ClaudeSettingsSubset {
                    model: None,
                    env: ClaudeEnvironmentSubset {
                        base_url: process
                            .get("ANTHROPIC_BASE_URL")
                            .map(|value| value.as_str().to_owned()),
                        model: process
                            .get("ANTHROPIC_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        default_opus_model: process
                            .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        default_sonnet_model: process
                            .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        default_haiku_model: process
                            .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        small_fast_model: process
                            .get("ANTHROPIC_SMALL_FAST_MODEL")
                            .map(|value| value.as_str().to_owned()),
                        context_environment: hiroute_domain::CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT
                            .into_iter()
                            .filter_map(|key| {
                                process
                                    .get(key)
                                    .map(|value| (key.to_owned(), value.as_str().to_owned()))
                            })
                            .collect(),
                        present_environment_fields: process_presence.clone(),
                    },
                    api_key_helper_present: false,
                };
                Ok((
                    value,
                    revision_from_digest(&safe_settings_digest(&settings)),
                ))
            }
            Self::File(_, path) => read_settings_auth_field(path, process, field),
        }
    }

    pub(super) fn source_ref(&self) -> String {
        match self {
            Self::Process => "claude/process-environment".to_owned(),
            Self::File(layer, path) => source_ref(path, *layer),
        }
    }
}

#[derive(Clone)]
pub(super) struct ObservedClaudeSettings {
    pub(super) layer: ConfigLayerV1,
    pub(super) revision: u64,
    pub(super) digest: CanonicalDigest,
    pub(super) source_ref: String,
    pub(super) settings: ClaudeSettingsSubset,
}

#[derive(Clone, Default)]
pub(super) struct ClaudeSettingsSubset {
    /// Claude's initial selection in settings.json, distinct from preset alias mappings.
    pub(super) model: Option<String>,
    pub(super) env: ClaudeEnvironmentSubset,
    pub(super) api_key_helper_present: bool,
}

#[derive(Clone, Default)]
pub(super) struct ClaudeEnvironmentSubset {
    pub(super) context_environment: BTreeMap<String, String>,
    pub(super) base_url: Option<String>,
    pub(super) model: Option<String>,
    pub(super) default_opus_model: Option<String>,
    pub(super) default_sonnet_model: Option<String>,
    pub(super) default_haiku_model: Option<String>,
    pub(super) small_fast_model: Option<String>,
    pub(super) present_environment_fields: BTreeSet<String>,
}

impl<'de> Deserialize<'de> for ClaudeSettingsSubset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SettingsVisitor;
        impl<'de> Visitor<'de> for SettingsVisitor {
            type Value = ClaudeSettingsSubset;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a Claude settings object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut env = None;
                let mut model = None;
                let mut api_key_helper_present = false;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "model" {
                        if model.is_some() {
                            return Err(serde::de::Error::duplicate_field("model"));
                        }
                        model = Some(map.next_value()?);
                    } else if key == "env" {
                        if env.is_some() {
                            return Err(serde::de::Error::duplicate_field("env"));
                        }
                        env = Some(map.next_value()?);
                    } else if key == "apiKeyHelper" {
                        map.next_value::<IgnoredAny>()?;
                        api_key_helper_present = true;
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(ClaudeSettingsSubset {
                    model,
                    env: env.unwrap_or_default(),
                    api_key_helper_present,
                })
            }
        }
        deserializer.deserialize_map(SettingsVisitor)
    }
}

impl<'de> Deserialize<'de> for ClaudeEnvironmentSubset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct EnvironmentVisitor;
        impl<'de> Visitor<'de> for EnvironmentVisitor {
            type Value = ClaudeEnvironmentSubset;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a Claude environment object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut result = ClaudeEnvironmentSubset::default();
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        key if hiroute_domain::CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT
                            .contains(&key) =>
                        {
                            result
                                .context_environment
                                .insert(key.to_owned(), map.next_value()?);
                        }
                        "ANTHROPIC_BASE_URL" => {
                            result.base_url = Some(map.next_value()?);
                            result.present_environment_fields.insert(key);
                        }
                        "ANTHROPIC_MODEL" => {
                            result.model = Some(map.next_value()?);
                            result.present_environment_fields.insert(key);
                        }
                        "ANTHROPIC_DEFAULT_OPUS_MODEL" => {
                            result.default_opus_model = Some(map.next_value()?);
                            result.present_environment_fields.insert(key);
                        }
                        "ANTHROPIC_DEFAULT_SONNET_MODEL" => {
                            result.default_sonnet_model = Some(map.next_value()?);
                            result.present_environment_fields.insert(key);
                        }
                        "ANTHROPIC_DEFAULT_HAIKU_MODEL" => {
                            result.default_haiku_model = Some(map.next_value()?);
                            result.present_environment_fields.insert(key);
                        }
                        "ANTHROPIC_SMALL_FAST_MODEL" => {
                            result.small_fast_model = Some(map.next_value()?);
                            result.present_environment_fields.insert(key);
                        }
                        "ANTHROPIC_AUTH_TOKEN"
                        | "ANTHROPIC_API_KEY"
                        | "ANTHROPIC_CUSTOM_HEADERS"
                        | "CLAUDE_CODE_USE_BEDROCK"
                        | "CLAUDE_CODE_USE_FOUNDRY"
                        | "CLAUDE_CODE_USE_VERTEX" => {
                            map.next_value::<IgnoredAny>()?;
                            result.present_environment_fields.insert(key);
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                Ok(result)
            }
        }
        deserializer.deserialize_map(EnvironmentVisitor)
    }
}

pub(super) fn same_layer_secret_conflict(values: &[ObservedClaudeSettings]) -> bool {
    for (index, left) in values.iter().enumerate() {
        if !left
            .settings
            .env
            .present_environment_fields
            .contains("ANTHROPIC_AUTH_TOKEN")
        {
            continue;
        }
        if values[index + 1..].iter().any(|right| {
            right.layer == left.layer
                && right
                    .settings
                    .env
                    .present_environment_fields
                    .contains("ANTHROPIC_AUTH_TOKEN")
        }) {
            return true;
        }
    }
    false
}

fn read_settings_file(
    path: &Path,
) -> Result<Option<(ClaudeSettingsSubset, CanonicalDigest, u64)>, AgentFilesystemScanError> {
    let Some((bytes, metadata)) = read_validated_config_bytes(path)? else {
        return Ok(None);
    };
    let digest = CanonicalDigest::of_bytes(&bytes);
    let settings =
        serde_json::from_slice(&bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    let revision = metadata_revision(&metadata, &digest);
    Ok(Some((settings, digest, revision)))
}

pub(super) fn read_validated_config_bytes(
    path: &Path,
) -> Result<Option<ValidatedConfigBytes>, AgentFilesystemScanError> {
    read_config_with_policy(path, false)
}

/// System layers are read-only and may be root-owned/public-readable; never harden or edit them.
pub(super) fn read_system_config_bytes(
    path: &Path,
) -> Result<Option<ValidatedConfigBytes>, AgentFilesystemScanError> {
    read_config_with_policy(path, true)
}

fn read_config_with_policy(
    path: &Path,
    system: bool,
) -> Result<Option<ValidatedConfigBytes>, AgentFilesystemScanError> {
    let expected = if system {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                validate_system_config_metadata(&metadata)?;
                Some(metadata)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        }
    } else {
        validate_config_file(path)?
    };
    let Some(expected) = expected else {
        return Ok(None);
    };
    if expected.len() > MAX_CONFIG_BYTES {
        return Err(AgentFilesystemScanError::ConfigTooLarge);
    }

    #[cfg(unix)]
    let mut file = {
        use nix::fcntl::{OFlag, open};
        use nix::sys::stat::Mode;

        let descriptor = open(
            path,
            OFlag::O_RDONLY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| AgentFilesystemScanError::SourceChanged)?;
        File::from(descriptor)
    };
    #[cfg(not(unix))]
    let mut file = File::open(path)?;

    let opened = file.metadata()?;
    if system {
        validate_system_config_metadata(&opened)?;
    } else {
        validate_file_kind_and_owner(&opened)?;
    }
    if !same_file_identity(&expected, &opened) {
        return Err(AgentFilesystemScanError::SourceChanged);
    }
    let mut bytes = Zeroizing::new(Vec::with_capacity(opened.len() as usize));
    file.by_ref()
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(AgentFilesystemScanError::ConfigTooLarge);
    }
    let after = file.metadata()?;
    if !same_file_identity(&opened, &after) || after.len() != bytes.len() as u64 {
        return Err(AgentFilesystemScanError::SourceChanged);
    }
    Ok(Some((bytes, after)))
}

fn validate_system_config_metadata(
    metadata: &std::fs::Metadata,
) -> Result<(), AgentFilesystemScanError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(AgentFilesystemScanError::InvalidConfig);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if ![0, nix::unistd::geteuid().as_raw()].contains(&metadata.uid()) {
            return Err(AgentFilesystemScanError::WrongOwner);
        }
        if metadata.nlink() != 1 || metadata.mode() & 0o022 != 0 {
            return Err(AgentFilesystemScanError::UnsafePermissions);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(AgentFilesystemScanError::SourceUnavailable)
    }
}

pub(super) fn same_file_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        left.dev() == right.dev()
            && left.ino() == right.ino()
            && left.uid() == right.uid()
            && left.gid() == right.gid()
            && left.nlink() == right.nlink()
            && left.len() == right.len()
            && left.mtime() == right.mtime()
            && left.mtime_nsec() == right.mtime_nsec()
            && left.ctime() == right.ctime()
            && left.ctime_nsec() == right.ctime_nsec()
            && left.permissions().mode() & 0o777 == right.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        left.len() == right.len() && left.permissions().readonly() == right.permissions().readonly()
    }
}

fn validate_config_file(
    path: &Path,
) -> Result<Option<std::fs::Metadata>, AgentFilesystemScanError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    validate_file_kind_and_owner(&metadata)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(AgentFilesystemScanError::InvalidConfig);
        }
        if metadata.mode() & 0o022 != 0 {
            return Err(AgentFilesystemScanError::UnsafePermissions);
        }
    }
    Ok(Some(metadata))
}

fn validate_file_kind_and_owner(
    metadata: &std::fs::Metadata,
) -> Result<(), AgentFilesystemScanError> {
    if metadata.file_type().is_symlink() {
        return Err(AgentFilesystemScanError::SymlinkConfig);
    }
    if !metadata.file_type().is_file() {
        return Err(AgentFilesystemScanError::InvalidConfig);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != nix::unistd::geteuid().as_raw() {
            return Err(AgentFilesystemScanError::WrongOwner);
        }
    }
    Ok(())
}

fn resolve_explicit_environment_reference(
    token: Zeroizing<String>,
    process: &BTreeMap<String, Zeroizing<String>>,
) -> Result<Zeroizing<String>, AgentFilesystemScanError> {
    let Some(name) = explicit_environment_name(token.as_str()) else {
        return Ok(token);
    };
    process
        .get(name)
        .cloned()
        .or_else(|| std::env::var(name).ok().map(Zeroizing::new))
        .ok_or(AgentFilesystemScanError::SourceUnavailable)
}

fn explicit_environment_name(value: &str) -> Option<&str> {
    let name = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'))?;
    (!name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then_some(name)
}

fn source_ref(path: &Path, layer: ConfigLayerV1) -> String {
    let digest = CanonicalDigest::of_bytes(
        format!("claude-settings\0{layer:?}\0{}", path.to_string_lossy()).as_bytes(),
    );
    format!("claude/settings/{}", &digest.as_str()[7..39])
}

/// Historical process-source hash from 9e1ddbb. Only the recovery verifier uses it;
/// never hide newly observed context settings behind an older digest format.
pub(super) fn recover_pre_context_process_digest(observed: &mut ObservedClaudeSettings) {
    if observed.layer != ConfigLayerV1::Process
        || !observed.settings.env.context_environment.is_empty()
    {
        return;
    }
    let settings = &observed.settings;
    let present = settings
        .env
        .present_environment_fields
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    observed.digest = CanonicalDigest::of_bytes(
        format!(
            "{}\0{}\0{}\0{}\0{}",
            settings.env.base_url.as_deref().unwrap_or(""),
            settings.env.model.as_deref().unwrap_or(""),
            settings.env.default_opus_model.as_deref().unwrap_or(""),
            present,
            settings.api_key_helper_present
        )
        .as_bytes(),
    );
    observed.revision = revision_from_digest(&observed.digest);
}

fn safe_settings_digest(settings: &ClaudeSettingsSubset) -> CanonicalDigest {
    let present = settings
        .env
        .present_environment_fields
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(",");
    CanonicalDigest::of_bytes(
        format!(
            "{}\0{}\0{}\0{}\0{}\0{:?}",
            settings.env.base_url.as_deref().unwrap_or(""),
            settings.env.model.as_deref().unwrap_or(""),
            settings.env.default_opus_model.as_deref().unwrap_or(""),
            present,
            settings.api_key_helper_present,
            settings.env.context_environment,
        )
        .as_bytes(),
    )
}

fn metadata_revision(metadata: &std::fs::Metadata, digest: &CanonicalDigest) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let revision = CanonicalDigest::of_bytes(
            format!(
                "{}\0{}\0{}\0{}\0{}",
                metadata.dev(),
                metadata.ino(),
                metadata.mode(),
                metadata.len(),
                digest
            )
            .as_bytes(),
        );
        revision_from_digest(&revision)
    }
    #[cfg(not(unix))]
    revision_from_digest(digest)
}

#[cfg(unix)]
fn stable_identity_from_metadata(metadata: &std::fs::Metadata) -> CanonicalDigest {
    use std::os::unix::fs::MetadataExt;
    // Permission-only hardening leaves mtime unchanged. Bind nanosecond mtime as well as
    // inode identity so deletion/recreation with an immediately reused inode is stale.
    CanonicalDigest::of_bytes(
        format!(
            "agent-config-identity/v2\0{}\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
            metadata.dev(),
            metadata.ino(),
            metadata.uid(),
            metadata.gid(),
            metadata.nlink(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
        )
        .as_bytes(),
    )
}

fn permission_revision(identity: &CanonicalDigest, mode: u32) -> u64 {
    let digest = CanonicalDigest::of_bytes(
        format!("agent-config-permission/v1\0{identity}\0{}", mode & 0o777).as_bytes(),
    );
    revision_from_digest(&digest)
}

#[cfg(test)]
fn display_path(layer: ConfigLayerV1, path: &Path) -> String {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.json");
    match layer {
        ConfigLayerV1::Process => "process environment".to_owned(),
        ConfigLayerV1::Launch => format!("launch/{name}"),
        ConfigLayerV1::Project => format!(".claude/{name}"),
        ConfigLayerV1::User => format!("~/.claude/{name}"),
        ConfigLayerV1::Managed => format!("managed/{name}"),
    }
}

fn revision_from_digest(digest: &CanonicalDigest) -> u64 {
    u64::from_str_radix(&digest.as_str()[7..23], 16)
        .unwrap_or(1)
        .max(1)
}

pub(super) fn managed_settings_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    return vec![PathBuf::from(
        "/Library/Application Support/ClaudeCode/managed-settings.json",
    )];
    #[cfg(not(target_os = "macos"))]
    return vec![PathBuf::from("/etc/claude-code/managed-settings.json")];
}
