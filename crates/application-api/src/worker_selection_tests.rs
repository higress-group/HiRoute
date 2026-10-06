//! Product contract: preserve saved adapter selections while accepting one native ACP CLI.
use super::*;
use hiroute_domain::{StoredOperationInputV1, TransactionPlanV1};

fn generated_schema(path: &str) -> serde_json::Value {
    let file = crate::generated_contract_files()
        .into_iter()
        .find(|file| file.relative_path == path)
        .unwrap();
    serde_json::from_str(&file.contents).unwrap()
}

// Resolve a harness form by its discriminator, rather than a conditional's nesting depth.
fn harness_rule<'a>(
    schema: &'a serde_json::Value,
    harness: &serde_json::Value,
) -> &'a serde_json::Value {
    let mut rule = schema;
    while let Some(condition) = rule.get("if") {
        let discriminator = &condition["properties"]["harness"];
        let selected = discriminator["const"] == *harness
            || discriminator["enum"]
                .as_array()
                .is_some_and(|values| values.contains(harness));
        rule = &rule[if selected { "then" } else { "else" }];
    }
    rule
}

#[test]
fn saved_adapter_selections_preserve_json_and_confirmation_identity() {
    let schema = generated_schema("worker-dependencies-select-request.v1.schema.json");
    // Frozen V1 payloads and independently calculated canonical SHA-256 values. The expected
    // side deliberately does not serialize the current producer or call its digest algorithm.
    let cases = [
        (
            r#"{"harness":"codex_cli","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client","expected_selection_revision":7}"#,
            r#"{"harness":"codex_cli","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client"}"#,
            "sha256:01e16fae616552635d9b77cb2dbe3272cc05e8a548fc3e95f71d671d053c3f69",
            "worker-dependency-selection:5b5b9a089fa008bfc2a91d1833610695992b5e2a6d079442c98c79172dbbd4d1",
        ),
        (
            r#"{"harness":"codex_cli","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client","node_path":"/opt/bin/node","expected_selection_revision":7}"#,
            r#"{"harness":"codex_cli","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client","node_path":"/opt/bin/node"}"#,
            "sha256:796b38a26770d894814caaef51c7f8ae4180830a78623bac9cf9954b0abb568b",
            "worker-dependency-selection:f7cffc9d7ed25312cc899bf79fcf3a871b902202509c2dd771debf574c5d03b1",
        ),
        (
            r#"{"harness":"claude_code","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client","expected_selection_revision":7}"#,
            r#"{"harness":"claude_code","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client"}"#,
            "sha256:6ccc36de24affb1e4fc380d05003bb4e033e627c0db7d1b56226e4e1409cbd91",
            "worker-dependency-selection:c2db1091d6add9a405689945edf59485ee171b6b3164069a4ace6730b90cba49",
        ),
        (
            r#"{"harness":"claude_code","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client","node_path":"/opt/bin/node","expected_selection_revision":7}"#,
            r#"{"harness":"claude_code","adapter_path":"/opt/acp/adapter.js","cli_path":"/opt/bin/client","node_path":"/opt/bin/node"}"#,
            "sha256:e48f651bab522d28ad7d62964cafed5e6ad9ea81348158ed0a84ae0dbdb12e34",
            "worker-dependency-selection:52cc6ea2e04b2ca05bbd1e18f058076187d023b0201fcc968d3eae42bbac358c",
        ),
    ];
    for (request_json, selection_json, accepted, idempotency) in cases {
        let request: WorkerDependenciesSelectRequestV1 =
            serde_json::from_str(request_json).unwrap();
        // Adapter-based clients keep the required adapter, regardless of native forms added.
        assert_eq!(
            harness_rule(&schema, &serde_json::to_value(request.harness).unwrap())["required"],
            serde_json::json!(["adapter_path"])
        );
        assert_eq!(serde_json::to_string(&request).unwrap(), request_json);
        let plan = plan_worker_dependency_selection(&request).unwrap();
        assert_eq!(
            serde_json::to_string(&plan.change.after_selection).unwrap(),
            selection_json
        );
        assert_eq!(plan.accept_digest.as_str(), accepted);
        assert_eq!(plan.idempotency_key, idempotency);
        let transaction =
            TransactionPlanV1::from_worker_dependency_selection_planner(plan.spec, plan.change)
                .unwrap();
        let stored = StoredOperationInputV1::freeze(&transaction).unwrap();
        assert_eq!(
            StoredOperationInputV1::freeze(&stored.build().unwrap()).unwrap(),
            stored
        );
    }
}

