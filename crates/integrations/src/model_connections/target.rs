use std::{
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::mpsc,
    time::Duration,
};

use hiroute_application_api::ComputeCandidateTargetV2;
use hiroute_domain::{
    GatewayAuthenticationSemanticsV1, GatewayHeaderSemanticsV1, UpstreamProtocol,
};
use http::{HeaderName, HeaderValue, Uri};
use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelConnectionBaseKindV1 {
    /// User-entered API root. `/` receives `/v1`; the selected protocol suffix is appended.
    ApiRoot,
    /// Claude-style native base URL; the native client appends `/v1/messages`.
    NativeMessagesBase,
    /// Codex/OpenAI-style native base URL; the native client appends protocol paths directly.
    NativeResponsesBase,
}

#[derive(Clone, Copy, Debug)]
pub struct ModelConnectionTargetInputV1<'a> {
    pub base_url: &'a str,
    pub base_kind: ModelConnectionBaseKindV1,
    pub protocol: UpstreamProtocol,
    pub request_path_override: Option<&'a str>,
    pub inventory_path_override: Option<&'a str>,
    pub protocol_profile_id: &'a str,
    pub protocol_profile_revision: u64,
    pub protocol_header_semantics: &'a GatewayHeaderSemanticsV1,
    pub authentication: &'a GatewayAuthenticationSemanticsV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedModelConnectionTargetV1 {
    pub candidate_target: ComputeCandidateTargetV2,
    pub inventory_path: Option<String>,
    host_header: String,
    network_authority: String,
    directory_headers: Vec<(HeaderName, HeaderValue)>,
}

impl NormalizedModelConnectionTargetV1 {
    pub fn request_url(&self) -> String {
        self.url_for(&self.candidate_target.request_path)
    }

    pub fn inventory_url(&self, query: Option<&str>) -> String {
        let mut path = self
            .inventory_path
            .clone()
            .expect("directory probe requires a path");
        if let Some(query) = query {
            path.push('?');
            path.push_str(query);
        }
        self.url_for(&path)
    }

    pub fn host_header(&self) -> &str {
        &self.host_header
    }

    pub(crate) fn directory_headers(&self) -> &[(HeaderName, HeaderValue)] {
        &self.directory_headers
    }

