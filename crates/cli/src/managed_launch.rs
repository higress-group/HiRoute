use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::time::Duration;

use hiroute_application_api::{
    AgentGrantRawRequestV1, ManagedClaudeLaunchDescriptorV2, ManagedLaunchProfileV1,
};
use hiroute_integrations::ManagedClaudeProcessV1;
use serde_json::{Value, json};

mod settings;
use settings::resolve_user_settings;

pub(crate) struct ManagedLaunchInvocation {
    pub(crate) connection_id: String,
    pub(crate) child_arguments: Vec<OsString>,
    pub(crate) user_settings: Value,
}

pub(crate) fn parse_invocation(
    arguments: &[OsString],
) -> Result<ManagedLaunchInvocation, ManagedLaunchFailure> {
    let mut agent_selected = false;
    let mut context_id: Option<String> = None;
    let mut index = 0;
    let mut delimiter = false;
    while index < arguments.len() {
        let argument = arguments[index]
            .to_str()
            .ok_or(ManagedLaunchFailure::InvalidArguments)?;
        if argument == "--" {
            delimiter = true;
            index += 1;
            break;
        }
        if let Some(value) = argument.strip_prefix("--context=") {
            if context_id.replace(value.to_owned()).is_some() {
                return Err(ManagedLaunchFailure::InvalidArguments);
            }
            index += 1;
            continue;
        }
        match argument {
            "--agent" => {
                let claude_code = arguments
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value == "claude-code");
                if !claude_code || agent_selected {
                    return Err(ManagedLaunchFailure::InvalidArguments);
                }
                agent_selected = true;
                index += 2;
            }
            "--agent=claude-code" => {
                if agent_selected {
                    return Err(ManagedLaunchFailure::InvalidArguments);
                }
                agent_selected = true;
                index += 1;
            }
            "--context" => {
                let value = arguments
                    .get(index + 1)
                    .and_then(|value| value.to_str())
                    .ok_or(ManagedLaunchFailure::InvalidArguments)?;
                if context_id.replace(value.to_owned()).is_some() {
                    return Err(ManagedLaunchFailure::InvalidArguments);
                }
                index += 2;
            }
            _ => return Err(ManagedLaunchFailure::InvalidArguments),
        }
    }
    if !delimiter || !agent_selected {
        return Err(ManagedLaunchFailure::InvalidArguments);
    }
    let context_id = context_id.ok_or(ManagedLaunchFailure::InvalidArguments)?;
    let connection_id = format!("agent-connection/{context_id}");
    AgentGrantRawRequestV1::new(&connection_id)
        .map_err(|_| ManagedLaunchFailure::InvalidArguments)?;

    let profile = ManagedLaunchProfileV1::claude_code();
    let mut child_arguments = Vec::new();
    let mut user_settings = json!({});
    let mut rest = &arguments[index..];
    while let Some((argument, tail)) = rest.split_first() {
        rest = tail;
        if argument == "--" {
            child_arguments.push(argument.clone());
            child_arguments.extend_from_slice(tail);
            break;
        }
        if let Some(value) = settings_equals_value(argument) {
            user_settings = resolve_user_settings(value)?;
            continue;
        }
        if argument == "--settings" {
            let value = tail.first().ok_or(ManagedLaunchFailure::InvalidArguments)?;
            user_settings = resolve_user_settings(value)?;
            rest = &tail[1..];
            continue;
        }
        if forbidden_caller_argument(argument, &profile) {
            return Err(ManagedLaunchFailure::AuthPrecedenceConflict);
        }
        child_arguments.push(argument.clone());
    }
    Ok(ManagedLaunchInvocation {
        connection_id,
        child_arguments,
        user_settings,
    })
}

fn settings_equals_value(argument: &OsStr) -> Option<&OsStr> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        argument
            .as_bytes()
            .strip_prefix(b"--settings=")
            .map(OsStr::from_bytes)
    }
    #[cfg(not(unix))]
    {
        argument
            .to_str()?
            .strip_prefix("--settings=")
            .map(OsStr::new)
    }
}

