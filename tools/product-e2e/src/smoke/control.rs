use super::{
    Result,
    build::Artifact,
    digest, fixtures, nonce,
    process::{Process, private_file},
    require,
    run::Context,
};
use hiroute_application_api::{
    CanonicalDigest, ClientHelloV1, LOCAL_CONTROL_SCHEMA_V2, LocalControlWireRequestV2,
    MACHINE_ENVELOPE_SCHEMA_V2, MachineEnvelopeV2, OperationIdempotencyLookupV1, PrincipalKind,
    ServerHelloV1,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    os::unix::{fs::FileTypeExt, process::ExitStatusExt},
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

fn command(binary: &Path, root: &Path) -> Command {
    let mut c = Command::new(binary);
    c.env_clear()
        .current_dir(root)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("tmp"))
        .env("PATH", "/usr/bin:/bin")
        .env("HIROUTE_RUNTIME_DIR", root.join("control-runtime"));
    c
}
fn prepare(ctx: &mut Context<'_>) -> Result<(Artifact, Artifact)> {
    ctx.enter("control_build");
    let cli = ctx.build("hiroute-cli", "hiroute")?;
    let daemon = ctx.build("hiroute-daemon", "hirouted")?;
    ctx.record(
        "control_build",
        Some(0),
        None,
        &json!({"cli":cli,"daemon":daemon}),
    )?;
    for dir in ["home", "config", "cache", "tmp", "control-runtime"] {
        fs::create_dir(ctx.runtime.join(dir))?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(ctx.runtime.join(dir), fs::Permissions::from_mode(0o700))?;
    }
    ctx.product_deadline();
    Ok((cli, daemon))
}
struct CliObservation {
    exit: Option<i32>,
    signal: Option<i32>,
    envelope: Value,
}

fn observe_cli(
    ctx: &mut Context<'_>,
    artifact: &Artifact,
    id: &'static str,
    args: &[&str],
    input: Option<&[u8]>,
    secret: &str,
) -> Result<CliObservation> {
    ctx.verify_artifact(artifact)?;
    let mut c = command(&artifact.path, &ctx.runtime);
    c.args(args).args([
        "--output",
        "json",
        "--non-interactive",
        "--request-id",
        &format!("{}-{id}", ctx.run_id),
    ]);
    let input = input
        .map(|bytes| -> Result<std::fs::File> {
            let path = ctx.private.join(format!("{id}.stdin"));
            let mut file = private_file(&path)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            Ok(std::fs::File::open(path)?)
        })
        .transpose()?;
    let mut process = match input {
        Some(input) => {
            Process::spawn_with_input(&mut c, &ctx.private.join(id), ctx.cancel.clone(), input)?
        }
        None => Process::spawn(&mut c, &ctx.private.join(id), ctx.cancel.clone())?,
    };
    let exit = process.wait(ctx.step_deadline(), 1024 * 1024)?;
    let (stdout, stderr) = process.output(1024 * 1024)?;
    require(
        stderr.is_empty()
            && stdout
                .split(|b| *b == b'\n')
                .filter(|l| !l.is_empty())
                .count()
                == 1,
        "invalid_machine_output",
    )?;
    let envelope: MachineEnvelopeV2<Value> = serde_json::from_slice(&stdout)?;
    require(
        envelope.schema_version == MACHINE_ENVELOPE_SCHEMA_V2
            && envelope.request_id.as_deref() == Some(&format!("{}-{id}", ctx.run_id)),
        "machine_identity_mismatch",
    )?;
    let encoded = String::from_utf8_lossy(&stdout);
    require(
        (secret.is_empty() || !encoded.contains(secret))
            && !encoded.contains(ctx.runtime.to_string_lossy().as_ref()),
        "privacy_failure",
    )?;
    ctx.verify_artifact(artifact)?;
    Ok(CliObservation {
        exit: exit.code(),
        signal: exit.signal(),
        envelope: serde_json::to_value(envelope)?,
    })
}

fn internal_call(
    ctx: &Context<'_>,
    endpoint: &Path,
    id: &str,
    operation_id: &str,
    payload: Value,
) -> Result<Value> {
    let mut stream = UnixStream::connect(endpoint)?;
    serde_json::to_writer(
        &mut stream,
        &ClientHelloV1 {
            api_version: LOCAL_CONTROL_SCHEMA_V2,
            machine_schema_version: MACHINE_ENVELOPE_SCHEMA_V2,
            client_name: "hiroute-smoke-internal".into(),
            client_version: hiroute_application_api::LOCAL_CONTROL_RELEASE_VERSION.into(),
        },
    )?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    serde_json::from_str::<ServerHelloV1>(&line)?;
    serde_json::to_writer(
        &mut stream,
        &LocalControlWireRequestV2 {
            schema_version: LOCAL_CONTROL_SCHEMA_V2,
            request_id: format!("{}-{id}-internal", ctx.run_id),
            operation_id: operation_id.into(),
            payload,
            protected_grant: None,
        },
    )?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    line.clear();
    reader.read_line(&mut line)?;
    Ok(serde_json::to_value(serde_json::from_str::<
        MachineEnvelopeV2<Value>,
    >(&line)?)?)
}

