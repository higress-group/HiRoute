#![cfg(all(unix, feature = "integration-test-hooks"))]

#[path = "support/claude_catalog_product_regression.rs"]
mod claude_catalog_product_regression;

/// Decision services use released CLI codecs, the real journal and immutable Plan snapshots.
#[test]
fn real_process_decision_services_and_branch_publication() {
    run_script("decision_services_product.py");
}

/// A real daemon, protected source pool and synthetic OpenAI Decisions loopback.
#[test]
fn real_process_versioned_decision_connections_use_exact_source_keys() {
    run_script("decision_connections_v2_product.py");
}

/// Route actual model requests and inspect C facts through the existing product query.
#[test]
fn real_process_c_routing_execution_and_retained_observations() {
    run_script_case("decision_services_product.py", Some("routed"));
}

/// Inject at the real Product content sink, preserving output and later history/C.
#[test]
fn real_process_content_sink_failure_is_consumed_once_and_recovers() {
    run_script("observation_content_fault_product.py");
}

/// Current released CLI point scan/preparation shares the native multi-model exact-save boundary.
#[test]
fn released_cli_connection_points_prepare_and_save_exact_models() {
    run_script("connection_point_product.py");
}

/// Exact prepared CLI files survive both mid-publication and lost-terminal-response crashes.
#[test]
fn quickstart_exact_prepared_files_survive_publication_interruption() {
    run_script("quickstart_recovery.py");
}

/// Client-bundled release assets, actual binaries and production inherited authority FDs.
#[test]
fn real_process_publication_crashes_recover_without_reapplying() {
    run_script("publication_process.py");
}

/// Unmanaged native directories may have ordinary user permissions. Only operations that
/// actually use protected targets should reject them; startup must preserve their contents.
#[test]
fn real_process_starts_without_changing_unmanaged_native_directory_permissions() {
    run_script_case(
        "publication_process.py",
        Some("unmanaged-native-permissions"),
    );
}

#[test]
fn real_process_three_domain_interleavings_preserve_all_accepted_changes() {
    run_script("publication_interleaving.py");
}

#[test]
fn real_gateway_requests_retain_their_publication_across_installation() {
    run_script("publication_requests.py");
}

#[test]
fn real_gateway_upgrade_wait_cancels_and_rebuilds_untrusted_cache() {
    run_script("upgrade_requests.py");
}

#[test]
fn real_process_recovery_preserves_independent_agent_edits() {
    run_script("publication_agent_conflict.py");
}

#[test]
fn agent_settings_recover_after_target_before_service_receipt() {
    run_script_case("publication_agent_recovery.py", Some("after_target"));
}

#[test]
fn agent_settings_recover_after_active_before_service_receipt() {
    run_script_case("publication_agent_recovery.py", Some("after_active"));
}

#[test]
fn real_process_v2_plan_content_and_alias_recover_exactly() {
    run_script("plan_content_process.py");
}

#[test]
fn real_process_plan_lifecycle_preserves_content_and_checks_references() {
    run_script("plan_lifecycle_process.py");
}

#[test]
fn real_process_native_model_save_publishes_and_reaches_gateway() {
    run_script("model_connections_product.py");
}

/// Native source proxy configuration, checked routes and hot apply use real Control/CLI/Gateway.
#[test]
fn real_process_native_network_shared_decision_and_generation_retain_one_head_snapshot() {
    run_script("native_network_decision_product.py");
}

#[test]
fn real_process_native_network_proxy_applies_without_republishing_and_survives_restart() {
    run_script("native_network_product.py");
}

/// Execute both published recipes against the current catalog without mistaking local
/// candidate preparation for provider reachability or credential verification.
#[test]
fn real_process_connection_test_examples_report_unverified_openrouter() {
    run_script("connection_test_examples_product.py");
}

#[test]
fn real_process_custom_http_save_recheck_and_validation_errors() {
    run_script("custom_http_product.py");
}

/// Branch execution identity must not replace current managed source membership.
#[test]
fn real_process_native_branch_credentials_preserve_source_and_key_permissions() {
    run_script("native_branch_credentials_product.py");
}

#[test]
fn real_process_current_credential_pool_relays_without_republishing() {
    run_script("credential_pool_product.py");
}

#[test]
fn real_process_filtered_credential_pool_preserves_current_candidate_history() {
    run_script_case("credential_pool_product.py", Some("filtered-history"));
}

#[test]
fn real_process_route_options_preserve_opaque_ids_and_isolate_unavailable_models() {
    run_script("route_options_product.py");
}

