use super::*;
use hiroute_domain::{CompiledAgentPlanV1, GatewayPublicationRevision, PublicationRecordV1};
use hiroute_gateway::server::dispatch::GatewayRequestAuthority;
use hiroute_gateway::server::publication::GatewayCatalog;

#[test]
fn publication_plan_update_preserves_two_independent_agent_scopes_and_old_request_pin() {
    let mut publication = serde_json::from_slice::<hiroute_domain::GatewayPublicationV1>(
        include_bytes!("../../../../../e2e/product/fixtures/routing/current-publication.v3.json"),
    )
    .unwrap()
    .into_current()
    .unwrap();
    let tokens = ["scope-test-claude", "scope-test-codex"];
    for (grant, token) in publication.grants.iter_mut().zip(tokens) {
        grant.bearer_token_sha256 = CanonicalDigest::of_bytes(token.as_bytes());
    }
    let before = publication.clone();
    let directory = tempfile::tempdir().unwrap();
    let target = Arc::new(GatewayPublicationAdapter::new(Arc::new(
        GatewayPublicationInstaller::open(directory.path().join("lkg.json")).unwrap(),
    )));
    target
        .activate_verified(
            &PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
                .unwrap(),
        )
        .unwrap();
    target.resume_requests().unwrap();
    let authority = GatewayRequestAuthority::new(target.clone());
    let catalog = GatewayCatalog::new(target.clone());
    let changed_alias = publication.plans[0]
        .body
        .identity
        .model_alias
        .as_str()
        .to_owned();
    let old = authority
        .begin(IngressProtocol::Messages, Some("Bearer scope-test-claude"))
        .unwrap()
        .authorize_alias(&changed_alias, Instant::now())
        .unwrap();
    let mut body = publication.plans[0].body.as_ref().clone();
    body.agent_plan_revision += 1;
    publication.plans[0] = CompiledAgentPlanV1::seal_current(body).unwrap();
    publication
        .aliases
        .iter_mut()
        .find(|alias| alias.served_model_id.as_str() == changed_alias)
        .unwrap()
        .agent_plan_revision += 1;
    publication.publication_revision = GatewayPublicationRevision::new(12).unwrap();
    let after =
        PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
            .unwrap();
    target.suspend_requests().unwrap();
    assert!(catalog.get(Some("Bearer scope-test-claude"), None).is_err());
    assert!(
        authority
            .begin(IngressProtocol::Messages, Some("Bearer scope-test-claude"))
            .is_err()
    );
    target.activate_verified(&after).unwrap();
    target.resume_requests().unwrap();
    assert_eq!(&publication.plans[1..], &before.plans[1..]);
    assert_eq!(publication.grants, before.grants);
    assert_eq!(old.publication_revision(), 11);
    assert_eq!(old.agent_plan_revision(), Some(1));
    for ((grant, token), protocol) in publication
        .grants
        .iter()
        .zip(tokens)
        .zip([IngressProtocol::Messages, IngressProtocol::Responses])
    {
        let bearer = format!("Bearer {token}");
        let rendered = catalog.get(Some(&bearer), None).unwrap();
        let document: serde_json::Value = serde_json::from_slice(&rendered.body).unwrap();
        let visible: Vec<_> = document["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            visible,
            grant.model_grant.routes.keys().cloned().collect::<Vec<_>>()
        );
        for alias in &publication.aliases {
            let request = authority
                .begin(protocol, Some(&bearer))
                .unwrap()
                .authorize_alias(alias.served_model_id.as_str(), Instant::now());
            if grant.permits_plan(&alias.served_model_id) {
                assert_eq!(request.unwrap().publication_revision(), 12);
            } else {
                assert!(request.is_err(), "cross-Agent alias leaked");
            }
        }
    }
}
