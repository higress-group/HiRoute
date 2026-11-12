//! Native installed DSH Desktop Host, using the Windows product's public settings and Gateway.
use super::windows_business_http as http;
use super::windows_headless_business::{Business, cli};
use super::windows_routing_business::{MODEL, Plan};
use super::windows_user_models::{apply, run_native};
use super::{InstalledProduct, SECRET, gateway_ready, lifecycle, security};
use hiroute_integrations::{
    AgentFilesystemLayoutV1, ClaudeRegistrationIndexV1, FilesystemAgentScannerV1,
};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ANSWER: &str = "native Desktop model routing receipt";
const PROMPTS: [&str; 5] = [
    "original-before-save",
    "original-after-save",
    "managed-responses",
    "managed-messages",
    "original-after-restore",
];

pub(super) fn run(product: &InstalledProduct) {
    let home = product.root.join("DSH Desktop 用户");
    security::create_owner_only_directory(&home).unwrap();
    let dsh_home = home.join(".dsh");
    let profile = dsh_home.join("profiles/desktop");
    let bundle = profile.join("node_modules/hiroute-fixture-provider");
    std::fs::create_dir_all(&bundle).unwrap();
    let manifest = profile.join("package.json");
    write_json(
        &manifest,
        &json!({"name":"desktop","private":true,
        "dependencies":{"hiroute-fixture-provider":"1.0.0"},
        "dsh":{"profile":{"bundles":["@deepseek-ai/dsh-base","@deepseek-ai/dsh-web-app","hiroute-fixture-provider"]}}}),
    );
    write_json(
        &bundle.join("package.json"),
        &json!({"name":"hiroute-fixture-provider","version":"1.0.0",
        "dsh":{"bundle":{"patch":"./cordis.patch.yml"}}}),
    );
    let observed = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::clone(&observed);
    let upstream = http::Server::start(move |request| {
        assert!(request.authenticated(SECRET));
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/v1/responses");
        assert_eq!(request.body["model"], MODEL);
        if request.body["stream"] == false {
            return http::Reply::json(
                200,
                json!({"id":"desktop-check","object":"response","model":MODEL,"status":"completed",
                "output":[{"type":"function_call","id":"call-check","call_id":"call-check","name":"hiroute_probe","arguments":"{\"value\":\"ok\"}"}],
                "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}),
            );
        }
        let body = request.body.to_string();
        let prompt = PROMPTS
            .into_iter()
            .find(|prompt| body.contains(prompt))
            .unwrap();
        requests.lock().unwrap().push(prompt);
        response()
    });
    let bundle_patch = bundle.join("cordis.patch.yml");
    write_json(
        &bundle_patch,
        &json!([{"id":"llm-pi-ai","config":{"providers":{"fixture-original":{
            "api":"openai-responses","baseURL":format!("http://{}/v1",upstream.address),
            "headers":{"Authorization":format!("Bearer {SECRET}")},
            "models":[{"id":MODEL,"name":"Original model","contextWindow":32768,"maxTokens":512,"input":["text"]}]
        }}}}]),
    );
    let probe = profile.join("test-probe.mjs");
    std::fs::write(&probe, include_str!("dsh_desktop_native_probe.mjs")).unwrap();
    let patch = profile.join("cordis.patch.yml");
    let row = json!({"insert":[{"id":"test-probe","name":hiroute_integrations::native_routing_adapter_uri(&probe).unwrap()}]});
    let baseline = format!("# Desktop fixture\n- {row}\n");
    std::fs::write(&patch, &baseline).unwrap();
    let manifest_before = std::fs::read(&manifest).unwrap();
    let bundle_before = std::fs::read(&bundle_patch).unwrap();

    let vars = [("DSH_HOME".into(), dsh_home.as_os_str().to_owned())];
    let mut running = product.start_with_environment(false, &vars);
    gateway_ready(running.listen);
    let scan = cli(product, &["agents", "scan"], None);
    let desktop = scan["data"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["agent_id"] == "agent_dsh_desktop")
        .expect("official Windows Desktop discovery");
    let context = desktop["context_id"].as_str().unwrap();
    let version = desktop["version"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap();
    println!("Official Windows DSH Desktop version {version}");
    let exe = executable(product);
    let node =
        PathBuf::from(std::env::var_os("HIROUTE_WORKER_NODE").expect("configured Windows Node"));
    let invoke = |calls: Vec<Value>, absent: Vec<String>| {
        let resources = exe.parent().unwrap().join("resources");
        let input = serde_json::to_vec(&json!({"executable":exe,"resources":resources,"profile":profile,
            "home":home,"dshHome":dsh_home,"version":version,"calls":calls,"absent":absent,"answer":ANSWER})).unwrap();
        let args = [probe.as_os_str().to_owned()];
        let env: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(key, _)| {
                matches!(
                    key.to_string_lossy().to_ascii_uppercase().as_str(),
                    "SYSTEMROOT" | "WINDIR" | "COMSPEC" | "PATH"
                )
            })
            .collect();
        let (status, bytes) = run_native(
            &node,
            &args,
            &env,
            &profile,
            Some(&input),
            Duration::from_secs(50),
        );
        assert_eq!(
            status, 0,
            "actual Desktop Host did not complete the native model probe"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"calls":calls.len(),"desktopHostReady":true})
        );
    };
    let original = |prompt| json!({"provider":"fixture-original","model":MODEL,"prompt":prompt});
    invoke(vec![original(PROMPTS[0])], vec![]);
    let source = Business::save_on(
        product,
        "candidate/native/dsh-desktop-plugin",
        "dsh-desktop-source",
        upstream.address,
    );
    let responses = Plan::create(product, source.binding_id(), "desktop-plugin-responses");
    let messages = Plan::create(product, source.binding_id(), "desktop-plugin-messages");
    let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure",
        "settings":{"mode":"dsh_additional","allowed_plan_ids":[responses.plan_id,messages.plan_id],
            "plan_protocols":{(responses.plan_id.clone()):"responses",(messages.plan_id.clone()):"messages"}}},
        "collaboration":{"intent":"keep"},"access_token":{"intent":"keep"}});
    apply(product, "connect", &spec, "desktop-plugin-configure");
    let configured = zeroize::Zeroizing::new(std::fs::read(&patch).unwrap());
    let appended = configured
        .strip_prefix(baseline.as_bytes())
        .and_then(|bytes| bytes.strip_prefix(b"- "))
        .expect("Save appends one independent insert to the original profile");
    let row: Value = serde_json::from_slice(appended).unwrap();
    let providers = row["insert"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == "hiroute-routing")
        .unwrap()["config"]["providers"]
        .as_object()
        .unwrap();
    assert_eq!(providers.len(), 2);
    let managed = |plan: &Plan, prompt| {
        let (provider, _) = providers
            .iter()
            .find(|(_, value)| value["models"][0]["id"] == plan.alias)
            .unwrap();
        json!({"provider":provider,"model":plan.alias,"prompt":prompt})
    };
    invoke(
        vec![
            original(PROMPTS[1]),
            managed(&responses, PROMPTS[2]),
            managed(&messages, PROMPTS[3]),
        ],
        vec![],
    );
    let current = cli(product, &["agents", "connect", "status", context], None);
    assert_eq!(current["data"]["state"], "configured");
    apply(
        product,
        "restore",
        &json!({"schema_version":{"major":2,"minor":0},"context_id":context,
        "model":{"intent":"restore","restore_point_ref":current["data"]["restore_point_ref"]},
        "collaboration":{"intent":"keep"},"access_token":{"intent":"keep"}}),
        "desktop-plugin-restore",
    );
    invoke(
        vec![original(PROMPTS[4])],
        providers.keys().cloned().collect(),
    );
    assert!(std::fs::read(&patch).unwrap() == baseline.as_bytes());
    assert!(std::fs::read(&manifest).unwrap() == manifest_before);
    assert!(std::fs::read(&bundle_patch).unwrap() == bundle_before);
    assert_eq!(observed.lock().unwrap().as_slice(), PROMPTS);
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    running.wait(true);
}

