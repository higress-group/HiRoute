//! TCP/TLS forward proxy and CONNECT stream composition through pinned Pingora APIs.
use super::*;
use pingora_core::connectors::{
    ConnectorOptions, TransportConnector,
    http::{v1, v2},
};
use pingora_core::upstreams::peer::Peer;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) enum NetworkConnector {
    Direct(Box<pingora_core::connectors::http::Connector>),
    Proxy(Box<ProxyConnector>),
}

pub(super) struct ProxyConnector {
    route: ProxyDialRoute,
    target: TransportTarget,
    h1: v1::Connector,
    h2: v2::Connector,
    tcp: TransportConnector,
    proxy_tls: std::sync::OnceLock<Result<pingora_rustls::TlsConnector, ()>>,
    upstream_tls: std::sync::OnceLock<Result<pingora_rustls::TlsConnector, ()>>,
}

impl NetworkConnector {
    pub fn new(options: Option<ConnectorOptions>) -> Self {
        Self::Direct(Box::new(pingora_core::connectors::http::Connector::new(
            options,
        )))
    }

    pub fn for_route(target: &TransportTarget, route: Option<&NetworkDialRoute>) -> Self {
        match route.and_then(|r| r.proxy.as_ref()) {
            None => Self::new(None),
            Some(route) => Self::Proxy(Box::new(ProxyConnector {
                route: route.clone(),
                target: target.clone(),
                h1: v1::Connector::new(None),
                h2: v2::Connector::new(None),
                tcp: TransportConnector::new(None),
                proxy_tls: std::sync::OnceLock::new(),
                upstream_tls: std::sync::OnceLock::new(),
            })),
        }
    }

    pub async fn get_http_session(
        &self,
        peer: &HttpPeer,
    ) -> Result<(ClientSession, bool), AttemptError> {
        match self {
            Self::Direct(connector) => connector
                .get_http_session(peer)
                .await
                .map_err(|e| AttemptError::Transport(e.to_string().into())),
            Self::Proxy(connector) => connector.get(peer).await,
        }
    }

    pub async fn release_http_session(
        &self,
        session: ClientSession,
        peer: &HttpPeer,
        timeout: Option<Duration>,
    ) {
        match self {
            Self::Direct(connector) => connector.release_http_session(session, peer, timeout).await,
            Self::Proxy(connector) => match session {
                ClientSession::H1(h1) => connector.h1.release_http_session(h1, peer, timeout).await,
                ClientSession::H2(h2) => connector.h2.release_http_session(h2, peer, timeout),
                ClientSession::Custom(_) => {
                    unreachable!("HiRoute does not negotiate a custom proxy protocol")
                }
            },
        }
    }
}

impl ProxyConnector {
    async fn get(&self, peer: &HttpPeer) -> Result<(ClientSession, bool), AttemptError> {
        let forward = self.target.scheme == TransportScheme::Http;
        let h1_only = forward || peer.options.alpn.get_max_http_version() == 1;
        if !h1_only
            && let Some(session) = self
                .h2
                .reused_http_session(peer)
                .await
                .map_err(|_| AttemptError::Transport("upstream_h2_pool_unavailable".into()))?
        {
            return Ok((ClientSession::H2(session), true));
        }
        if let Some(session) = self.h1.reused_http_session(peer).await {
            return Ok((ClientSession::H1(session), true));
        }
        let stream = self.stream(peer).await?;
        if h1_only || !matches!(stream.selected_alpn_proto(), Some(ALPN::H2)) {
            return Ok((
                ClientSession::H1(Http1ClientSession::new_with_options(stream, peer)),
                false,
            ));
        }
        let mut settings = v2::H2HandshakeSettings::new();
        settings.max_streams = peer.options.max_h2_streams;
        settings.ping_interval = peer.h2_ping_interval();
        settings.stream_window_size = peer.options.h2_stream_window_size;
        settings.connection_window_size = peer.options.h2_connection_window_size;
        let connection = v2::handshake(stream, settings)
            .await
            .map_err(|_| AttemptError::Transport("upstream_h2_handshake_failed".into()))?;
        let session = connection
            .spawn_stream()
            .await
            .map_err(|_| AttemptError::Transport("upstream_h2_stream_failed".into()))?
            .ok_or_else(|| AttemptError::Transport("upstream_h2_stream_unavailable".into()))?;
        if connection.more_streams_allowed() {
            self.h2.in_use_pool().insert(peer.reuse_hash(), connection);
        }
        Ok((ClientSession::H2(session), false))
    }

