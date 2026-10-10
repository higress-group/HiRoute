use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use thiserror::Error;
use zeroize::Zeroizing;

const CAPABILITY_BYTES: usize = 32;
const MAX_CAPABILITY_FILE_BYTES: u64 = 512;

pub(crate) struct SecretText(Zeroizing<String>);

impl SecretText {
    pub(crate) fn generate() -> Result<Self, CpaConfigError> {
        let mut bytes = Zeroizing::new([0_u8; CAPABILITY_BYTES]);
        getrandom::fill(bytes.as_mut()).map_err(|_| CpaConfigError::Random)?;
        Ok(Self(Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_ref()))))
    }

    pub(crate) fn from_owned(value: String) -> Result<Self, CpaConfigError> {
        if value.len() < 32
            || value.len() > 128
            || value.contains(['\r', '\n', '\0'])
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(CpaConfigError::InvalidCapability);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretText([REDACTED])")
    }
}

#[derive(Debug)]
pub(crate) struct InstanceSecrets {
    pub(crate) downstream: Arc<SecretText>,
    pub(crate) management: Arc<SecretText>,
}

impl InstanceSecrets {
    pub(crate) fn generate() -> Result<Self, CpaConfigError> {
        let downstream = SecretText::generate()?;
        let management = SecretText::generate()?;
        if downstream.expose() == management.expose() {
            return Err(CpaConfigError::Random);
        }
        Ok(Self {
            downstream: Arc::new(downstream),
            management: Arc::new(management),
        })
    }

    pub(crate) fn write(&self, path: &Path) -> Result<(), CpaConfigError> {
        let bytes = Zeroizing::new(format!(
            "{}\n{}\n",
            self.downstream.expose(),
            self.management.expose()
        ));
        private_atomic_write(path, bytes.as_bytes())
    }

    pub(crate) fn read(path: &Path) -> Result<Self, CpaConfigError> {
        validate_private_file(path)?;
        let file = fs::File::open(path).map_err(CpaConfigError::Io)?;
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_CAPABILITY_FILE_BYTES + 1)
            .read_to_end(bytes.as_mut())
            .map_err(CpaConfigError::Io)?;
        if bytes.len() as u64 > MAX_CAPABILITY_FILE_BYTES {
            return Err(CpaConfigError::InvalidCapability);
        }
        let text = Zeroizing::new(
            String::from_utf8(std::mem::take(bytes.as_mut()))
                .map_err(|_| CpaConfigError::InvalidCapability)?,
        );
        let mut lines = text.lines();
        let downstream = SecretText::from_owned(lines.next().unwrap_or_default().to_owned())?;
        let management = SecretText::from_owned(lines.next().unwrap_or_default().to_owned())?;
        if lines.next().is_some() || downstream.expose() == management.expose() {
            return Err(CpaConfigError::InvalidCapability);
        }
        Ok(Self {
            downstream: Arc::new(downstream),
            management: Arc::new(management),
        })
    }
}

pub struct CpaManagedConfigContract;

impl CpaManagedConfigContract {
    pub fn validate_rendered(bytes: &[u8]) -> Result<(), CpaConfigError> {
        let value: serde_yaml::Value =
            serde_yaml::from_slice(bytes).map_err(CpaConfigError::Yaml)?;
        validate_wire_value(&value)
    }

    pub fn stock_contract_keys() -> BTreeSet<&'static str> {
        TOP_LEVEL_KEYS.into_iter().collect()
    }
}

