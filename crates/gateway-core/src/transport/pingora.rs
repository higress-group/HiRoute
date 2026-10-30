//! Narrow Pingora adapter pinned by the crate manifest.
//!
//! No `ProxyHttp` type appears here: `GatewayHttpApp` owns the HiRoute
//! lifecycle while Pingora supplies accept, codecs, pooling, TLS, and flow
//! control.

use std::collections::HashMap;
use std::io::Cursor;
use std::io::{self, IoSlice};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use std::{future::Future, pin::Pin};

use async_trait::async_trait;
use bytes::Bytes;
use pingora_core::apps::{
    HttpPersistentSettings, HttpServerApp, HttpServerOptions, ReusedHttpStream,
};
use pingora_core::listeners::ALPN;
use pingora_core::protocols::http::ServerSession;
use pingora_core::protocols::http::client::HttpSession as ClientSession;
use pingora_core::protocols::http::v1::client::HttpSession as Http1ClientSession;
use pingora_core::protocols::{
    Digest, GetProxyDigest, GetSocketDigest, GetTimingDigest, Peek, Shutdown, Ssl, Stream,
    UniqueID, UniqueIDType,
};
use pingora_core::server::ShutdownWatch;
use pingora_core::upstreams::peer::HttpPeer;
use pingora_core::utils::tls::{WrappedX509, parse_x509};
use pingora_http::{RequestHeader, ResponseHeader};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, ReadHalf, WriteHalf};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::execution_plan::{
    CaPolicy, ConfigScopeSnapshot, ConnectionEpochFingerprint, TransportReuseClassId,
    TransportScheme, TransportTarget,
};
use crate::runtime::attempt::{
    AttemptError, AttemptTransport, AttemptTransportFactory, NetworkDialRoute, PreparedRequestHead,
    ProxyDialRoute, ProxyFailurePhase, TransportPrecommitEvent, TransportPrecommitReceipt,
};
use crate::transport::{
    GatewayLifecycle, GatewayRequestHead, GatewayResponseHead, GatewaySession, HttpProtocol,
    SessionReuse, TransportError,
};

pub const PINNED_PINGORA_REVISION: &str = "0046038bd402bc82912da862dadf9a479f31e9f1";
const SHUTDOWN_REQUEST_JOIN_TIMEOUT: Duration = Duration::from_secs(2);
const UPSTREAM_READER_MAILBOX_CAPACITY: usize = 1;

mod client;
mod connector;
mod proxy;
mod server;
mod shared_h1;

pub use client::PingoraClientSession;
pub use connector::PingoraConnectorAdapter;
pub use server::GatewayHttpApp;

use connector::{PingoraConnectorRegistry, build_peer};
use proxy::NetworkConnector as Connector;
use shared_h1::{SharedH1Io, SharedH1Mode, SharedH1Role, SharedH1Stream, flatten_reused_h1_stream};