    fn url_for(&self, path: &str) -> String {
        format!(
            "{}://{}{}",
            self.candidate_target.scheme, self.network_authority, path
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ModelConnectionTargetError {
    #[error("model connection URL is invalid")]
    InvalidUrl,
    #[error("HTTP is allowed only when every resolved target address is loopback")]
    NonLoopbackHttp,
    #[error("model connection URL contains an unsafe path")]
    UnsafePath,
    #[error("model connection authentication header is unsafe")]
    UnsafeAuthenticationHeader,
    #[error("model connection protocol profile headers are unsupported or unsafe")]
    UnsafeProtocolHeader,
    #[error("loopback hostname resolution timed out")]
    ResolutionTimeout,
}

pub fn normalize_model_connection_target(
    input: ModelConnectionTargetInputV1<'_>,
) -> Result<NormalizedModelConnectionTargetV1, ModelConnectionTargetError> {
    normalize_model_connection_target_with_timeout(input, Duration::from_secs(2))
}

pub(crate) fn normalize_model_connection_target_with_timeout(
    input: ModelConnectionTargetInputV1<'_>,
    resolution_timeout: Duration,
) -> Result<NormalizedModelConnectionTargetV1, ModelConnectionTargetError> {
    normalize_target(input, Some(resolution_timeout))
}

/// Pure scan projection. It never resolves hostnames or opens a socket.
pub fn project_model_connection_target(
    input: ModelConnectionTargetInputV1<'_>,
) -> Result<NormalizedModelConnectionTargetV1, ModelConnectionTargetError> {
    normalize_target(input, None)
}

fn normalize_target(
    input: ModelConnectionTargetInputV1<'_>,
    resolution_timeout: Option<Duration>,
) -> Result<NormalizedModelConnectionTargetV1, ModelConnectionTargetError> {
    let ModelConnectionTargetInputV1 {
        base_url,
        base_kind,
        protocol,
        request_path_override,
        inventory_path_override,
        protocol_profile_id,
        protocol_profile_revision,
        protocol_header_semantics,
        authentication,
    } = input;
    if base_url.trim() != base_url
        || base_url.contains('#')
        || protocol_profile_id.trim().is_empty()
        || protocol_profile_revision == 0
    {
        return Err(ModelConnectionTargetError::InvalidUrl);
    }
    validate_authentication_header(authentication)?;
    let directory_headers =
        validated_directory_headers(protocol, authentication, protocol_header_semantics)?;
    let uri: Uri = base_url
        .parse()
        .map_err(|_| ModelConnectionTargetError::InvalidUrl)?;
    let scheme = uri
        .scheme_str()
        .ok_or(ModelConnectionTargetError::InvalidUrl)?
        .to_ascii_lowercase();
    if !matches!(scheme.as_str(), "http" | "https")
        || uri
            .path_and_query()
            .is_some_and(|value| value.query().is_some())
    {
        return Err(ModelConnectionTargetError::InvalidUrl);
    }
    let authority = uri
        .authority()
        .ok_or(ModelConnectionTargetError::InvalidUrl)?;
    if authority.as_str().contains('@') {
        return Err(ModelConnectionTargetError::InvalidUrl);
    }
    let host = authority.host().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return Err(ModelConnectionTargetError::InvalidUrl);
    }
    let port = authority
        .port_u16()
        .unwrap_or(if scheme == "https" { 443 } else { 80 });
    if port == 0 {
        return Err(ModelConnectionTargetError::InvalidUrl);
    }
    let base_path = normalize_base_path(uri.path())?;
    let (request_path, _) = paths(base_kind, protocol, &base_path);
    let request_path = request_path_override
        .map(validate_override_path)
        .transpose()?
        .unwrap_or(request_path);
    let inventory_path = inventory_path_override
        .map(validate_override_path)
        .transpose()?;

    let host_header = authority_for(&host, port, &scheme);
    let network_authority = if scheme == "http" {
        let addresses = match resolution_timeout {
            Some(timeout) => resolve_loopback(&host, port, timeout)?,
            None => match host.parse::<IpAddr>() {
                Ok(ip) if ip.is_loopback() => vec![SocketAddr::new(ip, port)],
                _ if host == "localhost" => {
                    vec![SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port)]
                }
                _ => return Err(ModelConnectionTargetError::NonLoopbackHttp),
            },
        };
        let address = addresses
            .first()
            .ok_or(ModelConnectionTargetError::NonLoopbackHttp)?;
        socket_authority(*address)
    } else {
        host_header.clone()
    };
    let published_authority = if scheme == "http" {
        addresses_host(&network_authority)?
    } else {
        host.clone()
    };
    Ok(NormalizedModelConnectionTargetV1 {
        candidate_target: ComputeCandidateTargetV2 {
            scheme,
            authority: published_authority,
            port,
            request_path,
            upstream_protocol: protocol,
            protocol_profile_id: protocol_profile_id.to_owned(),
            protocol_profile_revision,
        },
        inventory_path,
        host_header,
        network_authority,
        directory_headers,
    })
}

fn addresses_host(authority: &str) -> Result<String, ModelConnectionTargetError> {
    let address = authority
        .parse::<SocketAddr>()
        .map_err(|_| ModelConnectionTargetError::InvalidUrl)?;
    Ok(address.ip().to_string())
}

fn validated_directory_headers(
    protocol: UpstreamProtocol,
    authentication: &GatewayAuthenticationSemanticsV1,
    semantics: &GatewayHeaderSemanticsV1,
) -> Result<Vec<(HeaderName, HeaderValue)>, ModelConnectionTargetError> {
    let authentication_header = match authentication {
        GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } => Some(
            HeaderName::from_bytes(header.as_bytes())
                .map_err(|_| ModelConnectionTargetError::UnsafeAuthenticationHeader)?,
        ),
        GatewayAuthenticationSemanticsV1::Bearer | GatewayAuthenticationSemanticsV1::None => None,
    };
    let mut headers = Vec::new();
    for (raw_name, raw_value) in &semantics.required_headers {
        let name = HeaderName::from_bytes(raw_name.as_bytes())
            .map_err(|_| ModelConnectionTargetError::UnsafeProtocolHeader)?;
        if protocol != UpstreamProtocol::Messages
            || name.as_str() != "anthropic-version"
            || authentication_header.as_ref() == Some(&name)
            || headers.iter().any(|(current, _)| current == name)
            || raw_value.is_empty()
            || raw_value.trim() != raw_value
            || raw_value.len() > 256
        {
            return Err(ModelConnectionTargetError::UnsafeProtocolHeader);
        }
        let value = HeaderValue::from_bytes(raw_value.as_bytes())
            .map_err(|_| ModelConnectionTargetError::UnsafeProtocolHeader)?;
        headers.push((name, value));
    }
    Ok(headers)
}