/// A blank Linux HOME installs the candidate package, starts the installed daemon, and drives
/// the released CLI through compute, routing, Agent, observation, and Worker entry points.
#[test]
#[cfg(target_os = "linux")]
fn installed_standalone_cli_completes_the_headless_management_loop() {
    run_standalone_script("standalone_headless_product.py");
}

#[test]
#[cfg(target_os = "linux")]
fn installed_cli_quickstart_reuses_resources_and_exact_apply_files() {
    run_standalone_script("quickstart_product.py");
}

#[test]
#[cfg(target_os = "linux")]
fn installed_feed_withdrawal_preserves_published_sessions_workers_and_continue() {
    run_standalone_script("model_feed_product.py");
}

/// Runs only on request: five isolated one-candidate Plan publishes through the production
/// Local Control socket and Gateway installer. The opt-in fixture prints per-run wall time and
/// Debug stage counts; it is not a five-candidate or Desktop/WebView benchmark.
#[test]
#[ignore = "controlled #129 performance comparison"]
fn isolated_plan_publish_timing() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let storage = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("apps/desktop/tests/prepare_plan.py"))
        .arg(storage.path())
        .env("HIROUTE_PLAN_BENCHMARK_RUNS", "5")
        .env("HIROUTE_PLAN_BENCHMARK_EXPECT_ZERO_DECODE", "1")
        .output()
        .unwrap();
    println!("{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    if !output.status.success() {
        let retained = storage.keep();
        panic!(
            "isolated Plan benchmark failed; evidence: {}",
            retained.display()
        );
    }
}

#[test]
fn real_gateway_native_protocol_usage_populates_home_value() {
    run_script("usage_query_gateway_product.py");
}

fn run_script(script: &str) {
    run_script_case(script, None);
}

fn run_script_case(script: &str, scenario: Option<&str>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    if std::env::var_os("HIROUTE_VALIDATION_EXECUTION").is_some() {
        assert!(
            std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR").is_some_and(|path| {
                let directory = std::path::Path::new(&path);
                directory.join("hiroute").is_file() && directory.join("hirouted").is_file()
            }),
            "prepared publication binaries missing"
        );
    } else {
        let build = std::process::Command::new(env!("CARGO"))
            .current_dir(&root)
            .args(["build", "--locked", "-p", "hiroute-cli", "--bin", "hiroute"])
            .status()
            .unwrap();
        assert!(build.success());
    }
    let result = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("crates/daemon/tests/support").join(script))
        .arg(&root)
        .args(scenario)
        .status()
        .unwrap();
    assert!(result.success(), "production publication scenarios failed");
    assert!(
        !root
            .join("crates/daemon/tests/support/__pycache__")
            .exists(),
        "production publication scenarios must not dirty the candidate checkout"
    );
}

#[cfg(target_os = "linux")]
fn run_standalone_script(script: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let product_bin = if std::env::var_os("HIROUTE_VALIDATION_EXECUTION").is_some() {
        let directory = std::env::var_os("HIROUTE_VALIDATION_PRODUCT_BIN_DIR")
            .map(std::path::PathBuf::from)
            .expect("prepared standalone binaries missing");
        assert!(directory.join("hiroute").is_file());
        assert!(directory.join("hirouted").is_file());
        directory
    } else {
        // Cargo has already prepared the feature-qualified daemon for this integration target.
        // Rebuilding it without the target's features here races the parallel publication crash
        // scenarios and can replace their SUT. Only the cross-package CLI needs a nested build.
        let build = std::process::Command::new(env!("CARGO"))
            .current_dir(&root)
            .args(["build", "--locked", "-p", "hiroute-cli", "--bin", "hiroute"])
            .status()
            .unwrap();
        assert!(build.success());
        let daemon = std::path::Path::new(env!("CARGO_BIN_EXE_hirouted"));
        let directory = daemon
            .parent()
            .expect("Cargo daemon binary has a target directory")
            .to_path_buf();
        assert_eq!(directory.join("hirouted"), daemon);
        assert!(directory.join("hiroute").is_file());
        directory
    };
    let result = std::process::Command::new("python3")
        .arg("-B")
        .arg(root.join("crates/daemon/tests/support").join(script))
        .arg(&root)
        .env("HIROUTE_HEADLESS_PRODUCT_BIN_DIR", product_bin)
        .status()
        .unwrap();
    assert!(
        result.success(),
        "installed standalone headless scenario failed"
    );
    assert!(
        !root
            .join("crates/daemon/tests/support/__pycache__")
            .exists(),
        "standalone scenario must not dirty the candidate checkout"
    );
}