pub(crate) fn render_managed_config(
    port: u16,
    auth_dir: &Path,
    secrets: &InstanceSecrets,
) -> Result<Vec<u8>, CpaConfigError> {
    if port == 0 || !auth_dir.is_absolute() {
        return Err(CpaConfigError::InvalidConfig);
    }
    let config = WireConfig {
        host: "127.0.0.1",
        port,
        tls: TlsConfig {
            enable: false,
            cert: "",
            key: "",
        },
        remote_management: RemoteManagement {
            allow_remote: false,
            secret_key: secrets.management.expose(),
            disable_control_panel: true,
            disable_auto_update_panel: true,
        },
        auth_dir: auth_dir.to_path_buf(),
        api_keys: vec![secrets.downstream.expose()],
        debug: false,
        logging_to_file: false,
        usage_statistics_enabled: false,
        request_log: false,
        commercial_mode: true,
        websocket_auth: true,
        disable_cooling: true,
        save_cooldown_status: false,
        transient_error_cooldown_seconds: -1,
        disable_claude_cloak_mode: true,
        request_retry: 0,
        max_retry_credentials: 1,
        max_retry_interval: 0,
        force_model_prefix: true,
        quota_exceeded: QuotaExceeded {
            switch_project: false,
            switch_preview_model: false,
            antigravity_credits: false,
        },
        routing: Routing {
            strategy: "fill-first",
            session_affinity: false,
        },
        streaming: Streaming {
            keepalive_seconds: 0,
            bootstrap_retries: 0,
        },
        plugins: Plugins { enabled: false },
    };
    let bytes = serde_yaml::to_string(&config)
        .map_err(CpaConfigError::Yaml)?
        .into_bytes();
    CpaManagedConfigContract::validate_rendered(&bytes)?;
    Ok(bytes)
}

const TOP_LEVEL_KEYS: [&str; 23] = [
    "host",
    "port",
    "tls",
    "remote-management",
    "auth-dir",
    "api-keys",
    "debug",
    "logging-to-file",
    "usage-statistics-enabled",
    "request-log",
    "commercial-mode",
    "ws-auth",
    "disable-cooling",
    "save-cooldown-status",
    "transient-error-cooldown-seconds",
    "disable-claude-cloak-mode",
    "request-retry",
    "max-retry-credentials",
    "max-retry-interval",
    "force-model-prefix",
    "quota-exceeded",
    "routing",
    "streaming",
];

