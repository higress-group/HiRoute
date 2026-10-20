//! Shared retained analysis. These tables are projections of existing source
//! facts, never session identity, billing or routing authority.
mod comparison;
mod cost;
mod legacy;
mod trust_cost;
pub(crate) use legacy::ProjectedComparison;
pub(crate) use legacy::projected_comparison;
pub(crate) use trust_cost::trust_comparison;
mod members;
mod metadata;
mod native;
mod native_origin;
mod prices;
mod projection;
mod read;
#[cfg(test)]
mod read_admission_tests;
mod routing;
pub(crate) use routing::transitions as routing_transitions;
pub(crate) use views::Row as RoutingRow;
mod schema;
mod scope;
mod selection;
mod turns;
mod views;
pub(crate) use metadata::project as project_metadata;
pub(crate) use native::full_delete;

pub(crate) use projection::sync;
pub(crate) use schema::migrate;

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|n| i64::try_from(n.as_millis()).ok())
        .unwrap_or(0)
}
