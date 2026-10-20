//! OpenCode native ACP leaf; the shared executor owns authority, transport and process cleanup.
use super::*;
use std::collections::BTreeSet;

pub(super) const MODE: &str = "hiroute-worker";

/// Final admission runs under the existing initialization permit. In particular,
/// another first Worker may still be creating this native database during rendering.
pub(super) struct StartupAdmission {
    native: hiroute_integrations::OpenCodeNativeContext,
    config_digest: hiroute_domain::CanonicalDigest,
    migrations: &'static [&'static str],
    search_path: String,
}

impl StartupAdmission {
    pub(super) fn root(&self) -> &Path {
        &self.native.database
    }

    pub(super) fn validate(
        &self,
        workspace: &Path,
        new_session: bool,
    ) -> Result<(), DelegationErrorV1> {
        let unavailable = || DelegationErrorV1::CapabilityUnavailable;
        if !new_session
            && !self
                .native
                .database
                .try_exists()
                .map_err(|_| unavailable())?
        {
            return Err(DelegationErrorV1::ResumeUnavailable);
        }
        validate_database(&self.native.database, self.migrations)?;
        let config = self
            .native
            .read_config(Some((workspace, &self.search_path)))
            .map_err(|_| unavailable())?;
        if config.digest != self.config_digest {
            return Err(unavailable());
        }
        hiroute_integrations::validate_opencode_composition(
            &self.native,
            &config,
            workspace,
            &self.search_path,
        )
        .map_err(|_| unavailable())
    }
}