#[allow(clippy::too_many_arguments)]
fn planned_command(
    ctx: &mut Context<'_>,
    artifact: &Artifact,
    endpoint: &Path,
    id: &'static str,
    args: &[&str],
    input: Option<&[u8]>,
    secret: &str,
    operation_id: &str,
    payload: Value,
) -> Result<Value> {
    ctx.enter(id);
    let public = observe_cli(ctx, artifact, id, args, input, secret)?;
    require(
        public.exit == Some(2)
            && public.signal.is_none()
            && public.envelope["status"] == "usage_error"
            && public.envelope["error"]["code"] == "UNKNOWN_COMMAND",
        "planned_command_publicly_callable",
    )?;
    let internal = internal_call(ctx, endpoint, id, operation_id, payload)?;
    ctx.record(
        id,
        public.exit,
        public.signal,
        &json!({"public_lifecycle_gate":public.envelope,"internal_local_control":internal}),
    )?;
    Ok(internal)
}

#[allow(clippy::too_many_arguments)]
fn released_command(
    ctx: &mut Context<'_>,
    artifact: &Artifact,
    endpoint: &Path,
    id: &'static str,
    args: &[&str],
    input: Option<&[u8]>,
    secret: &str,
    operation_id: &str,
    payload: Value,
) -> Result<Value> {
    ctx.enter(id);
    let public = observe_cli(ctx, artifact, id, args, input, secret)?;
    require(
        public.exit == Some(0)
            && public.signal.is_none()
            && public.envelope["status"] == "succeeded"
            && public.envelope["error"].is_null(),
        "released_command_not_publicly_callable",
    )?;
    let internal = internal_call(ctx, endpoint, id, operation_id, payload)?;
    require(
        internal["status"] == "succeeded" && internal["error"].is_null(),
        "internal_local_control_failed",
    )?;
    ctx.record(
        id,
        public.exit,
        public.signal,
        &json!({"public_cli":public.envelope,"internal_local_control":internal}),
    )?;
    Ok(public.envelope)
}

pub(super) fn discover(ctx: &mut Context<'_>) -> Result<()> {
    let (cli_artifact, daemon) = prepare(ctx)?;
    let secret = nonce()?;
    let (home, bin) = fixtures::discovery(&ctx.runtime, &secret)?;
    let settings = home.join(".claude/settings.json");
    let original = fs::read(&settings)?;
    let endpoint = ctx.runtime.join("control-runtime/hiroute/control.sock");
    let mut c = command(&daemon.path, &ctx.runtime);
    c.args(["--role", "control", "--storage-root"])
        .arg(ctx.runtime.join("storage"))
        .arg("--runtime-root")
        .arg(ctx.runtime.join("control-runtime"))
        .args(["--diagnostic-level-override", "debug"])
        .env("HOME", &home)
        .env("PATH", &bin);
    ctx.enter("control_ready");
    ctx.verify_artifact(&daemon)?;
    let mut child = Process::spawn(&mut c, &ctx.private.join("daemon"), ctx.cancel.clone())?;
    let outcome: Result<()> = (|| {
        let deadline = ctx.step_deadline();
        loop {
            child.check()?;
            if fs::symlink_metadata(&endpoint).is_ok_and(|m| m.file_type().is_socket()) {
                break;
            }
            require(Instant::now() < deadline, "ready_timeout")?;
            std::thread::sleep(Duration::from_millis(20));
        }
        ctx.record(
            "control_ready",
            None,
            None,
            &json!({"child_pid":child.id(),"role":"control","executable_sha256":daemon.sha256}),
        )?;
        let status = released_command(
            ctx,
            &cli_artifact,
            &endpoint,
            "system_status",
            &["system", "status"],
            None,
            &secret,
            "GetSystemStatus",
            json!({}),
        )?;
        require(
            status["status"] == "succeeded"
                && status["data"]["daemon"] == "control_only"
                && status["data"]["gateway"] == "unavailable:not_composed",
            "control_status_mismatch",
        )?;
        let client_status = planned_command(
            ctx,
            &cli_artifact,
            &endpoint,
            "client_service_status",
            &["system", "client-status"],
            None,
            &secret,
            "GetClientServiceStatus",
            json!({}),
        )?;
        require(
            client_status["status"] == "succeeded"
                && client_status["data"]["schema"] == "hiroute.client-service-status/v1"
                && client_status["data"]["daemon_role"] == "control_only"
                && client_status["data"]["recovery_ready"] == true
                && client_status["data"]["mutation_available"] == false
                && client_status["data"]["gateway"] == "not_composed"
                && client_status["data"]["active_publication"].is_null(),
            "client_service_status_mismatch",
        )?;
        let lookup = OperationIdempotencyLookupV1 {
            principal_kind: PrincipalKind::InteractiveUser,
            operation_kind: "ApplyAgentPlanChange".into(),
            idempotency_key: format!("smoke-lookup-{}", &ctx.run_id[..16]),
            accepted_digest: CanonicalDigest::of_bytes(
                format!("mvp17-lookup-{}", ctx.run_id).as_bytes(),
            ),
        };
        let lookup_payload = serde_json::to_value(&lookup)?;
        let lookup_input = serde_json::to_vec(&lookup)?;
        let lookup = released_command(
            ctx,
            &cli_artifact,
            &endpoint,
            "operation_idempotency_lookup",
            &["operations", "find", "--request-stdin"],
            Some(&lookup_input),
            &secret,
            "FindOperationByIdempotency",
            lookup_payload,
        )?;
        require(
            lookup["status"] == "succeeded"
                && lookup["data"]["operation"].is_null()
                && lookup["data"]["digest_matches"] == true,
            "idempotency_lookup_mismatch",
        )?;
        for (step, verb, operation) in [
            ("agents_scan", "scan", "ScanAgents"),
            ("agents_list", "list", "ListAgents"),
        ] {
            child.check()?;
            let result = released_command(
                ctx,
                &cli_artifact,
                &endpoint,
                step,
                &["agents", verb],
                None,
                &secret,
                operation,
                json!({}),
            )?;
            let agents = result["data"]["agents"]
                .as_array()
                .ok_or(super::SmokeError("agent_discovery_missing"))?;
            require(
                result["status"] == "succeeded"
                    && agents.len() == 2
                    && agents.iter().any(|a| {
                        a["agent_id"] == "agent_claude_default"
                            && a["supported"] == true
                            && a["registered_configuration"]["connection_option_id"]
                                == "zhipu.coding-plan.cn.v1"
                    }),
                "agent_discovery_mismatch",
            )?;
        }
        require(
            fs::read(&settings)? == original,
            "synthetic_configuration_changed",
        )?;
        Ok(())
    })();
    let cleanup = child.stop();
    outcome?;
    cleanup?;
    ctx.enter("control_cleanup");
    ctx.verify_artifact(&daemon)?;
    let (_, stderr) = child.output(1024 * 1024)?;
    require(
        !String::from_utf8_lossy(&stderr).contains(&secret),
        "privacy_failure",
    )?;
    ctx.record(
        "control_cleanup",
        None,
        None,
        &json!({"reaped":true,"initial_configuration_digest":digest(&original)}),
    )
}

