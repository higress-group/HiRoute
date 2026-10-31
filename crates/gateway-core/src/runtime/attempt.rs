use std::collections::VecDeque;
use std::mem::size_of;
use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::core::execution_plan::{
    AttemptBodyPlans, AttemptTimeouts, CompiledAttemptPlan, ConfigScopeSnapshot, PlanRevision,
    TransportTarget, TransportTargetPolicy,
};
use crate::runtime::body::{
    BodyDirection, BodyError, BodyPlanExecutor, ChargedBodyQueue, ChargedBytes, FramingLedger,
    HttpFraming, MemoryRole, RequestBodyLease, Reservation, StreamBudget,
};
use crate::runtime::sse::SemanticProvenance;
use crate::runtime::telemetry::{
    CleanupKind, DispositionStage, ErrorClass, FenceKind, ReleasePoint, RequestTelemetry,
};
use crate::transport::HttpProtocol;

mod error;
mod exchange;
mod gate;
mod network;
mod transport;

pub use error::*;
pub use exchange::*;
pub(crate) use gate::PreexchangeDispositionGate;
pub use gate::*;
pub use network::*;
pub use transport::*;