pub(super) fn render(
    input: &ProfileInput<'_>,
    search_path: &str,
) -> Result<RenderedHarnessProfile, DelegationErrorV1> {
    let unavailable = || DelegationErrorV1::CapabilityUnavailable;
    let native = input.native_context.opencode().ok_or_else(unavailable)?;
    let ids = hiroute_integrations::opencode_native_format(input.harness_binary)
        .map_err(|_| unavailable())?;
    let config = native
        .read_config(Some((input.workspace, search_path)))
        .map_err(|_| unavailable())?;
    let task = input
        .session_root
        .path()
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(unavailable)?;
    let provider = format!("hiroute-worker-{task}");
    if config.value["provider"].get(&provider).is_some()
        || config.value["agent"].get(MODE).is_some()
    {
        return Err(unavailable());
    }
    let model = hiroute_domain::AdditionalAgentModelV1 {
        alias: input.alias.into(),
        protocol: input.protocol,
        context_window_tokens: input.context_window_tokens.ok_or_else(unavailable)?,
        max_output_tokens: input.max_output_tokens.ok_or_else(unavailable)?,
    };
    let mut declaration = hiroute_integrations::opencode_provider(
        &format!("http://{}/v1", input.gateway),
        &model,
        "{env:HIROUTE_RUN_TOKEN}",
        None,
    )
    .map_err(|_| unavailable())?;
    declaration["options"]["headers"] =
        json!({"X-HiRoute-Agent":"opencode","X-HiRoute-Request-Purpose":"main"});
    let auxiliary_provider = format!("{provider}-auxiliary");
    let mut auxiliary = declaration.clone();
    auxiliary["options"]["headers"]["X-HiRoute-Request-Purpose"] = json!("auxiliary");
    let auxiliary_selector = format!("{auxiliary_provider}/{}", input.alias);
    let selector = format!("{provider}/{}", input.alias);
    let permission = json!({"*":"deny","read":"allow","edit":"allow","bash":"allow","glob":"allow","grep":"allow","skill":"allow","external_directory":"allow","doom_loop":"allow"});
    let mut agents = serde_json::Map::new();
    for key in ["build", "plan", "general", "explore"].into_iter().chain(
        config.value["agent"]
            .as_object()
            .into_iter()
            .flat_map(|v| v.keys().map(String::as_str)),
    ) {
        agents.insert(key.into(), json!({"disable":true}));
    }
    for key in ["title", "summary", "compaction"] {
        agents.insert(
            key.into(),
            json!({"model":auxiliary_selector,"disable":false,"permission":{"*":"deny"}}),
        );
    }
    agents.insert(
        MODE.into(),
        json!({"mode":"primary","model":selector,"permission":permission}),
    );
    let mcp = config.value["mcp"]
        .as_object()
        .into_iter()
        .flat_map(|v| v.keys())
        .map(|k| (k.clone(), json!({"enabled":false})))
        .collect::<serde_json::Map<_, _>>();
    let overlay = json!({"$schema":"https://opencode.ai/config.json","model":selector,"small_model":auxiliary_selector,"default_agent":MODE,
        "enabled_providers":[provider,auxiliary_provider],"provider":{(provider):declaration,(auxiliary_provider):auxiliary},"agent":agents,"permission":permission,"mcp":mcp,
        "share":"disabled","autoupdate":false,"lsp":false,"formatter":false,"compaction":{"auto":true}});
    let mut listener_entropy = [0u8; 32];
    getrandom::fill(&mut listener_entropy).map_err(|_| unavailable())?;
    let listener_password = Zeroizing::new(
        listener_entropy
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    );
    use zeroize::Zeroize;
    listener_entropy.zeroize();
    let mut rendered = RenderedHarnessProfile {
        opencode_startup: Some(StartupAdmission {
            native: native.clone(),
            config_digest: config.digest.clone(),
            migrations: ids,
            search_path: search_path.into(),
        }),
        native_model_id: selector,
        native_args: vec![
            "acp".into(),
            "--cwd".into(),
            input.workspace.into(),
            "--hostname".into(),
            "127.0.0.1".into(),
            "--port".into(),
            "0".into(),
            "--mdns=false".into(),
        ],
        ..Default::default()
    };
    for (key, value) in [
        (
            "OPENCODE_CONFIG_CONTENT",
            serde_json::to_string(&overlay).map_err(|_| unavailable())?,
        ),
        ("HIROUTE_RUN_TOKEN", secret_string(&input.token)?),
        ("OPENCODE_SERVER_PASSWORD", listener_password.to_string()),
        ("OPENCODE_SERVER_USERNAME", "hiroute-worker".into()),
        ("OPENCODE_PURE", "true".into()),
        ("OPENCODE_DISABLE_DEFAULT_PLUGINS", "true".into()),
        ("OPENCODE_DISABLE_AUTOUPDATE", "true".into()),
        ("OPENCODE_DISABLE_MODELS_FETCH", "true".into()),
        ("OPENCODE_DISABLE_LSP_DOWNLOAD", "true".into()),
        ("OPENCODE_DISABLE_FFF", "true".into()),
        (
            "OPENCODE_PERMISSION",
            serde_json::to_string(&permission).map_err(|_| unavailable())?,
        ),
        ("OPENCODE_DB", path_string(&native.database)?),
        (
            "XDG_CONFIG_HOME",
            path_string(native.config_root.parent().ok_or_else(unavailable)?)?,
        ),
        (
            "XDG_DATA_HOME",
            path_string(native.data_root.parent().ok_or_else(unavailable)?)?,
        ),
        (
            "XDG_CACHE_HOME",
            path_string(native.cache_root.parent().ok_or_else(unavailable)?)?,
        ),
        (
            "XDG_STATE_HOME",
            path_string(native.state_root.parent().ok_or_else(unavailable)?)?,
        ),
    ] {
        rendered.env.insert(key.into(), Zeroizing::new(value));
    }
    for (key, path) in [
        ("OPENCODE_CONFIG", native.config_file.as_ref()),
        ("OPENCODE_CONFIG_DIR", native.extra_config_root.as_ref()),
    ] {
        if let Some(path) = path {
            rendered
                .env
                .insert(key.into(), Zeroizing::new(path_string(path)?));
        }
    }
    Ok(rendered)
}

