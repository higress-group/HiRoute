use super::*;

impl PingoraClientSession {
    pub(in super::super) fn new(
        registry: Arc<PingoraConnectorRegistry>,
        connection_config_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            registry,
            connection_config_fingerprint,
            connected: None,
            request_head_written: false,
            response_eos_emitted: false,
        }
    }

    pub fn reused(&self) -> bool {
        self.connected
            .as_ref()
            .is_some_and(|connected| connected.reused)
    }

    pub fn protocol(&self) -> HttpProtocol {
        self.connected
            .as_ref()
            .map_or(HttpProtocol::Http1, |connected| connected.protocol)
    }

    pub async fn write_request_head(
        &mut self,
        head: &PreparedRequestHead,
    ) -> Result<(), AttemptError> {
        let connected = self.connected_mut()?;
        let session = connected.reader_session.take().ok_or_else(|| {
            AttemptError::Transport("Pingora request head already written".into())
        })?;
        match session {
            ClientSession::H1(mut session) => {
                session
                    .write_request_header(Box::new(build_request_header(head)?))
                    .await
                    .map_err(|error| AttemptError::Transport(error.to_string().into()))?;

                let digest = session.digest().clone();
                let id = session.stream().id();
                let alpn = session.stream().selected_alpn_proto();
                let stream = flatten_reused_h1_stream(session.into_inner());
                let (read_half, write_half) = tokio::io::split(stream);
                let shared = Arc::new(SharedH1Stream {
                    read_half: Mutex::new(Some(read_half)),
                    write_half: Mutex::new(Some(write_half)),
                    unified: Mutex::new(None),
                    mode: AtomicU8::new(SharedH1Mode::Setup as u8),
                    digest,
                    id,
                    alpn,
                });
                let mut reader = Http1ClientSession::new_with_options(
                    Box::new(SharedH1Io {
                        shared: Arc::clone(&shared),
                        role: SharedH1Role::Reader,
                    }),
                    &connected.peer,
                );
                let mut writer = Http1ClientSession::new_with_options(
                    Box::new(SharedH1Io {
                        shared: Arc::clone(&shared),
                        role: SharedH1Role::Writer,
                    }),
                    &connected.peer,
                );
                // Setup mode sinks these two codec-initialization writes; the
                // original session above emitted the sole wire header.
                reader
                    .write_request_header(Box::new(build_request_header(head)?))
                    .await
                    .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
                writer
                    .write_request_header(Box::new(build_request_header(head)?))
                    .await
                    .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
                shared
                    .mode
                    .store(SharedH1Mode::Split as u8, Ordering::Release);
                connected.reader_session = Some(ClientSession::H1(reader));
                connected.writer = Some(PingoraRequestWriter::H1(H1WriterState::Ready(Box::new(
                    writer,
                ))));
                connected.h1_shared = Some(shared);
            }
            ClientSession::H2(mut session) => {
                session
                    .write_request_header(
                        Box::new(request::build_h2_request_header(
                            head,
                            connected.peer.is_tls(),
                        )?),
                        false,
                    )
                    .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
                let writer = session.take_request_body_writer().ok_or_else(|| {
                    AttemptError::Transport("Pingora H2 request writer is unavailable".into())
                })?;
                connected.reader_session = Some(ClientSession::H2(session));
                connected.writer = Some(PingoraRequestWriter::H2 {
                    stream: writer,
                    eos: false,
                });
            }
            ClientSession::Custom(_) => {
                return Err(AttemptError::Transport(
                    "custom Pingora client session is unsupported".into(),
                ));
            }
        }
        self.request_head_written = true;
        Ok(())
    }

    pub async fn write_request_body(
        &mut self,
        mut body: Bytes,
        end_stream: bool,
    ) -> Result<(), AttemptError> {
        std::future::poll_fn(|context| self.poll_request_body_write(context, &mut body, end_stream))
            .await
    }

    pub async fn finish_request_body(&mut self) -> Result<(), AttemptError> {
        std::future::poll_fn(|context| self.poll_request_body_finish(context)).await?;
        self.connected_mut()?.request_eos = true;
        Ok(())
    }

    pub async fn read_response_head(&mut self) -> Result<TransportPrecommitEvent, AttemptError> {
        self.activate_response_reader()?;
        match std::future::poll_fn(|context| self.poll_reader_event(context)).await? {
            Some(event @ TransportPrecommitEvent::ResponseHead { .. }) => Ok(event),
            Some(_) => Err(AttemptError::Transport(
                "Pingora response body arrived before response head".into(),
            )),
            None => Err(AttemptError::Transport(
                "Pingora response reader ended before response head".into(),
            )),
        }
    }

    pub async fn read_response_body(&mut self) -> Result<Option<Bytes>, AttemptError> {
        self.activate_response_reader()?;
        match std::future::poll_fn(|context| self.poll_reader_event(context)).await? {
            Some(TransportPrecommitEvent::Body(body)) => Ok(Some(body)),
            Some(TransportPrecommitEvent::EndStream) | None => {
                self.response_eos_emitted = true;
                Ok(None)
            }
            Some(TransportPrecommitEvent::ResponseHead { .. }) => Err(AttemptError::Transport(
                "unexpected additional response head while reading body".into(),
            )),
        }
    }

    pub async fn cancel_reset(&mut self) {
        if let Some(mut connected) = self.connected.take() {
            if let Some(mut reader) = connected.reader.take() {
                reader.task.abort();
                let _ = (&mut reader.task).await;
            }
            if let Some(writer) = connected.writer.take() {
                match writer {
                    PingoraRequestWriter::H1(H1WriterState::Running(mut task)) => {
                        task.abort();
                        let _ = (&mut task).await;
                    }
                    PingoraRequestWriter::H1(H1WriterState::Ready(mut writer)) => {
                        writer.shutdown().await;
                    }
                    PingoraRequestWriter::H1(H1WriterState::Transitioning) => {}
                    PingoraRequestWriter::H2 { mut stream, .. } => {
                        stream.send_reset(h2::Reason::CANCEL);
                    }
                }
            }
            if let Some(mut session) = connected.reader_session.take() {
                session.shutdown().await;
            } else if let Some(shared) = connected.h1_shared.take() {
                let mut io = SharedH1Io {
                    shared,
                    role: SharedH1Role::Reader,
                };
                io.shutdown().await;
            }
        }
    }

    pub(super) fn connected_mut(&mut self) -> Result<&mut ConnectedPingoraSession, AttemptError> {
        self.connected
            .as_mut()
            .ok_or_else(|| AttemptError::Transport("Pingora session is not connected".into()))
    }
}
