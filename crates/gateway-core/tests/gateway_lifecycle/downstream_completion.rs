use crate::fixture::*;

const PREFIX: &[u8] =
    b"data: {\"choices\":[{\"delta\":{\"content\":\"unfinished\"},\"finish_reason\":null}]}\n\n";

#[derive(Default)]
struct StreamingLifecycle {
    failed_prefix_observed: Option<Arc<tokio::sync::Notify>>,
}

#[async_trait]
impl GatewayLifecycle for StreamingLifecycle {
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError> {
        let request = session.request_head()?;
        while session.read_request_body().await?.is_some() {}
        let status = match request.path_and_query.as_ref() {
            "/204" => StatusCode::NO_CONTENT,
            "/304" => StatusCode::NOT_MODIFIED,
            _ => StatusCode::OK,
        };
        let no_body = request.method == Method::HEAD || status != StatusCode::OK;
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            HeaderValue::from_static("text/event-stream"),
        );
        if request.path_and_query.as_ref() == "/informational" {
            session
                .write_response_head(GatewayResponseHead {
                    status: StatusCode::EARLY_HINTS,
                    headers: HeaderMap::new(),
                })
                .await?;
        }
        if request.path_and_query.as_ref() == "/length" {
            headers.insert(CONTENT_LENGTH, HeaderValue::from(PREFIX.len() as u64));
        }
        if request.path_and_query.as_ref() == "/chunked" {
            headers.insert(
                http::header::TRANSFER_ENCODING,
                HeaderValue::from_static("chunked"),
            );
        }
        session
            .write_response_head(GatewayResponseHead { status, headers })
            .await?;
        if no_body {
            session.write_response_body(Bytes::new(), true).await?;
        } else {
            let failed = request.path_and_query.as_ref() == "/timeout";
            session
                .write_response_body(Bytes::from_static(PREFIX), !failed)
                .await?;
            if failed {
                if let Some(observed) = &self.failed_prefix_observed {
                    observed.notified().await;
                }
                return Err(TransportError::DeadlineExceeded);
            }
        }
        Ok(if request.path_and_query.as_ref() == "/reuse" {
            SessionReuse::Reusable
        } else {
            SessionReuse::Close
        })
    }
}

async fn exchange_h1(method: &str, path: &str, version: &str) -> Result<Vec<u8>, TestError> {
    let port = reserve_tcp_port()?;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = start_pingora_service(
        port,
        GatewayHttpApp::new(StreamingLifecycle::default()),
        shutdown_rx,
    );
    wait_until_listening(address).await?;
    let result = async {
        let mut client = tokio::net::TcpStream::connect(address).await?;
        client
            .write_all(
                format!("{method} {path} {version}\r\nHost: gateway.test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes(),
            )
            .await?;
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut response)).await??;
        Ok::<_, TestError>(response)
    }
    .await;
    stop_pingora_service(shutdown_tx, task).await?;
    result
}

fn split_response(wire: &[u8]) -> (&str, &[u8]) {
    let split = wire
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap();
    (
        std::str::from_utf8(&wire[..split]).unwrap(),
        &wire[split + 4..],
    )
}

#[tokio::test]
async fn http11_postcommit_failure_cannot_be_normal_close_delimited_eof() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let response = exchange_h1("POST", "/timeout", "HTTP/1.1").await?;
    let (head, body) = split_response(&response);
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(body.windows(PREFIX.len()).any(|part| part == PREFIX));
    assert!(
        head.to_ascii_lowercase()
            .contains("transfer-encoding: chunked"),
        "an unknown-length HTTP/1.1 response must distinguish failed close from entity completion: {head}"
    );
    assert!(
        !body.ends_with(b"0\r\n\r\n"),
        "failure must not finish the chunked entity"
    );
    Ok(())
}

#[tokio::test]
async fn http11_success_completes_unknown_length_chunked_entity() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let response = exchange_h1("POST", "/ok", "HTTP/1.1").await?;
    let (head, body) = split_response(&response);
    assert!(
        head.to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
    );
    assert_eq!(
        body,
        format!(
            "{:X}\r\n{}\r\n0\r\n\r\n",
            PREFIX.len(),
            std::str::from_utf8(PREFIX)?
        )
        .as_bytes()
    );
    Ok(())
}