pub fn validate_authentication_header(
    authentication: &GatewayAuthenticationSemanticsV1,
) -> Result<(), ModelConnectionTargetError> {
    let GatewayAuthenticationSemanticsV1::ApiKeyHeader { header } = authentication else {
        return Ok(());
    };
    let parsed = HeaderName::from_bytes(header.as_bytes())
        .map_err(|_| ModelConnectionTargetError::UnsafeAuthenticationHeader)?;
    let name = parsed.as_str();
    const DENIED: &[&str] = &[
        "authorization",
        "connection",
        "content-length",
        "cookie",
        "host",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "transfer-encoding",
        "upgrade",
    ];
    if DENIED.contains(&name) {
        Err(ModelConnectionTargetError::UnsafeAuthenticationHeader)
    } else {
        Ok(())
    }
}

fn normalize_base_path(path: &str) -> Result<String, ModelConnectionTargetError> {
    validate_path(path)?;
    let trimmed = path.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        String::new()
    } else {
        trimmed.to_owned()
    })
}

fn validate_override_path(path: &str) -> Result<String, ModelConnectionTargetError> {
    validate_path(path)?;
    if path == "/" {
        return Err(ModelConnectionTargetError::UnsafePath);
    }
    Ok(path.to_owned())
}

fn validate_path(path: &str) -> Result<(), ModelConnectionTargetError> {
    if !path.starts_with('/')
        || path.contains(['?', '#', '\\', '%'])
        || (path.len() > 1 && path.contains("//"))
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
    {
        Err(ModelConnectionTargetError::UnsafePath)
    } else {
        Ok(())
    }
}

fn paths(
    kind: ModelConnectionBaseKindV1,
    protocol: UpstreamProtocol,
    base_path: &str,
) -> (String, String) {
    match kind {
        ModelConnectionBaseKindV1::ApiRoot => {
            let root = if base_path.is_empty() {
                "/v1"
            } else {
                base_path
            };
            (
                append(root, protocol_suffix(protocol)),
                append(root, "models"),
            )
        }
        ModelConnectionBaseKindV1::NativeMessagesBase => (
            append(base_path, "v1/messages"),
            append(base_path, "v1/models"),
        ),
        ModelConnectionBaseKindV1::NativeResponsesBase => (
            append(base_path, protocol_suffix(protocol)),
            append(base_path, "models"),
        ),
    }
}

fn protocol_suffix(protocol: UpstreamProtocol) -> &'static str {
    match protocol {
        UpstreamProtocol::Responses => "responses",
        UpstreamProtocol::Messages => "messages",
        UpstreamProtocol::ChatCompletions => "chat/completions",
    }
}

fn append(base: &str, suffix: &str) -> String {
    if base.is_empty() {
        format!("/{suffix}")
    } else {
        format!("{base}/{suffix}")
    }
}

fn resolve_loopback(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<Vec<SocketAddr>, ModelConnectionTargetError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if ip.is_loopback() {
            Ok(vec![SocketAddr::new(ip, port)])
        } else {
            Err(ModelConnectionTargetError::NonLoopbackHttp)
        };
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let host = host.to_owned();
    std::thread::spawn(move || {
        let result = (host.as_str(), port)
            .to_socket_addrs()
            .map(|values| values.collect::<Vec<_>>());
        let _ = tx.send(result);
    });
    let mut addresses = rx
        .recv_timeout(timeout)
        .map_err(|_| ModelConnectionTargetError::ResolutionTimeout)?
        .map_err(|_| ModelConnectionTargetError::NonLoopbackHttp)?;
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() || addresses.iter().any(|address| !address.ip().is_loopback()) {
        Err(ModelConnectionTargetError::NonLoopbackHttp)
    } else {
        Ok(addresses)
    }
}

fn authority_for(host: &str, port: u16, scheme: &str) -> String {
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    if (scheme == "https" && port == 443) || (scheme == "http" && port == 80) {
        host
    } else {
        format!("{host}:{port}")
    }
}

