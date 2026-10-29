use super::*;
use hiroute_gateway::server::publication::ModelRouteV2;
use serde_json::{Value, json};

#[path = "decision_branches/chat_history.rs"]
mod chat_history;

#[path = "decision_branches/responses_chat_history.rs"]
mod responses_chat_history;

#[path = "decision_branches/extension.rs"]
mod extension;

fn reply(body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    }
}

fn hermes_context(session: &str, turn: &str) -> String {
    json!({"version":1,"agent":"hermes","session_id":session,"purpose":"main",
        "turn_id":turn,"continuation_requires_existing_turn":false})
    .to_string()
}

#[test]
fn follow_up_keeps_an_in_group_backup_and_still_rejudges_the_group() {
    use hiroute_gateway::server::publication::{AliasGroupIdV1, AliasRequestOwnedRouteV1};

    for smart in [true, false] {
        for legacy in [false, true] {
            let simple: &'static [u8] = if smart {
                br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
            } else {
                br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
            };
            let complex: &'static [u8] = if smart {
                br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.1,"1":0.9}}}}"#
            } else {
                br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.1,"1":0.9}}}}"#
            };
            let first = NativeProvider::start(vec![
                reply(br#"{"id":"empty","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1}}"#),
                reply(RESPONSES_OK),
            ]);
            let backup = NativeProvider::start(vec![reply(RESPONSES_OK); 2]);
            let decider = NativeProvider::start(vec![reply(simple), reply(simple), reply(complex)]);
            let fixture =
                RuntimeFixture::launch_builtin_branches(&[&first, &backup, &decider], smart);
            let mut publication = fixture.publication();
            let alias = &mut publication.aliases[0];
            let local_ids = alias
                .candidates
                .iter()
                .map(|c| c.local_id)
                .collect::<Vec<_>>();
            let routing = alias.routing.as_mut().unwrap();
            for group in &mut routing.groups {
                match group.group_id {
                    AliasGroupIdV1::Economy | AliasGroupIdV1::Branch(0) => {
                        group.candidate_local_ids = local_ids.clone();
                    }
                    AliasGroupIdV1::Primary | AliasGroupIdV1::BranchPrimary(0) => {
                        group.candidate_local_ids = vec![local_ids[0]];
                    }
                    _ => {}
                }
            }
            match &mut routing.request_owned {
                AliasRequestOwnedRouteV1::Classified {
                    reselect_on_user_message,
                    ..
                }
                | AliasRequestOwnedRouteV1::Branches {
                    reselect_on_user_message,
                    ..
                } => {
                    *reselect_on_user_message = legacy;
                }
                _ => unreachable!(),
            }
            publication.publication_revision += 1;
            fixture.publish(&mut publication);
            let mut history = vec![json!({"role":"user","content":"Say hello."})];
            for (message, expected) in [
                (None, (1, 1, 1)),
                (Some("Continue with one more greeting."), (1, 2, 2)),
                (
                    Some("Now design a distributed consistency protocol."),
                    (2, 2, 3),
                ),
            ] {
                if let Some(message) = message {
                    history.push(json!({"role":"user","content":message}));
                }
                let response = request(
                    fixture.address,
                    "POST",
                    "/v1/responses",
                    &[
                        ("X-HiRoute-Token", TOKEN),
                        ("session-id", "follow-up-backup"),
                    ],
                    &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":history}))
                        .unwrap(),
                );
                assert_eq!(
                    response.status,
                    200,
                    "{}",
                    String::from_utf8_lossy(&response.body)
                );
                assert_eq!(
                    (first.calls(), backup.calls(), decider.calls()),
                    expected,
                    "mode smart={smart}, legacy={legacy}: same-group continuity must not suppress a fresh decision"
                );
                let body: Value = serde_json::from_slice(&response.body).unwrap();
                history.extend(body["output"].as_array().unwrap().iter().cloned());
            }
        }
    }
}

