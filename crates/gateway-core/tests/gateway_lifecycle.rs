#![cfg(feature = "pingora-transport")]

#[path = "gateway_lifecycle/body_and_framing.rs"]
mod body_and_framing;
#[path = "gateway_lifecycle/configuration.rs"]
mod configuration;
#[path = "gateway_lifecycle/deadlines_and_cancel.rs"]
mod deadlines_and_cancel;
#[path = "gateway_lifecycle/downstream_completion.rs"]
mod downstream_completion;
#[path = "gateway_lifecycle/fallback_and_selection.rs"]
mod fallback_and_selection;
#[path = "gateway_lifecycle/fixture/mod.rs"]
mod fixture;
#[path = "gateway_lifecycle/local_replies.rs"]
mod local_replies;
#[path = "gateway_lifecycle/native_filters.rs"]
mod native_filters;
#[path = "gateway_lifecycle/preexchange.rs"]
mod preexchange;
#[path = "gateway_lifecycle/routing_and_facts.rs"]
mod routing_and_facts;
#[path = "gateway_lifecycle/sse.rs"]
mod sse;
#[path = "gateway_lifecycle/terminal_cleanup.rs"]
mod terminal_cleanup;
#[path = "gateway_lifecycle/transport.rs"]
mod transport;
