use super::*;
#[cfg(not(windows))]
use std::io::Write;
use std::path::Path;

pub(super) fn prepare(
    command: &str,
    payload: &Value,
    preview: Value,
    path: &str,
    key: Option<String>,
) -> Result<Value> {
    let key = key.map_or_else(fresh_key, Ok)?;
    let (request, apply_command, operation_kind) = request(command, payload, &preview, &key)?;
    let lookup = OperationIdempotencyLookupV1 {
        principal_kind: PrincipalKind::InteractiveUser,
        operation_kind: operation_kind.into(),
        idempotency_key: key,
        accepted_digest: serde_json::from_value(request["accept_digest"].clone())
            .map_err(|_| error(ErrorCode::Internal))?,
        include_model_budget_conflict: false,
    };
    write_new(Path::new(path), &request).map_err(|_| {
        choices(
            &["new_writable_private_apply_file"],
            json!({"path":path,"preview":preview}),
        )
    })?;
    let arguments = apply_command
        .split('.')
        .chain(["--request-file", path])
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok(json!(CliPreparedPreviewV1 {
        schema: "hiroute.cli-prepared-preview/v1".into(),
        preview,
        submission: CliPreparedSubmissionV1 {
            apply_file: path.into(),
            command_id: apply_command.into(),
            arguments,
            operation_lookup: lookup,
            state: "prepared_not_submitted".into(),
        },
    }))
}

fn request(
    command: &str,
    payload: &Value,
    preview: &Value,
    key: &str,
) -> Result<(Value, &'static str, &'static str)> {
    let internal = |_| error(ErrorCode::Internal);
    match command {
        "decision.services.apply" | "decision.connections.apply" => {
            let result: PreviewResultV1 =
                serde_json::from_value(preview.clone()).map_err(internal)?;
            if !result.blockers.is_empty() {
                return Err(choices(&["resolve_preview_blockers"], preview.clone()));
            }
            Ok((
                json!(ApplyRequestV1 {
                    schema_version: CHANGE_SPEC_SCHEMA_V1,
                    spec: result.normalized_spec,
                    accept_digest: result.change_digest,
                    expected_revisions: result.expected_revisions,
                    idempotency_key: key.into(),
                    apply_capability: None,
                }),
                if command == "decision.services.apply" {
                    "decision.services.apply"
                } else {
                    "decision.connections.apply"
                },
                if command == "decision.services.apply" {
                    "ApplyDecisionService"
                } else {
                    "ApplyDecisionServiceV2"
                },
            ))
        }
        "routing.preview" if payload["change"]["schema"] == PLAN_CONTENT_CHANGE_SCHEMA_V2 => {
            let input: PlanContentPreviewRequestV2 =
                serde_json::from_value(payload.clone()).map_err(internal)?;
            let result: PlanContentPreviewV2 =
                serde_json::from_value(preview.clone()).map_err(internal)?;
            Ok((
                json!(PlanContentApplyRequestV2 {
                    change: input.change,
                    accept_digest: result.change_digest,
                    expected_revisions: result.expected_revisions,
                    idempotency_key: key.into(),
                    include_model_budget_conflict: false,
                }),
                "routing.apply",
                "ApplyAgentPlanChange",
            ))
        }
        "compute.connection.preview" => {
            let (spec, digest, revisions, kind) =
                if payload["change"]["schema"] == COMPUTE_MANAGEMENT_CHANGE_SCHEMA_V2 {
                    let result: ComputeSavePreviewV2 =
                        serde_json::from_value(preview.clone()).map_err(internal)?;
                    (
                        result.spec,
                        result.accept_digest,
                        result.expected_revisions,
                        APPLY_COMPUTE_SAVE_OPERATION_V2,
                    )
                } else {
                    let result: ComputeConnectionPreviewResultV1 =
                        serde_json::from_value(preview.clone()).map_err(internal)?;
                    (
                        result.spec,
                        result.change_digest,
                        result.expected_revisions,
                        "ApplyComputeConnectionChange",
                    )
                };
            Ok((
                json!(ComputeConnectionApplyRequestV1 {
                    spec,
                    accept_digest: digest,
                    expected_revisions: revisions,
                    idempotency_key: key.into(),
                }),
                "compute.connection.apply",
                kind,
            ))
        }
        "agents.connect.preview" | "agents.restore.preview"
            if payload["spec"]["schema_version"] == json!(AGENT_SETTINGS_SCHEMA_V2) =>
        {
            let result: AgentSettingsPreviewV2 =
                serde_json::from_value(preview.clone()).map_err(internal)?;
            if !result.applicable || !result.preview.blockers.is_empty() {
                return Err(choices(&["resolve_preview_blockers"], preview.clone()));
            }
            if result.resident_service.login_item_required
                || result.resident_service.login_item_removal_required
            {
                return Err(choices(
                    &["desktop_resident_service_confirmation"],
                    preview.clone(),
                ));
            }
            let restore = command == "agents.restore.preview";
            Ok((
                json!(AgentSettingsApplyV2 {
                    expected_revisions: result.expected_revisions,
                    spec: result.preview.spec,
                    accept_digest: result.preview.accept_digest,
                    dependency_digest: result.preview.dependency_digest,
                    idempotency_key: key.into(),
                    login_item: None,
                }),
                if restore {
                    "agents.restore.apply"
                } else {
                    "agents.connect.apply"
                },
                if restore {
                    "ApplyAgentConnectionRestore"
                } else {
                    "ApplyAgentConnectionChange"
                },
            ))
        }
        _ => Err(choices(
            &["supported_apply_file_variant"],
            json!({"supported":["compute_save","plan_content_v2","agent_settings_v2","decision_service"],"advanced":"use_installed_typed_request_contract"}),
        )),
    }
}

/// The file appears at its final name only after a complete, synced write. Noclobber rejects
/// existing regular files and symlinks, including a concurrent creator. No parent is created.
fn write_new(path: &Path, value: &Value) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let mut bytes = serde_json::to_vec_pretty(value)?;
        bytes.push(b'\n');
        hiroute_platform::private_fs::write_material(
            path,
            &bytes,
            hiroute_platform::private_fs::FileMode::Data,
        )
    }
    #[cfg(not(windows))]
    {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if path.file_name().is_none() || path.symlink_metadata().is_ok() {
            return Err(std::io::ErrorKind::AlreadyExists.into());
        }
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist_noclobber(path).map_err(|error| error.error)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_request_is_private_and_never_overwrites_or_leaves_partial_destination() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("apply.json");
        let value = json!({"idempotency_key":"stable-key","spec":{"choice":"exact"}});
        write_new(&path, &value).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
            value
        );
        #[cfg(windows)]
        assert!(hiroute_platform::private_fs::open_private_file(&path).is_ok());
        assert!(write_new(&path, &json!({})).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
            value
        );
        assert!(write_new(&root.path().join("missing/apply.json"), &value).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            let link = root.path().join("link.json");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(write_new(&link, &json!({})).is_err());
        }
    }
}
