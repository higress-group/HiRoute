//! Publish the remaining account-free routing modes through the actual CLI.
//! Independent upstreams prove the selected candidate; public quality/receipts
//! prove that actual execution, decisions and a historical assessment persist.
#[path = "windows_routing_modes_observation.rs"]
mod observation;

use super::windows_business_http as http;
use super::windows_decision_business::mutate;
use super::windows_headless_business::{Business, cli, failure};
use super::windows_routing_business::{MODEL, Plan, grant, judgment};
use super::{InstalledProduct, SECRET};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
struct Execution {
    marker: String,
    lane: &'static str,
    input_tokens: u64,
    output_tokens: u64,
}
#[derive(Clone, Debug)]
struct DecisionCall {
    path: String,
    marker: String,
    has_target: bool,
}
pub struct Modes {
    pub smart: Plan,
    pub custom: Plan,
    economy: Business,
    primary: Business,
    _economy_server: http::Server,
    _primary_server: http::Server,
    _decision_server: http::Server,
    executions: Arc<Mutex<Vec<Execution>>>,
    decisions: Arc<Mutex<Vec<DecisionCall>>>,
    frozen_service: Value,
}
impl Modes {
    pub fn setup(product: &InstalledProduct) -> Self {
        let executions = Arc::new(Mutex::new(Vec::new()));
        let economy_server = model_server("economy", executions.clone());
        let primary_server = model_server("primary", executions.clone());
        let economy = Business::save_on_with_context(
            product,
            "candidate/native/windows-modes-economy",
            "windows-modes-source-economy",
            economy_server.address,
            131072,
        );
        let primary = Business::save_on_with_context(
            product,
            "candidate/native/windows-modes-primary",
            "windows-modes-source-primary",
            primary_server.address,
            131072,
        );
        assert!(executions.lock().unwrap().is_empty());
        let decisions = Arc::new(Mutex::new(Vec::new()));
        let calls = decisions.clone();
        let decision_server = http::Server::start(move |request| {
            assert_eq!(request.method, "POST");
            assert!(matches!(
                request.path.as_str(),
                "/decision-r1" | "/decision-r2"
            ));
            let body = request.body.as_object().unwrap();
            assert_eq!(body.len(), 5);
            for field in [
                "route",
                "latest_user",
                "visible_conversation",
                "history_partial",
                "preceding_stage",
            ] {
                assert!(
                    body.contains_key(field),
                    "custom extension must use its current five-field protocol"
                );
            }
            let marker = latest_marker(&request.body["latest_user"]);
            let has_target = request.body["preceding_stage"].is_object();
            if marker.contains("low") {
                assert!(
                    has_target,
                    "optional assessment must address an actually executed preceding stage"
                );
            }
            let group = if marker.contains("complex")
                || (marker.contains("low") && !marker.contains("regular"))
            {
                "primary"
            } else {
                "regular"
            };
            let mut response = match request.body["route"]["kind"].as_str() {
                Some("smart_saving") => {
                    assert_eq!(
                        request.body["route"],
                        json!({"kind":"smart_saving","groups":["regular","primary"]})
                    );
                    json!({"group":group})
                }
                Some("branches") => {
                    let choice = if marker.contains("docs") {
                        "docs"
                    } else {
                        "code"
                    };
                    let selected = request.body["route"]["categories"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|option| option["id"] == choice)
                        .unwrap();
                    if choice == "docs" {
                        assert!(
                            selected["groups"],
                            json!(["regular"]),
                            "the route declares only the published available groups"
                        );
                        // A single-group category ignores even an explicit primary.
                        json!({"category":choice,"group":"primary"})
                    } else {
                        assert_eq!(selected["groups"], json!(["regular", "primary"]));
                        json!({"category":choice,"group":group})
                    }
                }
                _ => panic!("published route sent an unsupported decision kind"),
            };
            calls.lock().unwrap().push(DecisionCall {
                path: request.path,
                marker: marker.clone(),
                has_target,
            });
            response["reason"] = json!("windows-native-extension-choice");
            // Scores describe prior work only; the extension chooses each group.
            // low-regular proves a low score cannot apply a built-in floor.
            if marker.contains("low") {
                response["assessment"] = json!({"score":0.1,"partial":false});
            }
            http::Reply::json(200, response)
        });
        let service = json!({"id":"windows-modes-decision","revision":1,"name":"Windows route decisions",
            "connection":{"kind":"custom","endpoint":format!("http://{}/decision-r1",decision_server.address),"timeout_ms":3000,"auth_header":null}});
        mutate(
            product,
            &json!({"id":service["id"],"expected_revision":0,"service":service}),
            "windows-modes-decision-create",
        );
        let classifier = json!({"kind":"decision_service","service":service});
        let mut smart_editor = base_editor("smart_saving");
        smart_editor["smart"] = json!({"economy":[{"binding_id":economy.binding_id()}],"primary":[{"binding_id":primary.binding_id()}],
            "judgment":judgment(),"reselect_on_user_message":true,"classifier":classifier,"complex_keywords":[]});
        let smart = Plan::create_with_editor(product, smart_editor, "windows-modes-smart");
        let mut custom_editor = base_editor("custom_branches");
        custom_editor["branch_routing"] = json!({"classifier":classifier,
            "branches":[
                {"id":"code","name":"Code","condition":"Programming and implementation tasks","candidates":[{"binding_id":economy.binding_id()}],
                    "primary_candidates":[{"binding_id":primary.binding_id()}],"judgment":null},
                {"id":"docs","name":"Documentation","condition":"Documentation tasks","candidates":[{"binding_id":primary.binding_id()}],
                    "primary_candidates":[],"judgment":null}],
            "default_branch_id":"docs","judgment":judgment(),"reselect_on_user_message":true});
        let custom = Plan::create_with_editor(product, custom_editor, "windows-modes-custom");
        let before = cli(product, &["routing", "list"], None)["data"].clone();
        let mut free_editor = base_editor("free_first");
        free_editor["free"]["candidates"] = json!([{"binding_id":economy.binding_id()}]);
        let options = cli(product, &["routing", "options"], Some(&json!({})));
        let candidate = options["data"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|candidate| candidate["binding_id"] == economy.binding_id())
            .unwrap();
        assert_eq!(candidate["billing_class"], "paid");
        failure(
            product,
            &["routing", "preview"],
            &json!({"change":{
            "schema":"hiroute.plan-content-change/v2","target":{"intent":"create","creation_key":"windows-modes-free-paid-rejected"},
            "editor":free_editor,"consumed_draft":null}}),
            "INVALID_ARGUMENTS",
        );
        assert_eq!(cli(product, &["routing", "list"], None)["data"], before);
        super::windows_free_admission::run(product, primary.binding_id());
        assert!(
            executions.lock().unwrap().is_empty() && decisions.lock().unwrap().is_empty(),
            "paid Free rejection and plan publication must make zero inference/decision requests"
        );
        let mut current = service.clone();
        current["revision"] = json!(2);
        current["name"] = json!("Edited Windows decision connection");
        current["connection"]["endpoint"] =
            json!(format!("http://{}/decision-r2", decision_server.address));
        mutate(
            product,
            &json!({"id":current["id"],"expected_revision":1,"service":current}),
            "windows-modes-decision-edit",
        );
        let result = Self {
            smart,
            custom,
            economy,
            primary,
            _economy_server: economy_server,
            _primary_server: primary_server,
            _decision_server: decision_server,
            executions,
            decisions,
            frozen_service: service,
        };
        result.assert_frozen(product);
        result
    }
    pub fn traffic(&self, product: &InstalledProduct, address: SocketAddr, connection: &str) {
        let token = grant(product, connection);
        let mut smart_input = Vec::new();
        for (marker, lane) in [
            ("windows-modes-smart-simple", "economy"),
            ("windows-modes-smart-low", "primary"),
            ("windows-modes-smart-complex", "primary"),
            ("windows-modes-smart-low-regular", "economy"),
        ] {
            self.send(
                product,
                address,
                &token,
                &self.smart,
                "windows-modes-smart-session",
                &mut smart_input,
                marker,
                lane,
            );
        }
        let mut custom_input = Vec::new();
        for (marker, lane) in [
            ("windows-modes-custom-code-simple", "economy"),
            ("windows-modes-custom-code-low", "primary"),
            ("windows-modes-custom-docs", "primary"),
            ("windows-modes-custom-code-low-regular", "economy"),
        ] {
            self.send(
                product,
                address,
                &token,
                &self.custom,
                "windows-modes-custom-session",
                &mut custom_input,
                marker,
                lane,
            );
        }
        self.assert_decisions();
        observation::assert_committed(product, &self.smart.plan_id, 4, true);
        observation::assert_committed(product, &self.custom.plan_id, 4, false);
    }
    pub fn recovered(&self, product: &InstalledProduct, address: SocketAddr, connection: &str) {
        self.assert_frozen(product);
        for source in [&self.economy, &self.primary] {
            let saved = cli(product, &["compute", "show", source.source_id()], None);
            assert_eq!(saved["data"]["sources"][0]["state"], "ready");
            assert_eq!(
                saved["data"]["sources"][0]["models"][0]["binding_id"],
                source.binding_id()
            );
        }
        let token = grant(product, connection);
        self.send(
            product,
            address,
            &token,
            &self.smart,
            "windows-modes-smart-restarted",
            &mut Vec::new(),
            "windows-modes-smart-restarted-simple",
            "economy",
        );
        self.send(
            product,
            address,
            &token,
            &self.custom,
            "windows-modes-custom-restarted",
            &mut Vec::new(),
            "windows-modes-custom-restarted-docs",
            "primary",
        );
        self.assert_decisions();
        // Re-query retained scored stages and ingest the new actual requests.
        observation::assert_committed(product, &self.smart.plan_id, 5, true);
        observation::assert_committed(product, &self.custom.plan_id, 5, false);
    }
    fn assert_frozen(&self, product: &InstalledProduct) {
        for (plan, pointer) in [
            (&self.smart, "/data/desired/strategy/classifier/service"),
            (
                &self.custom,
                "/data/desired/strategy/routing/classifier/service",
            ),
        ] {
            let shown = cli(product, &["routing", "show", &plan.plan_id], None);
            assert_eq!(
                shown.pointer(pointer),
                Some(&self.frozen_service),
                "published route must retain exact saved decision r1 after its editable service becomes r2"
            );
            assert_eq!(shown["data"]["head"]["model_alias"], plan.alias);
        }
        let services = cli(product, &["decision", "services", "list"], None);
        let service = services["data"]["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|service| service["id"] == self.frozen_service["id"])
            .unwrap();
        assert_eq!(service["revision"], 2);
    }
    fn assert_decisions(&self) {
        let executions = self.executions.lock().unwrap();
        let calls = self.decisions.lock().unwrap();
        assert_eq!(
            calls.len(),
            executions.len(),
            "every fresh user message must make one real decision"
        );
        for (execution, call) in executions.iter().zip(calls.iter()) {
            assert_eq!(execution.marker, call.marker);
            assert_eq!(
                call.path, "/decision-r1",
                "editable r2 cannot reinterpret the frozen published plan"
            );
            if call.marker.contains("low") {
                assert!(call.has_target);
            }
            assert!(execution.input_tokens > 0 && execution.output_tokens > 0);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn send(
        &self,
        product: &InstalledProduct,
        address: SocketAddr,
        token: &str,
        plan: &Plan,
        session: &str,
        input: &mut Vec<Value>,
        marker: &str,
        expected_lane: &str,
    ) {
        let before = self.executions.lock().unwrap().len();
        input.push(json!({"role":"user","content":marker}));
        let response = http::post_with_headers(
            address,
            "/v1/responses",
            None,
            &[("X-HiRoute-Token", token), ("session-id", session)],
            &json!({"model":plan.alias,"stream":false,"input":input}),
        );
        if response.status != 200 {
            let body = response.json();
            let code = body["code"]
                .as_str()
                .filter(|code| {
                    code.len() <= 80
                        && code
                            .bytes()
                            .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
                })
                .unwrap_or("UNCLASSIFIED");
            eprintln!(
                "Windows routing mode failure: marker={marker}, status={}, code={code}, executions={}, decisions={}",
                response.status,
                self.executions.lock().unwrap().len(),
                self.decisions.lock().unwrap().len()
            );
            for (lane, source) in [("economy", &self.economy), ("primary", &self.primary)] {
                let shown = cli(product, &["compute", "show", source.source_id()], None);
                let source: hiroute_application_api::ComputeManagedSourceViewV2 =
                    serde_json::from_value(shown["data"]["sources"][0].clone()).unwrap();
                eprintln!(
                    "Windows routing credential state: lane={lane}, state={:?}, keys={}, enabled={}, generations={:?}",
                    source.state,
                    source.keys.len(),
                    source.keys.iter().filter(|key| key.enabled).count(),
                    source
                        .keys
                        .iter()
                        .map(|key| key.generation)
                        .collect::<Vec<_>>()
                );
            }
            // Drain the product diagnostic writer before panic cleanup kills
            // the owned daemon. Request and assertion semantics stay intact.
            let _ = super::lifecycle::stop(
                &product.layout.lifecycle_endpoint(),
                &product.daemon,
                std::time::Duration::from_secs(30),
            );
        }
        assert_eq!(
            response.status, 200,
            "published mode must execute through actual Gateway authorization and planning"
        );
        let response = response.json();
        assert_eq!(response["status"], "completed");
        assert!(!response["output"].as_array().unwrap().is_empty());
        input.extend(response["output"].as_array().unwrap().iter().cloned());
        let ledger = self.executions.lock().unwrap();
        assert_eq!(
            ledger.len(),
            before + 1,
            "one successful request must execute exactly one candidate"
        );
        assert_eq!(ledger.last().unwrap().marker, marker);
        assert_eq!(
            ledger.last().unwrap().lane,
            expected_lane,
            "independent native upstream identifies the actual selected binding"
        );
    }
}
fn base_editor(mode: &str) -> Value {
    json!({"schema":"hiroute.plan-editor/v2","display_name":format!("Windows {mode}"),"purpose":"Actual public routing modes on Windows",
        "mode":mode,"candidates":[],"smart":{"economy":[],"primary":[],"judgment":judgment(),
            "reselect_on_user_message":false,"classifier":{"kind":"local_rules"},"complex_keywords":[]},
        "free":{"candidates":[],"primary":[],"primary_fallback":false},"delegation_enabled":false,"requirements":{},
        "limits":{"maximum_attempts":1,"request_timeout_ms":30000,"attempt_timeout_ms":30000}})
}
fn latest_marker(value: &Value) -> String {
    let text = serde_json::to_string(value).unwrap();
    let start = text
        .find("windows-modes-")
        .expect("independent oracle requires the actual current controlled user message");
    text[start..]
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect()
}
fn model_server(lane: &'static str, executions: Arc<Mutex<Vec<Execution>>>) -> http::Server {
    http::Server::start(move |request| {
        assert!(
            request.authenticated(&format!("Bearer {SECRET}")),
            "actual saved credential must authenticate each native upstream"
        );
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/responses");
        assert_eq!(request.body["model"], MODEL);
        if request.body["tools"]
            .as_array()
            .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == "hiroute_probe"))
        {
            return http::Reply::json(
                200,
                json!({"id":format!("windows-modes-check-{lane}"),"object":"response","model":MODEL,"status":"completed",
                "output":[{"type":"function_call","id":"probe","call_id":"probe","name":"hiroute_probe","arguments":"{\"value\":\"ok\"}"}],
                "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}),
            );
        }
        assert_eq!(request.body["stream"], false);
        let user = request.body["input"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|message| message["role"] == "user")
            .unwrap();
        let marker = latest_marker(user);
        let (input_tokens, output_tokens) = if lane == "economy" {
            (31, 12)
        } else {
            (37, 13)
        };
        let mut ledger = executions.lock().unwrap();
        let sequence = ledger.len() + 1;
        ledger.push(Execution {
            marker,
            lane,
            input_tokens,
            output_tokens,
        });
        http::Reply::json(
            200,
            json!({"id":format!("windows-modes-{lane}-{sequence}"),"object":"response","model":MODEL,"status":"completed",
            "output":[{"type":"message","id":format!("modes-message-{lane}-{sequence}"),"role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"windows modes actual native answer","annotations":[]}]}],
            "usage":{"input_tokens":input_tokens,"output_tokens":output_tokens,"total_tokens":input_tokens+output_tokens}}),
        )
    })
}

pub fn focused() {
    let product = InstalledProduct::create();
    let contexts = super::windows_user_models::Contexts::prepare(&product);
    let mut running = product.start_with_environment(false, contexts.variables());
    let modes = Modes::setup(&product);
    let scan = super::windows_headless_business::cli(&product, &["agents", "scan"], None);
    let agent = scan["data"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agent_id"] == "agent_pi_default")
        .unwrap();
    let context = agent["context_id"].as_str().unwrap();
    super::windows_user_models::checked(&product, "agent_pi_default");
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,
        "model":{"intent":"configure","settings":{"mode":"pi_additional",
            "allowed_plan_ids":[modes.smart.plan_id,modes.custom.plan_id],
            "plan_protocols":{&modes.smart.plan_id:"responses",&modes.custom.plan_id:"responses"}}}});
    super::windows_user_models::apply(&product, "connect", &spec, "windows-focused-modes-connect");
    modes.traffic(
        &product,
        running.listen,
        &format!("agent-connection/{context}"),
    );
    super::lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        std::time::Duration::from_secs(30),
    )
    .unwrap();
    running.wait(true);
}
