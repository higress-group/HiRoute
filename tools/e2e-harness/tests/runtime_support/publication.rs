use hiroute_e2e::gateway_fixture::sealed_native_candidate;
use hiroute_gateway::server::publication::{
    AliasComplexityClassifierV1, AliasCostPolicyV1, AliasGroupIdV1, AliasModelGroupV1, AliasPlanV1,
    AliasRequestOwnedRouteV1, AliasRoutingV1, GatewayPublicationSnapshotV3, GrantV1, token_sha256,
};
use hiroute_gateway::server::request_plan::IngressProtocol;

use super::{
    CHAT_TOKEN, MESSAGES_TOKEN, MODEL, NativeProvider, PublicationCandidate, TOKEN, credential_refs,
};

#[derive(Clone, Copy)]
pub(super) enum PublicationRouteMode<'a> {
    Ordered,
    ClassifiedLocal,
    ClassifiedRest {
        endpoint_override: Option<&'a str>,
        timeout_ms: u64,
    },
}

pub(super) fn snapshot(
    providers: &[&NativeProvider],
    max_attempts: u32,
    publication_candidates: Option<&[PublicationCandidate]>,
    endpoints: Option<&[String]>,
    key_counts: Option<&[usize]>,
    overall_timeout_ms: u64,
    route_mode: PublicationRouteMode<'_>,
) -> GatewayPublicationSnapshotV3 {
    let classified_route = !matches!(route_mode, PublicationRouteMode::Ordered);
    let (classifier_endpoint_override, classifier_timeout_ms) = match route_mode {
        PublicationRouteMode::ClassifiedRest {
            endpoint_override,
            timeout_ms,
        } => (endpoint_override, timeout_ms),
        PublicationRouteMode::Ordered | PublicationRouteMode::ClassifiedLocal => {
            (None, hiroute_domain::DEFAULT_REST_CLASSIFIER_TIMEOUT_MS)
        }
    };
    let rest_classifier = matches!(route_mode, PublicationRouteMode::ClassifiedRest { .. });
    if let Some(endpoints) = endpoints {
        assert_eq!(endpoints.len(), providers.len());
    }
    let default_candidates = (0..providers.len())
        .map(|provider_index| PublicationCandidate {
            provider_index,
            upstream_protocol: "responses",
            statically_enabled: true,
        })
        .collect::<Vec<_>>();
    let publication_candidates = publication_candidates.unwrap_or(&default_candidates);
    let mut seen = vec![false; providers.len()];
    let mut candidates = publication_candidates
        .iter()
        .filter(|candidate| candidate.statically_enabled)
        .map(|candidate| {
            let index = candidate.provider_index;
            assert!(
                index < providers.len(),
                "publication candidate index is in range"
            );
            assert!(!seen[index], "publication candidate identities are unique");
            seen[index] = true;
            let provider = providers[index];
            let upstream = parse_protocol(candidate.upstream_protocol);
            let authority = endpoints
                .and_then(|values| values.get(index))
                .and_then(|endpoint| {
                    endpoint
                        .contains("does-not-resolve.invalid")
                        .then_some("does-not-resolve.invalid")
                })
                .unwrap_or_else(|| provider.authority());
            let key_count = key_counts.map_or(1, |counts| counts[index]);
            let profiles = if rest_classifier {
                vec![
                    (IngressProtocol::Responses, upstream),
                    (IngressProtocol::ChatCompletions, upstream),
                    (IngressProtocol::Messages, upstream),
                ]
            } else {
                vec![(IngressProtocol::Responses, upstream)]
            };
            sealed_native_candidate(
                u32::try_from(index + 1).unwrap(),
                &format!("runtime-target-{}", index + 1),
                &credential_refs(index, key_count),
                authority,
                &format!("runtime-native-model-{}", index + 1),
                &profiles,
            )
        })
        .collect::<Vec<_>>();
    let classifier_endpoint = if rest_classifier {
        assert!(
            classified_route,
            "REST classification requires a classified route"
        );
        assert_eq!(
            candidates.len(),
            3,
            "REST-classified fixture has two branch candidates and one classifier service"
        );
        candidates.pop();
        Some(classifier_endpoint_override.map_or_else(
            || {
                format!(
                    "https://{}/v1/decisions",
                    providers.last().expect("classifier service").authority()
                )
            },
            str::to_owned,
        ))
    } else {
        None
    };
    assert!(
        !candidates.is_empty(),
        "a sealed publication alias must contain an executable candidate"
    );
    let (request_owned, groups) = if classified_route {
        assert_eq!(
            candidates.len(),
            2,
            "classified fixture has two enabled candidates"
        );
        (
            AliasRequestOwnedRouteV1::Classified {
                // These classified-route fixtures explicitly exercise branch
                // changes on a new user turn; the product authoring default
                // remains false.
                reselect_on_user_message: true,
                classifier: AliasComplexityClassifierV1 {
                    revision: "runtime-complexity/v1".into(),
                    mode: if let Some(endpoint) = classifier_endpoint {
                        hiroute_domain::ComplexityClassifierModeV1::Rest {
                            endpoint,
                            timeout_ms: classifier_timeout_ms,
                            auth_header: Some(hiroute_domain::ClassifierAuthHeaderV1 {
                                name: "Authorization".into(),
                                value_secret_ref: credential_refs(2, 1)
                                    .pop()
                                    .expect("classifier fixture credential"),
                            }),
                        }
                    } else {
                        hiroute_domain::ComplexityClassifierModeV1::LocalRules
                    },
                    user_keywords: vec!["complex-route".into()],
                },
                simple_groups: vec![AliasGroupIdV1::Economy],
                complex_groups: vec![AliasGroupIdV1::Primary],
            },
            vec![
                AliasModelGroupV1 {
                    group_id: AliasGroupIdV1::Economy,
                    candidate_local_ids: vec![candidates[0].local_id],
                },
                AliasModelGroupV1 {
                    group_id: AliasGroupIdV1::Primary,
                    candidate_local_ids: vec![candidates[1].local_id],
                },
            ],
        )
    } else {
        (
            AliasRequestOwnedRouteV1::Ordered {
                cost_policy: AliasCostPolicyV1::ApiEquivalent,
                ordered_groups: vec![AliasGroupIdV1::Custom],
            },
            vec![AliasModelGroupV1 {
                group_id: AliasGroupIdV1::Custom,
                candidate_local_ids: candidates
                    .iter()
                    .map(|candidate| candidate.local_id)
                    .collect(),
            }],
        )
    };
    let aliases = vec![AliasPlanV1 {
        served_model_id: MODEL.into(),
        purpose: "runtime listener verification".into(),
        agent_plan_revision: 91,
        protocols: if rest_classifier {
            vec![
                IngressProtocol::Responses,
                IngressProtocol::ChatCompletions,
                IngressProtocol::Messages,
            ]
        } else {
            vec![IngressProtocol::Responses]
        },
        overall_timeout_ms,
        max_attempts,
        routing: Some(AliasRoutingV1 {
            agent_plan_id: "agent-plan-91".into(),
            plan_display_name: Some("Runtime verification".into()),
            request_owned,
            groups,
        }),
        candidates,
    }];
    let routes = [(
        MODEL.into(),
        hiroute_gateway::server::publication::ModelRouteV2::Plan {
            plan_id: "agent-plan-91".into(),
            alias: MODEL.into(),
            revision: 91,
            semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(MODEL.as_bytes()),
        },
    )]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    GatewayPublicationSnapshotV3::seal(
        "personal/default",
        "runtime-authority",
        17,
        71,
        "runtime-renderer/v1",
        aliases,
        if rest_classifier {
            [
                (IngressProtocol::Responses, TOKEN),
                (IngressProtocol::ChatCompletions, CHAT_TOKEN),
                (IngressProtocol::Messages, MESSAGES_TOKEN),
            ]
            .into_iter()
            .map(|(protocol, token)| GrantV1 {
                grant_id: format!("runtime-grant-{}", protocol.path()),
                generation: 1,
                bearer_token_sha256: token_sha256(token),
                protocol,
                routes: routes.clone(),
            })
            .collect()
        } else {
            vec![GrantV1 {
                grant_id: "runtime-grant".into(),
                generation: 1,
                bearer_token_sha256: token_sha256(TOKEN),
                protocol: IngressProtocol::Responses,
                routes,
            }]
        },
    )
    .unwrap()
}

