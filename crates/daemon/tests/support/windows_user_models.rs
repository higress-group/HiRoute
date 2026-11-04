//! Saved ordinary user models, independent collaboration, token rotation and
//! Restore through released public commands. No native config bytes are seeded.
#[path = "windows_user_model_native.rs"]
mod native;
#[path = "windows_user_model_oracle.rs"]
mod oracle;
use super::windows_user_model_receipts as receipts;
pub use native::Contexts;

use super::windows_business_http as http;
use super::windows_headless_business::{Business, assert_operation, cli};
use super::windows_routing_business::{Plan, grant};
use super::{InstalledProduct, assert_no_secret, security};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::Zeroizing;

pub struct UserModels {
    models: Vec<Model>,
}
struct Model {
    pi: bool,
    context: String,
    provider: String,
    connection: String,
    native_root: PathBuf,
    file: PathBuf,
    skill: PathBuf,
    plan: Plan,
    _source: Business,
    oracle: oracle::Oracle,
    nonce: String,
    configured: Value,
    retained: BTreeSet<String>,
    retained_file: Zeroizing<Vec<u8>>,
    retained_permissions: Vec<u8>,
}
impl UserModels {
    pub fn run(product: &InstalledProduct, contexts: &Contexts, gateway: SocketAddr) -> Self {
        let mut models = Vec::new();
        for pi in [true, false] {
            let label = if pi { "pi" } else { "dsh" };
            let agent = format!("agent_{label}_default");
            let root = if pi {
                contexts.pi.clone()
            } else {
                contexts.dsh.clone()
            };
            let file = root.join(if pi {
                "models.json"
            } else {
                "profiles/web/cordis.patch.yml"
            });
            let skill = root.join("skills/hiroute-collaboration/SKILL.md");
            assert!(
                !file.exists() && !skill.exists(),
                "ordinary user settings scenario requires an unowned native target"
            );
            let scan = cli(product, &["agents", "scan"], None);
            let scanned = scan["data"]["agents"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["agent_id"] == agent)
                .expect(
                    "actual known npm producer must be discovered through its real generated shim",
                );
            let context = scanned["context_id"].as_str().unwrap().to_owned();
            checked(product, &agent);
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes).unwrap();
            let nonce: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            let oracle =
                oracle::Oracle::start(pi, skill.clone(), product.stable.clone(), nonce.clone());
            let source = Business::save_on_with_context(
                product,
                &format!("candidate/native/windows-user-{label}"),
                &format!("windows-user-{label}-source"),
                oracle.server.address,
                131072,
            );
            let plan = Plan::create(
                product,
                source.binding_id(),
                &format!("windows-user-{label}-plan"),
            );
            let spec = model_spec(&context, label, &plan.plan_id, false);
            let default = default_file(&root, pi);
            let before_default = optional(&default);
            let preview = apply(
                product,
                "connect",
                &spec,
                &format!("windows-user-{label}-configure"),
            );
            assert!(
                optional(&default) == before_default,
                "public model Configure must preserve native defaults"
            );
            let provider = preview["model_effect"]["provider_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let connection = format!("agent-connection/{context}");
            let _token = grant(product, &connection);
            let configured = status(product, &context);
            assert_configured(&configured, label, &plan.plan_id);
            let mut model = Model {
                pi,
                context,
                provider,
                connection,
                native_root: root,
                file,
                skill,
                plan,
                _source: source,
                oracle,
                nonce,
                configured,
                retained: BTreeSet::new(),
                retained_file: Zeroizing::new(Vec::new()),
                retained_permissions: Vec::new(),
            };
            model.assert_provider();
            model.invoke(contexts, "configured");
            let old = grant(product, &model.connection);
            let rotated = model_spec(&model.context, label, &model.plan.plan_id, true);
            let before_default = optional(&default);
            apply(
                product,
                "connect",
                &rotated,
                &format!("windows-user-{label}-rotate"),
            );
            assert!(
                optional(&default) == before_default,
                "public model rotation must preserve native defaults"
            );
            let new = grant(product, &model.connection);
            assert!(
                *old != *new,
                "model authority rotation must issue a fresh private grant"
            );
            let before = model.oracle.ledger.lock().unwrap().requests.len();
            let rejected = http::post_expect_continue_with_headers(
                gateway,
                "/v1/responses",
                None,
                &[("X-HiRoute-Token", old.as_str())],
                &json!({"model":model.plan.alias,"input":"removed model authority","max_output_tokens":16,"stream":false}),
            );
            assert_eq!(rejected.status, 401);
            assert_eq!(rejected.json()["code"], "GATEWAY_GRANT_UNAUTHORIZED");
            assert_eq!(
                model.oracle.ledger.lock().unwrap().requests.len(),
                before,
                "obsolete authority must not reach upstream"
            );
            model.assert_provider();
            model.invoke(contexts, "rotated");
            model.collaboration(product, contexts, &agent, label);
            model.configured = status(product, &model.context);
            assert_configured(&model.configured, label, &model.plan.plan_id);
            model.retained = model.receipts(product);
            model.retained_file = read(&model.file);
            model.retained_permissions = permission_facts(&model.file);
            models.push(model);
        }
        Self { models }
    }
    pub fn recovered(&mut self, product: &InstalledProduct, contexts: &Contexts) {
        for model in &mut self.models {
            let label = if model.pi { "pi" } else { "dsh" };
            assert_eq!(
                status(product, &model.context)["current_selection"],
                model.configured["current_selection"]
            );
            assert_eq!(
                status(product, &model.context)["restore_point_ref"],
                model.configured["restore_point_ref"]
            );
            assert_eq!(model.receipts(product), model.retained);
            assert!(
                *read(&model.file) == *model.retained_file,
                "actual restart must retain exact native model bytes/grant"
            );
            assert!(
                permission_facts(&model.file) == model.retained_permissions,
                "actual restart must retain native model Owner/DACL"
            );
            model.assert_provider();
            model.invoke(contexts, "restarted");
            let actual = model.receipts(product);
            assert!(model.retained.is_subset(&actual));
            let current = status(product, &model.context);
            let default = default_file(&model.native_root, model.pi);
            let before_default = optional(&default);
            let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":model.context,
                "model":{"intent":"restore","restore_point_ref":current["restore_point_ref"]},"collaboration":{"intent":"keep"},"access_token":{"intent":"keep"}});
            apply(
                product,
                "restore",
                &spec,
                &format!("windows-user-{label}-restore"),
            );
            let restored = status(product, &model.context);
            assert_eq!(restored["state"], "not_configured");
            assert!(
                restored["current_selection"].is_null() && restored["restore_point_ref"].is_null()
            );
            assert!(
                !model.file.exists(),
                "Restore must remove a new untouched user model file"
            );
            assert!(
                !model.skill.exists(),
                "independent Skill Restore must remain released across model Restore"
            );
            // Native clients may initialize their own defaults. The product
            // Restore must retain their actual bytes, never infer nonexistence.
            assert!(
                optional(&default) == before_default,
                "public model Restore must preserve native-owned defaults"
            );
            assert_eq!(
                model.receipts(product),
                actual,
                "Restore must retain actual user-model request receipts"
            );
            let ledger = model.oracle.ledger.lock().unwrap();
            assert_eq!(ledger.probe, 1);
            assert_eq!(
                ledger.completed,
                BTreeSet::from([
                    "configured".into(),
                    "rotated".into(),
                    "collaboration".into(),
                    "restarted".into()
                ])
            );
            assert!(
                ledger.cli_proven,
                "ordinary native must have invoked the public CLI from the installed Skill"
            );
        }
    }
}
impl Model {
    fn assert_provider(&self) {
        let bytes = read(&self.file);
        let value = if self.pi {
            serde_json::from_slice::<Value>(&bytes).unwrap()
        } else {
            patch(&bytes)
        };
        let declared =
            hiroute_domain::additional_model_plan_provider_id(&self.provider, &self.plan.alias);
        let providers = value["providers"].as_object().unwrap();
        assert_eq!(
            providers.len(),
            1,
            "fresh native context must contain only the publicly owned provider"
        );
        let provider = providers
            .get(&declared)
            .expect("owned provider follows its declared alias, never ordering");
        assert!(
            provider["models"]
                .as_array()
                .is_some_and(|models| models.len() == 1 && models[0]["id"] == self.plan.alias),
            "saved ordinary models must match the published alias"
        );
        assert_eq!(provider["api"], "openai-responses");
        assert!(
            provider["headers"]["X-HiRoute-Token"]
                .as_str()
                .is_some_and(|token| !token.is_empty())
        );
    }
    fn invoke(&self, contexts: &Contexts, stage: &str) {
        let provider =
            hiroute_domain::additional_model_plan_provider_id(&self.provider, &self.plan.alias);
        contexts.invoke(
            self.pi,
            &provider,
            &self.plan.alias,
            &format!(
                "WINDOWS-USER-MODEL-{}-{stage}: Reply with the independent receipt.",
                self.nonce
            ),
            &format!("user-model-{stage}-{}", self.nonce),
        );
    }
    fn collaboration(
        &self,
        product: &InstalledProduct,
        contexts: &Contexts,
        agent: &str,
        label: &str,
    ) {
        let before = read(&self.file);
        let permissions = permission_facts(&self.file);
        let default = default_file(&self.native_root, self.pi);
        let before_default = optional(&default);
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":self.context,
            "model":{"intent":"keep"},"collaboration":{"intent":"configure","settings":{"trigger_mode":"explicit"}},"access_token":{"intent":"keep"}});
        apply(
            product,
            "connect",
            &spec,
            &format!("windows-user-{label}-collaboration"),
        );
        checked(product, agent);
        assert!(self.skill.is_file());
        security::verify_owner_only(&self.skill, false).unwrap();
        let provider =
            hiroute_domain::additional_model_plan_provider_id(&self.provider, &self.plan.alias);
        contexts.invoke(self.pi,&provider,&self.plan.alias,&format!("WINDOWS-USER-MODEL-{}-collaboration: Read hiroute-collaboration and use its trusted public CLI to inspect the released schema.",self.nonce),
            &format!("user-model-collaboration-{}",self.nonce));
        let current = status(product, &self.context);
        let spec = json!({"schema_version":{"major":2,"minor":0},"context_id":self.context,"model":{"intent":"keep"},
            "collaboration":{"intent":"restore","restore_point_ref":current["collaboration"]["restore_point_ref"]},"access_token":{"intent":"keep"}});
        apply(
            product,
            "restore",
            &spec,
            &format!("windows-user-{label}-skill-restore"),
        );
        assert!(!self.skill.exists());
        assert!(
            *read(&self.file) == *before,
            "independent Skill Restore must preserve exact model bytes/private token"
        );
        assert!(
            permission_facts(&self.file) == permissions,
            "independent Skill Restore must preserve the native model Owner/DACL"
        );
        assert!(
            optional(&default) == before_default,
            "collaboration Configure/Restore must preserve native defaults"
        );
        assert_configured(&status(product, &self.context), label, &self.plan.plan_id);
    }
    fn receipts(&self, product: &InstalledProduct) -> BTreeSet<String> {
        let expected = self.oracle.ledger.lock().unwrap().requests.clone();
        receipts::committed(product, &self.plan.plan_id, &expected)
    }
}
fn default_file(root: &Path, pi: bool) -> PathBuf {
    root.join(if pi {
        "settings.json"
    } else {
        "cordis.patch.yml"
    })
}
fn optional(path: &Path) -> Option<Zeroizing<Vec<u8>>> {
    let file = match security::open_identity(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => panic!("native default must retain its actual reparse-free readable identity"),
    };
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(262145).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 262144);
    Some(bytes)
}
fn read(path: &Path) -> Zeroizing<Vec<u8>> {
    let mut file = security::open_identity(path, false).unwrap();
    security::verify_owner_only_handle(&file, false).unwrap();
    assert_eq!(security::file_link_count(&file).unwrap(), 1);
    let mut bytes = Zeroizing::new(Vec::new());
    file.by_ref().take(262145).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() <= 262144);
    bytes
}
fn permission_facts(path: &Path) -> Vec<u8> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetKernelObjectSecurity, OWNER_SECURITY_INFORMATION,
    };
    let file = security::open_identity(path, false).unwrap();
    security::verify_owner_only_handle(&file, false).unwrap();
    let information = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut needed = 0;
    // SAFETY: querying the held native object does not resolve its pathname.
    unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
    }
    assert!(needed > 0 && needed <= 65536);
    let mut descriptor = vec![0u8; needed as usize];
    // SAFETY: the destination has the exact size returned by the kernel.
    assert_ne!(
        unsafe {
            GetKernelObjectSecurity(
                file.as_raw_handle(),
                information,
                descriptor.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        },
        0
    );
    descriptor
}
fn patch(bytes: &[u8]) -> Value {
    // Current producer formats each affected row as block-sequence JSON, a
    // public YAML subset. No arbitrary YAML/composition parser is recreated here.
    let mut rows = Vec::new();
    let mut row = String::new();
    for line in std::str::from_utf8(bytes).unwrap().lines() {
        if let Some(start) = line.strip_prefix("- ") {
            if !row.is_empty() {
                rows.push(serde_json::from_str::<Value>(&row).unwrap());
            }
            row = start.into();
        } else if let Some(tail) = line.strip_prefix("  ") {
            row.push('\n');
            row.push_str(tail);
        } else {
            assert!(line.trim().is_empty());
        }
    }
    if !row.is_empty() {
        rows.push(serde_json::from_str::<Value>(&row).unwrap());
    }
    let module = rows.iter().find(|row| row["id"] == "llm-pi-ai").unwrap();
    json!({"providers":module["config"]["providers"]})
}
fn model_spec(context: &str, label: &str, plan: &str, rotate: bool) -> Value {
    json!({"schema_version":{"major":2,"minor":0},"context_id":context,"model":{"intent":"configure",
        "settings":{"mode":format!("{label}_additional"),"allowed_plan_ids":[plan],"plan_protocols":{plan:"responses"}}},
        "collaboration":{"intent":"keep"},"access_token":{"intent":if rotate {"regenerate"} else {"keep"}}})
}
fn status(product: &InstalledProduct, context: &str) -> Value {
    cli(product, &["agents", "connect", "status", context], None)["data"].clone()
}
fn assert_configured(value: &Value, label: &str, plan: &str) {
    assert_eq!(value["state"], "configured");
    assert!(value["restore_point_ref"].is_string());
    assert_eq!(
        value["current_selection"],
        json!({"mode":format!("{label}_additional"),"allowed_plan_ids":[plan],"plan_protocols":{plan:"responses"}})
    );
}
pub(super) fn checked(product: &InstalledProduct, agent: &str) {
    let args: Vec<OsString> = [
        "agents",
        "check",
        agent,
        "--scope",
        "collaboration",
        "--output",
        "json",
    ]
    .map(OsString::from)
    .into();
    let env: Vec<_> = std::env::vars_os().collect();
    let (_, output) = native::run(
        &product.stable,
        &args,
        &env,
        product.stable.parent().unwrap(),
        None,
        Duration::from_secs(35),
    );
    assert_no_secret(&output);
    let envelope: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(envelope["status"], "succeeded");
    assert_eq!(envelope["data"]["skill_loading"], "proven");
    assert_eq!(envelope["data"]["trusted_cli_execution"], "proven");
}
pub(super) fn apply(product: &InstalledProduct, verb: &str, spec: &Value, key: &str) -> Value {
    let preview = cli(
        product,
        &["agents", verb, "preview"],
        Some(&json!({"spec":spec})),
    )["data"]
        .clone();
    assert_eq!(
        preview["applicable"], true,
        "public settings preview must admit the actual native user context"
    );
    assert_ne!(
        preview["resident_service"]["login_item_required"], true,
        "foreground must not invent manager registration evidence"
    );
    let request = json!({"spec":preview["spec"],"accept_digest":preview["accept_digest"],"dependency_digest":preview["dependency_digest"],
        "expected_revisions":preview["expected_revisions"],"idempotency_key":key});
    let applied = cli(product, &["agents", verb, "apply"], Some(&request));
    let operation = applied["operation"]["operation_id"]
        .as_str()
        .or_else(|| applied["data"]["operation_id"].as_str())
        .unwrap();
    assert_operation(product, operation);
    let replay = cli(product, &["agents", verb, "apply"], Some(&request));
    assert_eq!(
        replay["operation"]["operation_id"]
            .as_str()
            .or_else(|| replay["data"]["operation_id"].as_str())
            .unwrap(),
        operation
    );
    preview
}
