//! Standalone HiRoute gateway process boundary.
//!
//! The production listener, frozen request authority, protocol adapters,
//! deterministic Planner and gateway-core lifecycle have explicit ownership.
//! Request-owned Replay plugs into that lifecycle without creating a second
//! request chain; observation remains outside this package.

#![cfg_attr(not(windows), forbid(unsafe_code))]
#![cfg_attr(windows, deny(unsafe_code))]

pub(crate) mod agent_turn_history;
pub mod attempt_outcome;
pub mod content_ref;
pub(crate) mod context_hold;
pub mod ports;
pub mod replay;
pub mod runtime;
pub mod server;