fn socket_authority(address: SocketAddr) -> String {
    match address {
        SocketAddr::V4(value) => value.to_string(),
        SocketAddr::V6(value) => format!("[{}]:{}", value.ip(), value.port()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(
        base_url: &'a str,
        base_kind: ModelConnectionBaseKindV1,
        protocol: UpstreamProtocol,
        protocol_profile_id: &'a str,
        authentication: &'a GatewayAuthenticationSemanticsV1,
    ) -> ModelConnectionTargetInputV1<'a> {
        static EMPTY_HEADERS: std::sync::LazyLock<GatewayHeaderSemanticsV1> =
            std::sync::LazyLock::new(|| GatewayHeaderSemanticsV1 {
                content_type: "application/json".into(),
                required_headers: Vec::new(),
                forbidden_forward_headers: vec!["authorization".into(), "x-api-key".into()],
            });
        ModelConnectionTargetInputV1 {
            base_url,
            base_kind,
            protocol,
            request_path_override: None,
            inventory_path_override: None,
            protocol_profile_id,
            protocol_profile_revision: 1,
            protocol_header_semantics: &EMPTY_HEADERS,
            authentication,
        }
    }

    #[test]
    fn api_root_and_native_base_have_distinct_suffix_rules() {
        let api = normalize_model_connection_target(input(
            "https://example.test/v1/",
            ModelConnectionBaseKindV1::ApiRoot,
            UpstreamProtocol::Messages,
            "profile/messages",
            &GatewayAuthenticationSemanticsV1::None,
        ))
        .unwrap();
        assert_eq!(api.candidate_target.request_path, "/v1/messages");
        assert_eq!(api.inventory_path, None);

        let native = normalize_model_connection_target(input(
            "https://example.test/proxy",
            ModelConnectionBaseKindV1::NativeMessagesBase,
            UpstreamProtocol::Messages,
            "profile/messages",
            &GatewayAuthenticationSemanticsV1::Bearer,
        ))
        .unwrap();
        assert_eq!(native.candidate_target.request_path, "/proxy/v1/messages");
        assert_eq!(native.inventory_path, None);

        let chat = normalize_model_connection_target(input(
            "https://example.test/compatible",
            ModelConnectionBaseKindV1::ApiRoot,
            UpstreamProtocol::ChatCompletions,
            "profile/chat-completions",
            &GatewayAuthenticationSemanticsV1::Bearer,
        ))
        .unwrap();
        assert_eq!(
            chat.candidate_target.request_path,
            "/compatible/chat/completions"
        );
        assert_eq!(chat.inventory_path, None);
    }

    #[test]
    fn plaintext_and_unsafe_headers_fail_before_network() {
        let loopback = normalize_model_connection_target(input(
            "http://127.0.0.1:8123/v1",
            ModelConnectionBaseKindV1::ApiRoot,
            UpstreamProtocol::Responses,
            "profile/responses",
            &GatewayAuthenticationSemanticsV1::None,
        ))
        .unwrap();
        assert_eq!(loopback.candidate_target.authority, "127.0.0.1");
        assert_eq!(loopback.candidate_target.port, 8123);
        assert_eq!(loopback.request_url(), "http://127.0.0.1:8123/v1/responses");

        assert_eq!(
            normalize_model_connection_target(input(
                "http://192.0.2.10/v1",
                ModelConnectionBaseKindV1::ApiRoot,
                UpstreamProtocol::Responses,
                "profile/responses",
                &GatewayAuthenticationSemanticsV1::Bearer,
            )),
            Err(ModelConnectionTargetError::NonLoopbackHttp)
        );
        assert_eq!(
            validate_authentication_header(&GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: "Proxy-Authorization".into()
            }),
            Err(ModelConnectionTargetError::UnsafeAuthenticationHeader)
        );
    }

    #[test]
    fn embedded_secrets_and_ambiguous_paths_are_rejected() {
        for url in [
            "https://user:secret@example.test/v1",
            "https://example.test/v1?api_key=secret",
            "https://example.test/v1/%2e%2e/private",
            "https://example.test/v1//models",
        ] {
            assert!(
                normalize_model_connection_target(input(
                    url,
                    ModelConnectionBaseKindV1::ApiRoot,
                    UpstreamProtocol::Responses,
                    "profile/responses",
                    &GatewayAuthenticationSemanticsV1::Bearer,
                ))
                .is_err(),
                "unsafe URL unexpectedly accepted: {url}"
            );
        }
        assert!(
            validate_authentication_header(&GatewayAuthenticationSemanticsV1::ApiKeyHeader {
                header: "bad\r\nheader".into(),
            })
            .is_err()
        );
    }
}
