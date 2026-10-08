use super::*;

fn branch_snapshot() -> GatewayPublicationSnapshotV3 {
    let mut snapshot = custom_extension_snapshot();
    let routing = snapshot.aliases[0].routing.as_mut().unwrap();
    let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &routing.request_owned else {
        unreachable!()
    };
    let mut judgment = hiroute_domain::JudgmentSettingsV1::default();
    judgment.degree.instructions = "Degree standard\n".repeat(1000);
    let branches = ["writing", "review"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| hiroute_domain::MaterializedBranchV1 {
            id: id.into(),
            name: id.into(),
            condition: "Task condition\n".repeat(1500),
            group: AliasGroupIdV1::Branch(index as u16),
            primary_group: (index == 0).then_some(AliasGroupIdV1::BranchPrimary(0)),
            judgment: judgment.clone(),
        })
        .collect();
    routing.request_owned = AliasRequestOwnedRouteV1::Branches {
        classifier: classifier.clone(),
        branches,
        default_branch_id: "review".into(),
        reselect_on_user_message: false,
    };
    routing.groups = [
        AliasGroupIdV1::Branch(0),
        AliasGroupIdV1::BranchPrimary(0),
        AliasGroupIdV1::Branch(1),
    ]
    .into_iter()
    .map(|group_id| AliasModelGroupV1 {
        group_id,
        candidate_local_ids: vec![1, 2],
    })
    .collect();
    reseal(snapshot)
}

#[test]
fn publication_preserves_long_category_and_degree_prompts() {
    let snapshot = branch_snapshot();
    let compiled = super::super::compiler::compile(&snapshot).unwrap();
    let authority = compiled.aliases["smart"]
        .execution
        .classifier
        .as_ref()
        .unwrap();
    let hiroute_domain::DecisionDefinitionV1::Categorical { options, .. } = &authority.decision
    else {
        panic!("categorical definition")
    };
    assert_eq!(options[0].criterion, "Task condition\n".repeat(1500));
    let instructions = &options[0].refinement.as_ref().unwrap().instructions;
    assert!(instructions.contains(&options[0].criterion));
    assert!(instructions.ends_with(&"Degree standard\n".repeat(1000)));
    assert!(instructions.len() > 4096);
    assert!(
        options[1].refinement.is_none(),
        "single group has no degree question"
    );
}

#[test]
fn publication_rejects_custom_category_with_builtin_scope_id() {
    let mut snapshot = branch_snapshot();
    let AliasRequestOwnedRouteV1::Branches { branches, .. } =
        &mut snapshot.aliases[0].routing.as_mut().unwrap().request_owned
    else {
        unreachable!()
    };
    branches[0].id = hiroute_domain::SMART_SAVING_SCOPE_ID.into();
    assert!(super::super::compiler::compile(&reseal(snapshot)).is_err());
}

fn custom_connection(
    mode: &mut hiroute_domain::ComplexityClassifierModeV1,
) -> &mut hiroute_domain::DecisionConnectionV1 {
    let hiroute_domain::ComplexityClassifierModeV1::DecisionService { service } = mode else {
        panic!("expected saved extension");
    };
    &mut service.connection
}

