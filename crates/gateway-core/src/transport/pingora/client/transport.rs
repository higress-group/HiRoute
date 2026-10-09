use super::*;

#[async_trait]
impl AttemptTransport for PingoraClientSession {
    fn requires_codec_reservation(&self) -> bool {
        true
    }

    async fn connect(
        &mut self,
        target: &TransportTarget,
        address: SocketAddr,
    ) -> Result<(), AttemptError> {
        if self.connected.is_some() {
            return Ok(());
        }
        if target.requires_resolution() {
            return Err(AttemptError::InvalidTarget(
                "transport authority requires exact-target resolution".into(),
            ));
        }
        let peer = build_peer(target, address, self.connection_config_fingerprint)?;
        let connector = self
            .registry
            .connector_for(target, self.connection_config_fingerprint)?;
        let (session, reused) = connector
            .get_http_session(&peer)
            .await
            .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
        let protocol = if session.as_http2().is_some() {
            HttpProtocol::Http2
        } else {
            HttpProtocol::Http1
        };
        self.connected = Some(ConnectedPingoraSession {
            connector,
            reader_session: Some(session),
            reader: None,
            writer: None,
            h1_shared: None,
            peer,
            reused,
            protocol,
            request_eos: false,
        });
        self.request_head_written = false;
        self.response_eos_emitted = false;
        Ok(())
    }

    async fn write_request_head(&mut self, head: &PreparedRequestHead) -> Result<(), AttemptError> {
        PingoraClientSession::write_request_head(self, head).await
    }

    async fn write_request_body(
        &mut self,
        body: Bytes,
        end_stream: bool,
    ) -> Result<(), AttemptError> {
        PingoraClientSession::write_request_body(self, body, end_stream).await
    }

    async fn finish_request_body(&mut self) -> Result<(), AttemptError> {
        PingoraClientSession::finish_request_body(self).await
    }

    fn activate_precommit_reader(&mut self) -> Result<(), AttemptError> {
        self.activate_response_reader()
    }

    fn supports_duplex_request_body(&self) -> bool {
        self.connected
            .as_ref()
            .is_some_and(|connected| connected.writer.is_some())
    }

    fn poll_write_request_body(
        &mut self,
        context: &mut Context<'_>,
        body: &mut Bytes,
        end_stream: bool,
    ) -> Poll<Result<(), AttemptError>> {
        self.poll_request_body_write(context, body, end_stream)
    }

    fn poll_finish_request_body(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), AttemptError>> {
        let result = self.poll_request_body_finish(context);
        if matches!(result, Poll::Ready(Ok(())))
            && let Some(connected) = self.connected.as_mut()
        {
            connected.request_eos = true;
        }
        result
    }

    fn poll_precommit(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitEvent>, AttemptError>> {
        if !self.request_head_written {
            return Poll::Pending;
        }
        if self.response_eos_emitted {
            return Poll::Ready(Ok(None));
        }
        if let Err(error) = self.activate_response_reader() {
            return Poll::Ready(Err(error));
        }
        match self.poll_reader_event(context) {
            Poll::Ready(Ok(Some(TransportPrecommitEvent::EndStream))) => {
                self.response_eos_emitted = true;
                Poll::Ready(Ok(Some(TransportPrecommitEvent::EndStream)))
            }
            other => other,
        }
    }

    fn poll_precommit_receipt(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitReceipt>, AttemptError>> {
        if !self.request_head_written {
            return Poll::Pending;
        }
        if self.response_eos_emitted {
            return Poll::Ready(Ok(None));
        }
        if let Err(error) = self.activate_response_reader() {
            return Poll::Ready(Err(error));
        }
        match self.poll_reader_receipt(context) {
            Poll::Ready(Ok(Some(receipt)))
                if matches!(receipt.event, TransportPrecommitEvent::EndStream) =>
            {
                self.response_eos_emitted = true;
                Poll::Ready(Ok(Some(receipt)))
            }
            other => other,
        }
    }

    fn local_read_suppression_total(&self) -> Option<Duration> {
        self.connected
            .as_ref()
            .and_then(|connected| connected.reader.as_ref())
            .map(|reader| {
                reader
                    .local_read_suppression
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .total()
            })
    }

    async fn cancel_reset(&mut self) -> Result<(), AttemptError> {
        PingoraClientSession::cancel_reset(self).await;
        Ok(())
    }

    async fn finish_accepted(&mut self, reusable: bool) -> Result<(), AttemptError> {
        let Some(mut connected) = self.connected.take() else {
            return Ok(());
        };
        let writer_ready = match connected.writer.take() {
            Some(PingoraRequestWriter::H1(H1WriterState::Ready(_writer))) => true,
            Some(PingoraRequestWriter::H1(H1WriterState::Running(mut task))) => {
                task.abort();
                let _ = (&mut task).await;
                false
            }
            Some(PingoraRequestWriter::H1(H1WriterState::Transitioning)) => false,
            Some(PingoraRequestWriter::H2 { stream: _, eos }) => eos,
            None => false,
        };
        let mut session = if let Some(mut reader) = connected.reader.take() {
            // A provider may complete its protocol before HTTP EOS. The bounded reader
            // mailbox still owns trailing wire chunks; joining without consuming it can
            // deadlock the reader on sender.reserve(), even after upstream sent HTTP EOS.
            // Drain only transport receipts, never emit another downstream model event.
            let join = async {
                // A finished reader can still have its final EOS queued. Sender closure,
                // rather than task readiness, establishes that every receipt was consumed.
                while let Some(receipt) = reader.events.recv().await {
                    if matches!(receipt, Ok(receipt) if matches!(receipt.event, TransportPrecommitEvent::EndStream))
                    {
                        self.response_eos_emitted = true;
                    }
                }
                (&mut reader.task).await
            };
            match tokio::time::timeout(SHUTDOWN_REQUEST_JOIN_TIMEOUT, join).await {
                Ok(Ok(session)) => session,
                Ok(Err(error)) => {
                    return Err(AttemptError::Transport(
                        format!("Pingora response reader task failed: {error}").into(),
                    ));
                }
                Err(_) => {
                    reader.task.abort();
                    let _ =
                        tokio::time::timeout(SHUTDOWN_REQUEST_JOIN_TIMEOUT, &mut reader.task).await;
                    return Err(AttemptError::Transport(
                        "Pingora response reader did not quiesce after response EOS".into(),
                    ));
                }
            }
        } else {
            connected.reader_session.take().ok_or_else(|| {
                AttemptError::Transport("Pingora accepted session lost response ownership".into())
            })?
        };
        let response_done = session.response_done();
        let reusable = reusable
            && connected.request_eos
            && writer_ready
            && self.response_eos_emitted
            && response_done;
        if reusable {
            if let Some(shared) = &connected.h1_shared {
                shared.reunify().map_err(|error| {
                    AttemptError::Transport(
                        format!("Pingora H1 stream reunification failed: {error}").into(),
                    )
                })?;
            }
            connected
                .connector
                .release_http_session(session, &connected.peer, None)
                .await;
        } else {
            session.shutdown().await;
        }
        Ok(())
    }

    fn protocol(&self) -> HttpProtocol {
        PingoraClientSession::protocol(self)
    }
}
