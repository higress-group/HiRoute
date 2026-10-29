use super::*;

pub(super) const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EofPolicy {
    /// Reject unfinished fields; a comment-only tail has no pending event.
    Strict,
    DropIncompleteObserver,
    ProviderFinalEvent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SseLimits {
    pub max_event_bytes: usize,
    pub max_pending_bytes: usize,
    pub max_output_event_bytes: usize,
    pub expansion_ratio_numerator: usize,
    pub expansion_ratio_denominator: usize,
    pub expansion_slack_bytes: usize,
    pub retained_capacity_threshold: usize,
    pub eof_policy: EofPolicy,
}

impl SseLimits {
    pub fn validate(&self) -> Result<(), SseError> {
        if self.max_event_bytes == 0
            || self.max_pending_bytes == 0
            || self.max_pending_bytes < self.max_event_bytes
            || self.max_output_event_bytes == 0
            || self.expansion_ratio_numerator == 0
            || self.expansion_ratio_denominator == 0
            || self.retained_capacity_threshold == 0
        {
            return Err(SseError::InvalidLimits);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SseFeedOutcome {
    Complete,
    /// The visitor's bounded handoff is full. `consumed_bytes` describes the
    /// accepted prefix of the supplied chunk; the framer retains a charged,
    /// exact owner for the unconsumed suffix until `resume` succeeds.
    NeedDrain {
        consumed_bytes: usize,
    },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SseComplexity {
    pub scanned_bytes: usize,
    pub copied_bytes: usize,
    pub allocations: usize,
    pub promoted_events: usize,
}

#[derive(Clone, Copy, Debug)]
struct ScanState {
    at_line_start: bool,
    pending_cr_blank: Option<bool>,
}

impl Default for ScanState {
    fn default() -> Self {
        Self {
            at_line_start: true,
            pending_cr_blank: None,
        }
    }
}

impl ScanState {
    fn next_boundary(
        &mut self,
        bytes: &[u8],
        cursor: &mut usize,
        scanned: &mut usize,
    ) -> Option<usize> {
        if let Some(blank) = self.pending_cr_blank.take() {
            if *cursor >= bytes.len() {
                self.pending_cr_blank = Some(blank);
                return None;
            }
            if bytes[*cursor] == b'\n' {
                *cursor += 1;
                *scanned += 1;
            }
            self.at_line_start = true;
            if blank {
                return Some(*cursor);
            }
        }

        while *cursor < bytes.len() {
            match bytes[*cursor] {
                b'\r' => {
                    let blank = self.at_line_start;
                    *cursor += 1;
                    *scanned += 1;
                    if *cursor == bytes.len() {
                        self.pending_cr_blank = Some(blank);
                        return None;
                    }
                    if bytes[*cursor] == b'\n' {
                        *cursor += 1;
                        *scanned += 1;
                    }
                    self.at_line_start = true;
                    if blank {
                        return Some(*cursor);
                    }
                }
                b'\n' => {
                    let blank = self.at_line_start;
                    *cursor += 1;
                    *scanned += 1;
                    self.at_line_start = true;
                    if blank {
                        return Some(*cursor);
                    }
                }
                _ => {
                    *cursor += 1;
                    *scanned += 1;
                    self.at_line_start = false;
                }
            }
        }
        None
    }

    fn finish_boundary_at_eof(&mut self) -> bool {
        let boundary = self.pending_cr_blank.take().unwrap_or(false);
        self.at_line_start = true;
        boundary
    }
}

struct PendingBuffer {
    bytes: Vec<u8>,
    charge: Option<Reservation>,
}

struct DeferredInput {
    bytes: ChargedBytes,
    end_stream: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FeedProgress {
    Complete,
    NeedDrain { retain_from: usize },
}

impl PendingBuffer {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            charge: None,
        }
    }

    fn append(
        &mut self,
        input: &[u8],
        budget: &StreamBudget,
        limits: &SseLimits,
        complexity: &mut SseComplexity,
    ) -> Result<(), SseError> {
        let required = self
            .bytes
            .len()
            .checked_add(input.len())
            .ok_or(SseError::PendingLimit)?;
        if required > limits.max_pending_bytes || required > limits.max_event_bytes {
            return Err(SseError::PendingLimit);
        }
        if required > self.bytes.capacity() {
            // Reserve the replacement before allocating/copying so the
            // transient old+new peak is represented by the hierarchy.
            let replacement_capacity = required
                .max(self.bytes.capacity().saturating_mul(2).max(1))
                .min(limits.max_pending_bytes);
            let replacement_charge = budget
                .reserve(MemoryRole::SseFrame, replacement_capacity)
                .map_err(|_| SseError::BudgetExceeded)?;
            let mut replacement = Vec::with_capacity(replacement_capacity);
            replacement.extend_from_slice(&self.bytes);
            replacement.extend_from_slice(input);
            complexity.copied_bytes += self.bytes.len() + input.len();
            complexity.allocations += 1;
            self.bytes = replacement;
            self.charge = Some(replacement_charge);
        } else {
            self.bytes.extend_from_slice(input);
            complexity.copied_bytes += input.len();
        }
        Ok(())
    }

    fn clear_event(&mut self, retain_threshold: usize) {
        self.bytes.clear();
        if self.bytes.capacity() > retain_threshold {
            self.clear_and_release();
        }
    }

    fn clear_and_release(&mut self) {
        self.bytes = Vec::new();
        self.charge = None;
    }
}

pub struct SseFramer {
    limits: SseLimits,
    budget: StreamBudget,
    pending: PendingBuffer,
    pending_scan: ScanState,
    pending_complete: bool,
    pending_complete_end_stream: bool,
    deferred: Option<DeferredInput>,
    first_event: bool,
    failed: bool,
    complexity: SseComplexity,
}

impl SseFramer {
    pub fn new(limits: SseLimits, budget: StreamBudget) -> Result<Self, SseError> {
        limits.validate()?;
        Ok(Self {
            limits,
            budget,
            pending: PendingBuffer::new(),
            pending_scan: ScanState::default(),
            pending_complete: false,
            pending_complete_end_stream: false,
            deferred: None,
            first_event: true,
            failed: false,
            complexity: SseComplexity::default(),
        })
    }

    pub fn complexity(&self) -> SseComplexity {
        self.complexity
    }

    pub fn pending_bytes(&self) -> usize {
        self.pending.bytes.len()
    }

    pub fn reset(&mut self) {
        self.pending.clear_and_release();
        self.pending_scan = ScanState::default();
        self.pending_complete = false;
        self.pending_complete_end_stream = false;
        self.deferred = None;
        self.first_event = true;
        // A protocol/budget failure is terminal for this stream owner. Reset
        // may discard a healthy partial stream, but it must not resurrect a
        // failed framer and accidentally reinterpret later transport bytes.
    }

    pub fn feed<V, S>(
        &mut self,
        input: ChargedBytes,
        end_stream: bool,
        visitor: &mut V,
        sink: &mut S,
    ) -> Result<SseFeedOutcome, SseError>
    where
        V: SseVisitor,
        S: BoundedOutputSink,
    {
        if self.failed {
            return Err(SseError::FramerFailed);
        }
        if self.pending_complete || self.deferred.is_some() {
            return Err(SseError::DrainRequired);
        }
        let result = self.process_owned(input, end_stream, visitor, sink);
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.fail_terminal();
                Err(error)
            }
        }
    }

    /// Continues a recoverably blocked visitor handoff. No new transport bytes
    /// may be supplied until this returns `Complete`.
    pub fn resume<V, S>(
        &mut self,
        visitor: &mut V,
        sink: &mut S,
    ) -> Result<SseFeedOutcome, SseError>
    where
        V: SseVisitor,
        S: BoundedOutputSink,
    {
        if self.failed {
            return Err(SseError::FramerFailed);
        }
        let result = (|| {
            if self.pending_complete {
                let first_event = self.first_event;
                match dispatch_event(
                    &self.pending.bytes,
                    first_event,
                    &self.limits,
                    &self.budget,
                    visitor,
                    sink,
                    &mut self.complexity,
                ) {
                    Ok(()) => {
                        self.first_event = false;
                        self.pending
                            .clear_event(self.limits.retained_capacity_threshold);
                        self.pending_scan = ScanState::default();
                        self.pending_complete = false;
                    }
                    Err(SseError::NeedDrain) => {
                        return Ok(SseFeedOutcome::NeedDrain { consumed_bytes: 0 });
                    }
                    Err(error) => return Err(error),
                }
                if std::mem::take(&mut self.pending_complete_end_stream) {
                    return Ok(SseFeedOutcome::Complete);
                }
            }
            let Some(deferred) = self.deferred.take() else {
                return Ok(SseFeedOutcome::Complete);
            };
            self.process_owned(deferred.bytes, deferred.end_stream, visitor, sink)
        })();
        match result {
            Ok(outcome) => Ok(outcome),
            Err(error) => {
                self.fail_terminal();
                Err(error)
            }
        }
    }

    pub fn needs_drain(&self) -> bool {
        self.pending_complete || self.deferred.is_some()
    }

    fn fail_terminal(&mut self) {
        self.failed = true;
        self.pending.clear_and_release();
        self.pending_scan = ScanState::default();
        self.pending_complete = false;
        self.pending_complete_end_stream = false;
        self.deferred = None;
    }

    fn process_owned<V, S>(
        &mut self,
        input: ChargedBytes,
        end_stream: bool,
        visitor: &mut V,
        sink: &mut S,
    ) -> Result<SseFeedOutcome, SseError>
    where
        V: SseVisitor,
        S: BoundedOutputSink,
    {
        let progress = if self.pending.bytes.is_empty() {
            self.feed_direct(input.bytes().as_ref(), visitor, sink)?
        } else {
            self.feed_after_pending(input.bytes().as_ref(), end_stream, visitor, sink)?
        };
        if let FeedProgress::NeedDrain { retain_from } = progress {
            let input_len = input.bytes().len();
            if retain_from < input_len {
                let retained = if retain_from == 0 {
                    input
                        .transfer_role(MemoryRole::SseFrame)
                        .map_err(|_| SseError::BudgetExceeded)?
                } else {
                    ChargedBytes::copy_from_opaque(
                        &self.budget,
                        MemoryRole::SseFrame,
                        &input.bytes()[retain_from..],
                    )
                    .map_err(|_| SseError::BudgetExceeded)?
                };
                self.deferred = Some(DeferredInput {
                    bytes: retained,
                    end_stream,
                });
            } else if end_stream {
                self.pending_complete_end_stream = true;
            }
            return Ok(SseFeedOutcome::NeedDrain {
                consumed_bytes: retain_from,
            });
        }
        if end_stream {
            match self.finish_eof(visitor, sink)? {
                FeedProgress::Complete => {}
                FeedProgress::NeedDrain { .. } => {
                    self.pending_complete_end_stream = true;
                    return Ok(SseFeedOutcome::NeedDrain {
                        consumed_bytes: input.bytes().len(),
                    });
                }
            }
        }
        Ok(SseFeedOutcome::Complete)
    }

    fn feed_direct<V, S>(
        &mut self,
        data: &[u8],
        visitor: &mut V,
        sink: &mut S,
    ) -> Result<FeedProgress, SseError>
    where
        V: SseVisitor,
        S: BoundedOutputSink,
    {
        let mut scan = ScanState::default();
        let mut cursor = 0;
        let mut event_start = 0;
        while let Some(end) =
            scan.next_boundary(data, &mut cursor, &mut self.complexity.scanned_bytes)
        {
            if end - event_start > self.limits.max_event_bytes {
                return Err(SseError::EventLimit);
            }
            match self.dispatch(&data[event_start..end], visitor, sink) {
                Ok(()) => {}
                Err(SseError::NeedDrain) => {
                    return Ok(FeedProgress::NeedDrain {
                        retain_from: event_start,
                    });
                }
                Err(error) => return Err(error),
            }
            event_start = end;
        }
        if event_start < data.len() {
            let tail = &data[event_start..];
            self.pending
                .append(tail, &self.budget, &self.limits, &mut self.complexity)?;
            self.pending_scan = scan;
        }
        Ok(FeedProgress::Complete)
    }

    fn feed_after_pending<V, S>(
        &mut self,
        data: &[u8],
        end_stream: bool,
        visitor: &mut V,
        sink: &mut S,
    ) -> Result<FeedProgress, SseError>
    where
        V: SseVisitor,
        S: BoundedOutputSink,
    {
        let mut cursor = 0;
        let boundary =
            self.pending_scan
                .next_boundary(data, &mut cursor, &mut self.complexity.scanned_bytes);
        let append_end = boundary.unwrap_or(data.len());
        let event_bytes = self
            .pending
            .bytes
            .len()
            .checked_add(append_end)
            .ok_or(SseError::EventLimit)?;
        if event_bytes > self.limits.max_event_bytes {
            return Err(SseError::EventLimit);
        }
        self.pending.append(
            &data[..append_end],
            &self.budget,
            &self.limits,
            &mut self.complexity,
        )?;
        let Some(end) = boundary else {
            return Ok(FeedProgress::Complete);
        };

        // Only the prefix completing the prior partial event was copied. All
        // subsequent whole events remain borrowed from this transport chunk.
        let first_event = self.first_event;
        match dispatch_event(
            &self.pending.bytes,
            first_event,
            &self.limits,
            &self.budget,
            visitor,
            sink,
            &mut self.complexity,
        ) {
            Ok(()) => {}
            Err(SseError::NeedDrain) => {
                self.pending_complete = true;
                self.pending_complete_end_stream = end_stream && end == data.len();
                return Ok(FeedProgress::NeedDrain { retain_from: end });
            }
            Err(error) => return Err(error),
        }
        self.first_event = false;
        self.pending
            .clear_event(self.limits.retained_capacity_threshold);
        self.pending_scan = ScanState::default();
        match self.feed_direct(&data[end..], visitor, sink)? {
            FeedProgress::Complete => Ok(FeedProgress::Complete),
            FeedProgress::NeedDrain { retain_from } => Ok(FeedProgress::NeedDrain {
                retain_from: end + retain_from,
            }),
        }
    }

    fn dispatch<V, S>(&mut self, raw: &[u8], visitor: &mut V, sink: &mut S) -> Result<(), SseError>
    where
        V: SseVisitor,
        S: BoundedOutputSink,
    {
        dispatch_event(
            raw,
            self.first_event,
            &self.limits,
            &self.budget,
            visitor,
            sink,
            &mut self.complexity,
        )?;
        self.first_event = false;
        Ok(())
    }

    fn finish_eof<V, S>(&mut self, visitor: &mut V, sink: &mut S) -> Result<FeedProgress, SseError>
    where
        V: SseVisitor,
        S: BoundedOutputSink,
    {
        if self.pending.bytes.is_empty() {
            return Ok(FeedProgress::Complete);
        }
        // A comment-only tail carries no event data or control fields awaiting a
        // separator. Use the shared field parser, then the same bounded dispatch
        // path: NeedDrain retains this charged buffer and retries it unchanged.
        let complete = self.pending_scan.finish_boundary_at_eof()
            || SseEventView {
                raw: &self.pending.bytes,
                strip_bom: self.first_event && self.pending.bytes.starts_with(UTF8_BOM),
                _not_send: PhantomData,
            }
            .fields()
            .all(|field| field.comment);
        if complete {
            let first_event = self.first_event;
            match dispatch_event(
                &self.pending.bytes,
                first_event,
                &self.limits,
                &self.budget,
                visitor,
                sink,
                &mut self.complexity,
            ) {
                Ok(()) => {}
                Err(SseError::NeedDrain) => {
                    self.pending_complete = true;
                    return Ok(FeedProgress::NeedDrain {
                        retain_from: self.pending.bytes.len(),
                    });
                }
                Err(error) => return Err(error),
            }
            self.first_event = false;
            self.pending.clear_and_release();
            self.pending_scan = ScanState::default();
            return Ok(FeedProgress::Complete);
        }
        match self.limits.eof_policy {
            EofPolicy::Strict => Err(SseError::IncompleteEventAtEof),
            EofPolicy::DropIncompleteObserver => {
                self.pending.clear_and_release();
                self.pending_scan = ScanState::default();
                Ok(FeedProgress::Complete)
            }
            EofPolicy::ProviderFinalEvent => {
                let first_event = self.first_event;
                match dispatch_event(
                    &self.pending.bytes,
                    first_event,
                    &self.limits,
                    &self.budget,
                    visitor,
                    sink,
                    &mut self.complexity,
                ) {
                    Ok(()) => {}
                    Err(SseError::NeedDrain) => {
                        self.pending_complete = true;
                        return Ok(FeedProgress::NeedDrain {
                            retain_from: self.pending.bytes.len(),
                        });
                    }
                    Err(error) => return Err(error),
                }
                self.first_event = false;
                self.pending.clear_and_release();
                self.pending_scan = ScanState::default();
                Ok(FeedProgress::Complete)
            }
        }
    }
}

fn dispatch_event<V, S>(
    raw: &[u8],
    first_event: bool,
    limits: &SseLimits,
    budget: &StreamBudget,
    visitor: &mut V,
    sink: &mut S,
    complexity: &mut SseComplexity,
) -> Result<(), SseError>
where
    V: SseVisitor,
    S: BoundedOutputSink,
{
    let view = SseEventView {
        raw,
        strip_bom: first_event && raw.starts_with(UTF8_BOM),
        _not_send: PhantomData,
    };
    let mut emitter = BoundedEventEmitter {
        raw,
        limits,
        budget,
        sink,
        emitted: 0,
        emitted_bytes: 0,
        decided: false,
    };
    visitor.on_event(view, &mut emitter)?;
    if !emitter.decided {
        return Err(SseError::NoEmitterDecision);
    }
    complexity.promoted_events += emitter.emitted;
    Ok(())
}