pub(super) fn embedded_catalog(ctx: &mut Context<'_>) -> Result<()> {
    let (cli_artifact, daemon) = prepare(ctx)?;
    let endpoint = ctx.runtime.join("control-runtime/hiroute/control.sock");
    let mut c = command(&daemon.path, &ctx.runtime);
    c.args(["--role", "control", "--storage-root"])
        .arg(ctx.runtime.join("storage"))
        .arg("--runtime-root")
        .arg(ctx.runtime.join("control-runtime"))
        .args(["--diagnostic-level-override", "debug"]);
    fixtures::initialize_current_storage(
        &daemon.path,
        &ctx.runtime.join("storage"),
        &ctx.private.join("embedded-catalog-initialize"),
        ctx.cancel.clone(),
        ctx.step_deadline(),
    )?;
    fixtures::install_storage_catalog_tampering(&ctx.runtime.join("storage"))?;
    ctx.enter("embedded_catalog_ready");
    ctx.verify_artifact(&daemon)?;
    let mut child = Process::spawn(
        &mut c,
        &ctx.private.join("embedded-catalog-daemon"),
        ctx.cancel.clone(),
    )?;
    let outcome: Result<()> = (|| {
        let deadline = ctx.step_deadline();
        loop {
            child.check()?;
            if fs::symlink_metadata(&endpoint).is_ok_and(|m| m.file_type().is_socket()) {
                break;
            }
            require(Instant::now() < deadline, "ready_timeout")?;
            std::thread::sleep(Duration::from_millis(20));
        }
        ctx.record(
            "embedded_catalog_ready",
            None,
            None,
            &json!({"child_pid":child.id(),"catalog_source":"daemon_embedded_current"}),
        )?;
        let result = released_command(
            ctx,
            &cli_artifact,
            &endpoint,
            "storage_catalog_ignored",
            &["compute", "scan"],
            None,
            "",
            "ScanCompute",
            json!({}),
        )?;
        require(
            result["status"] == "succeeded"
                && fs::read(
                    ctx.runtime
                        .join("storage/release-facts/current/manifest.json"),
                )? == b"{\"schema\":\"untrusted\"}\n",
            "embedded_catalog_not_proven",
        )?;
        Ok(())
    })();
    let cleanup = child.stop();
    outcome?;
    cleanup?;
    let (_, stderr) = child.output(1024 * 1024)?;
    require(stderr.is_empty(), "unexpected_daemon_stderr")?;
    ctx.enter("embedded_catalog_cleanup");
    ctx.record(
        "embedded_catalog_cleanup",
        None,
        None,
        &json!({"reaped":true,"mutable_storage_catalog_ignored":true}),
    )?;
    ctx.verify_artifact(&daemon)
}
