use super::*;

#[test]
fn publication_revision_gap_is_rejected_before_lkg_or_core_swap() {
    let directory = TestDirectory::new();
    let lkg = directory.path().join("publication.json");
    let installer = GatewayPublicationInstaller::open(&lkg).unwrap();
    publish(&installer, snapshot(1, "renderer-1"));
    let last_good = std::fs::read(&lkg).unwrap();

    assert!(matches!(
        installer.prepare(snapshot(3, "renderer-3")),
        Err(PublicationInstallError::Core(
            InstallError::ResyncRequired { .. }
        ))
    ));
    assert_eq!(installer.active().unwrap().publication_revision(), 1);
    assert_eq!(std::fs::read(&lkg).unwrap(), last_good);
    drop(installer);
    assert_eq!(
        GatewayPublicationInstaller::open(&lkg)
            .unwrap()
            .active()
            .unwrap()
            .publication_revision(),
        1
    );
}

#[test]
fn active_plan_revision_accepts_grant_reachable_protocol_expansion() {
    let directory = TestDirectory::new();
    let lkg = directory.path().join("publication.json");
    let installer = GatewayPublicationInstaller::open(&lkg).unwrap();
    let product = serde_json::from_slice::<hiroute_domain::GatewayPublicationV1>(include_bytes!(
        "../../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let mut initial: GatewayPublicationSnapshotV3 =
        serde_json::from_value(serde_json::to_value(product.gateway_snapshot().unwrap()).unwrap())
            .unwrap();
    initial.validate().unwrap();
    publish(&installer, initial.clone());

    let alias = initial
        .aliases
        .iter_mut()
        .find(|alias| {
            alias.protocols == [IngressProtocol::Responses]
                && alias.candidates.iter().all(|candidate| {
                    candidate.protocol_profiles.iter().any(|profile| {
                        profile.ingress_protocol == hiroute_domain::UpstreamProtocol::Messages
                    })
                })
        })
        .expect("fixture must contain an ungranted Messages-capable Plan");
    let alias_id = alias.served_model_id.clone();
    alias.protocols.push(IngressProtocol::Messages);
    let plan_route = initial
        .grants
        .iter()
        .find_map(|grant| grant.routes.get(&alias_id))
        .unwrap()
        .clone();
    let grant = initial
        .grants
        .iter_mut()
        .find(|grant| grant.protocol == IngressProtocol::Messages)
        .expect("fixture must contain a Messages grant");
    grant.routes.insert(alias_id.clone(), plan_route);
    initial.publication_revision += 1;
    let expanded = reseal(initial);

    publish(&installer, expanded.clone());
    assert!(
        installer.active().unwrap().aliases[&*alias_id]
            .execution
            .protocols
            .contains(&IngressProtocol::Messages)
    );
    drop(installer);
    assert_eq!(
        GatewayPublicationInstaller::open(&lkg)
            .unwrap()
            .active()
            .unwrap()
            .publication_revision(),
        expanded.publication_revision
    );
}

#[test]
fn restored_plan_can_serve_codex_and_claude_without_a_new_plan_revision() {
    let directory = TestDirectory::new();
    let lkg = directory.path().join("publication.json");
    let installer = GatewayPublicationInstaller::open(&lkg).unwrap();
    let product = serde_json::from_slice::<hiroute_domain::GatewayPublicationV1>(include_bytes!(
        "../../../../../e2e/product/fixtures/routing/current-publication.v3.json"
    ))
    .unwrap();
    let initial: GatewayPublicationSnapshotV3 =
        serde_json::from_value(serde_json::to_value(product.gateway_snapshot().unwrap()).unwrap())
            .unwrap();
    let alias = initial
        .aliases
        .iter()
        .find(|alias| {
            alias.protocols == [IngressProtocol::Responses]
                && alias.candidates.iter().all(|candidate| {
                    candidate.protocol_profiles.iter().any(|profile| {
                        profile.ingress_protocol == hiroute_domain::UpstreamProtocol::Messages
                    })
                })
        })
        .expect("fixture has a Plan usable by both clients");
    let alias_id = alias.served_model_id.clone();
    let original_revision = alias.agent_plan_revision;
    let plan_route = initial
        .grants
        .iter()
        .find_map(|grant| grant.routes.get(&alias_id))
        .unwrap()
        .clone();
    publish(&installer, initial.clone());

    let mut disconnected = initial.clone();
    disconnected.publication_revision += 1;
    disconnected
        .aliases
        .retain(|alias| alias.served_model_id != alias_id);
    for grant in &mut disconnected.grants {
        grant.routes.remove(&alias_id);
    }
    publish(&installer, reseal(disconnected.clone()));
    drop(installer);

    let restarted = GatewayPublicationInstaller::open(&lkg).unwrap();
    let mut both = disconnected;
    both.publication_revision += 1;
    let mut restored = alias.clone();
    restored.protocols = vec![IngressProtocol::Responses, IngressProtocol::Messages];
    both.aliases.push(restored);
    for grant in &mut both.grants {
        if matches!(
            grant.protocol,
            IngressProtocol::Responses | IngressProtocol::Messages
        ) {
            grant.routes.insert(alias_id.clone(), plan_route.clone());
        }
    }
    publish(&restarted, reseal(both));
    let active = restarted.active().unwrap();
    assert_eq!(
        active.agent_plan_revision(&alias_id),
        Some(original_revision)
    );
    let protocols = &active.aliases[&*alias_id].execution.protocols;
    assert!(protocols.contains(&IngressProtocol::Responses));
    assert!(protocols.contains(&IngressProtocol::Messages));
}
