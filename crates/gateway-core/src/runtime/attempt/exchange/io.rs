use super::*;

impl<T: AttemptTransport> AttemptExchange<T> {
    /// Address fallback is only a connection sub-attempt. Once the semantic
    /// request fence advances, this method cannot replay on another address.
    pub async fn connect(&mut self) -> Result<(), AttemptError> {
        if !self.upstream_request_fence.is_clear() {
            return Err(AttemptError::ReconnectAfterCommit);
        }
        if self.connected {
            return Ok(());
        }
        let mut last_error = None;
        for address in self.target.target().addresses.iter().copied() {
            if self.cancellation.is_cancelled() {
                return Err(AttemptError::Cancelled);
            }
            self.connection_sub_attempts += 1;
            let connect_started = Instant::now();
            let phase_deadline = connect_started
                .checked_add(self.target.target().connect_timeout)
                .unwrap_or(self.attempt_deadline);
            let (connect_deadline, timeout_kind, timeout_error) =
                if self.attempt_deadline <= phase_deadline {
                    (
                        self.attempt_deadline,
                        AttemptTimeoutKind::AttemptDeadline,
                        AttemptError::DeadlineExceeded,
                    )
                } else {
                    (
                        phase_deadline,
                        AttemptTimeoutKind::Connect,
                        AttemptError::ConnectTimeout,
                    )
                };
            let cancellation = self.cancellation.clone();
            let connect = match tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(AttemptError::Cancelled),
                result = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(connect_deadline),
                    self.transport
                        .as_mut()
                        .expect("active exchange owns transport")
                        .connect(self.target.target(), address),
                ) => result,
            } {
                Ok(result) => result,
                Err(_) => {
                    self.timeout_kind = Some(timeout_kind);
                    if timeout_kind == AttemptTimeoutKind::AttemptDeadline {
                        return Err(timeout_error);
                    }
                    last_error = Some(timeout_error);
                    continue;
                }
            };
            match connect {
                Ok(()) => {
                    self.connect_elapsed = Some(self.attempt_started_at.elapsed());
                    self.negotiated_protocol =
                        self.transport.as_ref().map(AttemptTransport::protocol);
                    self.connected = true;
                    return Ok(());
                }
                Err(error) => {
                    last_error = Some(error);
                }
            }
        }
        self.connect_elapsed = Some(self.attempt_started_at.elapsed());
        if let Some(telemetry) = &self.telemetry {
            telemetry.error(ErrorClass::UpstreamConnect);
        }
        Err(last_error.unwrap_or(AttemptError::ConnectFailed))
    }

    /// Drives at most one bounded request quantum and polls the response first.
    /// Repeated calls provide H1 chunk-boundary fairness; H2 adapters may
    /// advance their independent send stream using the same owner loop.
    pub async fn drive_writer_once(&mut self) -> Result<(), AttemptError> {
        self.ensure_active()?;
        if !self.connected {
            self.connect().await?;
        }

        if self.response_window.len() < self.response_window_capacity
            && let Some(event) = self.poll_precommit_now()?
        {
            self.response_window.push_back(event);
            self.response_window_high_water = self
                .response_window_high_water
                .max(self.response_window.len());
            return Ok(());
        }
        // A full prefix window backpressures only the response reader. The
        // same attempt owner must continue advancing the bounded request
        // writer toward normal EOS; otherwise an early Accept can deadlock
        // forever behind its own retained response prefix.

        match self.writer_state {
            WriterState::NotStarted => {
                if !self.request_framing_reconciled {
                    let mut framing = FramingLedger::default();
                    if self
                        .request
                        .head
                        .headers
                        .contains_key(http::header::CONTENT_LENGTH)
                    {
                        framing.record_header_mutation(&http::header::CONTENT_LENGTH);
                    }
                    if self
                        .request
                        .head
                        .headers
                        .contains_key(http::header::TRANSFER_ENCODING)
                    {
                        framing.record_header_mutation(&http::header::TRANSFER_ENCODING);
                    }
                    framing.buffered_eos(self.request.body.visible_bytes())?;
                    framing.finalize(
                        &mut self.request.head.headers,
                        match self
                            .transport
                            .as_ref()
                            .expect("active exchange owns transport")
                            .protocol()
                        {
                            HttpProtocol::Http1 => HttpFraming::Http1,
                            HttpProtocol::Http2 => HttpFraming::Http2,
                        },
                        None,
                        None,
                    )?;
                    self.request_framing_reconciled = true;
                }
                self.upstream_request_fence.begin_write()?;
                let request_write_deadline = self.ensure_request_write_deadline();
                // This is the semantic exchange boundary: construction and
                // connection fallback are preparatory, while invoking the
                // request-head write may expose provider-visible bytes.
                self.semantic_upstream_calls = 1;
                if let Some(telemetry) = &self.telemetry {
                    telemetry.commit(
                        FenceKind::UpstreamAttemptRequest,
                        self.upstream_request_fence,
                        self.connection_sub_attempts,
                    );
                }
                self.writer_state = WriterState::HeaderWriteStarted;
                let cancellation = self.cancellation.clone();
                let result = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return Err(AttemptError::Cancelled),
                    result = tokio::time::timeout_at(
                        tokio::time::Instant::from_std(request_write_deadline),
                        self.transport
                            .as_mut()
                            .expect("active exchange owns transport")
                            .write_request_head(&self.request.head),
                    ) => result,
                };
                let result = match result {
                    Ok(result) => result,
                    Err(_) => return Err(self.request_write_timeout_error()),
                };
                self.record_timeout_error(&result);
                result?;
                self.upstream_request_fence.confirm()?;
                if let Some(telemetry) = &self.telemetry {
                    telemetry.commit(
                        FenceKind::UpstreamAttemptRequest,
                        self.upstream_request_fence,
                        self.connection_sub_attempts,
                    );
                }
                self.writer_state = WriterState::HeaderWritten;
            }
            WriterState::HeaderWritten | WriterState::BodyWriting => {
                if self.pending_request_write.is_none() {
                    self.pending_request_write = self.request.body.begin_next_send()?;
                }
                if self.pending_request_write.is_some() {
                    self.writer_state = WriterState::BodyWriting;
                    let request_write_deadline = self.ensure_request_write_deadline();
                    let cancellation = self.cancellation.clone();
                    let progress = if self
                        .transport
                        .as_ref()
                        .expect("active exchange owns transport")
                        .supports_duplex_request_body()
                    {
                        let result = tokio::select! {
                            biased;
                            _ = cancellation.cancelled() => return Err(AttemptError::Cancelled),
                            result = tokio::time::timeout_at(
                                tokio::time::Instant::from_std(request_write_deadline),
                                std::future::poll_fn(|context| self.poll_duplex_body_write(context)),
                            ) => result,
                        };
                        let result = match result {
                            Ok(result) => result,
                            Err(_) => return Err(self.request_write_timeout_error()),
                        };
                        self.record_timeout_error(&result);
                        result?
                    } else {
                        let write = self
                            .pending_request_write
                            .take()
                            .expect("pending request write was checked above");
                        let result = tokio::select! {
                            biased;
                            _ = cancellation.cancelled() => return Err(AttemptError::Cancelled),
                            result = tokio::time::timeout_at(
                                tokio::time::Instant::from_std(request_write_deadline),
                                self.transport
                                    .as_mut()
                                    .expect("active exchange owns transport")
                                    .write_request_body(write, false),
                            ) => result,
                        };
                        let result = match result {
                            Ok(result) => result,
                            Err(_) => return Err(self.request_write_timeout_error()),
                        };
                        self.record_timeout_error(&result);
                        result?;
                        DuplexIoProgress::WriteComplete
                    };
                    if progress == DuplexIoProgress::ResponseBuffered {
                        return Ok(());
                    }
                } else {
                    self.writer_state = WriterState::EosWriting;
                    self.drive_eos_writer_once().await?;
                }
            }
            WriterState::EosWriting => {
                // A concurrently ready response can preempt the first EOS poll.
                // Resume that retained writer on the next owner-loop turn.
                self.drive_eos_writer_once().await?;
            }
            WriterState::QuiescedNormalEos
            | WriterState::Cancelling
            | WriterState::QuiescedCancelReset
            | WriterState::HeaderWriteStarted => {}
        }
        // The next owner-loop turn polls the response before issuing another
        // request quantum. Polling again here would not expose the event any
        // earlier to the caller and, for Pingora H1, would allocate and drop a
        // second cancellation-safe response-read future for the same quantum.
        Ok(())
    }

    async fn drive_eos_writer_once(&mut self) -> Result<(), AttemptError> {
        let request_write_deadline = self.ensure_request_write_deadline();
        let cancellation = self.cancellation.clone();
        let progress = if self
            .transport
            .as_ref()
            .expect("active exchange owns transport")
            .supports_duplex_request_body()
        {
            let result = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(AttemptError::Cancelled),
                result = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(request_write_deadline),
                    std::future::poll_fn(|context| self.poll_duplex_body_finish(context)),
                ) => result,
            };
            let result = match result {
                Ok(result) => result,
                Err(_) => return Err(self.request_write_timeout_error()),
            };
            self.record_timeout_error(&result);
            result?
        } else {
            let result = tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Err(AttemptError::Cancelled),
                result = tokio::time::timeout_at(
                    tokio::time::Instant::from_std(request_write_deadline),
                    self.transport
                        .as_mut()
                        .expect("active exchange owns transport")
                        .finish_request_body(),
                ) => result,
            };
            let result = match result {
                Ok(result) => result,
                Err(_) => return Err(self.request_write_timeout_error()),
            };
            self.record_timeout_error(&result);
            result?;
            DuplexIoProgress::WriteComplete
        };
        if progress == DuplexIoProgress::ResponseBuffered {
            return Ok(());
        }
        self.writer_state = WriterState::QuiescedNormalEos;
        self.request_write_elapsed = self
            .request_write_started_at
            .map(|started| started.elapsed());
        self.request.body.release();
        if let Some(telemetry) = &self.telemetry {
            telemetry.release(ReleasePoint::WireSourceConsumedOrCancelled, Duration::ZERO);
            telemetry.release(ReleasePoint::RequestWriterQuiesced, Duration::ZERO);
        }
        Ok(())
    }

    /// Converts an attempt-local synthetic/cache response into a quiesced
    /// no-upstream exchange. This drops the prepared request source and its
    /// lease without connecting or invoking any semantic transport write, so
    /// an #15 readiness may lawfully be validated/published as Accept by #13.
    pub fn quiesce_without_upstream_exchange(&mut self) -> Result<(), AttemptError> {
        self.ensure_active()?;
        if self.writer_state != WriterState::NotStarted
            || self.connected
            || !self.upstream_request_fence.is_clear()
            || self.semantic_upstream_calls != 0
        {
            return Err(AttemptError::SyntheticReplyAfterUpstreamStart);
        }
        self.request.body.release();
        self.writer_state = WriterState::QuiescedNormalEos;
        if let Some(telemetry) = &self.telemetry {
            telemetry.release(ReleasePoint::WireSourceConsumedOrCancelled, Duration::ZERO);
            telemetry.release(ReleasePoint::RequestWriterQuiesced, Duration::ZERO);
        }
        Ok(())
    }

    /// Waits for one response event after first checking the already bounded
    /// precommit mailbox. The absolute attempt/overall deadline and the
    /// phase-local first-byte or stream-idle deadline are both enforced.
    pub async fn wait_precommit_event(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<PrecommitEvent>, AttemptError> {
        self.ensure_active()?;
        if let Some(event) = self.response_window.pop_front() {
            return Ok(Some(event));
        }
        self.ensure_transport_codec_reservation()?;
        self.refresh_transport_suppression();
        let (wait_deadline, timeout_kind, timeout_error) = self.precommit_wait_deadline(deadline);
        let cancellation = self.cancellation.clone();
        let receipt = match tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(AttemptError::Cancelled),
            result = tokio::time::timeout_at(
                tokio::time::Instant::from_std(wait_deadline),
                std::future::poll_fn(|context| {
                    self.transport
                        .as_mut()
                        .expect("active exchange owns transport")
                        .poll_precommit_receipt(context)
                }),
            ) => result,
        } {
            Ok(result) => result?,
            Err(_) => {
                self.timeout_kind = Some(timeout_kind);
                return Err(timeout_error);
            }
        };
        receipt
            .map(|receipt| self.charge_precommit_receipt(receipt))
            .transpose()
    }

    pub fn next_precommit_event(&mut self) -> Option<PrecommitEvent> {
        self.response_window.pop_front()
    }

    fn poll_precommit_now(&mut self) -> Result<Option<PrecommitEvent>, AttemptError> {
        self.ensure_transport_codec_reservation()?;
        let waker = futures::task::noop_waker_ref();
        let mut context = Context::from_waker(waker);
        match self
            .transport
            .as_mut()
            .expect("active exchange owns transport")
            .poll_precommit_receipt(&mut context)
        {
            Poll::Ready(result) => result?
                .map(|receipt| self.charge_precommit_receipt(receipt))
                .transpose(),
            Poll::Pending => Ok(None),
        }
    }

    fn poll_duplex_body_write(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<DuplexIoProgress, AttemptError>> {
        if let Err(error) = self.ensure_transport_codec_reservation() {
            return Poll::Ready(Err(error));
        }
        if self.response_window.len() < self.response_window_capacity {
            let response = self
                .transport
                .as_mut()
                .expect("active exchange owns transport")
                .poll_precommit_receipt(context);
            match response {
                Poll::Ready(Ok(Some(receipt))) => {
                    let event = match self.charge_precommit_receipt(receipt) {
                        Ok(event) => event,
                        Err(error) => return Poll::Ready(Err(error)),
                    };
                    self.response_window.push_back(event);
                    self.response_window_high_water = self
                        .response_window_high_water
                        .max(self.response_window.len());
                    return Poll::Ready(Ok(DuplexIoProgress::ResponseBuffered));
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(None)) | Poll::Pending => {}
            }
        }
        let Some(write) = self.pending_request_write.as_mut() else {
            return Poll::Ready(Err(AttemptError::MissingPendingDuplexWrite));
        };
        match self
            .transport
            .as_mut()
            .expect("active exchange owns transport")
            .poll_write_request_body(context, write, false)
        {
            Poll::Ready(Ok(())) => {
                self.pending_request_write.take();
                Poll::Ready(Ok(DuplexIoProgress::WriteComplete))
            }
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_duplex_body_finish(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<Result<DuplexIoProgress, AttemptError>> {
        if let Err(error) = self.ensure_transport_codec_reservation() {
            return Poll::Ready(Err(error));
        }
        if self.response_window.len() < self.response_window_capacity {
            let response = self
                .transport
                .as_mut()
                .expect("active exchange owns transport")
                .poll_precommit_receipt(context);
            match response {
                Poll::Ready(Ok(Some(receipt))) => {
                    let event = match self.charge_precommit_receipt(receipt) {
                        Ok(event) => event,
                        Err(error) => return Poll::Ready(Err(error)),
                    };
                    self.response_window.push_back(event);
                    self.response_window_high_water = self
                        .response_window_high_water
                        .max(self.response_window.len());
                    return Poll::Ready(Ok(DuplexIoProgress::ResponseBuffered));
                }
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(None)) | Poll::Pending => {}
            }
        }
        match self
            .transport
            .as_mut()
            .expect("active exchange owns transport")
            .poll_finish_request_body(context)
        {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(DuplexIoProgress::WriteComplete)),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn charge_precommit_event(
        &mut self,
        event: TransportPrecommitEvent,
    ) -> Result<PrecommitEvent, AttemptError> {
        match event {
            TransportPrecommitEvent::ResponseHead { status, headers } => {
                let retained_bytes = size_of::<HeaderMap>()
                    .checked_add(
                        headers
                            .capacity()
                            .checked_mul(size_of::<(HeaderName, HeaderValue)>())
                            .ok_or(AttemptError::Body(BodyError::BudgetExceeded))?,
                    )
                    .and_then(|bytes| {
                        bytes.checked_add(
                            headers
                                .iter()
                                .map(|(name, value)| name.as_str().len() + value.as_bytes().len())
                                .sum::<usize>(),
                        )
                    })
                    .ok_or(AttemptError::Body(BodyError::BudgetExceeded))?;
                let reservation = self
                    .budget
                    .reserve(MemoryRole::ResponsePrefix, retained_bytes)?;
                Ok(PrecommitEvent::ResponseHead(ChargedResponseHead {
                    status,
                    // The transport event transfers this owned HeaderMap to
                    // the bounded precommit window. Charge its full bucket
                    // capacity before retaining it; no second parse/copy is
                    // needed at the owner handoff.
                    headers,
                    _reservation: reservation,
                }))
            }
            TransportPrecommitEvent::Body(bytes) => {
                self.precommit_body_owner.admit_chunk(bytes.len())?;
                let charged = ChargedBytes::copy_from_opaque(
                    &self.budget,
                    MemoryRole::ResponsePrefix,
                    &bytes,
                )?;
                Ok(PrecommitEvent::Body(charged))
            }
            TransportPrecommitEvent::EndStream => {
                let admitted = self.precommit_body_owner.finish()?;
                if let Some(telemetry) = &self.telemetry {
                    let queue_high_water_bytes = self
                        .budget
                        .snapshot()
                        .map(|snapshot| snapshot.role_peak[MemoryRole::ResponsePrefix as usize])
                        .unwrap_or(0)
                        .max(
                            self.response_window_high_water
                                .saturating_mul(size_of::<PrecommitEvent>()),
                        );
                    telemetry.body(
                        BodyDirection::AttemptResponsePrecommit,
                        &self.body_plans.attempt_response_precommit,
                        admitted,
                        queue_high_water_bytes,
                    );
                }
                Ok(PrecommitEvent::EndStream)
            }
        }
    }

    fn charge_precommit_receipt(
        &mut self,
        receipt: TransportPrecommitReceipt,
    ) -> Result<PrecommitEvent, AttemptError> {
        let TransportPrecommitReceipt {
            event,
            received_at,
            local_read_suppressed,
            local_read_suppression_total_at_receipt,
        } = receipt;
        let transport_accounts_suppression = self.refresh_transport_suppression();
        if !transport_accounts_suppression {
            self.local_read_suppressed = self
                .local_read_suppressed
                .saturating_add(local_read_suppressed);
        }
        let suppression_after_receipt = if transport_accounts_suppression {
            local_read_suppression_total_at_receipt.map_or(Duration::ZERO, |at_receipt| {
                self.transport_suppression_observed
                    .saturating_sub(at_receipt)
            })
        } else {
            Duration::ZERO
        };
        if self.first_upstream_receipt_at.is_none() {
            self.first_upstream_receipt_at = Some(received_at);
            if let Some(telemetry) = &self.telemetry {
                telemetry.upstream_first_byte(
                    received_at.saturating_duration_since(self.attempt_started_at),
                );
            }
        }
        self.last_upstream_progress_at = Some(received_at);
        self.idle_suppression_credit = suppression_after_receipt;
        if let TransportPrecommitEvent::Body(bytes) = &event {
            self.upstream_body_bytes = self
                .upstream_body_bytes
                .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        }
        self.charge_precommit_event(event)
    }

    fn ensure_request_write_deadline(&mut self) -> Instant {
        let started = *self
            .request_write_started_at
            .get_or_insert_with(Instant::now);
        self.attempt_deadline.min(
            started
                .checked_add(self.timeouts.request_write)
                .unwrap_or(self.attempt_deadline),
        )
    }

    fn request_write_timeout_error(&mut self) -> AttemptError {
        let phase_deadline = self
            .request_write_started_at
            .expect("request write deadline initializes its phase clock")
            .checked_add(self.timeouts.request_write);
        if phase_deadline.is_none_or(|phase_deadline| self.attempt_deadline <= phase_deadline) {
            self.timeout_kind = Some(AttemptTimeoutKind::AttemptDeadline);
            AttemptError::DeadlineExceeded
        } else {
            self.timeout_kind = Some(AttemptTimeoutKind::RequestWrite);
            AttemptError::RequestWriteTimeout
        }
    }

    fn record_timeout_error<R>(&mut self, result: &Result<R, AttemptError>) {
        match result {
            Err(AttemptError::ConnectTimeout) => {
                self.timeout_kind = Some(AttemptTimeoutKind::Connect)
            }
            Err(AttemptError::RequestWriteTimeout) => {
                self.timeout_kind = Some(AttemptTimeoutKind::RequestWrite)
            }
            Err(AttemptError::FirstByteTimeout) => {
                self.timeout_kind = Some(AttemptTimeoutKind::FirstByte)
            }
            Err(AttemptError::StreamIdleTimeout) => {
                self.timeout_kind = Some(AttemptTimeoutKind::StreamIdle)
            }
            Err(AttemptError::DeadlineExceeded) => {
                self.timeout_kind = Some(AttemptTimeoutKind::AttemptDeadline)
            }
            _ => {}
        }
    }

    fn precommit_wait_deadline(
        &self,
        caller_deadline: Instant,
    ) -> (Instant, AttemptTimeoutKind, AttemptError) {
        let absolute_deadline = caller_deadline.min(self.attempt_deadline);
        let (phase_deadline, phase_kind, phase_error) =
            if let Some(last_progress) = self.last_upstream_progress_at {
                (
                    last_progress
                        .checked_add(
                            self.idle_suppression_credit
                                .saturating_add(self.timeouts.stream_idle),
                        )
                        .unwrap_or(absolute_deadline),
                    AttemptTimeoutKind::StreamIdle,
                    AttemptError::StreamIdleTimeout,
                )
            } else {
                let first_byte_anchor = self
                    .request_write_started_at
                    .unwrap_or(self.attempt_started_at);
                (
                    first_byte_anchor
                        .checked_add(self.timeouts.first_byte)
                        .unwrap_or(absolute_deadline),
                    AttemptTimeoutKind::FirstByte,
                    AttemptError::FirstByteTimeout,
                )
            };
        if absolute_deadline <= phase_deadline {
            (
                absolute_deadline,
                AttemptTimeoutKind::AttemptDeadline,
                AttemptError::DeadlineExceeded,
            )
        } else {
            (phase_deadline, phase_kind, phase_error)
        }
    }

    /// Absorbs transport-owned suppression without waiting for another
    /// upstream receipt. This closes the mailbox-full race where local
    /// backpressure ends, the next upstream read is still pending, and an old
    /// receipt timestamp would otherwise expire immediately.
    fn refresh_transport_suppression(&mut self) -> bool {
        let Some(total) = self
            .transport
            .as_ref()
            .and_then(AttemptTransport::local_read_suppression_total)
        else {
            return false;
        };
        let delta = total.saturating_sub(self.transport_suppression_observed);
        self.transport_suppression_observed = total;
        self.local_read_suppressed = self.local_read_suppressed.saturating_add(delta);
        if self.last_upstream_progress_at.is_some() {
            self.idle_suppression_credit = self.idle_suppression_credit.saturating_add(delta);
        }
        true
    }

    fn ensure_transport_codec_reservation(&mut self) -> Result<(), AttemptError> {
        if self.transport_codec_reservation.is_none()
            && self
                .transport
                .as_ref()
                .is_some_and(AttemptTransport::requires_codec_reservation)
        {
            self.transport_codec_reservation = Some(self.budget.reserve(
                MemoryRole::TransportInflight,
                self.target.target().transport_codec_reservation_bytes(),
            )?);
        }
        self.transport
            .as_mut()
            .expect("active exchange owns transport")
            .activate_precommit_reader()
    }
}
