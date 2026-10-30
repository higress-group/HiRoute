//! Source-owned network policy. Resolution is pure: no ambient environment, DNS or I/O.
use std::{collections::BTreeMap, fmt, sync::Arc};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::{CanonicalDigest, ProtectedSecret};

mod bypass;
mod failure;
pub use failure::*;
mod records;
mod runtime;
mod transaction;
pub use records::*;
pub use runtime::*;
pub use transaction::*;

#[derive(schemars::JsonSchema, Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeNetworkPolicyV1 {
    #[default]
    Direct,
    Inherit,
    Manual {
        proxy_url: String,
        #[serde(default)]
        no_proxy: String,
    },
}

impl<'de> Deserialize<'de> for NativeNetworkPolicyV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde's internally tagged *unit* variants otherwise ignore unknown fields even with
        // deny_unknown_fields. Empty struct variants enforce the active-mode wire boundary.
        #[derive(Deserialize)]
        #[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Direct {},
            Inherit {},
            Manual {
                proxy_url: String,
                #[serde(default)]
                no_proxy: String,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Direct {} => Self::Direct,
            Wire::Inherit {} => Self::Inherit,
            Wire::Manual {
                proxy_url,
                no_proxy,
            } => Self::Manual {
                proxy_url,
                no_proxy,
            },
        })
    }
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeNetworkModeV1 {
    Direct,
    Inherit,
    Manual,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum NativeNetworkErrorV1 {
    #[error("native.network.proxy_url_invalid")]
    InvalidProxyUrl,
    #[error("native.network.bypass_invalid")]
    InvalidBypass,
    #[error("native.network.destination_invalid")]
    InvalidDestination,
    #[error("native.network.environment_invalid")]
    InvalidEnvironment,
    #[error("native.network.state_invalid")]
    InvalidState,
}

impl NativeNetworkPolicyV1 {
    pub const fn mode(&self) -> NativeNetworkModeV1 {
        match self {
            Self::Direct => NativeNetworkModeV1::Direct,
            Self::Inherit => NativeNetworkModeV1::Inherit,
            Self::Manual { .. } => NativeNetworkModeV1::Manual,
        }
    }

    pub fn normalized(&self) -> Result<Self, NativeNetworkErrorV1> {
        match self {
            Self::Direct | Self::Inherit => Ok(self.clone()),
            Self::Manual {
                proxy_url,
                no_proxy,
            } => {
                if proxy_url.chars().any(char::is_control) {
                    return Err(NativeNetworkErrorV1::InvalidProxyUrl);
                }
                let proxy = parse_proxy(proxy_url.trim(), false)?;
                bypass::Rules::parse(no_proxy)?;
                Ok(Self::Manual {
                    proxy_url: proxy.to_string(),
                    no_proxy: no_proxy
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(","),
                })
            }
        }
    }
}

/// The host supplies its startup snapshot and a fresh opaque epoch, never a client shell.
/// Protected values are not serializable or printable, including inside an Arc or route.
#[derive(Clone)]
pub struct NativeProxyEnvironmentV1 {
    epoch: String,
    values: BTreeMap<String, Arc<ProtectedSecret>>,
}

impl fmt::Debug for NativeProxyEnvironmentV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeProxyEnvironmentV1([REDACTED])")
    }
}

