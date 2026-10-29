use std::future::pending;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::runtime::body::{ChargedBytes, MemoryRole, StreamBudget};

#[cfg(feature = "pingora-transport")]
pub mod pingora;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpProtocol {
    Http1,
    Http2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayRequestHead {
    pub method: Method,
    pub path_and_query: Arc<str>,
    /// URI authority as carried by the protocol. HTTP/2 transports preserve
    /// `:authority` here because it is not required to synthesize a `Host`
    /// header; HTTP/1 adapters may leave it absent and rely on `Host`.
    pub authority: Option<Arc<str>>,
    pub headers: HeaderMap,
    pub protocol: HttpProtocol,
}

#[derive(Clone, Debug)]
pub struct GatewayResponseHead {
    pub status: StatusCode,
    pub headers: HeaderMap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionReuse {
    Reusable,
    Close,
}

#[async_trait]
pub trait GatewaySession: Send {
    fn request_head(&self) -> Result<GatewayRequestHead, TransportError>;
    /// Request-scoped cancellation supplied by the listener owner. The
    /// concrete Pingora adapter cancels this token on server shutdown; test
    /// and embedding adapters may also use it for an explicit client cancel.
    fn cancellation_token(&self) -> CancellationToken {
        CancellationToken::new()
    }
    /// Waits until the downstream stream/connection is no longer live. The
    /// lifecycle selects this alongside every potentially blocking upstream,
    /// provider, and filter operation so no detached watcher owns the session.
    async fn wait_for_disconnect(&mut self) {
        pending::<()>().await;
    }
    async fn read_request_body(&mut self) -> Result<Option<Bytes>, TransportError>;
    /// Bounds the sole opaque body reader by an already-frozen logical
    /// request deadline. This default keeps every concrete transport adapter
    /// on the same cancellation-safe timer without adding another read owner.
    async fn read_request_body_before(
        &mut self,
        deadline: Instant,
    ) -> Result<Option<Bytes>, TransportError> {
        tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.read_request_body(),
        )
        .await
        .map_err(|_| TransportError::DeadlineExceeded)?
    }
    /// Acquires the opaque-codec allowance before polling the sole body read
    /// owner, then attaches that permit to the returned backing without a
    /// second allocation.
    async fn read_request_body_charged(
        &mut self,
        budget: &StreamBudget,
        max_chunk_bytes: usize,
    ) -> Result<Option<ChargedBytes>, TransportError> {
        let permit = budget
            .reserve(MemoryRole::TransportInflight, max_chunk_bytes)
            .map_err(|error| TransportError::Io(error.to_string().into()))?;
        let Some(bytes) = self.read_request_body().await? else {
            return Ok(None);
        };
        if bytes.len() > permit.bytes() {
            return Err(TransportError::Io(
                "opaque request chunk exceeded its pre-admitted bound".into(),
            ));
        }
        // The conservative pre-read permit remains live while the unknown
        // codec backing is copied into an exact charged owner. Once both the
        // opaque Bytes and permit drop, retained request frames pin only their
        // exact backing rather than one max-sized reservation per codec read.
        let exact =
            ChargedBytes::copy_from_opaque(budget, MemoryRole::TransportInflight, bytes.as_ref())
                .map_err(|error| TransportError::Io(error.to_string().into()))?;
        drop(bytes);
        drop(permit);
        Ok(Some(exact))
    }
    async fn write_response_head(
        &mut self,
        head: GatewayResponseHead,
    ) -> Result<(), TransportError>;
    async fn write_response_body(
        &mut self,
        body: Bytes,
        end_stream: bool,
    ) -> Result<(), TransportError>;
    /// Moves the charge into the wire Bytes owner, so codec clones retain the
    /// budget reservation until the final clone is actually released.
    async fn write_response_body_charged(
        &mut self,
        body: Option<ChargedBytes>,
        end_stream: bool,
    ) -> Result<(), TransportError> {
        let bytes = body.map_or_else(Bytes::new, ChargedBytes::into_drop_tracked_bytes);
        self.write_response_body(bytes, end_stream).await
    }
}

#[async_trait]
pub trait GatewayLifecycle: Send + Sync + 'static {
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError>;
}