fn executable(product: &InstalledProduct) -> PathBuf {
    let registry = serde_json::from_slice(include_bytes!(
        "../../../../assets/connector-registry/current/registry-seed.json"
    ))
    .unwrap();
    let data: hiroute_domain::ReleaseModelDataBundleV2 = serde_json::from_slice(include_bytes!(
        "../../../../assets/release-facts/current/bundle/model-data.json"
    ))
    .unwrap();
    let index = ClaudeRegistrationIndexV1::from_verified_model_data(&registry, &data.data).unwrap();
    let layout = AgentFilesystemLayoutV1::from_process(&product.layout.home, &product.root);
    FilesystemAgentScannerV1::new(layout, index)
        .dsh_desktop_executable_target()
        .unwrap()
}

fn write_json(path: &Path, value: &Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
}

fn response() -> http::Reply {
    let part = json!({"type":"output_text","text":ANSWER,"annotations":[]});
    let item = json!({"type":"message","id":"native-desktop-message","role":"assistant","status":"completed","content":[part]});
    let events = [
        (
            "response.created",
            json!({"response":{"id":"native-desktop-response","object":"response","model":MODEL,"status":"in_progress","output":[]}}),
        ),
        (
            "response.output_item.added",
            json!({"output_index":0,"item":{"type":"message","id":"native-desktop-message","role":"assistant","status":"in_progress","content":[]}}),
        ),
        (
            "response.content_part.added",
            json!({"item_id":"native-desktop-message","output_index":0,"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}),
        ),
        (
            "response.output_text.delta",
            json!({"item_id":"native-desktop-message","output_index":0,"content_index":0,"delta":ANSWER}),
        ),
        (
            "response.output_item.done",
            json!({"output_index":0,"item":item}),
        ),
        (
            "response.completed",
            json!({"response":{"id":"native-desktop-response","object":"response","model":MODEL,"status":"completed","output":[item],
            "usage":{"input_tokens":4,"output_tokens":3,"total_tokens":7,"input_tokens_details":{"cached_tokens":0},"output_tokens_details":{"reasoning_tokens":0}}}}),
        ),
    ];
    http::Reply::sse(
        events
            .into_iter()
            .map(|(kind, mut event)| {
                event["type"] = json!(kind);
                format!("event: {kind}\ndata: {event}\n\n")
            })
            .collect(),
    )
}