fn validate_database(path: &Path, expected: &[&str]) -> Result<(), DelegationErrorV1> {
    let unavailable = || DelegationErrorV1::CapabilityUnavailable;
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {}
        _ => return Err(unavailable()),
    }
    let db = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| unavailable())?;
    db.busy_timeout(std::time::Duration::from_millis(100))
        .map_err(|_| unavailable())?;
    let kind: String = db
        .query_row(
            "SELECT type FROM sqlite_schema WHERE name='migration'",
            [],
            |r| r.get(0),
        )
        .map_err(|_| unavailable())?;
    if kind != "table" {
        return Err(unavailable());
    }
    let mut query = db
        .prepare(
            "SELECT CASE WHEN length(CAST(id AS BLOB)) <= 256 THEN id END FROM migration LIMIT 129",
        )
        .map_err(|_| unavailable())?;
    let rows = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| unavailable())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| unavailable())?;
    if rows.len() > 128 || rows.iter().any(|x| x.len() > 256) {
        return Err(unavailable());
    }
    let actual = rows.iter().map(String::as_str).collect::<BTreeSet<_>>();
    if actual.len() != rows.len() || actual != expected.iter().copied().collect() {
        return Err(unavailable());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_workspace(home: &Path) -> String {
        let path = worker_path(&home.join("opencode"), None, None).unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .arg(home)
            .env_clear()
            .env("HOME", home)
            .env("PATH", &path)
            .status()
            .unwrap();
        assert!(status.success());
        path
    }

    #[test]
    fn opencode_startup_rejects_rewritable_schema_without_touching_borrowed_files() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let native =
            hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap();
        std::fs::create_dir_all(&native.config_root).unwrap();
        let search_path = git_workspace(&home);
        for schema in [
            "",
            "{env:HIROUTE_UNSET_SCHEMA}",
            "{file:schema.txt}",
            "https://opencode.ai/config.json",
        ] {
            let bytes = serde_json::to_vec(&json!({"$schema":schema})).unwrap();
            std::fs::write(native.model_target(), &bytes).unwrap();
            let config = native.read_config(Some((&home, &search_path))).unwrap();
            let admission = StartupAdmission {
                native: native.clone(),
                config_digest: config.digest.clone(),
                migrations: &["known"],
                search_path: search_path.clone(),
            };
            assert_eq!(
                admission.validate(&home, true).is_ok(),
                schema == "https://opencode.ai/config.json",
                "{schema}"
            );
            assert_eq!(std::fs::read(native.model_target()).unwrap(), bytes);
            assert!(!native.database.exists());
        }
    }

    #[test]
    fn opencode_startup_refuses_incomplete_ancestry_before_native_tool_loading() {
        for git in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let home = root.path().canonicalize().unwrap();
            let project = home.join("project");
            std::fs::create_dir_all(project.join(".opencode/tools")).unwrap();
            if git {
                std::fs::create_dir(project.join(".git")).unwrap();
            }
            let tool = project.join(".opencode/tools/custom.ts");
            let content = b"throw new Error('must never be imported')";
            std::fs::write(&tool, content).unwrap();
            let native =
                hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap();
            let search_path = git_workspace(&home);
            let config = native.read_config(Some((&home, &search_path))).unwrap();
            let admission = StartupAdmission {
                native: native.clone(),
                config_digest: config.digest.clone(),
                migrations: &["known"],
                search_path: search_path.clone(),
            };
            let deep = (0..65).fold(project, |path, _| path.join("d"));
            std::fs::create_dir_all(&deep).unwrap();
            assert_eq!(
                admission.validate(&deep, true),
                Err(DelegationErrorV1::CapabilityUnavailable)
            );
            assert_eq!(std::fs::read(tool).unwrap(), content);
            assert!(!native.database.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn opencode_startup_refuses_false_git_stops_and_rechecks_late_boundary_changes() {
        for kind in [
            "empty",
            "corrupt",
            "broken-worktree",
            "missing-git",
            "rejected-git",
            "late-corruption",
        ] {
            let root = tempfile::tempdir().unwrap();
            let base = root.path().canonicalize().unwrap();
            let home = base.join("home");
            let ancestor = base.join("ancestor");
            let project = ancestor.join("project");
            std::fs::create_dir(&home).unwrap();
            std::fs::create_dir_all(&project).unwrap();
            let native =
                hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap();
            let path = worker_path(&home.join("opencode"), None, None).unwrap();
            let marker = project.join(".git");
            if matches!(kind, "missing-git" | "rejected-git" | "late-corruption") {
                let status = std::process::Command::new("git")
                    .args(["init", "-q"])
                    .arg(&project)
                    .env_clear()
                    .env("HOME", &home)
                    .env("PATH", &path)
                    .status()
                    .unwrap();
                assert!(status.success());
            } else if kind == "empty" {
                std::fs::create_dir(&marker).unwrap();
            } else {
                std::fs::write(
                    &marker,
                    if kind == "corrupt" {
                        "not a gitdir"
                    } else {
                        "gitdir: missing-worktree\n"
                    },
                )
                .unwrap();
            }
            let config_bytes = br#"{"$schema":""}"#;
            let config_path = ancestor.join("opencode.json");
            std::fs::write(&config_path, config_bytes).unwrap();
            std::fs::create_dir_all(ancestor.join(".opencode/tools")).unwrap();
            let tool = ancestor.join(".opencode/tools/custom.ts");
            let tool_bytes = b"throw new Error('must never be imported')";
            std::fs::write(&tool, tool_bytes).unwrap();
            let config = native.read_config(None).unwrap();
            let mut admission = StartupAdmission {
                native: native.clone(),
                config_digest: config.digest.clone(),
                migrations: &["known"],
                search_path: path.clone(),
            };
            if kind == "late-corruption" {
                assert!(admission.validate(&project, true).is_ok());
                std::fs::write(marker.join("config"), b"[invalid git config").unwrap();
            }
            if kind == "missing-git" {
                admission.search_path = home.to_str().unwrap().into();
            }
            if kind == "rejected-git" {
                // A real Git rejection, without changing process-global test environment.
                std::fs::write(marker.join("config"), b"[invalid git config").unwrap();
            }
            assert_eq!(
                admission.validate(&project, true),
                Err(DelegationErrorV1::CapabilityUnavailable),
                "{kind}"
            );
            assert!(
                native
                    .read_config(Some((&project, &admission.search_path)))
                    .is_err(),
                "{kind}"
            );
            assert_eq!(std::fs::read(&config_path).unwrap(), config_bytes);
            assert_eq!(std::fs::read(&tool).unwrap(), tool_bytes);
            assert!(!native.database.exists());
        }
    }

    #[test]
    fn opencode_startup_rechecks_configuration_and_missing_continue_history() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let native =
            hiroute_integrations::OpenCodeNativeContext::from_lookup(&home, |_| None).unwrap();
        let search_path = git_workspace(&home);
        let config = native.read_config(Some((&home, &search_path))).unwrap();
        let admission = StartupAdmission {
            native: native.clone(),
            config_digest: config.digest.clone(),
            migrations: &["known"],
            search_path: search_path.clone(),
        };
        assert_eq!(admission.root(), native.database);
        assert!(admission.validate(&home, true).is_ok());
        assert_eq!(
            admission.validate(&home, false),
            Err(DelegationErrorV1::ResumeUnavailable)
        );
        assert!(!native.database.exists());

        std::fs::create_dir_all(&native.config_root).unwrap();
        std::fs::write(
            native.model_target(),
            br#"{"$schema":"https://opencode.ai/config.json","mcp":{"late":{"enabled":true}}}"#,
        )
        .unwrap();
        assert_eq!(
            admission.validate(&home, true),
            Err(DelegationErrorV1::CapabilityUnavailable)
        );
        assert!(!native.database.exists());
    }

    #[test]
    fn opencode_database_admission_refuses_pending_unknown_and_legacy_without_writes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("native.db");
        assert!(validate_database(&path, &["known"]).is_ok());
        assert!(!path.exists());
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE migration(id TEXT PRIMARY KEY);CREATE TABLE session(id TEXT PRIMARY KEY,content TEXT);INSERT INTO session VALUES('native-exact','opaque-must-survive')").unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(validate_database(&path, &["known"]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("INSERT INTO migration VALUES('known')", [])
            .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(validate_database(&path, &["known"]).is_ok());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute("INSERT INTO migration VALUES('future')", [])
            .unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(validate_database(&path, &["known"]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}
