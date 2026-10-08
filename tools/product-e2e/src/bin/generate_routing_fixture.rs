//! Rebuild the current routing fixture from its typed stored business choices.
//! This is a development producer, not a legacy reader or runtime migration.
use hiroute_domain::{AgentModelGrantV2, AgentModelRouteV2, StoredPublicationV1};
use std::path::PathBuf;
fn main() {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("e2e/product/fixtures/routing"));
    let path = root.join("stored-publication.v1.json");
    let mut stored: StoredPublicationV1 =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let plans = stored
        .plans
        .iter()
        .map(|p| p.build().unwrap())
        .collect::<Vec<_>>();
    for grant in &mut stored.grants {
        let mut routes = grant.model_grant.routes.clone();
        for route in routes.values_mut() {
            if let AgentModelRouteV2::Plan {
                plan_id,
                semantic_digest,
                ..
            } = route
            {
                let plan = plans.iter().find(|p| p.agent_plan_id() == plan_id).unwrap();
                *semantic_digest = plan.body.materialized_route_digest.clone();
            }
        }
        grant.model_grant = AgentModelGrantV2::seal(grant.model_grant.protocol, routes).unwrap();
    }
    let publication = stored.build().unwrap();
    assert_eq!(StoredPublicationV1::freeze(&publication).unwrap(), stored);
    let mut compiled = serde_json::to_vec_pretty(&publication).unwrap();
    compiled.push(b'\n');
    std::fs::write(root.join("current-publication.v3.json"), compiled).unwrap();
    let mut bytes = serde_json::to_vec(&stored).unwrap();
    bytes.push(b'\n');
    std::fs::write(path, bytes).unwrap();
}