fn validate_wire_value(value: &serde_yaml::Value) -> Result<(), CpaConfigError> {
    let map = value.as_mapping().ok_or(CpaConfigError::InvalidConfig)?;
    let keys = map
        .keys()
        .map(|key| key.as_str().ok_or(CpaConfigError::InvalidConfig))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let mut expected = CpaManagedConfigContract::stock_contract_keys();
    expected.insert("plugins");
    if keys != expected {
        return Err(CpaConfigError::InvalidConfig);
    }
    expect_scalar(value, "host", "127.0.0.1")?;
    let port = child(value, "port")
        .as_i64()
        .ok_or(CpaConfigError::InvalidConfig)?;
    let auth_dir = child(value, "auth-dir")
        .as_str()
        .map(Path::new)
        .ok_or(CpaConfigError::InvalidConfig)?;
    if !(1..=i64::from(u16::MAX)).contains(&port) || !auth_dir.is_absolute() {
        return Err(CpaConfigError::InvalidConfig);
    }
    validate_map_exact(child(value, "tls"), &["enable", "cert", "key"])?;
    expect_nested_bool(value, "tls", "enable", false)?;
    expect_nested_scalar(value, "tls", "cert", "")?;
    expect_nested_scalar(value, "tls", "key", "")?;
    expect_bool(value, "debug", false)?;
    expect_bool(value, "logging-to-file", false)?;
    expect_bool(value, "usage-statistics-enabled", false)?;
    expect_bool(value, "request-log", false)?;
    expect_bool(value, "commercial-mode", true)?;
    expect_bool(value, "ws-auth", true)?;
    expect_bool(value, "disable-cooling", true)?;
    expect_bool(value, "save-cooldown-status", false)?;
    expect_integer(value, "transient-error-cooldown-seconds", -1)?;
    expect_bool(value, "disable-claude-cloak-mode", true)?;
    expect_integer(value, "request-retry", 0)?;
    expect_integer(value, "max-retry-credentials", 1)?;
    expect_integer(value, "max-retry-interval", 0)?;
    expect_bool(value, "force-model-prefix", true)?;

    let api_keys = child(value, "api-keys")
        .as_sequence()
        .ok_or(CpaConfigError::InvalidConfig)?;
    let downstream = api_keys
        .first()
        .and_then(serde_yaml::Value::as_str)
        .ok_or(CpaConfigError::InvalidConfig)?;
    if api_keys.len() != 1 || SecretText::from_owned(downstream.to_owned()).is_err() {
        return Err(CpaConfigError::InvalidConfig);
    }
    validate_map_exact(
        child(value, "remote-management"),
        &[
            "allow-remote",
            "secret-key",
            "disable-control-panel",
            "disable-auto-update-panel",
        ],
    )?;
    expect_nested_bool(value, "remote-management", "allow-remote", false)?;
    expect_nested_bool(value, "remote-management", "disable-control-panel", true)?;
    expect_nested_bool(
        value,
        "remote-management",
        "disable-auto-update-panel",
        true,
    )?;
    let management = child(child(value, "remote-management"), "secret-key")
        .as_str()
        .ok_or(CpaConfigError::InvalidConfig)?;
    SecretText::from_owned(management.to_owned())?;
    if management == downstream {
        return Err(CpaConfigError::InvalidConfig);
    }

    validate_map_exact(
        child(value, "quota-exceeded"),
        &[
            "switch-project",
            "switch-preview-model",
            "antigravity-credits",
        ],
    )?;
    for key in [
        "switch-project",
        "switch-preview-model",
        "antigravity-credits",
    ] {
        expect_nested_bool(value, "quota-exceeded", key, false)?;
    }
    validate_map_exact(child(value, "routing"), &["strategy", "session-affinity"])?;
    expect_nested_scalar(value, "routing", "strategy", "fill-first")?;
    expect_nested_bool(value, "routing", "session-affinity", false)?;
    validate_map_exact(
        child(value, "streaming"),
        &["keepalive-seconds", "bootstrap-retries"],
    )?;
    expect_nested_integer(value, "streaming", "keepalive-seconds", 0)?;
    expect_nested_integer(value, "streaming", "bootstrap-retries", 0)?;
    validate_map_exact(child(value, "plugins"), &["enabled"])?;
    expect_nested_bool(value, "plugins", "enabled", false)?;
    Ok(())
}

fn child<'a>(value: &'a serde_yaml::Value, key: &str) -> &'a serde_yaml::Value {
    &value[key]
}

fn validate_map_exact(value: &serde_yaml::Value, expected: &[&str]) -> Result<(), CpaConfigError> {
    let map = value.as_mapping().ok_or(CpaConfigError::InvalidConfig)?;
    let keys = map
        .keys()
        .map(|key| key.as_str().ok_or(CpaConfigError::InvalidConfig))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if keys != expected.iter().copied().collect() {
        return Err(CpaConfigError::InvalidConfig);
    }
    Ok(())
}

fn expect_scalar(
    value: &serde_yaml::Value,
    key: &str,
    expected: &str,
) -> Result<(), CpaConfigError> {
    if child(value, key).as_str() != Some(expected) {
        return Err(CpaConfigError::InvalidConfig);
    }
    Ok(())
}

fn expect_bool(value: &serde_yaml::Value, key: &str, expected: bool) -> Result<(), CpaConfigError> {
    if child(value, key).as_bool() != Some(expected) {
        return Err(CpaConfigError::InvalidConfig);
    }
    Ok(())
}

fn expect_integer(
    value: &serde_yaml::Value,
    key: &str,
    expected: i64,
) -> Result<(), CpaConfigError> {
    if child(value, key).as_i64() != Some(expected) {
        return Err(CpaConfigError::InvalidConfig);
    }
    Ok(())
}

