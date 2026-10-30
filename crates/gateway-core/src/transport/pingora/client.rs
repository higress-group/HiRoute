use super::*;

mod basic;
mod reader;
mod request;
mod transport;

use request::{build_forward_proxy_header, build_request_header, run_upstream_reader};

pub struct PingoraClientSession {
    registry: Arc<PingoraConnectorRegistry>,
    connection_config_fingerprint: [u8; 32],
    network_route: Option<NetworkDialRoute>,
    connected: Option<ConnectedPingoraSession>,
    request_head_written: bool,
    response_eos_emitted: bool,
}

struct ConnectedPingoraSession {
    connector: Arc<Connector>,
    reader_session: Option<ClientSession>,
    reader: Option<PingoraReaderOwner>,
    writer: Option<PingoraRequestWriter>,
    h1_shared: Option<Arc<SharedH1Stream>>,
    peer: HttpPeer,
    reused: bool,
    protocol: HttpProtocol,
    request_eos: bool,
    forward_proxy_authority: Option<Arc<str>>,
}

struct PingoraReaderOwner {
    events: mpsc::Receiver<Result<TransportPrecommitReceipt, AttemptError>>,
    local_read_suppression: Arc<Mutex<ReaderSuppressionClock>>,
    task: AbortOnDropTask<ClientSession>,
}

#[derive(Default)]
struct ReaderSuppressionClock {
    completed: Duration,
    active_since: Option<Instant>,
}

impl ReaderSuppressionClock {
    fn total(&self) -> Duration {
        self.completed.saturating_add(
            self.active_since
                .map_or(Duration::ZERO, |started| started.elapsed()),
        )
    }

    fn begin(&mut self, started: Instant) {
        debug_assert!(self.active_since.is_none());
        self.active_since = Some(started);
    }

    fn finish(&mut self) -> Duration {
        let elapsed = self
            .active_since
            .take()
            .map_or(Duration::ZERO, |started| started.elapsed());
        self.completed = self.completed.saturating_add(elapsed);
        elapsed
    }
}

enum PingoraRequestWriter {
    H1(H1WriterState),
    H2 {
        stream: h2::SendStream<Bytes>,
        eos: bool,
    },
}

enum H1WriterState {
    Ready(Box<Http1ClientSession>),
    Running(AbortOnDropTask<H1WriterCompletion>),
    Transitioning,
}

struct H1WriterCompletion {
    writer: Box<Http1ClientSession>,
    result: Result<(), AttemptError>,
}

/// A join handle whose task can never detach when a surrounding cleanup
/// future is cancelled. Polling by mutable reference is cancellation-safe;
/// dropping the owning transport or a local cleanup state aborts the task
/// synchronously, and explicit timeout paths can still perform a bounded join.
pub(super) struct AbortOnDropTask<T> {
    task: Option<tokio::task::JoinHandle<T>>,
}

impl<T: Send + 'static> AbortOnDropTask<T> {
    fn spawn(future: impl Future<Output = T> + Send + 'static) -> Self {
        Self {
            task: Some(tokio::spawn(future)),
        }
    }

    fn abort(&self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl<T> Future for AbortOnDropTask<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let poll = Pin::new(
            self.task
                .as_mut()
                .expect("completed abort-on-drop task was polled twice"),
        )
        .poll(context);
        if poll.is_ready() {
            self.task.take();
        }
        poll
    }
}

impl<T> Drop for AbortOnDropTask<T> {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Drop for ConnectedPingoraSession {
    fn drop(&mut self) {
        if let Some(reader) = &self.reader {
            reader.task.abort();
        }
        match self.writer.as_mut() {
            Some(PingoraRequestWriter::H1(H1WriterState::Running(task))) => task.abort(),
            Some(PingoraRequestWriter::H2 { stream, .. }) => {
                stream.send_reset(h2::Reason::CANCEL);
            }
            Some(PingoraRequestWriter::H1(
                H1WriterState::Ready(_) | H1WriterState::Transitioning,
            ))
            | None => {}
        }
    }
}

#[cfg(test)]
mod tests;