    async fn stream(&self, peer: &HttpPeer) -> Result<Stream, AttemptError> {
        let address = *peer
            .address()
            .as_inet()
            .ok_or(AttemptError::Proxy(ProxyFailurePhase::Connect))?;
        let mut tcp_peer = HttpPeer::new(address, false, String::new());
        tcp_peer.options.connection_timeout = peer.options.connection_timeout;
        tcp_peer.options.total_connection_timeout = peer.options.total_connection_timeout;
        let mut stream = self
            .tcp
            .new_stream(&tcp_peer)
            .await
            .map_err(|_| AttemptError::Proxy(ProxyFailurePhase::Connect))?;
        if self.route.scheme == TransportScheme::Https {
            let authority = self
                .route
                .authority
                .parse::<http::uri::Authority>()
                .map_err(|_| AttemptError::Proxy(ProxyFailurePhase::Tls))?;
            let host = authority
                .host()
                .trim_start_matches('[')
                .trim_end_matches(']');
            let config = self
                .proxy_tls
                .get_or_init(|| verified_tls_connector(&self.route.ca, &ALPN::H1))
                .as_ref()
                .map_err(|_| AttemptError::Proxy(ProxyFailurePhase::Tls))?;
            stream = Box::new(
                pingora_core::protocols::tls::client::handshake(
                    config,
                    host,
                    stream::LayeredStream(stream),
                )
                .await
                .map_err(|_| AttemptError::Proxy(ProxyFailurePhase::Tls))?,
            );
        }
        if self.target.scheme == TransportScheme::Https {
            self.connect_tunnel(&mut stream).await?;
            let config = self
                .upstream_tls
                .get_or_init(|| verified_tls_connector(&self.target.ca, &peer.options.alpn))
                .as_ref()
                .map_err(|_| AttemptError::Proxy(ProxyFailurePhase::UpstreamTls))?;
            stream = Box::new(
                pingora_core::protocols::tls::client::handshake(
                    config,
                    peer.sni(),
                    stream::LayeredStream(stream),
                )
                .await
                .map_err(|_| AttemptError::Proxy(ProxyFailurePhase::UpstreamTls))?,
            );
        }
        Ok(stream)
    }

    async fn connect_tunnel(&self, stream: &mut Stream) -> Result<(), AttemptError> {
        let error = || AttemptError::Proxy(ProxyFailurePhase::Negotiation);
        let parsed = self
            .target
            .authority
            .parse::<http::uri::Authority>()
            .map_err(|_| error())?;
        let authority = if parsed.port_u16().is_some() {
            self.target.authority.to_string()
        } else {
            format!("{}:443", parsed.host())
        };
        let mut request =
            format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n").into_bytes();
        if let Some(auth) = self.route.authorization() {
            request.extend_from_slice(b"Proxy-Authorization: ");
            request.extend_from_slice(auth.as_bytes());
            request.extend_from_slice(b"\r\n");
        }
        request.extend_from_slice(b"\r\n");
        stream.write_all(&request).await.map_err(|_| error())?;
        stream.flush().await.map_err(|_| error())?;
        let mut response = Vec::with_capacity(512);
        // Read exactly the bounded header so no bytes are lost between outer TLS, CONNECT
        // and inner TLS. The enclosing connect/attempt deadlines bound hostile slow headers.
        while response.len() < 32 * 1024 {
            response.push(stream.read_u8().await.map_err(|_| error())?);
            if response.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let mut headers = [httparse::EMPTY_HEADER; 128];
        let mut parsed = httparse::Response::new(&mut headers);
        if !matches!(parsed.parse(&response), Ok(httparse::Status::Complete(n)) if n == response.len())
        {
            return Err(error());
        }
        match parsed.code {
            Some(200..=299) => Ok(()),
            Some(407) => Err(AttemptError::Proxy(ProxyFailurePhase::Authentication)),
            _ => Err(error()),
        }
    }
}

mod stream;

#[cfg(test)]
mod tests;

fn verified_tls_connector(ca: &CaPolicy, alpn: &ALPN) -> Result<pingora_rustls::TlsConnector, ()> {
    use pingora_rustls::{ClientConfig, RootCertStore, version};
    pingora_rustls::install_default_crypto_provider();
    let mut roots = RootCertStore::empty();
    match ca {
        CaPolicy::System => {
            pingora_rustls::load_platform_certs_incl_env_into_store(&mut roots).map_err(|_| ())?
        }
        CaPolicy::Pem(pem) => {
            let certs = rustls_pemfile::certs(&mut Cursor::new(pem.as_ref()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ())?;
            if certs.is_empty() {
                return Err(());
            }
            for cert in certs {
                roots.add(cert).map_err(|_| ())?;
            }
        }
    }
    let mut config =
        ClientConfig::builder_with_protocol_versions(&[&version::TLS12, &version::TLS13])
            .with_root_certificates(roots)
            .with_no_client_auth();
    config.alpn_protocols = match alpn {
        ALPN::H1 => vec![b"http/1.1".to_vec()],
        ALPN::H2 => vec![b"h2".to_vec()],
        ALPN::H2H1 => vec![b"h2".to_vec(), b"http/1.1".to_vec()],
        _ => return Err(()),
    };
    Ok(pingora_rustls::TlsConnector::from(Arc::new(config)))
}
