use super::*;
use crate::test_support::plain_target;
use http::{HeaderMap, HeaderValue, Method, header};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

trait TestIo: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> TestIo for T {}
type Io = Box<dyn TestIo>;

fn certificate(host: &str, h2: bool) -> (TlsAcceptor, Arc<str>) {
    pingora_rustls::install_default_crypto_provider();
    let key = rcgen::generate_simple_self_signed(vec![host.into()]).unwrap();
    let pem: Arc<str> = key.cert.pem().into();
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![key.cert.der().clone()],
            PrivatePkcs8KeyDer::from(key.key_pair.serialize_der()).into(),
        )
        .unwrap();
    config.alpn_protocols = if h2 {
        vec![b"h2".to_vec()]
    } else {
        vec![b"http/1.1".to_vec()]
    };
    (TlsAcceptor::from(Arc::new(config)), pem)
}

async fn header(io: &mut Io) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        bytes.push(io.read_u8().await.unwrap());
        assert!(bytes.len() < 8192);
    }
    String::from_utf8(bytes).unwrap()
}

async fn round_trip(
    proxy_tls: bool,
    upstream_tls: bool,
    h2: bool,
    reject: bool,
    wrong_proxy_name: bool,
    wrong_upstream_name: bool,
) -> Result<(), AttemptError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (proxy_acceptor, proxy_ca) = certificate(
        if wrong_proxy_name {
            "wrong-proxy.invalid"
        } else {
            "proxy.invalid"
        },
        false,
    );
    let (upstream_acceptor, upstream_ca) = certificate(
        if wrong_upstream_name {
            "wrong-origin.invalid"
        } else {
            "origin.invalid"
        },
        h2,
    );
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut io: Io = if proxy_tls {
            let Ok(tls) = proxy_acceptor.accept(socket).await else {
                return;
            };
            Box::new(tls)
        } else {
            Box::new(socket)
        };
        if upstream_tls {
            let connect = header(&mut io).await;
            assert!(connect.starts_with("CONNECT origin.invalid:443 HTTP/1.1\r\n"));
            assert!(connect.contains("Proxy-Authorization: Basic cHJveHk6c2VjcmV0"));
            assert!(!connect.contains("provider-secret"));
            assert!(!connect.to_ascii_lowercase().contains("\r\nauthorization:"));
            if reject {
                io.write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                )
                .await
                .unwrap();
                return;
            }
            io.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .unwrap();
            let Ok(tls) = upstream_acceptor.accept(io).await else {
                return;
            };
            io = Box::new(tls);
        }
        if h2 {
            let mut connection = h2::server::handshake(io).await.unwrap();
            let (request, mut respond) = connection.accept().await.unwrap().unwrap();
            assert_eq!(
                request.uri().authority().unwrap().as_str(),
                "origin.invalid"
            );
            assert_eq!(request.uri().path(), "/v1/test");
            assert_eq!(
                request.headers()[header::AUTHORIZATION],
                "Bearer provider-secret"
            );
            assert!(!request.headers().contains_key(header::PROXY_AUTHORIZATION));
            respond
                .send_response(http::Response::new(()), false)
                .unwrap()
                .send_data(Bytes::from_static(b"ok"), true)
                .unwrap();
            tokio::select! { _ = done_rx => {}, _ = connection.accept() => {} }
        } else {
            let request = header(&mut io).await;
            let expected = if upstream_tls {
                "POST /v1/test HTTP/1.1"
            } else {
                "POST http://origin.invalid/v1/test HTTP/1.1"
            };
            assert!(request.starts_with(expected), "{request}");
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("host: origin.invalid\r\n")
            );
            assert!(request.contains("Bearer provider-secret"));
            assert_eq!(
                request
                    .to_ascii_lowercase()
                    .contains("proxy-authorization:"),
                !upstream_tls
            );
            io.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        }
    });
    let mut target = plain_target("192.0.2.1:443".parse().unwrap(), 500);
    target.authority = "origin.invalid".into();
    target.scheme = if upstream_tls {
        TransportScheme::Https
    } else {
        TransportScheme::Http
    };
    target.sni = upstream_tls.then(|| Arc::from("origin.invalid"));
    target.ca = CaPolicy::Pem(upstream_ca.into());
    target.alpn = Arc::from([Arc::from(if h2 { "h2" } else { "http/1.1" })]);
    target = target.with_derived_connection_fingerprint();
    let route = NetworkDialRoute {
        scope: [7; 32],
        identity: [8; 32],
        generation: 1,
        proxy: Some(
            ProxyDialRoute::new(
                if proxy_tls {
                    TransportScheme::Https
                } else {
                    TransportScheme::Http
                },
                "proxy.invalid:8443".into(),
                Arc::from([address]),
                CaPolicy::Pem(proxy_ca.into()),
                Some(HeaderValue::from_static("Basic cHJveHk6c2VjcmV0")),
            )
            .unwrap(),
        ),
    };
    let mut client =
        PingoraClientSession::new(Arc::new(PingoraConnectorRegistry::default()), [0; 32]);
    let result = async {
        client.configure_network_route(Some(&route))?;
        client.connect(&target, address).await?;
        assert_eq!(
            client.protocol(),
            if h2 {
                HttpProtocol::Http2
            } else {
                HttpProtocol::Http1
            }
        );
        client
            .write_request_head(&PreparedRequestHead {
                method: Method::POST,
                path_and_query: "/v1/test".into(),
                headers: HeaderMap::from_iter([
                    (header::HOST, HeaderValue::from_static("origin.invalid")),
                    (header::CONTENT_LENGTH, HeaderValue::from_static("0")),
                    (
                        header::AUTHORIZATION,
                        HeaderValue::from_static("Bearer provider-secret"),
                    ),
                ]),
            })
            .await?;
        client.finish_request_body().await?;
        let head = client.read_response_head().await?;
        assert!(matches!(
            head,
            TransportPrecommitEvent::ResponseHead {
                status: http::StatusCode::OK,
                ..
            }
        ));
        let mut body = Vec::new();
        loop {
            match std::future::poll_fn(|cx| client.poll_precommit(cx)).await? {
                Some(TransportPrecommitEvent::Body(bytes)) => body.extend_from_slice(&bytes),
                Some(TransportPrecommitEvent::EndStream) | None => break,
                _ => {}
            }
        }
        assert_eq!(body, b"ok");
        Ok(())
    }
    .await;
    let _ = done_tx.send(());
    client.cancel_reset().await;
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap_or_else(|error| panic!("proxy server failed: {error}; client: {result:?}; proxy_tls={proxy_tls}, upstream_tls={upstream_tls}, h2={h2}"));
    result
}