fn custom_extension_snapshot() -> GatewayPublicationSnapshotV3 {
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "authority",
        1,
        1,
        "renderer",
        vec![AliasPlanV1 {
            served_model_id: "smart".into(),
            purpose: "smart".into(),
            agent_plan_revision: 10,
            protocols: vec![IngressProtocol::Responses],
            overall_timeout_ms: 5_000,
            max_attempts: 2,
            routing: Some(AliasRoutingV1 {
                agent_plan_id: "legacy/smart".into(),
                plan_display_name: Some("Smart".into()),
                request_owned: AliasRequestOwnedRouteV1::Classified {
                    judgment: Default::default(),
                    reselect_on_user_message: false,
                    classifier: AliasComplexityClassifierV1 {
                        revision: "classifier/v1".into(),
                        mode: hiroute_domain::ComplexityClassifierModeV1::DecisionService {
                            service: Box::new(hiroute_domain::DecisionServiceV1 {
                                id: "decision-fixture".into(),
                                revision: 1,
                                name: "Fixture extension".into(),
                                connection: hiroute_domain::DecisionConnectionV1::Custom {
                                    endpoint: "http://127.0.0.1:4317/v1/decisions".into(),
                                    timeout_ms: hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS,
                                    auth_header: None,
                                },
                            }),
                        },
                        user_keywords: vec!["complex".into()],
                    },
                    simple_groups: vec![AliasGroupIdV1::Economy],
                    complex_groups: vec![AliasGroupIdV1::Primary],
                },
                groups: vec![
                    AliasModelGroupV1 {
                        group_id: AliasGroupIdV1::Economy,
                        candidate_local_ids: vec![1],
                    },
                    AliasModelGroupV1 {
                        group_id: AliasGroupIdV1::Primary,
                        candidate_local_ids: vec![2],
                    },
                ],
            }),
            candidates: vec![candidate(1), candidate(2)],
        }],
        vec![GrantV1 {
            route_protocols: Default::default(),
            grant_id: "grant".into(),
            generation: 1,
            bearer_token_sha256: token_sha256("token"),
            protocol: IngressProtocol::Responses,
            routes: [test_plan_route("smart", 10)].into(),
        }],
    )
    .unwrap()
}

#[test]
fn custom_extension_accepts_trusted_http_https_and_optional_custom_header() {
    let directory = TestDirectory::new();
    let installer =
        GatewayPublicationInstaller::open(directory.path().join("publication.json")).unwrap();
    assert!(installer.prepare(custom_extension_snapshot()).is_ok());

    let mut remote_http = custom_extension_snapshot();
    let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut remote_http.aliases[0]
        .routing
        .as_mut()
        .unwrap()
        .request_owned
    else {
        panic!("expected classified source")
    };
    let hiroute_domain::DecisionConnectionV1::Custom { endpoint, .. } =
        custom_connection(&mut classifier.mode)
    else {
        panic!("expected REST classifier")
    };
    *endpoint = "http://classifier.example/v1/decisions".into();
    assert!(installer.prepare(reseal(remote_http)).is_ok());

    let mut remote_without_auth = custom_extension_snapshot();
    let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut remote_without_auth.aliases
        [0]
    .routing
    .as_mut()
    .unwrap()
    .request_owned
    else {
        panic!("expected classified source")
    };
    let hiroute_domain::DecisionConnectionV1::Custom { endpoint, .. } =
        custom_connection(&mut classifier.mode)
    else {
        panic!("expected REST classifier")
    };
    *endpoint = "https://classifier.example/v1/decisions".into();
    assert!(installer.prepare(reseal(remote_without_auth)).is_ok());

    let mut remote_bearer = custom_extension_snapshot();
    let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut remote_bearer.aliases[0]
        .routing
        .as_mut()
        .unwrap()
        .request_owned
    else {
        panic!("expected classified source")
    };
    let hiroute_domain::DecisionConnectionV1::Custom {
        endpoint,
        auth_header,
        ..
    } = custom_connection(&mut classifier.mode)
    else {
        panic!("expected REST classifier")
    };
    *endpoint = "https://classifier.example/v1/decisions".into();
    *auth_header = Some(hiroute_domain::ClassifierAuthHeaderV1 {
        name: "X-API-Key".into(),
        value_secret_ref: "classifier/main".into(),
    });
    assert!(installer.prepare(reseal(remote_bearer)).is_ok());

    for timeout_ms in [0, hiroute_domain::MAX_REST_CLASSIFIER_TIMEOUT_MS + 1] {
        let mut invalid_timeout = custom_extension_snapshot();
        let AliasRequestOwnedRouteV1::Classified { classifier, .. } = &mut invalid_timeout.aliases
            [0]
        .routing
        .as_mut()
        .unwrap()
        .request_owned
        else {
            panic!("expected classified source")
        };
        let hiroute_domain::DecisionConnectionV1::Custom {
            timeout_ms: value, ..
        } = custom_connection(&mut classifier.mode)
        else {
            panic!("expected REST classifier")
        };
        *value = timeout_ms;
        assert!(matches!(
            compile_classifier_mode_authority(&classifier.mode, 1),
            Err(PublicationInstallError::InvalidPlannerPolicy)
        ));
        assert!(installer.prepare(reseal(invalid_timeout)).is_err());
    }
}
