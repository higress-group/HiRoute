use super::*;
use crate::agent_turn_history::{AgentTurnStatus, ToolStatus, VisibleContentPart};
use crate::context_hold::{ContextHoldKey, visible_history_with_replay};
use crate::replay::{ReplayConfig, ReplayManager};
use crate::server::composition::ProductionPorts;
use crate::server::core_runtime::model_ir::{
    ExactProviderPathV1, ModelEvent, ModelStreamEventV1, ToolKindV1, ToolResultStatusV1,
};
use crate::server::publication::GatewayPublicationInstaller;
use crate::server::request_plan::IngressProtocol;
use hiroute_gateway_core::runtime::body::BudgetTree;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

struct Fixture {
    runtime: ProductionGatewayRuntime,
    replay: ReplayStore,
    _manager: ReplayManager,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hiroute-admission-{}-{}",
            std::process::id(),
            super::super::observation::unix_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let publications =
            Arc::new(GatewayPublicationInstaller::open(root.join("publication.json")).unwrap());
        let runtime = ProductionGatewayRuntime::compose(ProductionPorts::fail_closed(publications));
        let manager = ReplayManager::open(ReplayConfig {
            root: root.join("replay"),
            memory_threshold_bytes: 4096,
            record_bytes: 4096,
            orphan_ttl: Duration::from_secs(60),
        })
        .unwrap();
        let budget = BudgetTree::new(1024 * 1024, 1024 * 1024)
            .unwrap()
            .stream(1024 * 1024)
            .unwrap();
        let replay = manager.begin_request(budget).unwrap();
        Self {
            runtime,
            replay,
            _manager: manager,
            root,
        }
    }

    fn request(input: Value) -> model_ir::ModelRequestIRV1 {
        super::super::adapters::decode_ingress_request(
            IngressProtocol::Responses,
            &json!({"model":"route", "input":input, "stream":false}),
        )
        .unwrap()
    }

    fn prepare(&self, request: &model_ir::ModelRequestIRV1) -> PreparedHold {
        let history = visible_history_with_replay(request, &[23; 32], Some(&self.replay)).unwrap();
        self.runtime
            .context_holds
            .prepare(
                ContextHoldKey::for_runtime_test(1),
                &history,
                Instant::now(),
            )
            .unwrap()
    }

    fn begin(&self, request: &model_ir::ModelRequestIRV1, hold: PreparedHold) -> DecisionHistory {
        self.runtime
            .begin_decision_history(
                &super::super::observation::accepted_request_for_runtime_test(),
                PlanSnapshot {
                    plan_id: "plan".into(),
                    plan_revision: 1,
                },
                request,
                &self.replay,
                Some(hold),
            )
            .unwrap()
    }

    fn finish(&self, history: &mut DecisionHistory, events: &[ModelStreamEventV1], terminal: bool) {
        let guard = history.guard.as_mut().unwrap();
        if guard.ticket().new_turn {
            let strategy = profiles::ComplexityV1::compile(Vec::<(String, String)>::new()).unwrap();
            let (decision, _) =
                profiles::ComplexityV1::decide(Some("simple"), None, &strategy).unwrap();
            self.runtime
                .agent_turn_history
                .commit_decision(guard.ticket(), decision)
                .unwrap();
        }
        self.runtime
            .agent_turn_history
            .accept_output(guard.ticket(), events);
        self.runtime
            .agent_turn_history
            .finish_request(
                guard.ticket(),
                "accepted".into(),
                AgentTurnStatus::Completed,
                vec![crate::agent_turn_history::AcceptedExecution {
                    branch_execution: None,
                    model_configuration_id: "model".into(),
                    profile_digest: "profile".into(),
                    executed_branch_id: "smart_saving_simple".into(),
                    request_id: "accepted".into(),
                }],
                terminal,
                Instant::now(),
            )
            .unwrap();
        guard.disarm();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn text_event(text: &str) -> ModelStreamEventV1 {
    ModelStreamEventV1::new(
        0,
        ModelEvent::TextDelta {
            index: 0,
            text: text.into(),
        },
    )
}

#[test]
fn p2_failed_hold_admission_preserves_accepted_turns_and_reports_the_capture_gap() {
    for failure in ["busy", "stale"] {
        let fixture = Fixture::new();
        let first = Fixture::request(json!([{"role":"user","content":"first"}]));
        let mut admitted = fixture.begin(&first, fixture.prepare(&first));
        fixture.finish(&mut admitted, &[text_event("first answer")], true);
        drop(admitted);

        let second = Fixture::request(json!([
            {"role":"user","content":"first"}, {"role":"assistant","content":"first answer"},
            {"role":"user","content":"second"}
        ]));
        let prepared = fixture.prepare(&second);
        let rejected = if failure == "busy" {
            let _lock = fixture.runtime.context_holds.lock_for_runtime_test();
            fixture.begin(&second, prepared)
        } else {
            fixture
                .runtime
                .context_holds
                .commit(fixture.prepare(&first), Instant::now())
                .unwrap();
            fixture.begin(&second, prepared)
        };
        assert!(rejected.guard.is_none());
        assert!(rejected.hold_ticket.is_none());
        drop(rejected);

        // The detached second request still executes normally. Its accepted output
        // can be present in the next native transcript but was not captured here.
        let third = Fixture::request(json!([
            {"role":"user","content":"first"}, {"role":"assistant","content":"first answer"},
            {"role":"user","content":"second"}, {"role":"assistant","content":"second answer"},
            {"role":"user","content":"third"}
        ]));
        let next = fixture.begin(&third, fixture.prepare(&third));
        assert!(
            next.history.history_partial,
            "{failure}: an uncaptured request is a history gap"
        );
        assert!(
            next.history.assessment_target.is_none(),
            "{failure}: do not assess a stale segment across the gap"
        );
        assert_eq!(next.history.visible_conversation.len(), 1);
        assert_eq!(
            next.history.visible_conversation[0].user,
            vec![VisibleContentPart::Text {
                text: "first".into()
            }]
        );
        assert_eq!(
            next.history.visible_conversation[0].steps,
            vec![vec![VisibleContentPart::Text {
                text: "first answer".into()
            }]]
        );
    }
}

fn tool_event() -> ModelStreamEventV1 {
    ModelStreamEventV1::new(
        0,
        ModelEvent::ToolCallStarted {
            index: 0,
            logical_id: "tool-1".into(),
            native_id: "tool-1".into(),
            tool_kind: ToolKindV1::Function,
            namespace: None,
            name: "read".into(),
            item_id: None,
            owner: Box::new(ExactProviderPathV1 {
                provider_id: "p".into(),
                endpoint_id: "e".into(),
                entitlement_id: "e".into(),
                connector_id: "c".into(),
                connector_revision: "1".into(),
                capability_id: "c".into(),
                capability_revision: "1".into(),
                model_configuration_id: "m".into(),
                native_model: "m".into(),
                upstream_protocol: IngressProtocol::Responses,
                adapter_revision: "1".into(),
                serializer_revision: "1".into(),
                decoder_revision: "1".into(),
            }),
        },
    )
}

fn tool_continuation(status: ToolResultStatusV1) -> model_ir::ModelRequestIRV1 {
    let mut request = Fixture::request(json!([
        {"role":"user","content":"first"},
        {"type":"function_call","call_id":"tool-1","name":"read","arguments":"{}"},
        {"type":"function_call_output","call_id":"tool-1","output":"result"}
    ]));
    let Some(model_ir::ContentPart::ToolResult { status: result, .. }) = request
        .messages
        .last_mut()
        .and_then(|m| m.content.first_mut())
    else {
        panic!("expected native tool result");
    };
    *result = status;
    request
}

#[test]
fn p2_failed_hold_admission_cannot_apply_an_unadmitted_tool_result() {
    let fixture = Fixture::new();
    let first = Fixture::request(json!([{"role":"user","content":"first"}]));
    let mut admitted = fixture.begin(&first, fixture.prepare(&first));
    fixture.finish(&mut admitted, &[tool_event()], false);
    drop(admitted);

    let rejected = tool_continuation(ToolResultStatusV1::Failed);
    let prepared = fixture.prepare(&rejected);
    let _lock = fixture.runtime.context_holds.lock_for_runtime_test();
    let bypassed = fixture.begin(&rejected, prepared);
    assert!(bypassed.guard.is_none());
    drop(_lock);
    drop(bypassed);

    let continued = tool_continuation(ToolResultStatusV1::Completed);
    let mut admitted = fixture.begin(&continued, fixture.prepare(&continued));
    assert!(admitted.correlated.is_some());
    fixture.finish(&mut admitted, &[text_event("completed")], true);
    drop(admitted);
    let next = Fixture::request(json!([
        {"role":"user","content":"first"}, {"role":"assistant","content":"completed"},
        {"role":"user","content":"next"}
    ]));
    let admitted = fixture.begin(&next, fixture.prepare(&next));
    let parts = admitted
        .history
        .visible_conversation
        .iter()
        .flat_map(|turn| &turn.steps)
        .flatten()
        .collect::<Vec<_>>();
    assert!(parts.iter().any(|part| matches!(
        part,
        VisibleContentPart::ToolActivity {
            status: ToolStatus::Completed,
            ..
        }
    )));
    assert!(!parts.iter().any(|part| matches!(
        part,
        VisibleContentPart::ToolActivity {
            status: ToolStatus::Failed,
            ..
        }
    )));
}