pub(super) fn add_reasoning_choices(snapshot: &mut GatewayPublicationSnapshotV3) {
    for (index, candidate) in snapshot.aliases[0].candidates.iter_mut().enumerate() {
        for profile in &mut candidate.protocol_profiles {
            profile.capability.reasoning_profiles = ["low", "high"]
                .into_iter()
                .map(|effort| {
                    serde_json::from_value(serde_json::json!({
                        "profile_id":effort,"control_kind":"discrete",
                        "render":{"kind":"exact_fields","protocol":"responses","fields":[
                            {"path":["reasoning","effort"],"value":{"kind":"string","value":effort}}
                        ]},"accounting":"within_output_cap","additional_reservation_tokens":0
                    }))
                    .unwrap()
                })
                .collect();
            profile.capability.selected_reasoning_profile_id =
                if index == 0 { "low" } else { "high" }.into();
        }
        candidate.protocol_profile_digest =
            hiroute_domain::CanonicalDigest::of(&candidate.protocol_profiles).unwrap();
    }
    snapshot.payload_digest = snapshot.canonical_digest().unwrap();
    snapshot.validate().unwrap();
}

pub(super) fn make_first_route_fixed(snapshot: &mut GatewayPublicationSnapshotV3) {
    use hiroute_gateway::server::publication::ModelRouteV2;
    let plan = &mut snapshot.aliases[0];
    let binding = plan.candidates.remove(0);
    assert!(!plan.candidates.is_empty());
    for group in &mut plan.routing.as_mut().unwrap().groups {
        group
            .candidate_local_ids
            .retain(|id| *id != binding.local_id);
        assert!(!group.candidate_local_ids.is_empty());
    }
    let plan_name = format!("{MODEL}-alternate-plan");
    plan.served_model_id = plan_name.clone();
    let routes = &mut snapshot.grants[0].routes;
    let mut plan_route = routes.remove(MODEL).unwrap();
    let ModelRouteV2::Plan { alias, .. } = &mut plan_route else {
        panic!("expected plan fixture");
    };
    *alias = plan_name.clone();
    routes.insert(plan_name, plan_route);
    routes.insert(
        MODEL.into(),
        ModelRouteV2::Fixed {
            binding_digest: hiroute_domain::CanonicalDigest::of(&binding).unwrap(),
            binding: Box::new(binding),
            overall_timeout_ms: plan.overall_timeout_ms,
            max_attempts: plan.max_attempts,
        },
    );
    snapshot.payload_digest = snapshot.canonical_digest().unwrap();
    snapshot.validate().unwrap();
}

