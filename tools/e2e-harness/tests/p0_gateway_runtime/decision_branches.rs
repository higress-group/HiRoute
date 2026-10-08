use super::*;
use serde_json::{Value, json};

fn reply(body: &'static [u8]) -> ProviderReply {
    ProviderReply::Complete {
        status: 200,
        error_kind: None,
        body,
    }
}

#[test]
fn primary_relay_stays_in_this_turn_but_new_users_decide_again() {
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
            let primary =
                NativeProvider::start(vec![reply(RESPONSES_TOOL_CALL), reply(RESPONSES_OK)]);
            let decider = NativeProvider::start(vec![reply(simple), reply(simple)]);
            let fixture =
                RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], smart);
            let send = |input: &[Value], instructions: &str| {
                let response = request(fixture.address, "POST", "/v1/responses", &[
                    ("X-HiRoute-Token", TOKEN), ("session-id", "relay-turn-boundary"),
                ], &serde_json::to_vec(&json!({"model":MODEL,"stream":false,"instructions":instructions,"input":input})).unwrap());
                assert_eq!(
                    response.status,
                    200,
                    "{}",
                    String::from_utf8_lossy(&response.body)
                );
                serde_json::from_slice::<Value>(&response.body).unwrap()
            };
            let mut history = vec![json!({"role":"user","content":"Look up the record."})];
            let first = send(&history, "Use the lookup tool.");
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
            let continued = send(&history, instructions);
            assert_eq!(
                (regular.calls(), primary.calls(), decider.calls()),
                (1, 2, 1),
                "same-turn relay must not retry regular, even when instructions invalidate the candidate hold"
            );
            history.extend(continued["output"].as_array().unwrap().iter().cloned());
            history.push(json!({"role":"user","content":"A new simple task: say hello."}));
            send(&history, instructions);
            assert_eq!(
                (regular.calls(), primary.calls(), decider.calls()),
                (2, 2, 2),
                "the next real user message must decide again, without a persistent primary floor"
            );
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
