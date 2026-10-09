use super::*;

pub(crate) static NETWORK_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(crate) fn reserve_tcp_port() -> io::Result<u16> {
    Ok(StdTcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

pub(crate) fn start_pingora_service<A>(port: u16, app: A, shutdown: ShutdownWatch) -> JoinHandle<()>
where
    A: pingora_core::apps::ServerApp + Send + Sync + 'static,
{
    let mut service = ListeningService::new(format!("gateway-core-test-{port}"), app);
    service.add_tcp(&format!("127.0.0.1:{port}"));
    tokio::spawn(async move {
        #[cfg(unix)]
        ServiceContract::start_service(&mut service, None, shutdown, 1).await;
        #[cfg(not(unix))]
        ServiceContract::start_service(&mut service, shutdown, 1).await;
    })
}

pub(crate) async fn wait_until_listening(address: std::net::SocketAddr) -> Result<(), TestError> {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(format!("Pingora test listener {address} did not start").into())
}

pub(crate) async fn stop_pingora_service(
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
) -> Result<(), TestError> {
    let _ = shutdown.send(true);
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .map_err(|_| "Pingora test service did not stop")??;
    Ok(())
}

pub(crate) async fn read_h1_head<S>(stream: &mut S) -> io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut head = Vec::with_capacity(512);
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
        if head.len() > 16 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request head exceeds test limit",
            ));
        }
    }
    Ok(head)
}

pub(crate) fn content_length(head: &[u8]) -> usize {
    String::from_utf8_lossy(head)
        .lines()
        .find_map(|line| {
            line.strip_prefix("content-length: ")
                .or_else(|| line.strip_prefix("Content-Length: "))
                .and_then(|value| value.trim().parse().ok())
        })
        .unwrap_or(0)
}

pub(crate) async fn serve_h1_once<S>(
    mut stream: S,
    status: StatusCode,
    response_body: &'static [u8],
    respond_before_body: bool,
) -> Result<bool, TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let body_len = content_length(&head);
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("Response"),
        response_body.len()
    );
    if respond_before_body {
        stream.write_all(response.as_bytes()).await?;
        stream.write_all(response_body).await?;
        stream.flush().await?;
    }
    let mut request_body = vec![0_u8; body_len];
    let body_result = stream.read_exact(&mut request_body).await;
    let body_was_reset = body_result.is_err();
    if !respond_before_body {
        body_result?;
        stream.write_all(response.as_bytes()).await?;
        stream.write_all(response_body).await?;
        stream.flush().await?;
    }
    Ok(body_was_reset)
}

pub(crate) async fn serve_h1_then_observe_client_close<S>(
    mut stream: S,
    response_body: &'static [u8],
) -> Result<bool, TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let mut request_body = vec![0_u8; content_length(&head)];
    stream.read_exact(&mut request_body).await?;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        response_body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(response_body).await?;
    stream.flush().await?;
    let mut byte = [0_u8; 1];
    Ok(matches!(
        tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte)).await,
        Ok(Ok(0) | Err(_))
    ))
}

pub(crate) async fn serve_h1_after_response_gap<S>(
    mut stream: S,
    response_gap: Duration,
    response_body: &'static [u8],
) -> Result<(), TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let mut request_body = vec![0_u8; content_length(&head)];
    stream.read_exact(&mut request_body).await?;
    tokio::time::sleep(response_gap).await;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        response_body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(response_body).await?;
    stream.flush().await?;
    Ok(())
}

pub(crate) async fn serve_h1_semantic_chunk_then_gap<S>(
    mut stream: S,
    response_gap: Duration,
) -> Result<bool, TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let mut request_body = vec![0_u8; content_length(&head)];
    stream.read_exact(&mut request_body).await?;
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n5\r\nfirst\r\n",
        )
        .await?;
    stream.flush().await?;
    tokio::time::sleep(response_gap).await;
    if stream.write_all(b"6\r\nsecond\r\n0\r\n\r\n").await.is_err() {
        return Ok(true);
    }
    stream.flush().await?;
    let mut byte = [0_u8; 1];
    Ok(matches!(
        tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte)).await,
        Ok(Ok(0) | Err(_))
    ))
}