#[tokio::test]
async fn real_proxy_all_http_https_combinations_and_tunneled_h2() {
    tokio::time::timeout(Duration::from_secs(20), async {
        for proxy_tls in [false, true] {
            for upstream_tls in [false, true] {
                round_trip(proxy_tls, upstream_tls, false, false, false, false)
                    .await
                    .unwrap();
            }
            round_trip(proxy_tls, true, true, false, false, false)
                .await
                .unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn proxy_authentication_and_separate_tls_identities_fail_explicitly() {
    tokio::time::timeout(Duration::from_secs(15), async {
        assert_eq!(
            round_trip(false, true, false, true, false, false)
                .await
                .unwrap_err(),
            AttemptError::Proxy(ProxyFailurePhase::Authentication)
        );
        assert_eq!(
            round_trip(true, true, false, false, true, false)
                .await
                .unwrap_err(),
            AttemptError::Proxy(ProxyFailurePhase::Tls)
        );
        assert_eq!(
            round_trip(true, true, false, false, false, true)
                .await
                .unwrap_err(),
            AttemptError::Proxy(ProxyFailurePhase::UpstreamTls)
        );
    })
    .await
    .unwrap();
}

#[test]
fn old_network_generation_gets_an_isolated_pool_without_rolling_live_generation_back() {
    let target = plain_target("127.0.0.1:8123".parse().unwrap(), 510);
    let registry = PingoraConnectorRegistry::default();
    let old = NetworkDialRoute {
        scope: [1; 32],
        identity: [2; 32],
        generation: 1,
        proxy: None,
    };
    let new = NetworkDialRoute {
        identity: [3; 32],
        generation: 2,
        ..old.clone()
    };
    let first = registry
        .connector_for_network(&target, [0; 32], Some(&old))
        .unwrap();
    let current = registry
        .connector_for_network(&target, [0; 32], Some(&new))
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &current));
    let late = registry
        .connector_for_network(&target, [0; 32], Some(&old))
        .unwrap();
    assert!(!Arc::ptr_eq(&late, &current));
    assert!(!Arc::ptr_eq(&late, &first));
    assert!(Arc::ptr_eq(
        &current,
        &registry
            .connector_for_network(&target, [0; 32], Some(&new))
            .unwrap()
    ));
}

async fn rejected_tunnel(response: Vec<u8>) {
    let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    origin.set_nonblocking(true).unwrap();
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = proxy.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let mut io: Io = Box::new(socket);
        assert!(
            header(&mut io)
                .await
                .starts_with("CONNECT origin.invalid:443 HTTP/1.1\r\n")
        );
        let _ = io.write_all(&response).await;
        let _ = io.shutdown().await;
    });
    let (mut client, target) = tunnel_client(address, origin.local_addr().unwrap());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(3), client.connect(&target, address))
            .await
            .unwrap()
            .unwrap_err(),
        AttemptError::Proxy(ProxyFailurePhase::Negotiation)
    );
    client.cancel_reset().await;
    server.await.unwrap();
    assert_eq!(
        origin.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn tunnel_client(
    proxy: std::net::SocketAddr,
    origin: std::net::SocketAddr,
) -> (PingoraClientSession, TransportTarget) {
    let mut target = plain_target(origin, 600);
    target.authority = "origin.invalid".into();
    target.scheme = TransportScheme::Https;
    target.sni = Some("origin.invalid".into());
    target = target.with_derived_connection_fingerprint();
    let route = NetworkDialRoute {
        scope: [11; 32],
        identity: [12; 32],
        generation: 1,
        proxy: Some(
            ProxyDialRoute::new(
                TransportScheme::Http,
                "proxy.invalid:8080".into(),
                Arc::from([proxy]),
                CaPolicy::System,
                None,
            )
            .unwrap(),
        ),
    };
    let mut client =
        PingoraClientSession::new(Arc::new(PingoraConnectorRegistry::default()), [0; 32]);
    client.configure_network_route(Some(&route)).unwrap();
    (client, target)
}

#[tokio::test]
async fn malformed_rejected_and_oversized_connect_do_not_attempt_the_origin() {
    for response in [
        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".to_vec(),
        b"invalid response\r\n\r\n".to_vec(),
        format!("HTTP/1.1 200 OK\r\n{}\r\n", "X: a\r\n".repeat(129)).into_bytes(),
        format!("HTTP/1.1 200 OK\r\nX: {}\r\n\r\n", "a".repeat(32 * 1024)).into_bytes(),
        b"HTTP/1.1 200".to_vec(),
    ] {
        rejected_tunnel(response).await;
    }
}

#[tokio::test]
async fn cancelling_slow_connect_negotiation_closes_the_proxy_stream() {
    let origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    origin.set_nonblocking(true).unwrap();
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = proxy.local_addr().unwrap();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = proxy.accept().await.unwrap();
        let mut io: Io = Box::new(socket);
        header(&mut io).await;
        io.write_all(b"HTTP/1.1 200 Connection Established\r\n")
            .await
            .unwrap();
        seen_tx.send(()).unwrap();
        let mut buffer = [0; 1];
        // Cancelling with unread partial headers may close TCP with FIN or RST.
        // Both terminate the stream; a timeout or additional data is a failure.
        match tokio::time::timeout(Duration::from_secs(3), io.read(&mut buffer))
            .await
            .unwrap()
        {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                ) => {}
            other => panic!("cancelled proxy stream remained readable: {other:?}"),
        }
    });
    let (mut client, target) = tunnel_client(address, origin.local_addr().unwrap());
    {
        let connection = client.connect(&target, address);
        tokio::pin!(connection);
        tokio::select! {
            result = &mut connection => panic!("partial CONNECT completed: {result:?}"),
            seen = seen_rx => seen.unwrap(),
        }
    }
    client.cancel_reset().await;
    server.await.unwrap();
    assert_eq!(
        origin.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