fn forbidden_caller_argument(argument: &OsStr, profile: &ManagedLaunchProfileV1) -> bool {
    if let Some(argument) = argument.to_str() {
        return profile.forbids_caller_option(argument);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let argument = argument.as_bytes();
        profile.caller_forbidden_options.iter().any(|option| {
            argument == option.as_bytes()
                || argument
                    .strip_prefix(option.as_bytes())
                    .is_some_and(|suffix| suffix.starts_with(b"="))
        })
    }
    #[cfg(not(unix))]
    {
        // A platform-native argument that cannot be compared with the compiled denylist cannot
        // safely cross the managed auth/routing boundary.
        true
    }
}

pub(crate) fn launch(
    descriptor: &ManagedClaudeLaunchDescriptorV2,
    invocation: &ManagedLaunchInvocation,
    trusted_hiroute_executable: &str,
) -> Result<u8, ManagedLaunchFailure> {
    let mut process = ManagedClaudeProcessV1::prepare(
        descriptor,
        &invocation.user_settings,
        &invocation.child_arguments,
        trusted_hiroute_executable,
    )
    .map_err(|_| ManagedLaunchFailure::Unavailable)?;
    prepend_validated_user_bin(process.command_mut(), trusted_hiroute_executable);
    spawn_and_wait(process.command_mut())
}

/// Desktop may expose its bundled CLI through one owner-controlled stable link. Managed Agents
/// inherit that directory first only after the link is revalidated against this exact executable;
/// ordinary shells remain untouched.
#[cfg(unix)]
fn prepend_validated_user_bin(command: &mut Command, trusted_hiroute_executable: &str) {
    let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) else {
        return;
    };
    let Some(path) = validated_managed_path(
        &home,
        Path::new(trusted_hiroute_executable),
        std::env::var_os("PATH"),
    ) else {
        return;
    };
    command.env("PATH", path);
}

#[cfg(not(unix))]
fn prepend_validated_user_bin(_command: &mut Command, _trusted_hiroute_executable: &str) {}

#[cfg(unix)]
fn validated_managed_path(
    home: &Path,
    trusted_hiroute_executable: &Path,
    inherited: Option<OsString>,
) -> Option<OsString> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if !home.is_absolute() || !trusted_hiroute_executable.is_absolute() {
        return None;
    }
    let current = std::fs::canonicalize(trusted_hiroute_executable).ok()?;
    let contents = current.parent()?.parent()?;
    let bundle = contents.parent()?;
    let user_applications = home.join("Applications");
    if current.file_name()? != "hiroute"
        || current.parent()?.file_name()? != "MacOS"
        || contents.file_name()? != "Contents"
        || bundle.file_name()? != "HiRoute.app"
        || ![Path::new("/Applications"), user_applications.as_path()]
            .into_iter()
            .any(|root| bundle.parent() == Some(root))
    {
        return None;
    }
    let bin = home.join(".local/bin");
    let metadata = std::fs::symlink_metadata(&bin).ok()?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return None;
    }
    let entry = bin.join("hiroute");
    if !std::fs::symlink_metadata(&entry)
        .ok()?
        .file_type()
        .is_symlink()
        || !std::fs::read_link(&entry).ok()?.is_absolute()
        || std::fs::canonicalize(&entry).ok()? != current
    {
        return None;
    }
    let mut entries = vec![bin.clone()];
    entries.extend(
        inherited
            .as_ref()
            .map(std::env::split_paths)
            .into_iter()
            .flatten()
            .filter(|entry| entry != &bin),
    );
    std::env::join_paths(entries).ok()
}

#[cfg(unix)]
fn spawn_and_wait(command: &mut Command) -> Result<u8, ManagedLaunchFailure> {
    use tokio::signal::unix::{SignalKind, signal};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|_| ManagedLaunchFailure::Unavailable)?;
    runtime.block_on(async {
        let mut terminate =
            signal(SignalKind::terminate()).map_err(|_| ManagedLaunchFailure::Unavailable)?;
        let mut hangup =
            signal(SignalKind::hangup()).map_err(|_| ManagedLaunchFailure::Unavailable)?;
        let mut interrupt =
            signal(SignalKind::interrupt()).map_err(|_| ManagedLaunchFailure::Unavailable)?;
        let mut child = command
            .spawn()
            .map_err(|_| ManagedLaunchFailure::Unavailable)?;
        let child_pid = nix::unistd::Pid::from_raw(child.id() as i32);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(exit_code(status)),
                Ok(None) => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ManagedLaunchFailure::Unavailable);
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
                _ = terminate.recv() => {
                    let _ = nix::sys::signal::kill(child_pid, nix::sys::signal::Signal::SIGTERM);
                }
                _ = hangup.recv() => {
                    let _ = nix::sys::signal::kill(child_pid, nix::sys::signal::Signal::SIGHUP);
                }
                // The terminal already delivers Ctrl-C to the shared foreground process group.
                _ = interrupt.recv() => {}
            }
        }
    })
}