#[test]
fn primary_relay_stays_in_this_turn_but_new_users_decide_again() {
    assert_turn_routing(false);
}
#[test]
fn hermes_native_turns_keep_tools_and_restore_observed_user_routing() {
    assert_turn_routing(true);
}
fn assert_turn_routing(hermes: bool) {
    for c in [false, true] {
        for smart in [true, false] {
            for change_instructions in [false, true] {
                let simple: &'static [u8] = if smart {
                    br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
                } else {
                    br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
                };
                // Empty native output is a request-local protocol failure. The
                // regular provider is healthy again by the next request, so cooldown
                // cannot hide a backwards attempt on the tool continuation.
                let regular = NativeProvider::start(vec![
                reply(br#"{"id":"empty","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1}}"#),
                reply(RESPONSES_OK),
                reply(RESPONSES_OK),
            ]);
                let primary = NativeProvider::start(vec![
                    reply(RESPONSES_TOOL_CALL),
                    reply(RESPONSES_OK),
                    reply(RESPONSES_OK),
                ]);
                let decider = NativeProvider::start(vec![reply(simple); 3]);
                let fixture =
                    RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], smart);
                if hermes || c {
                    let mut publication = fixture.publication();
                    if hermes {
                        publication
                            .grants
                            .iter_mut()
                            .filter(|grant| {
                                grant.protocol
                                == hiroute_gateway::server::request_plan::IngressProtocol::Responses
                            })
                            .for_each(|grant| {
                                grant.consumer = Some(hiroute_domain::AgentModelConsumerV1::Hermes)
                            });
                    }
                    if c {
                        enable_c_in_publication(&mut publication);
                    } else {
                        publication.publication_revision += 1;
                    }
                    fixture.publish(&mut publication);
                }
                let send = |input: &[Value], instructions: &str, turn: &str| {
                    let context = hermes_context("relay-turn-boundary", turn);
                    let mut headers = vec![
                        ("X-HiRoute-Token", TOKEN),
                        (
                            if hermes {
                                "x-hermes-session-id"
                            } else {
                                "session-id"
                            },
                            "relay-turn-boundary",
                        ),
                    ];
                    if hermes {
                        headers.extend([
                            ("x-hiroute-agent", "hermes"),
                            ("x-hiroute-routing-context", context.as_str()),
                        ]);
                    }
                    let response = request(fixture.address, "POST", "/v1/responses", &headers,
                        &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"instructions":instructions,"input":input})).unwrap());
                    assert_eq!(
                        response.status,
                        200,
                        "{}",
                        String::from_utf8_lossy(&response.body)
                    );
                    serde_json::from_slice::<Value>(&response.body).unwrap()
                };
                let mut history = vec![json!({"role":"user","content":"Look up the record."})];
                let first = send(&history, "Use the lookup tool.", "real-user-1");
                assert_eq!(
                    (regular.calls(), primary.calls(), decider.calls()),
                    (1, 1, 1)
                );
                history.extend(first["output"].as_array().unwrap().iter().cloned());
                history.push(json!({"type":"function_call_output","call_id":first["output"][0]["call_id"],"output":"record found"}));
                let instructions = if change_instructions {
                    "Summarize the lookup result."
                } else {
                    "Use the lookup tool."
                };
                let continued = send(&history, instructions, "real-user-1");
                assert_eq!(
                    (regular.calls(), primary.calls(), decider.calls()),
                    (1, 2, 1),
                    "the same actual tool execution inherits its primary relay even when request-local instructions change"
                );
                history.extend(continued["output"].as_array().unwrap().iter().cloned());
                history.push(json!({"role":"user","content":"A new simple task: say hello."}));
                send(&history, instructions, "real-user-2");
                assert_eq!(
                    (regular.calls(), primary.calls(), decider.calls()),
                    (2, 2, 2),
                    "{}",
                    if hermes {
                        "a real new question must restore routing without asserting human-origin proof"
                    } else {
                        "the next real user message must decide again, without a persistent primary floor"
                    }
                );
            }
        }
    }
}

