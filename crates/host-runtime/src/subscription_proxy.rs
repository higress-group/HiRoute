//! Persisted host policy for managed subscriptions. Never exports inherited credentials.
use crate::ServiceProxyEnvironment;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

const SCHEMA: &str = "hiroute.subscription-proxy/v1";
const LIMIT: u64 = 16 * 1024;

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubscriptionProxyPolicy {
    #[default]
    Inherit,
    Direct,
    Manual {
        url: String,
        #[serde(default)]
        no_proxy: String,
    },
}

// Serde's internally tagged unit variants ignore extra fields. Deserialize through
// a strict object so inherit/direct cannot silently accept a stale manual URL.
impl<'de> Deserialize<'de> for SubscriptionProxyPolicy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            mode: String,
            #[serde(default)]
            url: Option<String>,
            #[serde(default)]
            no_proxy: Option<String>,
        }
        let wire = Wire::deserialize(deserializer)?;
        match wire.mode.as_str() {
            "inherit" if wire.url.is_none() && wire.no_proxy.is_none() => Ok(Self::Inherit),
            "direct" if wire.url.is_none() && wire.no_proxy.is_none() => Ok(Self::Direct),
            "manual" => Ok(Self::Manual {
                url: wire
                    .url
                    .ok_or_else(|| serde::de::Error::missing_field("url"))?,
                no_proxy: wire.no_proxy.unwrap_or_default(),
            }),
            _ => Err(serde::de::Error::custom(
                "invalid subscription proxy policy",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionProxyConfig {
    pub schema: String,
    pub revision: String,
    pub policy: SubscriptionProxyPolicy,
}

#[derive(Clone, Debug, Serialize)]
pub struct SubscriptionProxyView {
    pub config: SubscriptionProxyConfig,
    pub applied: bool,
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("SUBSCRIPTION_PROXY_INVALID_OR_UNAVAILABLE")]
pub struct SubscriptionProxyError;

impl SubscriptionProxyPolicy {
    pub fn validate(&self) -> Result<(), SubscriptionProxyError> {
        if let Self::Manual { url, no_proxy } = self {
            canonical_proxy_url(url)?;
            if no_proxy.len() > 4096 || no_proxy.chars().any(char::is_control) {
                return Err(SubscriptionProxyError);
            }
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        inherited: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<ServiceProxyEnvironment, SubscriptionProxyError> {
        self.validate()?;
        let values = match self {
            Self::Inherit => inherited.into_iter().collect(),
            Self::Direct => Vec::new(),
            Self::Manual { url, no_proxy } => vec![
                ("HTTP_PROXY".into(), canonical_proxy_url(url)?.into()),
                ("HTTPS_PROXY".into(), canonical_proxy_url(url)?.into()),
                (
                    "NO_PROXY".into(),
                    format!(
                        "localhost,127.0.0.1,::1{suffix}",
                        suffix = if no_proxy.is_empty() {
                            String::new()
                        } else {
                            format!(",{no_proxy}")
                        }
                    )
                    .into(),
                ),
            ],
        };
        ServiceProxyEnvironment::capture(values).map_err(|_| SubscriptionProxyError)
    }
}

// Both reqwest and CPA must receive the same authority. Reject browser-style
// repairs (missing authority/backslashes), then normalize the accepted URL once.
fn canonical_proxy_url(value: &str) -> Result<String, SubscriptionProxyError> {
    let Some((scheme, remainder)) = value.split_once("://") else {
        return Err(SubscriptionProxyError);
    };
    let authority = remainder.split('/').next().unwrap_or_default();
    if !matches!(scheme, "http" | "https")
        || authority.is_empty()
        || value.contains('\\')
        || value.len() > 2048
        || value
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
    {
        return Err(SubscriptionProxyError);
    }
    let parsed = url::Url::parse(value).map_err(|_| SubscriptionProxyError)?;
    if parsed.host_str().is_none()
        || parsed.port_or_known_default().is_none_or(|p| p == 0)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        return Err(SubscriptionProxyError);
    }
    Ok(parsed[..url::Position::BeforePath].to_owned())
}

impl Default for SubscriptionProxyConfig {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            revision: "default".into(),
            policy: SubscriptionProxyPolicy::Inherit,
        }
    }
}
impl SubscriptionProxyConfig {
    fn validate(&self) -> Result<(), SubscriptionProxyError> {
        if self.schema != SCHEMA
            || !(self.revision == "default"
                || (self.revision.len() == 32
                    && self.revision.bytes().all(|b| b.is_ascii_hexdigit())))
        {
            return Err(SubscriptionProxyError);
        }
        self.policy.validate()
    }
}

/// Root is the host's configuration directory, never its database storage directory.
pub struct SubscriptionProxyStore {
    root: PathBuf,
}
impl SubscriptionProxyStore {
    pub fn new(host_root: &Path) -> Self {
        Self {
            root: host_root.into(),
        }
    }
    pub fn load(&self) -> Result<SubscriptionProxyConfig, SubscriptionProxyError> {
        let result = read::<SubscriptionProxyConfig>(&self.root.join("subscription-proxy.json"))?
            .unwrap_or_default();
        result.validate()?;
        Ok(result)
    }
    pub fn configure(
        &self,
        policy: SubscriptionProxyPolicy,
    ) -> Result<SubscriptionProxyConfig, SubscriptionProxyError> {
        policy.validate()?;
        let policy = match policy {
            SubscriptionProxyPolicy::Manual { url, no_proxy } => SubscriptionProxyPolicy::Manual {
                url: canonical_proxy_url(&url)?,
                no_proxy,
            },
            policy => policy,
        };
        let config = SubscriptionProxyConfig {
            schema: SCHEMA.into(),
            revision: nonce()?,
            policy,
        };
        self.write("subscription-proxy.json", &config)?;
        Ok(config)
    }
    pub fn view(&self) -> Result<SubscriptionProxyView, SubscriptionProxyError> {
        let config = self.load()?;
        let applied =
            read::<SubscriptionProxyConfig>(&self.root.join("subscription-proxy-applied.json"))?
                .is_some_and(|value| value == config);
        Ok(SubscriptionProxyView { config, applied })
    }
    /// Called with the exact launch snapshot, never a freshly reloaded desired value.
    pub fn mark_applied(
        &self,
        config: &SubscriptionProxyConfig,
    ) -> Result<(), SubscriptionProxyError> {
        config.validate()?;
        self.write("subscription-proxy-applied.json", config)
    }
    pub fn clear_applied(&self) -> Result<(), SubscriptionProxyError> {
        match fs::remove_file(self.root.join("subscription-proxy-applied.json")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(SubscriptionProxyError),
        }
    }
    fn write(&self, name: &str, value: &impl Serialize) -> Result<(), SubscriptionProxyError> {
        use std::io::Write;
        if !self.root.is_absolute() {
            return Err(SubscriptionProxyError);
        }
        fs::create_dir_all(&self.root).map_err(|_| SubscriptionProxyError)?;
        let meta = fs::symlink_metadata(&self.root).map_err(|_| SubscriptionProxyError)?;
        if !meta.is_dir() {
            return Err(SubscriptionProxyError);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.uid() != nix::unistd::geteuid().as_raw() || meta.mode() & 0o022 != 0 {
                return Err(SubscriptionProxyError);
            }
        }
        let path = self.root.join(name);
        // Refuse unsafe existing records, including symlinks.
        let _ = read_bytes(&path)?;
        let temp = self
            .root
            .join(format!(".subscription-proxy-{}.tmp", nonce()?));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
            }
            let mut file = options.open(&temp).map_err(|_| SubscriptionProxyError)?;
            let bytes = serde_json::to_vec(value).map_err(|_| SubscriptionProxyError)?;
            if bytes.len() as u64 > LIMIT {
                return Err(SubscriptionProxyError);
            }
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| SubscriptionProxyError)?;
            fs::rename(&temp, path).map_err(|_| SubscriptionProxyError)
        })();
        let _ = fs::remove_file(temp);
        result
    }
}
fn nonce() -> Result<String, SubscriptionProxyError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| SubscriptionProxyError)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, SubscriptionProxyError> {
    read_bytes(path)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| SubscriptionProxyError))
        .transpose()
}
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, SubscriptionProxyError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(SubscriptionProxyError),
    };
    let meta = file.metadata().map_err(|_| SubscriptionProxyError)?;
    if !meta.is_file() || meta.len() > LIMIT {
        return Err(SubscriptionProxyError);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != nix::unistd::geteuid().as_raw() || meta.mode() & 0o077 != 0 {
            return Err(SubscriptionProxyError);
        }
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SubscriptionProxyError)?;
    if bytes.len() as u64 > LIMIT {
        return Err(SubscriptionProxyError);
    }
    Ok(Some(bytes))
}

#[cfg(test)]
#[path = "subscription_proxy_tests.rs"]
mod tests;