#[test]
fn native_acp_requires_only_cli_and_rejects_cross_form_fields() {
    for json in [
        r#"{"harness":"qoder_cli","cli_path":"/opt/bin/qodercli","expected_selection_revision":0}"#,
        r#"{"harness":"deepseek_harness","cli_path":"/opt/bin/dsh","expected_selection_revision":0}"#,
    ] {
        let request: WorkerDependenciesSelectRequestV1 = serde_json::from_str(json).unwrap();
        let schema = generated_schema("worker-dependencies-select-request.v1.schema.json");
        assert_eq!(
            schema["required"],
            serde_json::json!(["harness", "cli_path", "expected_selection_revision"])
        );
        assert_eq!(
            harness_rule(&schema, &serde_json::to_value(request.harness).unwrap())["properties"],
            serde_json::json!({"adapter_path":false,"node_path":false})
        );
        assert_eq!(serde_json::to_string(&request).unwrap(), json);
        let plan = plan_worker_dependency_selection(&request).unwrap();
        let hiroute_domain::delegation::WorkerLaunchFormV1::NativeAcp { cli } =
            plan.change.after_selection.validated_launch().unwrap()
        else {
            panic!("native ACP CLI")
        };
        assert_eq!(cli, request.cli_path);
        for (adapter, node) in [
            (Some("/opt/adapter".into()), None),
            (None, Some("/opt/node".into())),
        ] {
            let invalid = WorkerDependenciesSelectRequestV1 {
                adapter_path: adapter,
                node_path: node,
                ..request.clone()
            };
            assert!(plan_worker_dependency_selection(&invalid).is_none());
        }
        for harness in [WorkerHarnessV1::CodexCli, WorkerHarnessV1::ClaudeCode] {
            assert!(
                plan_worker_dependency_selection(&WorkerDependenciesSelectRequestV1 {
                    harness,
                    ..request.clone()
                })
                .is_none()
            );
        }
        let invalid = WorkerDependencySelectionRecordV1 {
            harness: request.harness,
            adapter_path: Some("/opt/adapter".into()),
            cli_path: "/opt/qoder".into(),
            node_path: None,
        };
        let mut forged = plan.change;
        forged.after_selection = invalid;
        let mut spec = plan.spec;
        spec.desired_state = serde_json::to_value(&forged.after_selection).unwrap();
        assert!(TransactionPlanV1::from_worker_dependency_selection_planner(spec, forged).is_err());
    }
}

#[test]
fn published_worker_harnesses_match_typed_discovery_selection_and_plan_bindings() {
    let discover = generated_schema("worker-dependencies-discover-request.v1.schema.json");
    let select = generated_schema("worker-dependencies-select-request.v1.schema.json");
    let editor = generated_schema("plan-editor.v2.schema.json");
    let expected = serde_json::json!([
        "codex_cli",
        "claude_code",
        "qoder_cli",
        "pi",
        "deepseek_harness"
    ]);
    for declared in [
        &discover["properties"]["harness"]["enum"],
        &select["properties"]["harness"]["enum"],
        &editor["$defs"]["editor"]["properties"]["work"]["properties"]["harness"]["enum"],
    ] {
        assert_eq!(declared, &expected);
    }
    for harness in expected.as_array().unwrap() {
        let discovery: WorkerDependenciesDiscoverRequestV1 =
            serde_json::from_value(serde_json::json!({"harness":harness})).unwrap();
        assert!(discovery.valid());
        assert_eq!(
            serde_json::to_value(discovery).unwrap()["harness"],
            *harness
        );
        let work: hiroute_domain::WorkerPlanV1 =
            serde_json::from_value(serde_json::json!({"harness":harness,"protocol":"responses"}))
                .unwrap();
        assert_eq!(serde_json::to_value(work).unwrap()["harness"], *harness);
    }
}