#[test]
fn unrelated_publication_preserves_tool_decision_but_not_new_turn_or_changed_authority() {
    use hiroute_gateway::server::publication::{ModelRouteV2, token_sha256};

    for smart in [true, false] {
        let simple: &'static [u8] = if smart {
            br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
        } else {
            br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
        };
        let complex: &'static [u8] = if smart {
            br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.1,"1":0.9}}}}"#
        } else {
            br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.1,"1":0.9}}}}"#
        };
        let regular = NativeProvider::start(vec![reply(RESPONSES_TOOL_CALL), reply(RESPONSES_OK)]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 4]);
        let decider = NativeProvider::start(vec![
            reply(simple),
            reply(complex),
            reply(complex),
            reply(complex),
            reply(complex),
        ]);
        let fixture =
            RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], smart);
        let send = |input: &[Value]| {
            let response = request(
                fixture.address,
                "POST",
                "/v1/responses",
                &[
                    ("X-HiRoute-Token", TOKEN),
                    ("session-id", "publication-continuation"),
                ],
                &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":input})).unwrap(),
            );
            assert_eq!(
                response.status,
                200,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
            serde_json::from_slice::<Value>(&response.body).unwrap()
        };
        let mut history = vec![json!({"role":"user","content":"Look up the record."})];
        let first = send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (1, 0, 1)
        );

        let mut publication = fixture.publication();
        let current_alias = publication.aliases[0].clone();
        let current_grants = publication.grants.clone();
        let mut unrelated = current_alias.clone();
        unrelated.served_model_id = "unrelated-model".into();
        unrelated.routing.as_mut().unwrap().agent_plan_id = "unrelated-plan".into();
        for candidate in &mut unrelated.candidates {
            candidate.local_id += 100;
        }
        for group in &mut unrelated.routing.as_mut().unwrap().groups {
            for candidate in &mut group.candidate_local_ids {
                *candidate += 100;
            }
        }
        publication.aliases.push(unrelated);
        let mut grant = current_grants[0].clone();
        grant.grant_id = "unrelated-grant".into();
        grant.bearer_token_sha256 = token_sha256("unrelated-token");
        grant.routes = [(
            "unrelated-model".into(),
            ModelRouteV2::Plan {
                plan_id: "unrelated-plan".into(),
                alias: "unrelated-model".into(),
                revision: 91,
                semantic_digest: hiroute_domain::CanonicalDigest::of_bytes(b"unrelated-plan"),
            },
        )]
        .into_iter()
        .collect();
        publication.grants.push(grant);
        publication.publication_revision += 1;
        fixture.publish(&mut publication);
        assert_eq!(publication.aliases[0], current_alias);
        assert_eq!(
            &publication.grants[..current_grants.len()],
            current_grants.as_slice()
        );

        history.extend(first["output"].as_array().unwrap().iter().cloned());
        history.push(json!({"type":"function_call_output","call_id":first["output"][0]["call_id"],"output":"record found"}));
        let continuation = send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (2, 0, 1),
            "publishing an unrelated plan must preserve the frozen tool-turn decision"
        );

        history.extend(continuation["output"].as_array().unwrap().iter().cloned());
        history.push(
            json!({"role":"user","content":"Now design a distributed consistency protocol."}),
        );
        send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (2, 1, 2),
            "a new user turn must decide again and may select primary"
        );

        // Even unchanged visible history cannot inherit across a changed plan,
        // grant generation or authority epoch. Each request uses new authority.
        publication.aliases[0].agent_plan_revision += 1;
        for grant in &mut publication.grants {
            if let Some(ModelRouteV2::Plan {
                revision,
                semantic_digest,
                ..
            }) = grant.routes.get_mut(MODEL)
            {
                *revision += 1;
                *semantic_digest = hiroute_domain::CanonicalDigest::of_bytes(b"new-current-plan");
            }
        }
        publication.publication_revision += 1;
        fixture.publish(&mut publication);
        send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (2, 2, 3)
        );

        for grant in &mut publication.grants {
            if grant.routes.contains_key(MODEL) {
                grant.generation += 1;
            }
        }
        publication.publication_revision += 1;
        fixture.publish(&mut publication);
        send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (2, 3, 4)
        );

        publication.authority_epoch += 1;
        publication.publication_revision += 1;
        fixture.publish(&mut publication);
        send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (2, 4, 5)
        );
    }
}