fn expect_nested_scalar(
    value: &serde_yaml::Value,
    parent: &str,
    key: &str,
    expected: &str,
) -> Result<(), CpaConfigError> {
    expect_scalar(child(value, parent), key, expected)
}

fn expect_nested_bool(
    value: &serde_yaml::Value,
    parent: &str,
    key: &str,
    expected: bool,
) -> Result<(), CpaConfigError> {
    expect_bool(child(value, parent), key, expected)
}

fn expect_nested_integer(
    value: &serde_yaml::Value,
    parent: &str,
    key: &str,
    expected: i64,
) -> Result<(), CpaConfigError> {
    expect_integer(child(value, parent), key, expected)
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct WireConfig<'a> {
    host: &'a str,
    port: u16,
    tls: TlsConfig<'a>,
    remote_management: RemoteManagement<'a>,
    auth_dir: PathBuf,
    api_keys: Vec<&'a str>,
    debug: bool,
    logging_to_file: bool,
    usage_statistics_enabled: bool,
    request_log: bool,
    commercial_mode: bool,
    #[serde(rename = "ws-auth")]
    websocket_auth: bool,
    disable_cooling: bool,
    save_cooldown_status: bool,
    transient_error_cooldown_seconds: i32,
    disable_claude_cloak_mode: bool,
    request_retry: u8,
    max_retry_credentials: u8,
    max_retry_interval: u8,
    force_model_prefix: bool,
    quota_exceeded: QuotaExceeded,
    routing: Routing<'a>,
    streaming: Streaming,
    plugins: Plugins,
}

