//! Installed Windows Control and native file checks for confirmed Codex Disable.
use super::{
    InstalledProduct, gateway_ready, lifecycle, security, windows_execution_host,
    windows_headless_business::cli, windows_routing_business, windows_user_models,
    windows_worker_business,
};
use serde_json::json;
use std::io::{Read, Write};
use std::time::Duration;

#[test]
#[ignore = "requires the pinned native Codex executable and a disposable Windows profile"]
fn actual_windows_codex_disables_edited_managed_settings() {
    let product = InstalledProduct::create();
    let codex = windows_worker_business::selected("HIROUTE_WORKER_CODEX_BINARY");
    windows_execution_host::image(&codex);
    let codex_home = product.root.join("isolated Codex 用户");
    security::create_owner_only_directory(&codex_home).unwrap();
    let mut path = vec![codex.parent().unwrap().to_path_buf()];
    path.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let environment = [
        ("CODEX_HOME".into(), codex_home.clone().into_os_string()),
        ("PATH".into(), std::env::join_paths(path).unwrap()),
    ];
    let config = codex_home.join("config.toml");
    let original = b"# user settings\nmodel = 'native-before-connection'\n";
    let mut file = security::create_owner_only_file(&config).unwrap();
    file.write_all(original).unwrap();
    drop(file);
    let mut running = product.start_with_environment(false, &environment);
    gateway_ready(running.listen);
    let routing = windows_routing_business::Routing::setup(&product);
    let scan = cli(&product, &["agents", "scan"], None);
    let access = scan["data"]["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agent_id"] == "agent_codex_default")
        .unwrap()["codex_access"]
        .clone();
    let apply = windows_user_models::apply;
    for context in [&access["root_context_id"], &access["profile_context_id"]] {
        let independent = context == &access["profile_context_id"];
        let target = if independent {
            codex_home.join("hiroute.config.toml")
        } else {
            config.clone()
        };
        let suffix = if independent { "profile" } else { "root" };
        apply(
            &product,
            "connect",
            &json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "model":{"intent":"configure","settings":{"mode":"codex_default","native_model_mode":"hiroute_only",
                "fixed_models":[],"allowed_plan_ids":[routing.plan.plan_id],
                "default_selection":{"kind":"plan","plan_id":routing.plan.plan_id}}}}),
            &format!("windows-disable-{suffix}-configure"),
        );
        let configured = cli(
            &product,
            &["agents", "connect", "status", context.as_str().unwrap()],
            None,
        );
        let text = std::fs::read_to_string(&target).unwrap();
        let edited = text.replacen(
            &format!("model = \"{}\"", routing.plan.alias),
            "model = 'manually-edited'",
            1,
        );
        assert_ne!(text, edited);
        let mut file = security::open_read_write(&target).unwrap();
        security::verify_owner_only_handle(&file, false).unwrap();
        file.set_len(0).unwrap();
        file.write_all(edited.as_bytes()).unwrap();
        drop(file);
        let preview = apply(
            &product,
            "restore",
            &json!({"schema_version":{"major":2,"minor":0},"context_id":context,
            "restore_managed_changes":true,"model":{"intent":"restore","restore_point_ref":configured["data"]["restore_point_ref"]}}),
            &format!("windows-disable-{suffix}-restore"),
        );
        assert_eq!(
            preview["model_effect"]["archive_codex_profile"] == true,
            independent
        );
        let restored = cli(
            &product,
            &["agents", "connect", "status", context.as_str().unwrap()],
            None,
        );
        assert!(restored["data"]["restore_point_ref"].is_null());
        let mut observed = String::new();
        security::open_identity(&config, false)
            .unwrap()
            .read_to_string(&mut observed)
            .unwrap();
        assert_eq!(
            hiroute_integrations::codex_explicit_model(&observed)
                .unwrap()
                .as_deref(),
            Some("native-before-connection")
        );
        assert!(observed.contains("# user settings"));
        assert!(!observed.contains("model_providers.hiroute"));
        assert!(!codex_home.join("hiroute.config.toml").exists());
    }
    lifecycle::stop(
        &product.layout.lifecycle_endpoint(),
        &product.daemon,
        Duration::from_secs(30),
    )
    .unwrap();
    running.wait(true);
}