#[async_trait]
impl<L> GatewayLifecycle for Arc<L>
where
    L: GatewayLifecycle,
{
    async fn process(
        &self,
        session: &mut dyn GatewaySession,
    ) -> Result<SessionReuse, TransportError> {
        self.as_ref().process(session).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use http::Method;

    use super::*;
    use crate::runtime::body::{BudgetTree, MemoryRole};

    struct CodecProbeSession {
        budget: StreamBudget,
        read_live_at_poll: Arc<AtomicUsize>,
        request_body: Option<Bytes>,
        retained_wire_clone: Option<Bytes>,
    }

    impl CodecProbeSession {
        fn new(budget: &StreamBudget, request_body: Option<Bytes>) -> Self {
            Self {
                budget: budget.clone(),
                read_live_at_poll: Arc::new(AtomicUsize::new(0)),
                request_body,
                retained_wire_clone: None,
            }
        }
    }

    #[async_trait]
    impl GatewaySession for CodecProbeSession {
        fn request_head(&self) -> Result<GatewayRequestHead, TransportError> {
            Ok(GatewayRequestHead {
                method: Method::POST,
                path_and_query: Arc::from("/"),
                authority: Some(Arc::from("gateway.test")),
                headers: HeaderMap::new(),
                protocol: HttpProtocol::Http2,
            })
        }

        async fn read_request_body(&mut self) -> Result<Option<Bytes>, TransportError> {
            self.read_live_at_poll.store(
                self.budget
                    .snapshot()
                    .expect("probe stream remains live")
                    .role_live[MemoryRole::TransportInflight as usize],
                Ordering::Release,
            );
            Ok(self.request_body.take())
        }

        async fn write_response_head(
            &mut self,
            _head: GatewayResponseHead,
        ) -> Result<(), TransportError> {
            Ok(())
        }

        async fn write_response_body(
            &mut self,
            body: Bytes,
            _end_stream: bool,
        ) -> Result<(), TransportError> {
            self.retained_wire_clone = Some(body.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn opaque_request_read_is_reserved_before_polling_codec() {
        let tree = BudgetTree::new(128, 128).expect("budget tree");
        let budget = tree.stream(128).expect("stream budget");
        let mut session = CodecProbeSession::new(&budget, Some(Bytes::from_static(b"body")));
        let charged = session
            .read_request_body_charged(&budget, 32)
            .await
            .expect("charged read")
            .expect("request frame");

        assert_eq!(session.read_live_at_poll.load(Ordering::Acquire), 32);
        assert_eq!(charged.role(), MemoryRole::TransportInflight);
        assert_eq!(charged.retained_capacity(), 4);
        drop(charged);
        assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    }

    #[tokio::test]
    async fn downstream_codec_clone_owns_charge_until_final_drop() {
        let tree = BudgetTree::new(128, 128).expect("budget tree");
        let budget = tree.stream(128).expect("stream budget");
        let mut session = CodecProbeSession::new(&budget, None);
        let body = ChargedBytes::copy_from_opaque(&budget, MemoryRole::OutputQueue, b"response")
            .expect("charged response");
        let charged_capacity = body.retained_capacity();

        session
            .write_response_body_charged(Some(body), true)
            .await
            .expect("wire write");
        assert_eq!(
            budget.snapshot().expect("snapshot").live,
            charged_capacity,
            "the codec-retained Bytes clone must keep the reservation live",
        );
        session.retained_wire_clone.take();
        assert_eq!(budget.snapshot().expect("snapshot").live, 0);
    }

    struct PendingBodySession;

    #[async_trait]
    impl GatewaySession for PendingBodySession {
        fn request_head(&self) -> Result<GatewayRequestHead, TransportError> {
            Err(TransportError::InvalidMetadata("unused".into()))
        }

        async fn read_request_body(&mut self) -> Result<Option<Bytes>, TransportError> {
            pending().await
        }

        async fn write_response_head(
            &mut self,
            _head: GatewayResponseHead,
        ) -> Result<(), TransportError> {
            Ok(())
        }

        async fn write_response_body(
            &mut self,
            _body: Bytes,
            _end_stream: bool,
        ) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn opaque_body_reader_obeys_the_preexisting_logical_deadline() {
        let mut session = PendingBodySession;
        let error = session
            .read_request_body_before(Instant::now() + std::time::Duration::from_millis(10))
            .await
            .expect_err("trickle input must be bounded");
        assert_eq!(error, TransportError::DeadlineExceeded);
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TransportError {
    /// The request owner observed cancellation, independently of whether an
    /// upstream attempt or a downstream response had started.
    #[error("request was cancelled")]
    Cancelled,
    #[error("transport I/O failed: {0}")]
    Io(Arc<str>),
    #[error("transport metadata is invalid: {0}")]
    InvalidMetadata(Arc<str>),
    #[error("transport capability is unsupported: {0}")]
    Unsupported(Arc<str>),
    #[error("transport operation exceeded the frozen logical request deadline")]
    DeadlineExceeded,
}