#[derive(Serialize)]
struct TlsConfig<'a> {
    enable: bool,
    cert: &'a str,
    key: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct RemoteManagement<'a> {
    allow_remote: bool,
    secret_key: &'a str,
    disable_control_panel: bool,
    disable_auto_update_panel: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct QuotaExceeded {
    switch_project: bool,
    switch_preview_model: bool,
    antigravity_credits: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct Routing<'a> {
    strategy: &'a str,
    session_affinity: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct Streaming {
    keepalive_seconds: u8,
    bootstrap_retries: u8,
}

#[derive(Serialize)]
struct Plugins {
    enabled: bool,
}

pub(crate) fn ensure_private_dir(path: &Path) -> Result<PathBuf, CpaConfigError> {
    if !path.is_absolute() {
        return Err(CpaConfigError::InvalidPrivatePath);
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(CpaConfigError::Io)?;
    validate_private_dir(path)?;
    path.canonicalize().map_err(CpaConfigError::Io)
}

pub(crate) fn private_atomic_write(path: &Path, bytes: &[u8]) -> Result<(), CpaConfigError> {
    crate::request_context::io_check().map_err(CpaConfigError::Io)?;
    let parent = path.parent().ok_or(CpaConfigError::InvalidPrivatePath)?;
    validate_private_dir(parent)?;
    let suffix = SecretText::generate()?;
    let temp_path = parent.join(format!(".write-{}.tmp", suffix.expose()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    set_private_file_create_mode(&mut options);
    let result = (|| {
        let mut file = options.open(&temp_path).map_err(CpaConfigError::Io)?;
        file.write_all(bytes).map_err(CpaConfigError::Io)?;
        file.sync_all().map_err(CpaConfigError::Io)?;
        validate_private_file(&temp_path)?;
        crate::request_context::io_check().map_err(CpaConfigError::Io)?;
        fs::rename(&temp_path, path).map_err(CpaConfigError::Io)?;
        validate_private_file(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(unix)]
fn validate_private_dir(path: &Path) -> Result<(), CpaConfigError> {
    let metadata = fs::symlink_metadata(path).map_err(CpaConfigError::Io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CpaConfigError::InsecurePermissions);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_dir(path: &Path) -> Result<(), CpaConfigError> {
    let metadata = fs::symlink_metadata(path).map_err(CpaConfigError::Io)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CpaConfigError::InsecurePermissions);
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_file_create_mode(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn set_private_file_create_mode(_options: &mut OpenOptions) {}

#[cfg(unix)]
pub(crate) fn validate_private_file(path: &Path) -> Result<(), CpaConfigError> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = fs::symlink_metadata(path).map_err(CpaConfigError::Io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
        return Err(CpaConfigError::InsecurePermissions);
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn validate_private_file(path: &Path) -> Result<(), CpaConfigError> {
    let metadata = fs::symlink_metadata(path).map_err(CpaConfigError::Io)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(CpaConfigError::InsecurePermissions);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum CpaConfigError {
    #[error("CPA managed configuration is invalid or permits internal routing")]
    InvalidConfig,
    #[error("CPA capability generation failed")]
    Random,
    #[error("CPA local capability is malformed")]
    InvalidCapability,
    #[error("CPA private path is not absolute")]
    InvalidPrivatePath,
    #[error("CPA state/config path is not a safe regular file or directory")]
    InsecurePermissions,
    #[error("CPA state/config I/O failed: {0}")]
    Io(std::io::Error),
    #[error("CPA managed YAML could not be encoded or decoded: {0}")]
    Yaml(serde_yaml::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_config_has_one_local_capability_and_no_provider_key_sections() {
        let temp = tempfile::tempdir().unwrap();
        let auth = temp.path().join("auth");
        ensure_private_dir(&auth).unwrap();
        let secrets = InstanceSecrets::generate().unwrap();
        let bytes = render_managed_config(18080, &auth, &secrets).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("codex-api-key"));
        assert!(!text.contains("claude-api-key"));
        assert!(!text.contains("openai-compatibility"));
        CpaManagedConfigContract::validate_rendered(&bytes).unwrap();
    }

    #[test]
    fn managed_config_rejects_retry_or_non_loopback_drift() {
        let temp = tempfile::tempdir().unwrap();
        let auth = temp.path().join("auth");
        ensure_private_dir(&auth).unwrap();
        let secrets = InstanceSecrets::generate().unwrap();
        let bytes = render_managed_config(18081, &auth, &secrets).unwrap();
        let retry = String::from_utf8(bytes.clone())
            .unwrap()
            .replace("request-retry: 0", "request-retry: 1");
        assert!(CpaManagedConfigContract::validate_rendered(retry.as_bytes()).is_err());
        let remote = String::from_utf8(bytes)
            .unwrap()
            .replace("host: 127.0.0.1", "host: 0.0.0.0");
        assert!(CpaManagedConfigContract::validate_rendered(remote.as_bytes()).is_err());
    }

    #[test]
    fn managed_config_rejects_insecure_transport_and_capability_aliasing() {
        let temp = tempfile::tempdir().unwrap();
        let auth = temp.path().join("auth");
        ensure_private_dir(&auth).unwrap();
        let secrets = InstanceSecrets::generate().unwrap();
        let bytes = render_managed_config(18082, &auth, &secrets).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        for drift in [
            text.replace("port: 18082", "port: 0"),
            text.replace("enable: false", "enable: true"),
            text.replace(secrets.management.expose(), secrets.downstream.expose()),
        ] {
            assert!(CpaManagedConfigContract::validate_rendered(drift.as_bytes()).is_err());
        }
    }

    #[test]
    fn capability_file_is_bounded_and_round_trips_without_debug_exposure() {
        let temp = tempfile::tempdir().unwrap();
        let root = ensure_private_dir(temp.path()).unwrap();
        let path = root.join("capability");
        let secrets = InstanceSecrets::generate().unwrap();
        secrets.write(&path).unwrap();
        let loaded = InstanceSecrets::read(&path).unwrap();
        assert_eq!(loaded.downstream.expose(), secrets.downstream.expose());
        assert_eq!(format!("{:?}", loaded.downstream), "SecretText([REDACTED])");
    }
}