impl NativeProxyEnvironmentV1 {
    pub fn from_variables(
        epoch: String,
        values: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, NativeNetworkErrorV1> {
        if epoch.is_empty() || epoch.len() > 256 || epoch.chars().any(char::is_control) {
            return Err(NativeNetworkErrorV1::InvalidEnvironment);
        }
        let mut captured = BTreeMap::new();
        let mut size = 0usize;
        for (key, value) in values {
            if !matches!(
                key.as_str(),
                "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "NO_PROXY"
                    | "http_proxy"
                    | "https_proxy"
                    | "no_proxy"
            ) || value.is_empty()
            {
                continue;
            }
            size = size.saturating_add(key.len() + value.len());
            if size > 16 * 1024 || value.contains(['\0', '\r', '\n']) || captured.contains_key(&key)
            {
                return Err(NativeNetworkErrorV1::InvalidEnvironment);
            }
            captured.insert(
                key,
                Arc::new(
                    ProtectedSecret::new(value.into_bytes())
                        .map_err(|_| NativeNetworkErrorV1::InvalidEnvironment)?,
                ),
            );
        }
        Ok(Self {
            epoch,
            values: captured,
        })
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    fn value(&self, upper: &str, lower: &str) -> Option<&str> {
        self.values
            .get(upper)
            .or_else(|| self.values.get(lower))
            .and_then(|v| std::str::from_utf8(v.expose()).ok())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeNetworkDestinationV1 {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl NativeNetworkDestinationV1 {
    pub fn from_url(value: &str) -> Result<Self, NativeNetworkErrorV1> {
        let url = Url::parse(value).map_err(|_| NativeNetworkErrorV1::InvalidDestination)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(NativeNetworkErrorV1::InvalidDestination);
        }
        Ok(Self {
            scheme: url.scheme().into(),
            host: bypass::url_host(&url).ok_or(NativeNetworkErrorV1::InvalidDestination)?,
            port: url
                .port_or_known_default()
                .filter(|p| *p != 0)
                .ok_or(NativeNetworkErrorV1::InvalidDestination)?,
        })
    }

    pub fn validate(&self) -> Result<(), NativeNetworkErrorV1> {
        if !matches!(self.scheme.as_str(), "http" | "https")
            || self.port == 0
            || bypass::canonical_host(&self.host).as_deref() != Some(self.host.as_str())
        {
            return Err(NativeNetworkErrorV1::InvalidDestination);
        }
        Ok(())
    }
}

#[derive(schemars::JsonSchema, Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeNetworkRouteReasonV1 {
    PolicyDirect,
    Loopback,
    BypassMatched,
    NoProxyConfigured,
    SelectedProxy,
}

#[derive(Clone)]
pub struct NativeProxyEndpointV1 {
    url: Arc<ProtectedSecret>,
    basic_auth: Option<Arc<ProtectedSecret>>,
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl fmt::Debug for NativeProxyEndpointV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeProxyEndpointV1([REDACTED])")
    }
}

impl NativeProxyEndpointV1 {
    /// Protected `username:password` bytes for the trusted proxy transport only.
    pub fn basic_auth_material(&self) -> Option<&ProtectedSecret> {
        self.basic_auth.as_deref()
    }
    /// Trusted transport boundary only. Never use this value in diagnostics or public views.
    pub fn protected_url(&self) -> &ProtectedSecret {
        &self.url
    }

    /// Internal pool identity, not a public environment fingerprint.
    pub fn transport_fingerprint(&self) -> CanonicalDigest {
        CanonicalDigest::of_bytes(self.url.expose())
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedNativeNetworkRouteV1 {
    pub mode: NativeNetworkModeV1,
    pub reason: NativeNetworkRouteReasonV1,
    pub destination: NativeNetworkDestinationV1,
    pub environment_epoch: Option<String>,
    pub proxy: Option<NativeProxyEndpointV1>,
}

/// Resolve before selecting a socket address. Bypass never performs DNS or retries a proxy.
pub fn resolve_native_network_route(
    policy: &NativeNetworkPolicyV1,
    environment: &NativeProxyEnvironmentV1,
    destination: &NativeNetworkDestinationV1,
) -> Result<ResolvedNativeNetworkRouteV1, NativeNetworkErrorV1> {
    destination.validate()?;
    let policy = policy.normalized()?;
    let mut route = ResolvedNativeNetworkRouteV1 {
        mode: policy.mode(),
        reason: NativeNetworkRouteReasonV1::PolicyDirect,
        destination: destination.clone(),
        environment_epoch: matches!(policy, NativeNetworkPolicyV1::Inherit)
            .then(|| environment.epoch.clone()),
        proxy: None,
    };
    let (url, bypass) = match &policy {
        NativeNetworkPolicyV1::Direct => return Ok(route),
        NativeNetworkPolicyV1::Inherit => (
            if destination.scheme == "https" {
                environment.value("HTTPS_PROXY", "https_proxy")
            } else {
                environment.value("HTTP_PROXY", "http_proxy")
            },
            environment
                .value("NO_PROXY", "no_proxy")
                .unwrap_or_default(),
        ),
        NativeNetworkPolicyV1::Manual {
            proxy_url,
            no_proxy,
        } => (Some(proxy_url.as_str()), no_proxy.as_str()),
    };
    // Admit selected configuration even when this destination would bypass it. Corrupt policy
    // must not be disguised as a direct route; callers can still save it as unavailable.
    let parsed = url
        .map(|u| parse_proxy(u, matches!(policy, NativeNetworkPolicyV1::Inherit)))
        .transpose()?;
    let rules = bypass::Rules::parse(bypass)?;
    route.reason = if bypass::is_loopback(&destination.host) {
        NativeNetworkRouteReasonV1::Loopback
    } else if rules.matches(destination) {
        NativeNetworkRouteReasonV1::BypassMatched
    } else if parsed.is_none() {
        NativeNetworkRouteReasonV1::NoProxyConfigured
    } else {
        NativeNetworkRouteReasonV1::SelectedProxy
    };
    if route.reason == NativeNetworkRouteReasonV1::SelectedProxy {
        let url = parsed.expect("selected proxy has an admitted URL");
        route.proxy = Some(NativeProxyEndpointV1 {
            basic_auth: basic_auth(&url)?.map(Arc::new),
            scheme: url.scheme().into(),
            host: bypass::url_host(&url).ok_or(NativeNetworkErrorV1::InvalidProxyUrl)?,
            port: url
                .port_or_known_default()
                .ok_or(NativeNetworkErrorV1::InvalidProxyUrl)?,
            url: Arc::new(
                ProtectedSecret::new(url.to_string().into_bytes())
                    .map_err(|_| NativeNetworkErrorV1::InvalidProxyUrl)?,
            ),
        });
    }
    Ok(route)
}

fn parse_proxy(value: &str, credentials_allowed: bool) -> Result<Url, NativeNetworkErrorV1> {
    let invalid = NativeNetworkErrorV1::InvalidProxyUrl;
    if value.is_empty()
        || value.len() > 8192
        || value.chars().any(char::is_control)
        || value.contains('\\')
        || value.trim() != value
    {
        return Err(invalid);
    }
    let url = Url::parse(value).map_err(|_| invalid)?;
    let userinfo = value
        .split_once("://")
        .is_some_and(|(_, v)| v.split('/').next().is_some_and(|a| a.contains('@')));
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || url.port_or_known_default().is_none_or(|p| p == 0)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || (!credentials_allowed && userinfo)
    {
        return Err(invalid);
    }
    if credentials_allowed {
        basic_auth(&url)?;
    }
    Ok(url)
}

fn basic_auth(url: &Url) -> Result<Option<ProtectedSecret>, NativeNetworkErrorV1> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(None);
    }
    let invalid = NativeNetworkErrorV1::InvalidProxyUrl;
    let user = percent_encoding::percent_decode_str(url.username())
        .decode_utf8()
        .map_err(|_| invalid)?;
    let password = percent_encoding::percent_decode_str(url.password().unwrap_or_default())
        .decode_utf8()
        .map_err(|_| invalid)?;
    if user.contains(':') {
        return Err(invalid);
    }
    let mut bytes = user.as_bytes().to_vec();
    bytes.push(b':');
    bytes.extend_from_slice(password.as_bytes());
    ProtectedSecret::new(bytes).map(Some).map_err(|_| invalid)
}

#[cfg(test)]
mod tests;
