use super::*;

#[test]
fn native_search_history_extensions_preserve_payload_with_unknown_history() {
    for extension in [
        json!({"action":{"type":"provider_future_action","query":"Example"}}),
        json!({"action":{"type":"find_in_page","url":"https://example.com/",
            "pattern":"Example","provider_options":{"max_matches":2}}}),
        json!({"metadata":{"provider_note":"native-only"}}),
        json!({"status":"incomplete"}),
        json!({"status":"in_progress"}),
        json!({"action":{"type":"find_in_page","url":23,"pattern":"Example"}}),
    ] {
        let mut search = json!({"type":"web_search_call","id":"ws-native","status":"completed",
            "action":{"type":"search","query":"Example"}});
        search
            .as_object_mut()
            .unwrap()
            .extend(extension.as_object().unwrap().clone());
        let body = json!({"model":"alias","input":[
            {"role":"user","content":"Inspect the example"}, search,
            {"role":"user","content":"Continue"}
        ]});
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert!(request.native_only);
        assert!(request.responses_search_history.is_empty());
        assert!(!request.native_history_layout().unwrap()[1].modeled);
        assert!(crate::context_hold::visible_history(&request, &[0; 32]).is_none());
        assert_eq!(request.messages.len(), 3);
        assert_eq!(request.messages[1].role, MessageRole::Assistant);
        assert!(request.messages[1].content.is_empty());
        for model in ["original-model", "next-model"] {
            let mut profile =
                exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
            profile.capability.native_model = model.into();
            profile.capability.request.function_tools = Fidelity::Unsupported;
            let wire = project_candidate_request(&request, &profile).unwrap();
            assert_eq!(wire.body["input"], body["input"]);
            assert_eq!(wire.body["model"], model);
            assert!(
                project_candidate_request_template_with_cleanup(&request, &profile, Some(1))
                    .is_err()
            );
        }
        for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
            assert!(
                project_candidate_request(
                    &request,
                    &exact_state_profile(IngressProtocol::Responses, target)
                )
                .is_err()
            );
        }
    }
}

#[test]
fn native_search_declaration_extensions_remain_visible_to_network_authority() {
    for extension in [
        json!({"filters":{"allowed_domains":["example.com"]}}),
        json!({"search_context_size":"low","user_location":{"type":"approximate","country":"US"}}),
        json!({"provider_options":{"max_results":3}}),
        json!({"external_web_access":"provider-validates-this"}),
    ] {
        let mut search = json!({"type":"web_search","external_web_access":false});
        search
            .as_object_mut()
            .unwrap()
            .extend(extension.as_object().unwrap().clone());
        let body = json!({"model":"alias","input":"Inspect example.com","tools":[search]});
        let request = decode_ingress_request(IngressProtocol::Responses, &body).unwrap();
        assert!(request.native_only);
        assert!(request.web_search.is_some());
        assert!(request.tools.is_empty());
        assert!(crate::context_hold::visible_history(&request, &[0; 32]).is_none());
        let mut profile =
            exact_state_profile(IngressProtocol::Responses, IngressProtocol::Responses);
        profile.capability.request.function_tools = Fidelity::Unsupported;
        assert_eq!(
            project_candidate_request(&request, &profile).unwrap().body["tools"],
            body["tools"]
        );
        for target in [IngressProtocol::ChatCompletions, IngressProtocol::Messages] {
            assert!(
                project_candidate_request(
                    &request,
                    &exact_state_profile(IngressProtocol::Responses, target)
                )
                .is_err()
            );
        }
    }
}
