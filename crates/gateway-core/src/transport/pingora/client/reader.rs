use super::*;

impl PingoraClientSession {
    pub(super) fn activate_response_reader(&mut self) -> Result<(), AttemptError> {
        if !self.request_head_written {
            return Ok(());
        }
        let connected = self.connected_mut()?;
        if connected.reader.is_some() {
            return Ok(());
        }
        let session = connected.reader_session.take().ok_or_else(|| {
            AttemptError::Transport("Pingora response reader session is unavailable".into())
        })?;
        let (sender, events) = mpsc::channel(UPSTREAM_READER_MAILBOX_CAPACITY);
        let local_read_suppression = Arc::new(Mutex::new(ReaderSuppressionClock::default()));
        let task = AbortOnDropTask::spawn(run_upstream_reader(
            session,
            sender,
            local_read_suppression.clone(),
            connected.forward_proxy_authority.is_some(),
        ));
        connected.reader = Some(PingoraReaderOwner {
            events,
            local_read_suppression,
            task,
        });
        Ok(())
    }

    pub(super) fn poll_reader_receipt(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitReceipt>, AttemptError>> {
        let connected = match self.connected.as_mut() {
            Some(connected) => connected,
            None => {
                return Poll::Ready(Err(AttemptError::Transport(
                    "Pingora session is not connected".into(),
                )));
            }
        };
        let Some(reader) = connected.reader.as_mut() else {
            return Poll::Pending;
        };
        match reader.events.poll_recv(context) {
            Poll::Ready(Some(Ok(receipt))) => Poll::Ready(Ok(Some(receipt))),
            Poll::Ready(Some(Err(error))) => Poll::Ready(Err(error)),
            Poll::Ready(None) => Poll::Ready(Ok(None)),
            Poll::Pending => Poll::Pending,
        }
    }

    pub(super) fn poll_reader_event(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<Option<TransportPrecommitEvent>, AttemptError>> {
        match self.poll_reader_receipt(context) {
            Poll::Ready(Ok(Some(receipt))) => Poll::Ready(Ok(Some(receipt.event))),
            Poll::Ready(Ok(None)) => Poll::Ready(Ok(None)),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }

    pub(super) fn poll_request_body_write(
        &mut self,
        context: &mut Context<'_>,
        body: &mut Bytes,
        end_stream: bool,
    ) -> Poll<Result<(), AttemptError>> {
        let connected = match self.connected.as_mut() {
            Some(connected) => connected,
            None => {
                return Poll::Ready(Err(AttemptError::Transport(
                    "Pingora session is not connected".into(),
                )));
            }
        };
        let Some(writer) = connected.writer.as_mut() else {
            return Poll::Ready(Err(AttemptError::Transport(
                "Pingora request writer is unavailable".into(),
            )));
        };
        match writer {
            PingoraRequestWriter::H1(state) => loop {
                match state {
                    H1WriterState::Ready(_) => {
                        let H1WriterState::Ready(mut writer) =
                            std::mem::replace(state, H1WriterState::Transitioning)
                        else {
                            unreachable!()
                        };
                        let payload = std::mem::take(body);
                        *state = H1WriterState::Running(AbortOnDropTask::spawn(async move {
                            let result = writer
                                .write_body(payload.as_ref())
                                .await
                                .map(|_| ())
                                .map_err(|error| {
                                    AttemptError::Transport(
                                        format!("Pingora H1 request body write failed: {error}")
                                            .into(),
                                    )
                                });
                            H1WriterCompletion { writer, result }
                        }));
                    }
                    H1WriterState::Running(task) => match Pin::new(task).poll(context) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Ok(completion)) => {
                            let result = completion.result;
                            *state = H1WriterState::Ready(completion.writer);
                            return Poll::Ready(result);
                        }
                        Poll::Ready(Err(error)) => {
                            *state = H1WriterState::Transitioning;
                            return Poll::Ready(Err(AttemptError::Transport(
                                format!("Pingora H1 writer task failed: {error}").into(),
                            )));
                        }
                    },
                    H1WriterState::Transitioning => {
                        return Poll::Ready(Err(AttemptError::Transport(
                            "Pingora H1 writer lost linear ownership".into(),
                        )));
                    }
                }
            },
            PingoraRequestWriter::H2 { stream, eos } => {
                if *eos {
                    return Poll::Ready(Err(AttemptError::Transport(
                        "Pingora H2 request body write followed EOS".into(),
                    )));
                }
                if body.is_empty() {
                    if end_stream {
                        stream
                            .send_data(Bytes::new(), true)
                            .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
                        *eos = true;
                    }
                    return Poll::Ready(Ok(()));
                }
                stream.reserve_capacity(body.len());
                match stream.poll_capacity(context) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(None) => Poll::Ready(Err(AttemptError::Transport(
                        "Pingora H2 request stream closed while awaiting capacity".into(),
                    ))),
                    Poll::Ready(Some(Err(error))) => {
                        Poll::Ready(Err(AttemptError::Transport(error.to_string().into())))
                    }
                    Poll::Ready(Some(Ok(capacity))) => {
                        let chunk = body.split_to(capacity.min(body.len()));
                        let completes = body.is_empty();
                        stream
                            .send_data(chunk, completes && end_stream)
                            .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
                        if completes {
                            *eos = end_stream;
                            Poll::Ready(Ok(()))
                        } else {
                            context.waker().wake_by_ref();
                            Poll::Pending
                        }
                    }
                }
            }
        }
    }

    pub(super) fn poll_request_body_finish(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), AttemptError>> {
        let connected = match self.connected.as_mut() {
            Some(connected) => connected,
            None => {
                return Poll::Ready(Err(AttemptError::Transport(
                    "Pingora session is not connected".into(),
                )));
            }
        };
        let Some(writer) = connected.writer.as_mut() else {
            return Poll::Ready(Err(AttemptError::Transport(
                "Pingora request writer is unavailable".into(),
            )));
        };
        match writer {
            PingoraRequestWriter::H1(state) => loop {
                match state {
                    H1WriterState::Ready(_) => {
                        let H1WriterState::Ready(mut writer) =
                            std::mem::replace(state, H1WriterState::Transitioning)
                        else {
                            unreachable!()
                        };
                        *state = H1WriterState::Running(AbortOnDropTask::spawn(async move {
                            let result = writer.finish_body().await.map(|_| ()).map_err(|error| {
                                AttemptError::Transport(
                                    format!("Pingora H1 request EOS failed: {error}").into(),
                                )
                            });
                            H1WriterCompletion { writer, result }
                        }));
                    }
                    H1WriterState::Running(task) => match Pin::new(task).poll(context) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(Ok(completion)) => {
                            let result = completion.result;
                            *state = H1WriterState::Ready(completion.writer);
                            return Poll::Ready(result);
                        }
                        Poll::Ready(Err(error)) => {
                            *state = H1WriterState::Transitioning;
                            return Poll::Ready(Err(AttemptError::Transport(
                                format!("Pingora H1 writer task failed: {error}").into(),
                            )));
                        }
                    },
                    H1WriterState::Transitioning => {
                        return Poll::Ready(Err(AttemptError::Transport(
                            "Pingora H1 writer lost linear ownership".into(),
                        )));
                    }
                }
            },
            PingoraRequestWriter::H2 { stream, eos } => {
                if !*eos {
                    stream
                        .send_data(Bytes::new(), true)
                        .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
                    *eos = true;
                }
                Poll::Ready(Ok(()))
            }
        }
    }
}