pub(crate) async fn serve_h1_semantic_chunk_until_client_close<S>(
    mut stream: S,
) -> Result<bool, TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let mut request_body = vec![0_u8; content_length(&head)];
    stream.read_exact(&mut request_body).await?;
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n5\r\nfirst\r\n",
        )
        .await?;
    stream.flush().await?;
    let mut byte = [0_u8; 1];
    Ok(matches!(
        tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte)).await,
        Ok(Ok(0) | Err(_))
    ))
}

pub(crate) async fn serve_h1_chunked_sse_until_release<S>(
    mut stream: S,
    events_sent: Arc<Notify>,
    release_eos: Arc<Notify>,
    post_terminal_tail: bool,
) -> Result<(), TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let mut request_body = vec![0_u8; content_length(&head)];
    stream.read_exact(&mut request_body).await?;
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n",
        )
        .await?;
    for event in [
        b": discarded\n\n".as_slice(),
        b"data: choose\n\n".as_slice(),
    ] {
        stream
            .write_all(format!("{:x}\r\n", event.len()).as_bytes())
            .await?;
        stream.write_all(event).await?;
        stream.write_all(b"\r\n").await?;
    }
    stream.flush().await?;
    events_sent.notify_one();
    release_eos.notified().await;
    if post_terminal_tail {
        let tail = vec![b' '; 64 * 1024];
        stream
            .write_all(format!("{:x}\r\n", tail.len()).as_bytes())
            .await?;
        stream.write_all(&tail).await?;
        stream.write_all(b"\r\n").await?;
    }
    stream.write_all(b"0\r\n\r\n").await?;
    stream.flush().await?;
    Ok(())
}

/// Sends a final response while deliberately withholding request-body reads.
/// The split response head proves the client keeps one persistent parser, and
/// `received < Content-Length` proves it can classify/reset that response
/// while a large H1 write is still blocked rather than awaiting the write to
/// completion before polling the response direction.
pub(crate) async fn serve_segmented_h1_early_then_observe_reset<S>(
    mut stream: S,
) -> Result<(usize, usize), TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let expected = content_length(&head);
    stream.write_all(b"HTTP/1.1 429 Too Many Req").await?;
    stream.flush().await?;
    tokio::time::sleep(Duration::from_millis(20)).await;
    stream
        .write_all(b"uests\r\nContent-Length: 5\r\nConnection: keep-alive\r\n\r\nretry")
        .await?;
    stream.flush().await?;

    // Keep the client send window/socket buffer exhausted long enough for the
    // independent response owner to classify Continue and reset this attempt.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut received = 0;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer)).await {
            Ok(Ok(0) | Err(_)) => break,
            Ok(Ok(bytes)) => received += bytes,
            Err(_) => return Err("H1 attempt was not reset after the early response".into()),
        }
    }
    Ok((received, expected))
}

pub(crate) async fn serve_h1_early_sse_without_reading_body<S>(
    mut stream: S,
    hold_reads_for: Duration,
) -> Result<(usize, usize), TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let head = read_h1_head(&mut stream).await?;
    let expected = content_length(&head);
    let event = b"data: choose\n\n";
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n",
        )
        .await?;
    stream
        .write_all(format!("{:x}\r\n", event.len()).as_bytes())
        .await?;
    stream.write_all(event).await?;
    stream.write_all(b"\r\n").await?;
    stream.flush().await?;

    // Keep the socket receive window full past the attempt grant. This makes
    // the post-classification writer gate, rather than classification itself,
    // own the timeout.
    tokio::time::sleep(hold_reads_for).await;
    let mut received = 0;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match tokio::time::timeout(Duration::from_secs(2), stream.read(&mut buffer)).await {
            Ok(Ok(0) | Err(_)) => break,
            Ok(Ok(bytes)) => received += bytes,
            Err(_) => break,
        }
    }
    Ok((received, expected))
}

pub(crate) async fn serve_h1_keepalive_pair<S>(mut stream: S) -> Result<(), TestError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for (index, response_body) in [b"first".as_slice(), b"second".as_slice()]
        .into_iter()
        .enumerate()
    {
        let head = read_h1_head(&mut stream).await?;
        let mut request_body = vec![0_u8; content_length(&head)];
        stream.read_exact(&mut request_body).await?;
        let connection = if index == 0 { "keep-alive" } else { "close" };
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: {connection}\r\n\r\n",
            response_body.len()
        );
        stream.write_all(response.as_bytes()).await?;
        stream.write_all(response_body).await?;
        stream.flush().await?;
    }
    Ok(())
}