pub(super) fn make_isolated_fixed_grants(snapshot: &mut GatewayPublicationSnapshotV3) {
    let plan = snapshot.aliases.remove(0);
    assert!(snapshot.aliases.is_empty());
    assert_eq!(plan.candidates.len(), 2);
    snapshot.grants = plan
        .candidates
        .into_iter()
        .enumerate()
        .map(|(index, binding)| {
            let routes = [MODEL.into(), format!("private-model-{index}")]
                .into_iter()
                .enumerate()
                .map(|(route_index, name)| {
                    let mut binding = binding.clone();
                    binding.local_id = u32::try_from(index * 2 + route_index + 1).unwrap();
                    let route = hiroute_gateway::server::publication::ModelRouteV2::Fixed {
                        binding_digest: hiroute_domain::CanonicalDigest::of(&binding).unwrap(),
                        binding: Box::new(binding),
                        overall_timeout_ms: plan.overall_timeout_ms,
                        max_attempts: plan.max_attempts,
                    };
                    (name, route)
                })
                .collect();
            GrantV1 {
                grant_id: format!("isolated-grant-{index}"),
                generation: 1,
                bearer_token_sha256: token_sha256(&format!("isolated-token-{index}")),
                protocol: IngressProtocol::Responses,
                routes,
            }
        })
        .collect();
    snapshot.payload_digest = snapshot.canonical_digest().unwrap();
    snapshot.validate().unwrap();
}

fn parse_protocol(value: &str) -> IngressProtocol {
    match value {
        "responses" => IngressProtocol::Responses,
        "chat_completions" => IngressProtocol::ChatCompletions,
        "messages" => IngressProtocol::Messages,
        other => panic!("unknown fixture protocol {other}"),
    }
}
