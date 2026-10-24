use super::*;
use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use http::{HeaderValue, Method, StatusCode, Version};

pub struct GatewayHttpApp<L> {
    lifecycle: L,
    server_options: HttpServerOptions,
}

impl<L> GatewayHttpApp<L> {
    pub fn new(lifecycle: L) -> Self {
        let mut server_options = HttpServerOptions::default();
        server_options.h2c = true;
        Self {
            lifecycle,
            server_options,
        }
    }
}

#[async_trait]
impl<L> HttpServerApp for GatewayHttpApp<L>
where
    L: GatewayLifecycle,
{
    async fn process_new_http(
        self: &Arc<Self>,
        mut session: ServerSession,
        shutdown: &ShutdownWatch,
    ) -> Option<ReusedHttpStream> {
        if !session.read_request().await.ok()? {
            return None;
        }
        if *shutdown.borrow() {
            session.set_keepalive(None);
        }
        let persistent_settings = HttpPersistentSettings::for_session(&session);
        let cancellation = CancellationToken::new();
        if *shutdown.borrow() {
            cancellation.cancel();
        }
        let result = {
            let mut adapter = PingoraServerSession {
                session: &mut session,
                cancellation: cancellation.clone(),
                request_body_eos: false,
            };
            let process = self.lifecycle.process(&mut adapter);
            tokio::pin!(process);
            let mut shutdown = shutdown.clone();
            tokio::select! {
                result = &mut process => result,
                _ = wait_for_shutdown(&mut shutdown) => {
                    cancellation.cancel();
                    match tokio::time::timeout(SHUTDOWN_REQUEST_JOIN_TIMEOUT, &mut process).await {
                        Ok(result) => result,
                        Err(_) => Err(TransportError::Io(
                            "request cleanup exceeded shutdown join timeout".into(),
                        )),
                    }
                }
            }
        };
        if !matches!(result, Ok(SessionReuse::Reusable)) {
            return None;
        }
        session
            .finish()
            .await
            .ok()
            .flatten()
            .map(|stream| ReusedHttpStream::from_reusable_stream(stream, persistent_settings))
    }

    fn server_options(&self) -> Option<&HttpServerOptions> {
        Some(&self.server_options)
    }
}

struct PingoraServerSession<'a> {
    session: &'a mut ServerSession,
    cancellation: CancellationToken,
    request_body_eos: bool,
}

#[async_trait]
impl GatewaySession for PingoraServerSession<'_> {
    fn request_head(&self) -> Result<GatewayRequestHead, TransportError> {
        let request = self.session.req_header();
        Ok(GatewayRequestHead {
            method: request.method.clone(),
            path_and_query: request
                .uri
                .path_and_query()
                .map_or_else(|| Arc::from("/"), |value| Arc::from(value.as_str())),
            authority: request
                .uri
                .authority()
                .map(|authority| Arc::from(authority.as_str())),
            headers: request.headers.clone(),
            protocol: if self.session.is_http2() {
                HttpProtocol::Http2
            } else {
                HttpProtocol::Http1
            },
        })
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    async fn wait_for_disconnect(&mut self) {
        if self.request_body_eos {
            let _ = self.session.read_body_or_idle(true).await;
        } else {
            // Pingora's `read_body_or_idle(true)` is only a non-consuming
            // liveness wait after request EOS. Before EOS it is the unique H1
            // body reader and treats ordinary DATA as data-after-end. Keep the
            // read owner with `read_request_body`; shutdown/caller
            // cancellation is selected independently by the lifecycle.
            self.cancellation.cancelled().await;
        }
    }

    async fn read_request_body(&mut self) -> Result<Option<Bytes>, TransportError> {
        let body = self
            .session
            .read_request_body()
            .await
            .map_err(|error| TransportError::Io(error.to_string().into()))?;
        self.request_body_eos = body.is_none();
        Ok(body)
    }

    async fn write_response_head(
        &mut self,
        mut head: GatewayResponseHead,
    ) -> Result<(), TransportError> {
        let request = self.session.req_header();
        let body_allowed = request.method != Method::HEAD
            && !head.status.is_informational()
            && head.status != StatusCode::NO_CONTENT
            && head.status != StatusCode::NOT_MODIFIED
            && !(request.method == Method::CONNECT && head.status.is_success());
        if request.version == Version::HTTP_11
            && body_allowed
            && !head.headers.contains_key(CONTENT_LENGTH)
            && !head.headers.contains_key(TRANSFER_ENCODING)
        {
            // Close-delimited EOF cannot distinguish successful completion from
            // a failed lifecycle after headers/output were committed. Let the
            // codec emit the final chunk only on an explicit end_stream write.
            head.headers
                .insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        }
        let mut response = ResponseHeader::build(head.status.as_u16(), Some(head.headers.len()))
            .map_err(|error| TransportError::InvalidMetadata(error.to_string().into()))?;
        for (name, value) in &head.headers {
            response
                .append_header(name.clone(), value.clone())
                .map_err(|error| TransportError::InvalidMetadata(error.to_string().into()))?;
        }
        self.session
            .write_response_header(Box::new(response))
            .await
            .map_err(|error| TransportError::Io(error.to_string().into()))
    }

    async fn write_response_body(
        &mut self,
        body: Bytes,
        end_stream: bool,
    ) -> Result<(), TransportError> {
        self.session
            .write_response_body(body, end_stream)
            .await
            .map_err(|error| TransportError::Io(error.to_string().into()))
    }
}

async fn wait_for_shutdown(shutdown: &mut ShutdownWatch) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}
