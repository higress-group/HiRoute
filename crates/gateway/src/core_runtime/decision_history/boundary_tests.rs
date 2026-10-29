use super::*;
use crate::server::core_runtime::model_ir::{
    CanonicalInstruction, ContentPart, InstructionRole, ResponseItemStatus,
};

#[test]
fn actual_stage_continuity_and_strict_candidate_affinity_have_separate_authority() {
    for change in [
        "none",
        "instructions",
        "tool-setup",
        "omitted-reasoning",
        "old-user",
    ] {
        let fixture = Fixture::new();
        let first = Fixture::request(json!([{"role":"user","content":"A"}]));
        let mut admitted = fixture.begin(&first, fixture.prepare(&first));
        let first_id = admitted
            .guard
            .as_ref()
            .unwrap()
            .ticket()
            .agent_turn_id
            .clone();
        let mut tool = tool_event();
        if let ModelEvent::ToolCallStarted { index, .. } = &mut tool.event {
            *index = 1;
        }
        fixture.finish_named(
            &mut admitted,
            &[
                ModelStreamEventV1::new(
                    0,
                    ModelEvent::ReasoningFinished {
                        index: 0,
                        text: "actual reasoning".into(),
                        status: ResponseItemStatus::Completed,
                    },
                ),
                tool,
                ModelStreamEventV1::new(
                    2,
                    ModelEvent::ToolCallFinished {
                        encrypted_function_args: None,
                        index: 1,
                        logical_id: "tool-1".into(),
                        tool_kind: ToolKindV1::Function,
                        namespace: None,
                        name: "read".into(),
                        arguments: json!({}),
                        status: ResponseItemStatus::Completed,
                    },
                ),
            ],
            false,
            "request-tool",
        );
        fixture.runtime.context_holds.complete(
            admitted.hold_ticket.as_ref().unwrap(),
            profiles::HoldPreferenceV1 {
                stable_binding_id: "binding".into(),
                candidate_id: "candidate".into(),
                profile_digest: "profile".into(),
                reasoning_profile_id: "fixed".into(),
                origin_group_id: "smart_saving_simple".into(),
            },
            Instant::now(),
        );
        drop(admitted);
        let mut items =
            vec![json!({"role":"user","content":if change == "old-user" {"changed A"} else {"A"}})];
        if change != "omitted-reasoning" {
            items.push(json!({"type":"reasoning","summary":[{"type":"summary_text","text":"actual reasoning"}]}));
        }
        items.extend([
            json!({"type":"function_call","call_id":"tool-1","name":"read","arguments":"{}"}),
            json!({"type":"function_call_output","call_id":"tool-1","output":"actual result"}),
        ]);
        let mut next = Fixture::request(json!(items));
        if change == "instructions" {
            next.instructions.push(CanonicalInstruction {
                role: InstructionRole::System,
                content: vec![ContentPart::Text {
                    text: "current request budget".into(),
                }],
            });
        }
        if change == "tool-setup" {
            next.parallel_tool_calls = true;
        }
        let continued = fixture.begin(&next, fixture.prepare(&next));
        let guard = continued.guard.as_ref().unwrap();
        if change == "old-user" {
            assert_ne!(guard.ticket().agent_turn_id, first_id);
            assert!(continued.correlated.is_none());
        } else {
            assert_eq!(guard.ticket().agent_turn_id, first_id);
            assert!(continued.correlated.is_some());
            if change == "omitted-reasoning" {
                assert!(continued.hold_ticket.is_none());
            } else {
                assert_eq!(
                    continued.hold_ticket.as_ref().unwrap().hint.is_some(),
                    change == "none"
                );
            }
        }
    }
}