#[cfg(not(unix))]
fn spawn_and_wait(command: &mut Command) -> Result<u8, ManagedLaunchFailure> {
    command
        .status()
        .map(exit_code)
        .map_err(|_| ManagedLaunchFailure::Unavailable)
}

fn exit_code(status: ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        return u8::try_from(code).unwrap_or(1);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return u8::try_from(128 + signal).unwrap_or(255);
        }
    }
    1
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedLaunchFailure {
    InvalidArguments,
    AuthPrecedenceConflict,
    Unavailable,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use hiroute_application_api::{
        AgentClaudePresetValuesV2, CanonicalDigest, ManagedClaudeLaunchDescriptorV2,
    };

    use super::*;

    const CONTEXT: &str = "agent-context/claude/0011223344556677";

    fn parse(arguments: &[&str]) -> Result<ManagedLaunchInvocation, ManagedLaunchFailure> {
        let arguments = arguments
            .iter()
            .map(|argument| OsString::from(*argument))
            .collect::<Vec<_>>();
        parse_invocation(&arguments)
    }

    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\\''"))
    }

    #[test]
    fn parser_requires_agent_context_and_delimiter_and_directs_model_settings() {
        let parsed = parse(&[
            "--agent",
            "claude-code",
            "--context",
            CONTEXT,
            "--",
            "--print",
            "--model",
            "opus",
            "--setting-sources",
            "user,project",
            "--fallback-model",
            "sonnet",
        ])
        .unwrap();
        assert_eq!(parsed.connection_id, format!("agent-connection/{CONTEXT}"));
        assert_eq!(
            parsed.child_arguments,
            [
                "--print",
                "--model",
                "opus",
                "--setting-sources",
                "user,project",
                "--fallback-model",
                "sonnet",
            ]
        );
        assert_eq!(parsed.user_settings, serde_json::json!({}));

        assert!(parse(&["--agent", "claude-code", "--context", CONTEXT]).is_err());
        assert!(parse(&["--context", CONTEXT, "--"]).is_err());
        assert!(parse(&["--agent", "codex", "--context", CONTEXT, "--"]).is_err());
        assert!(parse(&["--agent", "claude-code", "--", "--print"]).is_err());
        assert!(
            parse(&[
                "--agent",
                "claude-code",
                "--context",
                CONTEXT,
                "--context",
                CONTEXT,
                "--"
            ])
            .is_err()
        );
        for option in ["--setting-sources", "--model", "--fallback-model=other"] {
            assert!(
                parse(&["--agent", "claude-code", "--context", CONTEXT, "--", option]).is_ok(),
                "{option} is directed, not rejected"
            );
        }
        for option in [
            "--anthropic-auth-token=x",
            "--base-url=https://elsewhere",
            "--use-bedrock",
            "--provider=x",
        ] {
            assert!(matches!(
                parse(&["--agent", "claude-code", "--context", CONTEXT, "--", option]),
                Err(ManagedLaunchFailure::AuthPrecedenceConflict)
            ));
        }
    }

    #[test]
    fn parser_preserves_arguments_after_native_delimiter() {
        let parsed = parse(&[
            "--agent",
            "claude-code",
            "--context",
            CONTEXT,
            "--",
            "--",
            "--settings=not-json",
            "--api-key=prompt-text",
        ])
        .unwrap();
        assert_eq!(parsed.user_settings, json!({}));
        assert_eq!(
            parsed.child_arguments,
            ["--", "--settings=not-json", "--api-key=prompt-text"]
        );
    }

    #[test]
    fn parser_uses_last_settings_object_or_file_before_spawn() {
        let directory = tempfile::tempdir().unwrap();
        let settings_file = directory.path().join("user-settings.json");
        std::fs::write(
            &settings_file,
            serde_json::json!({
                "permissions": {"allow": ["Bash(ls)"]},
                "env": {"UNRELATED": "keep", "ANTHROPIC_BASE_URL": "https://leak.example"},
            })
            .to_string(),
        )
        .unwrap();
        let parsed = parse(&[
            "--agent=claude-code",
            format!("--context={CONTEXT}").as_str(),
            "--",
            "--settings={\"env\":{\"OTHER\":\"2\",\"UNRELATED\":\"user\"}}",
            "--settings",
            settings_file.to_str().unwrap(),
            "--settings",
            "{\"model\":\"opus\"}",
            "--print",
        ])
        .unwrap();
        assert_eq!(parsed.child_arguments, ["--print"]);
        assert_eq!(parsed.user_settings, serde_json::json!({"model": "opus"}));
        for invalid in [r#"{"env":null}"#, r#"{"env":[]}"#, r#"{"env":{"KEY":1}}"#] {
            assert!(resolve_user_settings(OsStr::new(invalid)).is_err());
        }
        // Invalid JSON and missing files fail before any spawn.
        assert!(
            parse(&[
                "--agent",
                "claude-code",
                "--context",
                CONTEXT,
                "--",
                "--settings={invalid",
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--agent",
                "claude-code",
                "--context",
                CONTEXT,
                "--",
                "--settings",
                "/nonexistent/settings.json",
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--agent",
                "claude-code",
                "--context",
                CONTEXT,
                "--",
                "--settings",
            ])
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn parser_handles_non_utf8_settings_paths_before_spawn() {
        use std::os::unix::ffi::OsStringExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join(OsString::from_vec(b"settings-\xff.json".to_vec()));
        let settings = json!({"model": "opus", "env": {"UNRELATED": "keep"}});
        let settings_bytes = serde_json::to_vec(&settings).unwrap();
        let path_is_supported = match std::fs::write(&path, &settings_bytes) {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(nix::libc::EILSEQ) => false,
            Err(error) => panic!("unexpected non-UTF-8 path error: {error}"),
        };
        let mut equals = OsString::from("--settings=");
        equals.push(path.as_os_str());
        for suffix in [
            vec![OsString::from("--settings"), path.as_os_str().to_owned()],
            vec![equals],
        ] {
            let mut arguments: Vec<OsString> =
                ["--agent", "claude-code", "--context", CONTEXT, "--"]
                    .into_iter()
                    .map(OsString::from)
                    .collect();
            arguments.extend(suffix);
            arguments.push("--print".into());
            if !path_is_supported {
                assert!(matches!(
                    parse_invocation(&arguments),
                    Err(ManagedLaunchFailure::InvalidArguments)
                ));
                continue;
            }
            let parsed = parse_invocation(&arguments).unwrap();
            assert_eq!(parsed.user_settings, settings);
            assert_eq!(parsed.child_arguments, ["--print"]);
            std::fs::write(&path, b"not-json").unwrap();
            assert!(matches!(
                parse_invocation(&arguments),
                Err(ManagedLaunchFailure::InvalidArguments)
            ));
            std::fs::write(&path, &settings_bytes).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn parser_preserves_non_utf8_child_argument_as_os_string() {
        use std::os::unix::ffi::OsStringExt;

        let raw = OsString::from_vec(vec![b'x', 0xff, b'y']);
        let arguments = vec![
            OsString::from("--agent"),
            OsString::from("claude-code"),
            OsString::from("--context"),
            OsString::from(CONTEXT),
            OsString::from("--"),
            raw.clone(),
        ];
        let parsed = parse_invocation(&arguments).unwrap();
        assert_eq!(parsed.child_arguments, [raw]);

        let disguised_override = OsString::from_vec(vec![
            b'-', b'-', b'a', b'p', b'i', b'-', b'k', b'e', b'y', b'=', 0xff,
        ]);
        let arguments = vec![
            OsString::from("--agent"),
            OsString::from("claude-code"),
            OsString::from("--context"),
            OsString::from(CONTEXT),
            OsString::from("--"),
            disguised_override,
        ];
        assert!(matches!(
            parse_invocation(&arguments),
            Err(ManagedLaunchFailure::AuthPrecedenceConflict)
        ));
    }

    #[test]
    fn configured_child_carries_routing_environment_without_empty_setting_sources() {
        let executable = std::env::current_exe().unwrap();
        let descriptor = descriptor(executable.to_str().unwrap(), "/opt/hiroute/bin/hiroute");
        let invocation = ManagedLaunchInvocation {
            connection_id: descriptor.connection_id.clone(),
            child_arguments: ["--model".into(), "opus".into()].to_vec(),
            user_settings: serde_json::json!({"env": {"UNRELATED": "keep"}}),
        };
        let mut process = ManagedClaudeProcessV1::prepare(
            &descriptor,
            &invocation.user_settings,
            &invocation.child_arguments,
            "/opt/hiroute/bin/hiroute",
        )
        .unwrap();
        let command = process.command_mut();
        let removed = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect::<BTreeSet<_>>();
        let expected_removed = ["ANTHROPIC_MODEL", "ANTHROPIC_DEFAULT_SONNET_MODEL"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(removed, expected_removed);
        let environment = command
            .get_envs()
            .filter_map(|(name, value)| {
                Some((name.to_str()?.to_owned(), value?.to_str()?.to_owned()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment.get("ANTHROPIC_BASE_URL").cloned(),
            Some("http://127.0.0.1:5837".to_owned())
        );
        assert_eq!(
            environment.get("ANTHROPIC_DEFAULT_OPUS_MODEL").cloned(),
            Some("hr-plan-opus".to_owned())
        );
        assert_eq!(
            environment.get("ANTHROPIC_DEFAULT_HAIKU_MODEL").cloned(),
            Some("native-haiku".to_owned())
        );
        assert!(!environment.contains_key("ANTHROPIC_DEFAULT_SONNET_MODEL"));
        for name in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_CUSTOM_HEADERS",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_FOUNDRY",
            "CLAUDE_CODE_USE_VERTEX",
        ] {
            assert_eq!(
                environment.get(name).map(String::as_str),
                Some(""),
                "{name}"
            );
        }
        assert_eq!(command.get_args().next(), Some(OsStr::new("--settings")));
        assert_eq!(command.get_args().nth(2), Some(OsStr::new("--model")));
        assert_eq!(command.get_args().nth(3), Some(OsStr::new("opus")));
    }

    #[cfg(unix)]
    #[test]
    fn managed_path_prepends_only_an_exact_owned_app_link() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("Users/test user");
        let target = home.join("Applications/HiRoute.app/Contents/MacOS/hiroute");
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(&target, b"binary").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, bin.join("hiroute")).unwrap();
        let inherited = std::env::join_paths([PathBuf::from("/usr/bin"), bin.clone()]).unwrap();

        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(validated_managed_path(&home, &target, Some(inherited.clone())).is_none());

        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = validated_managed_path(&home, &target, Some(inherited)).unwrap();
        assert_eq!(
            std::env::split_paths(&path).collect::<Vec<_>>(),
            [bin.clone(), PathBuf::from("/usr/bin")]
        );
        std::fs::remove_file(bin.join("hiroute")).unwrap();
        std::os::unix::fs::symlink("/usr/bin/false", bin.join("hiroute")).unwrap();
        assert!(validated_managed_path(&home, &target, None).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn temporary_overlay_is_owner_only_secret_free_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;

        let executable = std::env::current_exe().unwrap();
        let descriptor = descriptor(executable.to_str().unwrap(), "/opt/hiroute/bin/hiroute");
        let user_settings = serde_json::json!({
            "permissions": {"allow": ["Bash(ls)"]},
            "env": {"UNRELATED": "keep", "ANTHROPIC_API_KEY": "user-secret"},
        });
        let mut process = ManagedClaudeProcessV1::prepare(
            &descriptor,
            &user_settings,
            &[],
            "/opt/hiroute/bin/hiroute",
        )
        .unwrap();
        let mut concurrent = ManagedClaudeProcessV1::prepare(
            &descriptor,
            &user_settings,
            &[],
            "/opt/hiroute/bin/hiroute",
        )
        .unwrap();
        let path = PathBuf::from(process.command_mut().get_args().nth(1).unwrap());
        let concurrent_path = PathBuf::from(concurrent.command_mut().get_args().nth(1).unwrap());
        let directory = path.parent().unwrap().to_path_buf();
        let concurrent_directory = concurrent_path.parent().unwrap().to_path_buf();
        assert_ne!(directory, concurrent_directory);
        let suffix = directory
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .strip_prefix("hiroute-claude-launch-")
            .unwrap();
        assert_eq!(suffix.len(), 32);
        assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let contents = std::fs::read_to_string(&path).unwrap();
        let overlay_json: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            overlay_json["apiKeyHelper"],
            "'/opt/hiroute/bin/hiroute' '__internal-agent-grant-v1' 'agent-connection/agent-context/claude/0011223344556677'"
        );
        assert_eq!(
            overlay_json["env"]["ANTHROPIC_BASE_URL"],
            "http://127.0.0.1:5837"
        );
        assert_eq!(
            overlay_json["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
            "hr-plan-opus"
        );
        assert_eq!(
            overlay_json["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            "native-haiku"
        );
        assert_eq!(overlay_json["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"], "");
        assert_eq!(overlay_json["env"]["UNRELATED"], "keep");
        assert_eq!(
            overlay_json["permissions"],
            serde_json::json!({"allow": ["Bash(ls)"]})
        );
        for name in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_CUSTOM_HEADERS",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_FOUNDRY",
            "CLAUDE_CODE_USE_VERTEX",
        ] {
            assert_eq!(overlay_json["env"][name], "", "{name}");
        }
        assert!(overlay_json["env"].get("ANTHROPIC_MODEL").is_none());
        assert!(!contents.contains("user-secret"));

        drop(process);
        assert!(!path.exists());
        assert!(!directory.exists());
        assert!(concurrent_path.exists());
    }

    /// Signal handling is process-global, so the tests that install forwarding handlers or
    /// raise process-level signals must never run concurrently with each other.
    static SIGNAL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(unix)]
    #[test]
    fn launch_executes_exact_binary_without_version_probe_and_cleans_overlay_after_exit() {
        use std::os::unix::fs::PermissionsExt;

        let _signals = SIGNAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("claude");
        let captured_arguments = directory.path().join("arguments");
        let captured_overlay = directory.path().join("overlay");
        let captured_overlay_path = directory.path().join("overlay-path");
        // The script fails any --version probe with exit 9: a per-launch probe must not run.
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = '--version' ]; then exit 9; fi\nprintf '%s\\n' \"$@\" > {}\ncat \"$2\" > {}\nprintf '%s\\n' \"$2\" > {}\nexit 23\n",
            shell_quote(captured_arguments.to_str().unwrap()),
            shell_quote(captured_overlay.to_str().unwrap()),
            shell_quote(captured_overlay_path.to_str().unwrap()),
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executable = std::fs::canonicalize(executable)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap();
        let descriptor = descriptor(&executable, "/opt/hiroute/bin/hiroute");
        let invocation = ManagedLaunchInvocation {
            connection_id: descriptor.connection_id.clone(),
            child_arguments: ["--print".into(), "$(must-not-run)".into()].to_vec(),
            user_settings: serde_json::json!({}),
        };

        assert_eq!(
            launch(&descriptor, &invocation, "/opt/hiroute/bin/hiroute").unwrap(),
            23
        );
        assert_eq!(
            std::fs::read_to_string(captured_arguments).unwrap(),
            "--settings\n".to_owned()
                + std::fs::read_to_string(&captured_overlay_path)
                    .unwrap()
                    .trim_end()
                + "\n--print\n$(must-not-run)\n"
        );
        let overlay = std::fs::read_to_string(captured_overlay).unwrap();
        assert!(overlay.contains("apiKeyHelper"));
        assert!(overlay.contains("ANTHROPIC_DEFAULT_OPUS_MODEL"));
        let overlay: serde_json::Value = serde_json::from_str(&overlay).unwrap();
        assert_eq!(overlay["env"]["ANTHROPIC_AUTH_TOKEN"], "");
        let overlay_path = PathBuf::from(
            std::fs::read_to_string(captured_overlay_path)
                .unwrap()
                .trim(),
        );
        assert!(!overlay_path.exists());
        assert!(!overlay_path.parent().unwrap().exists());
    }

    #[cfg(unix)]
    #[test]
    fn signal_killed_child_preserves_signal_exit_semantics() {
        use std::os::unix::fs::PermissionsExt;

        let _signals = SIGNAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("claude");
        let script = "#!/bin/sh\nkill -TERM $$\n".to_owned();
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executable = std::fs::canonicalize(executable)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap();
        let descriptor = descriptor(&executable, "/opt/hiroute/bin/hiroute");
        let invocation = ManagedLaunchInvocation {
            connection_id: descriptor.connection_id.clone(),
            child_arguments: Vec::new(),
            user_settings: serde_json::json!({}),
        };
        assert_eq!(
            launch(&descriptor, &invocation, "/opt/hiroute/bin/hiroute").unwrap(),
            128 + 15
        );
    }

    #[cfg(unix)]
    #[test]
    fn directed_terminate_is_forwarded_to_only_this_child() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::process::CommandExt;

        const CHILD_SIGNAL: &str = "HIROUTE_LAUNCH_SIGNAL_TEST";
        let Ok(signal_name) = std::env::var(CHILD_SIGNAL) else {
            for signal_name in ["TERM", "HUP", "INT"] {
                let home = tempfile::tempdir().unwrap();
                let output = Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "managed_launch::tests::directed_terminate_is_forwarded_to_only_this_child",
                        "--nocapture",
                    ])
                    .env(CHILD_SIGNAL, signal_name)
                    .env("HOME", home.path())
                    .env("TMPDIR", home.path())
                    .process_group(0)
                    .output()
                    .unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    output.status.success() && stdout.contains("1 passed"),
                    "{signal_name}: {}\n{stdout}\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            return;
        };
        let signal = match signal_name.as_str() {
            "TERM" => nix::sys::signal::Signal::SIGTERM,
            "HUP" => nix::sys::signal::Signal::SIGHUP,
            "INT" => nix::sys::signal::Signal::SIGINT,
            _ => panic!("invalid isolated signal case"),
        };
        let _signals = SIGNAL_TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("claude");
        let forwarded = directory.path().join("forwarded");
        let ready = directory.path().join("ready");
        let script = format!(
            "#!/bin/sh\ntrap 'kill \"$sleeper\"; wait \"$sleeper\"; printf received > {}; exit 7' {signal_name}\nsleep 30 & sleeper=$!\nprintf ready > {}\nwait \"$sleeper\"\n",
            shell_quote(forwarded.to_str().unwrap()),
            shell_quote(ready.to_str().unwrap()),
        );
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let executable = std::fs::canonicalize(executable)
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap();
        let descriptor = descriptor(&executable, "/opt/hiroute/bin/hiroute");
        let invocation = ManagedLaunchInvocation {
            connection_id: descriptor.connection_id.clone(),
            child_arguments: Vec::new(),
            user_settings: serde_json::json!({}),
        };
        let own_pid = std::process::id() as i32;
        let signaller = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !ready.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(ready.exists(), "child did not become ready");
            let target = if signal == nix::sys::signal::Signal::SIGINT {
                -own_pid
            } else {
                own_pid
            };
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(target), signal).unwrap();
        });
        assert_eq!(
            launch(&descriptor, &invocation, "/opt/hiroute/bin/hiroute").unwrap(),
            7
        );
        signaller.join().unwrap();
        assert_eq!(
            std::fs::read_to_string(forwarded).unwrap(),
            "received".to_owned()
        );
    }

    fn descriptor(executable: &str, helper: &str) -> ManagedClaudeLaunchDescriptorV2 {
        ManagedClaudeLaunchDescriptorV2::trusted(
            format!("agent-connection/{CONTEXT}"),
            "claude-messages-v1",
            executable,
            CanonicalDigest::of_bytes(b"snapshot"),
            1,
            CanonicalDigest::of_bytes(b"publication"),
            "http://127.0.0.1:5837",
            AgentClaudePresetValuesV2 {
                opus: Some("hr-plan-opus".into()),
                sonnet: None,
                haiku: Some("native-haiku".into()),
            },
            helper,
        )
        .unwrap()
    }
}