#[tokio::test]
async fn framing_preserves_known_length_no_body_and_http10_boundaries() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let response = exchange_h1("POST", "/length", "HTTP/1.1").await?;
    let (head, body) = split_response(&response);
    assert!(
        head.to_ascii_lowercase()
            .contains(&format!("content-length: {}", PREFIX.len()))
    );
    assert!(!head.to_ascii_lowercase().contains("transfer-encoding"));
    assert_eq!(body, PREFIX);
    let response = exchange_h1("POST", "/chunked", "HTTP/1.1").await?;
    let (head, body) = split_response(&response);
    assert_eq!(
        head.to_ascii_lowercase()
            .matches("transfer-encoding:")
            .count(),
        1
    );
    assert!(body.ends_with(b"0\r\n\r\n"));
    for (method, path) in [("HEAD", "/ok"), ("GET", "/204"), ("GET", "/304")] {
        let response = exchange_h1(method, path, "HTTP/1.1").await?;
        let (head, body) = split_response(&response);
        assert!(!head.to_ascii_lowercase().contains("transfer-encoding"));
        assert!(body.is_empty());
    }
    let response = exchange_h1("CONNECT", "/ok", "HTTP/1.1").await?;
    let (head, body) = split_response(&response);
    assert!(!head.to_ascii_lowercase().contains("transfer-encoding"));
    assert_eq!(body, PREFIX);
    let response = exchange_h1("POST", "/informational", "HTTP/1.1").await?;
    let (informational, final_response) = split_response(&response);
    assert!(informational.starts_with("HTTP/1.1 103"));
    assert!(
        !informational
            .to_ascii_lowercase()
            .contains("transfer-encoding")
    );
    let (head, body) = split_response(final_response);
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(
        head.to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
    );
    assert!(body.ends_with(b"0\r\n\r\n"));
    let response = exchange_h1("POST", "/ok", "HTTP/1.0").await?;
    let (head, body) = split_response(&response);
    assert!(!head.to_ascii_lowercase().contains("transfer-encoding"));
    assert_eq!(body, PREFIX);
    Ok(())
}

#[tokio::test]
async fn http11_completed_chunked_entities_allow_connection_reuse() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let port = reserve_tcp_port()?;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = start_pingora_service(
        port,
        GatewayHttpApp::new(StreamingLifecycle::default()),
        shutdown_rx,
    );
    wait_until_listening(address).await?;
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let mut client = tokio::net::TcpStream::connect(address).await?;
        let expected = format!(
            "{:X}\r\n{}\r\n0\r\n\r\n",
            PREFIX.len(),
            std::str::from_utf8(PREFIX)?
        );
        for _ in 0..2 {
            client
                .write_all(b"POST /reuse HTTP/1.1\r\nHost: gateway.test\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n")
                .await?;
            let head = String::from_utf8(read_h1_head(&mut client).await?)?;
            assert!(head.starts_with("HTTP/1.1 200"));
            assert!(head.to_ascii_lowercase().contains("transfer-encoding: chunked"));
            let mut body = vec![0; expected.len()];
            client.read_exact(&mut body).await?;
            assert_eq!(body, expected.as_bytes());
        }
        Ok::<_, TestError>(())
    })
    .await;
    stop_pingora_service(shutdown_tx, task).await?;
    result??;
    Ok(())
}

#[tokio::test]
async fn http2_postcommit_failure_resets_without_successful_end_stream() -> Result<(), TestError> {
    let _network_guard = NETWORK_TEST_LOCK.lock().await;
    let port = reserve_tcp_port()?;
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let observed = Arc::new(tokio::sync::Notify::new());
    let task = start_pingora_service(
        port,
        GatewayHttpApp::new(StreamingLifecycle {
            failed_prefix_observed: Some(observed.clone()),
        }),
        shutdown_rx,
    );
    wait_until_listening(address).await?;
    let socket = tokio::net::TcpStream::connect(address).await?;
    let (mut sender, connection) = client::handshake(socket).await?;
    let driver = tokio::spawn(connection);
    let request = http::Request::builder()
        .method(Method::POST)
        .uri(format!("http://gateway.test:{port}/timeout"))
        .body(())?;
    let (response, _) = sender.send_request(request, true)?;
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let response = response.await?;
        assert!(
            !response
                .headers()
                .contains_key(http::header::TRANSFER_ENCODING)
        );
        let mut body = response.into_body();
        let mut prefix = Vec::new();
        while let Some(chunk) = body.data().await {
            match chunk {
                Ok(bytes) => {
                    prefix.extend_from_slice(&bytes);
                    body.flow_control().release_capacity(bytes.len())?;
                    if prefix == PREFIX {
                        observed.notify_one();
                    }
                }
                Err(error) => return Ok::<_, TestError>(prefix == PREFIX && error.is_reset()),
            }
        }
        Ok(false)
    })
    .await;
    drop(sender);
    driver.abort();
    let _ = driver.await;
    stop_pingora_service(shutdown_tx, task).await?;
    assert!(
        result??,
        "HTTP/2 must reset the failed response instead of normal EOS"
    );
    Ok(())
}
