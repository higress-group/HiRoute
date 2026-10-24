use super::*;

#[test]
fn extension_group_selection_preserves_turn_relay_and_new_user_boundaries() {
    use hiroute_domain::{ComplexityClassifierModeV1, DecisionConnectionV1};
    use hiroute_gateway::server::publication::AliasRequestOwnedRouteV1;

    for smart in [true, false] {
        let selected: &'static [u8] = if smart {
            br#"{"group":"regular","assessment":{"score":0,"partial":false}}"#
        } else {
            br#"{"category":"code","group":"regular","assessment":{"score":0,"partial":false}}"#
        };
        let regular = NativeProvider::start(vec![
            reply(br#"{"id":"empty","status":"completed","output":[],"usage":{"input_tokens":1,"output_tokens":0,"total_tokens":1}}"#),
            reply(RESPONSES_OK),
        ]);
        let primary = NativeProvider::start(vec![reply(RESPONSES_TOOL_CALL), reply(RESPONSES_OK)]);
        let decider = NativeProvider::start(vec![reply(selected); 2]);
        let fixture =
            RuntimeFixture::launch_builtin_branches(&[&regular, &primary, &decider], smart);
        let mut publication = fixture.publication();
        let route = &mut publication.aliases[0]
            .routing
            .as_mut()
            .unwrap()
            .request_owned;
        let classifier = match route {
            AliasRequestOwnedRouteV1::Classified {
                classifier,
                judgment,
                ..
            } => {
                *judgment = None;
                classifier
            }
            AliasRequestOwnedRouteV1::Branches {
                classifier,
                branches,
                ..
            } => {
                for branch in branches {
                    branch.judgment = None;
                }
                classifier
            }
            _ => unreachable!(),
        };
        let ComplexityClassifierModeV1::DecisionService { service } = &mut classifier.mode else {
            unreachable!()
        };
        let DecisionConnectionV1::SystemOne {
            endpoint,
            timeout_ms,
            auth_header,
            ..
        } = &service.connection
        else {
            unreachable!()
        };
        service.connection = DecisionConnectionV1::Custom {
            endpoint: endpoint.clone(),
            timeout_ms: *timeout_ms,
            auth_header: Some(auth_header.clone()),
        };
        publication.publication_revision += 1;
        fixture.publish(&mut publication);
        let send = |history: &[Value]| {
            let response = request(
                fixture.address,
                "POST",
                "/v1/responses",
                &[
                    ("X-HiRoute-Token", TOKEN),
                    ("session-id", "extension-direct-turns"),
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
            serde_json::from_slice::<Value>(&response.body).unwrap()
        };
        let mut history = vec![json!({"role":"user","content":"Look up the record."})];
        let first = send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (1, 1, 1)
        );
        history.extend(first["output"].as_array().unwrap().iter().cloned());
        history.push(json!({"type":"function_call_output","call_id":first["output"][0]["call_id"],"output":"record found"}));
        let continued = send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (1, 2, 1),
            "tool continuation keeps the relayed primary group"
        );
        history.extend(continued["output"].as_array().unwrap().iter().cloned());
        history.push(json!({"role":"user","content":"Now say hello."}));
        send(&history);
        assert_eq!(
            (regular.calls(), primary.calls(), decider.calls()),
            (2, 2, 2),
            "new user decisions may select regular despite a zero prior-stage score"
        );
        let requests = decider.requests();
        let wire = &requests[1];
        let start = wire.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
        let body: Value = serde_json::from_slice(&wire[start..]).unwrap();
        assert_eq!(body.as_object().unwrap().len(), 5);
        assert_eq!(
            body["route"]["kind"],
            if smart { "smart_saving" } else { "branches" }
        );
        assert!(body.get("definition").is_none());
        assert!(body["route"].get("judgment").is_none());
    }
}