#[test]
fn builtin_decision_reselects_groups_and_observes_actual_execution_in_both_modes() {
    for smart in [true, false] {
        let branch = if smart { "smart_saving" } else { "code" };
        let plain: &'static [u8] = if smart {
            br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
        } else {
            br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
        };
        let low: &'static [u8] = if smart {
            br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":0.2}}}"#
        } else {
            br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q2":{"type":"score","score":0.2}}}"#
        };
        let complex: &'static [u8] = if smart {
            br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.1,"1":0.9}}}}"#
        } else {
            br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.1,"1":0.9}}}}"#
        };
        let regular = NativeProvider::start(vec![reply(RESPONSES_OK), reply(RESPONSES_OK)]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK), reply(RESPONSES_OK)]);
        let decider =
            NativeProvider::start(vec![reply(plain), reply(low), reply(complex), reply(plain)]);
        let fixture =
            RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], smart);
        let headers = [
            ("X-HiRoute-Token", TOKEN),
            ("session-id", "builtin-per-turn-groups"),
        ];
        let message = |role: &str, text: &str| json!({"type":"message","role":role,"content":[{"type":if role == "user" {"input_text"} else {"output_text"},"text":text}]});
        // Native clients such as Codex prepend user-role context on their first request.
        // An unseen prefix must not invalidate a fully captured scoring stage.
        let mut input = vec![message("user", "Client-provided workspace context.")];
        for (index, (text, expected)) in [
            ("Correct a typo.", (1, 0, 1)),
            ("This was incorrect. Try again.", (1, 1, 2)),
            ("Now design a distributed consistency protocol.", (1, 2, 3)),
            ("Correct another typo.", (2, 2, 4)),
        ]
        .into_iter()
        .enumerate()
        {
            if index > 0 {
                input.push(message("assistant", "client rewritten output"));
            }
            input.push(message("user", text));
            let response = request(
                fixture.address,
                "POST",
                "/v1/responses",
                &headers,
                &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":input})).unwrap(),
            );
            assert_eq!(
                response.status,
                200,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
            assert_eq!(
                (regular.calls(), primary.calls(), decider.calls()),
                expected,
                "turn {index}: every new message decides; missing scores neither preserve an old group nor force regular"
            );
        }
        let requests = decider.requests();
        let body = |index: usize| {
            let wire = &requests[index];
            let offset = wire.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
            serde_json::from_slice::<Value>(&wire[offset..]).unwrap()
        };
        let first = body(0);
        let assessment_question = if smart { "q1" } else { "q2" };
        let degree_question = if smart { "q0" } else { "q1" };
        assert_eq!(first["model"], "fixture-jev");
        assert!(first["questions"].get(assessment_question).is_none());
        assert_eq!(first["questions"][degree_question]["type"], "score");
        assert_eq!(
            first["questions"][degree_question]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        if !smart {
            assert_eq!(
                first["questions"]["q0"]["criteria"][branch],
                format!("Condition for {branch}")
            );
            assert_eq!(
                first["questions"].as_object().unwrap().len(),
                2,
                "a single-group docs branch must not ask a degree question"
            );
        }
        let second = body(1);
        assert_eq!(second["state"]["history_partial"], true);
        assert_eq!(second["state"]["assessment_from"], 0);
        assert_eq!(
            second["state"]["visible_conversation"][0]["steps"][0][0]["text"],
            "ok"
        );
        for index in [1, 2, 3] {
            assert_eq!(
                body(index)["questions"][assessment_question]["criteria"][0],
                format!("{branch} failed"),
                "both groups retain the selected category's frozen rubric"
            );
        }
        let facts = wait_execution_facts(&fixture, 4);
        let scored = facts
            .iter()
            .find(|f| f.pointer("/fact/kind") == Some(&json!("branch_assessment_recorded")))
            .unwrap();
        assert_eq!(
            scored.pointer("/fact/score").and_then(Value::as_f64),
            Some(0.1)
        );
        assert_eq!(scored.pointer("/fact/partial"), Some(&json!(false)));
        let protected = facts
            .iter()
            .find(|f| f.pointer("/fact/complexity/competence_trigger").is_some())
            .expect("fresh low competence retains its trigger");
        assert_eq!(
            protected.pointer("/fact/complexity/branch_id"),
            Some(&json!(branch))
        );
        assert_eq!(
            protected.pointer("/fact/complexity/execution_group"),
            Some(&json!("primary"))
        );
        assert_eq!(
            protected.pointer("/fact/complexity/competence_trigger/from_group"),
            Some(&json!("regular"))
        );
        assert_eq!(
            protected.pointer("/fact/complexity/competence_trigger/floor_millis"),
            Some(&json!(500))
        );
        let groups: Vec<_> = facts
            .iter()
            .filter(|f| f.pointer("/fact/kind") == Some(&json!("agent_turn_finished")))
            .map(|f| {
                assert_eq!(
                    f.pointer("/fact/branch_execution/candidate_index"),
                    Some(&json!(0))
                );
                f.pointer("/fact/branch_execution/group")
                    .and_then(Value::as_str)
                    .unwrap()
            })
            .collect();
        assert_eq!(groups, ["regular", "primary", "primary", "regular"]);
        let selections: Vec<_> = facts
            .iter()
            .filter_map(|f| {
                f.pointer("/fact/complexity/selection_reason")
                    .and_then(Value::as_str)
            })
            .collect();
        assert_eq!(
            selections,
            [
                "simple_task",
                "low_competence",
                "complex_task",
                "simple_task"
            ]
        );
    }
}

