use super::*;

pub(super) fn prepare(
    client: &LocalControlClient,
    command: &str,
    options: &[String],
) -> Result<Value> {
    match command {
        "routing.preview" => routing(client, options),
        "compute.connection.preview" => compute(client, options),
        "decision.services.apply" => super::decision::prepare(client, options),
        "decision.services.test" => super::decision::test(client, options),
        "decision.connections.apply" => super::decision_v2::prepare(client, options),
        "decision.connections.test" => super::decision_v2::test(client, options),
        "agents.connect.preview" | "agents.restore.preview" => {
            settings::prepare(client, command, options)
        }
        _ => Err(error(ErrorCode::InvalidArguments)),
    }
}

fn routing(client: &LocalControlClient, options: &[String]) -> Result<Value> {
    let flags = Flags::parse(
        options,
        &[
            "--plan",
            "--name",
            "--purpose",
            "--mode",
            "--decision",
            "--decision-service",
            "--restore-from",
        ],
        &["--model", "--economy", "--primary"],
        &[],
    )?;
    if let Some(path) = flags.get("--restore-from") {
        // A complete baseline is an alternative to editing individual choices.
        if options.len() != 4 {
            return Err(error(ErrorCode::InvalidArguments));
        }
        return super::plan_restore::prepare(client, flags.required("--plan")?, path);
    }
    if flags.has("--decision") && flags.has("--decision-service") {
        return Err(error(ErrorCode::InvalidArguments));
    }
    if !flags.has("--plan") {
        flags.required("--name")?;
        flags.required("--purpose")?;
        flags.required("--mode")?;
    }
    if flags
        .get("--mode")
        .is_some_and(|v| !matches!(v, "fixed_model" | "smart_saving"))
        || flags.get("--decision").is_some_and(|v| v != "local_rules")
    {
        return Err(error(ErrorCode::InvalidArguments));
    }
    let mut authoring = json!({});
    for (flag, field) in [
        ("--plan", "plan_id"),
        ("--name", "display_name"),
        ("--purpose", "purpose"),
        ("--mode", "mode"),
    ] {
        if let Some(value) = flags.get(flag) {
            authoring[field] = json!(value);
        }
    }
    for (flag, field) in [
        ("--model", "candidates"),
        ("--economy", "economy"),
        ("--primary", "primary"),
    ] {
        if flags.has(flag) {
            authoring[field] = json!(
                flags
                    .all(flag)
                    .iter()
                    .map(|binding| json!({"binding_id":binding,"reasoning":null}))
                    .collect::<Vec<_>>()
            );
        }
    }
    if flags.has("--decision") {
        authoring["classifier"] = json!({"kind":"local_rules"});
    }
    if let Some(selected) = flags.get("--decision-service") {
        let (id, revision) = selected
            .rsplit_once('@')
            .ok_or_else(|| choices(&["--decision-service ID@REVISION"], json!({})))?;
        let revision = revision
            .parse::<u64>()
            .map_err(|_| error(ErrorCode::InvalidArguments))?;
        let services = data(client, "ListDecisionServicesV2", json!({}))
            .or_else(|_| data(client, "ListDecisionServices", json!({})))?;
        let matches = services["services"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::Internal))?
            .iter()
            .map(|row| row.get("service").unwrap_or(row))
            .filter(|service| service["id"] == id && service["revision"] == revision)
            .collect::<Vec<_>>();
        let [service] = matches.as_slice() else {
            return Err(choices(&["--decision-service"], services));
        };
        let kind = if service["schema"] == "hiroute.decision-service/v2" {
            "decision_service_v2"
        } else {
            "decision_service"
        };
        authoring["classifier"] = json!({"kind":kind,"service":service});
    }
    let authoring: PlanAuthoringRequestV1 =
        serde_json::from_value(authoring).map_err(|_| error(ErrorCode::InvalidArguments))?;
    let options = data(
        client,
        "GetPlanEditorOptions",
        json!({"authoring":authoring}),
    )?;
    let prepared: PlanAuthoringOptionsV1 = serde_json::from_value(options["authoring"].clone())
        .map_err(|_| {
            choices(
                &["server_supporting_shared_authoring"],
                json!({"action":"update_hiroute"}),
            )
        })?;
    if !prepared.required_choices.is_empty() {
        return Err(choices(&["authoring_choices"], json!(prepared)));
    }
    let mode =
        serde_json::to_value(prepared.editor.mode).map_err(|_| error(ErrorCode::Internal))?;
    if mode == "fixed_model"
        && (flags.has("--economy")
            || flags.has("--primary")
            || flags.has("--decision")
            || flags.has("--decision-service"))
        || mode == "smart_saving" && flags.has("--model")
        || mode != "fixed_model" && mode != "smart_saving"
    {
        return Err(choices(
            &["mode_compatible_choices_or_advanced_request"],
            json!({"mode":mode}),
        ));
    }
    let target = match prepared.plan_id {
        Some(plan_id) => PlanContentTargetV2::Update {
            plan_id,
            expected_head_revision: prepared
                .expected_head_revision
                .ok_or_else(|| error(ErrorCode::Internal))?,
        },
        None => PlanContentTargetV2::Create {
            creation_key: fresh_key()?,
        },
    };
    Ok(json!(PlanContentPreviewRequestV2 {
        change: PlanContentChangeV2 {
            schema: PLAN_CONTENT_CHANGE_SCHEMA_V2.into(),
            target,
            editor: prepared.editor,
            consumed_draft: None,
        }
    }))
}

fn compute(client: &LocalControlClient, options: &[String]) -> Result<Value> {
    let flags = Flags::parse(
        options,
        &["--candidate", "--candidate-revision"],
        &["--model-ref"],
        &["--all-selectable"],
    )?;
    if flags.has("--all-selectable") == flags.has("--model-ref") {
        return Err(choices(&["--all-selectable OR --model-ref"], json!({})));
    }
    let reference = ComputeCandidateRefV2 {
        candidate_ref: flags.required("--candidate")?.into(),
        candidate_revision: flags
            .required("--candidate-revision")?
            .parse()
            .map_err(|_| error(ErrorCode::InvalidArguments))?,
    };
    reference
        .validate_shape()
        .map_err(|_| error(ErrorCode::InvalidArguments))?;
    let options = data(
        client,
        "ListConnectionOptions",
        json!({"candidate":reference}),
    )?;
    let candidate: ComputeCandidateViewV2 = serde_json::from_value(options["candidate"].clone())
        .map_err(|_| error(ErrorCode::Internal))?;
    let selectable = candidate
        .models
        .iter()
        .filter(|model| model.selectable)
        .map(|model| model.model_ref.clone())
        .collect::<Vec<_>>();
    let selected = if flags.has("--all-selectable") {
        selectable.clone()
    } else {
        flags.all("--model-ref").to_vec()
    };
    if selected.is_empty()
        || selected.iter().any(|model| !selectable.contains(model))
        || selected
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != selected.len()
    {
        return Err(choices(
            &["nonempty_selectable_models"],
            json!({"candidate":candidate.candidate,"models":candidate.models}),
        ));
    }
    let snapshot = data(client, "ListCompute", json!({}))?;
    let request = json!({"change": {
        "schema":COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2,
        "subject":{"kind":"candidate","candidate":candidate.candidate},
        "expected_revisions":snapshot["revisions"], "selected_model_refs":selected,
        "intent":"save_ready", "validation":candidate.validation
    }});
    serde_json::from_value::<ComputeSavePreviewRequestV2>(request.clone())
        .map_err(|_| error(ErrorCode::Internal))?;
    Ok(request)
}