fn enable_c(
    fixture: &RuntimeFixture,
    protocol: &hiroute_domain::NativeProtocolV1,
) -> hiroute_domain::FrozenNativeProtocolV1 {
    let mut publication = fixture.publication();
    let protocol = enable_c_in_publication_with(&mut publication, protocol);
    fixture.publish(&mut publication);
    protocol
}

fn enable_c_in_publication(
    publication: &mut hiroute_gateway::server::publication::GatewayPublicationSnapshotV3,
) -> hiroute_domain::FrozenNativeProtocolV1 {
    enable_c_in_publication_with(publication, &hiroute_domain::NativeProtocolV1::default())
}

pub(super) fn enable_c_in_publication_with(
    publication: &mut hiroute_gateway::server::publication::GatewayPublicationSnapshotV3,
    protocol: &hiroute_domain::NativeProtocolV1,
) -> hiroute_domain::FrozenNativeProtocolV1 {
    use hiroute_domain::*;
    use hiroute_gateway::server::publication::AliasRequestOwnedRouteV1;
    match &mut publication.aliases[0]
        .routing
        .as_mut()
        .unwrap()
        .request_owned
    {
        AliasRequestOwnedRouteV1::Classified {
            judgment,
            native_protocol,
            ..
        } => {
            let p = protocol;
            let judgment = judgment.as_mut().unwrap();
            p.stock_assessment(judgment);
            *native_protocol = Some(
                FrozenNativeProtocolV1::freeze(
                    p,
                    judgment.degree.definition().into(),
                    [(SMART_SAVING_SCOPE_ID.into(), judgment.clone())],
                )
                .unwrap(),
            );
        }
        AliasRequestOwnedRouteV1::Branches {
            branches,
            default_branch_id,
            native_protocol,
            ..
        } => {
            let p = protocol;
            for b in branches.iter_mut() {
                p.stock_assessment(b.judgment.as_mut().unwrap());
            }
            *native_protocol = Some(
                FrozenNativeProtocolV1::freeze(
                    p,
                    branch_decision_definition(branches, default_branch_id),
                    branches
                        .iter()
                        .map(|b| (b.id.clone(), b.judgment.clone().unwrap())),
                )
                .unwrap(),
            );
        }
        _ => unreachable!(),
    }
    publication.aliases[0].agent_plan_revision += 1;
    let revision = publication.aliases[0].agent_plan_revision;
    let digest = CanonicalDigest::of(&publication.aliases[0].routing).unwrap();
    for grant in &mut publication.grants {
        if let Some(ModelRouteV2::Plan {
            revision: bound,
            semantic_digest,
            ..
        }) = grant.routes.get_mut(MODEL)
        {
            *bound = revision;
            *semantic_digest = digest.clone();
        }
    }
    publication.publication_revision += 1;
    match &publication.aliases[0]
        .routing
        .as_ref()
        .unwrap()
        .request_owned
    {
        AliasRequestOwnedRouteV1::Classified {
            native_protocol, ..
        }
        | AliasRequestOwnedRouteV1::Branches {
            native_protocol, ..
        } => native_protocol.as_ref().unwrap().clone(),
        _ => unreachable!(),
    }
}
pub(super) fn c_answer(smart: bool, applicable: bool) -> &'static [u8] {
    match (smart,applicable) {
        (true,true) => br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":0.2},"q2":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#,
        (true,false) => br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q1":{"type":"score","score":0.2},"q2":{"type":"choice","probabilities":{"applicable":0.1,"not_established":0.9}}}}"#,
        (false,true) => br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q2":{"type":"score","score":0.2},"q3":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#,
        (false,false) => br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}},"q2":{"type":"score","score":0.2},"q3":{"type":"choice","probabilities":{"applicable":0.1,"not_established":0.9}}}}"#,
    }
}
fn c_complex_answer(smart: bool, applicable: bool) -> &'static [u8] {
    match (smart, applicable) {
        (true, true) => br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.7,"1":0.3}},"q1":{"type":"score","score":0.2},"q2":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#,
        (true, false) => br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.7,"1":0.3}},"q1":{"type":"score","score":0.2},"q2":{"type":"choice","probabilities":{"applicable":0.1,"not_established":0.9}}}}"#,
        (false, true) => br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.7,"1":0.3}},"q2":{"type":"score","score":0.2},"q3":{"type":"choice","probabilities":{"applicable":0.9,"not_established":0.1}}}}"#,
        (false, false) => br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.7,"1":0.3}},"q2":{"type":"score","score":0.2},"q3":{"type":"choice","probabilities":{"applicable":0.1,"not_established":0.9}}}}"#,
    }
}
#[test]
fn c_protocol_rejudges_current_work_and_observes_protection_and_release_on_real_listeners() {
    for (protocol, smart) in [
        hiroute_domain::NativeProtocolV1::c_v1(),
        hiroute_domain::NativeProtocolV1::c_v2(),
    ]
    .into_iter()
    .flat_map(|protocol| [true, false].map(move |smart| (protocol.clone(), smart)))
    {
        let plain: &'static [u8] = if smart {
            br#"{"answers":{"q0":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
        } else {
            br#"{"answers":{"q0":{"type":"choice","choice":"code"},"q1":{"type":"score","probabilities":{"0":0.9,"1":0.1}}}}"#
        };
        let regular = NativeProvider::start(vec![reply(RESPONSES_OK); 3]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_OK); 4]);
        let decider = NativeProvider::start(vec![
            reply(plain),
            reply(c_answer(smart, true)),
            reply(c_answer(smart, false)),
            reply(c_answer(smart, true)),
            reply(c_complex_answer(smart, false)),
            reply(c_complex_answer(smart, true)),
        ]);
        let fixture =
            RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], smart);
        let frozen = enable_c(&fixture, &protocol);
        let mut input = Vec::new();
        for (index, text) in [
            "Provide the requested title.",
            "The title is still missing. Deliver it now.",
            "I saved the corrected title. Now correct an unrelated typo.",
            "The same required constraint is still not satisfied. Retry the required work.",
            "I fixed the issue. Now carry out the complex follow-up.",
            "That result is still unsatisfactory. Retry the complex follow-up.",
        ]
        .iter()
        .enumerate()
        {
            if index > 0 {
                input.push(json!({"role":"assistant","content":"client representation"}));
            }
            input.push(json!({"role":"user","content":text}));
            let response = request(
                fixture.address,
                "POST",
                "/v1/responses",
                &[
                    ("X-HiRoute-Token", TOKEN),
                    ("session-id", "c-protocol-rounds"),
                ],
                &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"input":input})).unwrap(),
            );
            assert_eq!(
                response.status,
                200,
                "{}",
                String::from_utf8_lossy(&response.body)
            );
            let expected = [
                (1, 0, 1),
                (1, 1, 2),
                (2, 1, 3),
                (2, 2, 4),
                (2, 3, 5),
                (2, 4, 6),
            ][index];
            assert_eq!(
                (regular.calls(), primary.calls(), decider.calls()),
                expected
            );
        }
        let wire = decider.requests();
        for (i, request) in wire.iter().enumerate() {
            let offset = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let body: Value = serde_json::from_slice(&request[offset..]).unwrap();
            match &frozen.decision {
                hiroute_domain::DecisionDefinitionV1::Ordinal { instructions, .. } => {
                    assert_eq!(
                        body["questions"]["q0"]["instructions"],
                        format!("{} {instructions}", frozen.protocol.fixed.degree_prefix)
                    );
                }
                hiroute_domain::DecisionDefinitionV1::Categorical {
                    instructions,
                    options,
                } => {
                    assert_eq!(body["questions"]["q0"]["instructions"], *instructions);
                    let degree = options[0].refinement.as_ref().unwrap();
                    assert_eq!(
                        body["questions"]["q1"]["instructions"],
                        format!(
                            "{} {}",
                            frozen.protocol.fixed.degree_prefix, degree.instructions
                        )
                    );
                }
            }
            assert!(body["state"].get("visible_conversation").is_none());
            assert!(body["state"].get("assessment_stage").is_some());
            let applicability = if smart { "q2" } else { "q3" };
            assert_eq!(body["questions"].get(applicability).is_some(), i > 0);
            if i > 0 {
                assert_eq!(
                    body["questions"][applicability],
                    protocol.fixed.applicability
                );
            }
        }
        let facts = wait_execution_facts(&fixture, 6);
        let actions: Vec<_> = facts
            .iter()
            .filter_map(|f| {
                f.pointer("/fact/complexity/routing_observation/protection_action")
                    .and_then(Value::as_str)
            })
            .collect();
        assert_eq!(
            actions,
            [
                "not_considered",
                "applied",
                "suppressed",
                "applied",
                "suppressed",
                "applied"
            ]
        );
        let decisions: Vec<_> = facts
            .iter()
            .filter_map(|f| {
                f.pointer("/fact/complexity/routing_observation/protection_action")
                    .map(|_| &f["fact"]["complexity"])
            })
            .collect();
        assert_eq!(decisions[4]["execution_group"], "primary");
        assert_eq!(decisions[4]["selection_reason"], "complex_task");
        assert!(
            decisions[4]
                .get("competence_trigger")
                .is_none_or(Value::is_null)
        );
        assert_eq!(decisions[5]["execution_group"], "primary");
        assert_eq!(decisions[5]["selection_reason"], "low_competence");
        assert_eq!(decisions[5]["competence_trigger"]["from_group"], "primary");
        assert!(
            decisions
                .iter()
                .all(|decision| decision["routing_observation"]["protocol"] == protocol.schema)
        );
        assert!(
            facts
                .iter()
                .filter(|f| f.pointer("/fact/kind") == Some(&json!("branch_assessment_recorded")))
                .all(|f| f.pointer("/fact/score") == Some(&json!(0.1)))
        );
    }
}
