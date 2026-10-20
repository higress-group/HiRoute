//! Explicit native Skill/CLI check. Reuses the shared Responses peer and owned process scope.
use super::qoder_probe::{
    QoderCollaborationProbeTarget as Target, cli_baseline, confirmed_skill, skill_body,
};
use super::{OpenCodeNativeContext, QoderNativeError, qoder_error};
use hiroute_domain::{
    AgentCapability, CanonicalDigest, CapabilityState, SupportedAgentInstallationV1,
};
use serde_json::{Value, json};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MODEL: &str = "hiroute-collaboration-probe";
const CONTRACT: &str = "hiroute.opencode-collaboration/v1";

#[derive(Default)]
pub(super) struct ProofCache {
    generation: u64,
    pub evidence: Option<Evidence>,
}
impl ProofCache {
    pub fn begin(&mut self) -> Result<u64, QoderNativeError> {
        self.evidence = None;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| qoder_error("check generation"))?;
        Ok(self.generation)
    }
    pub fn finish(&mut self, generation: u64, evidence: Evidence) -> Result<(), QoderNativeError> {
        if self.generation != generation {
            return Err(qoder_error("native check superseded"));
        }
        self.evidence = Some(evidence);
        Ok(())
    }
}
pub struct Evidence {
    cli: PathBuf,
    target: Target,
    dependencies: CanonicalDigest,
    observed: Instant,
    observed_at: u64,
}
impl Evidence {
    pub fn attach(
        &self,
        executable: &Path,
        native: &OpenCodeNativeContext,
        installation: &mut SupportedAgentInstallationV1,
    ) {
        if self.observed.elapsed() > Duration::from_secs(300)
            || dependencies(executable, native, &self.cli, &self.target)
                .ok()
                .as_ref()
                != Some(&self.dependencies)
        {
            return;
        }
        installation.observation_digest = CanonicalDigest::of(&(
            CONTRACT,
            &installation.observation_digest,
            &self.dependencies,
        ))
        .expect("bounded proof");
        for proof in &mut installation.capability_evidence {
            proof.dependency_digest = installation.observation_digest.clone();
            if matches!(
                proof.capability,
                AgentCapability::SkillLoading | AgentCapability::TrustedCliExecution
            ) {
                proof.state = CapabilityState::Proven;
                proof.reason = None;
                proof.adapter_contract = CONTRACT.into();
                proof.observed_at_unix_ms = self.observed_at;
            }
        }
    }
}

pub struct OpenCodeCollaborationProbe;
impl OpenCodeCollaborationProbe {
    pub fn run(
        executable: &Path,
        native: &OpenCodeNativeContext,
        cli: &Path,
        target: Target,
    ) -> Result<Evidence, QoderNativeError> {
        let fail = || qoder_error("OpenCode collaboration check");
        let executable = super::opencode_native_executable(executable)?.ok_or_else(fail)?;
        let cli = super::executable::resolve(cli)
            .map_err(|_| fail())?
            .ok_or_else(fail)?;
        let (before, before_parts) = dependency_parts(&executable, native, &cli, &target)?;
        let root = super::probe_files::temporary("hiroute-opencode-check-").map_err(|_| fail())?;
        let root_path = std::fs::canonicalize(root.path()).map_err(|_| fail())?;
        let workspace = root_path.join("workspace");
        let temporary = root_path.join("tmp");
        super::probe_files::directory(&temporary).map_err(|_| fail())?;
        super::probe_files::directory(&workspace).map_err(|_| fail())?;
        // Installation is independent of daily model configuration. The private native
        // runtime loads the actual installed file through its supported resource path;
        // path/body receipts below distinguish it from any substitute or shadow Skill.
        let probe_home = root_path.join("home");
        super::probe_files::directory(&probe_home).map_err(|_| fail())?;
        let runtime = OpenCodeNativeContext::from_lookup(&probe_home, |_| None)?;
        let (skill_path, content) = match &target {
            Target::InstalledUserSkill { expected_content } => {
                let path = native.skill_target();
                let content = confirmed_skill(&path, expected_content)?;
                (path, content)
            }
            Target::PreinstallCapability => {
                let mut nonce = [0u8; 16];
                getrandom::fill(&mut nonce).map_err(|_| fail())?;
                let nonce = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
                let name = format!("hiroute-check-{nonce}");
                let path = workspace
                    .join(".opencode/skills")
                    .join(&name)
                    .join("SKILL.md");
                for directory in [
                    workspace.join(".opencode"),
                    workspace.join(".opencode/skills"),
                    path.parent().ok_or_else(fail)?.to_owned(),
                ] {
                    super::probe_files::directory(&directory).map_err(|_| fail())?;
                }
                let content = format!(
                    "---\nname: {name}\ndescription: HiRoute local capability check\n---\n\nLocal receipt: {nonce}\n"
                );
                super::probe_files::file(&path)
                    .and_then(|mut f| f.write_all(content.as_bytes()))
                    .map_err(|_| fail())?;
                (path, content)
            }
        };
        let directory =
            std::fs::canonicalize(skill_path.parent().ok_or_else(fail)?).map_err(|_| fail())?;
        let name = directory
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(fail)?
            .to_owned();
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| fail())?;
        let token = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let (shell, cli_command) = probe_shell_command(&cli, &token)?;
        let mut challenge = Challenge {
            name,
            directory: directory.to_str().ok_or_else(fail)?.into(),
            body: skill_body(&content)?,
            command: cli_command,
            expected_cli: cli_baseline(&cli, &root_path)?,
            nonce: token.clone(),
            step: 0,
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| fail())?;
        listener.set_nonblocking(true).map_err(|_| fail())?;
        let provider = super::opencode_provider(
            &format!("http://{}/v1", listener.local_addr().map_err(|_| fail())?),
            &hiroute_domain::AdditionalAgentModelV1 {
                alias: MODEL.into(),
                protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
                context_window_tokens: 65536,
                max_output_tokens: 2048,
            },
            "{env:HIROUTE_OPENCODE_PROBE_TOKEN}",
            None,
        )?;
        let selector = format!("hiroute-native-probe/{MODEL}");
        let permission = json!({"*":"deny","skill":"allow","bash":"allow","external_directory":"allow","doom_loop":"allow"});

        let mut overlay = json!({"$schema":"https://opencode.ai/config.json","provider":{"hiroute-native-probe":provider},"model":selector,"small_model":selector,"default_agent":"hiroute-check","enabled_providers":["hiroute-native-probe"],"agent":{"hiroute-check":{"mode":"primary","model":selector,"permission":permission},"title":{"model":selector,"disable":true},"summary":{"model":selector,"disable":true},"compaction":{"model":selector,"disable":true}},"permission":permission,"skills":{"paths":[directory]},"share":"disabled","autoupdate":false,"lsp":false,"formatter":false});
        if let Some(shell) = shell {
            overlay["shell"] = json!(shell);
        }
        let output =
            super::probe_files::file(&root_path.join("output.private")).map_err(|_| fail())?;
        let observer = output.try_clone().map_err(|_| fail())?;
        let mut command = Command::new(&executable);
        command
            .args([
                "run",
                "--format",
                "json",
                "--model",
                &selector,
                "--agent",
                "hiroute-check",
                "Complete the local compatibility check.",
            ])
            .current_dir(&workspace)
            .env_clear()
            .env("HOME", &runtime.home)
            .env("USERPROFILE", &runtime.home)
            .env("TMPDIR", &temporary)
            .env("TMP", &temporary)
            .env("TEMP", &temporary)
            .env("PATH", super::native_ingress_probe::native_probe_path())
            .env("OPENCODE_CONFIG_CONTENT", overlay.to_string())
            .env("HIROUTE_OPENCODE_PROBE_TOKEN", &token)
            .env("OPENCODE_DB", root_path.join("probe.db"))
            .env("OPENCODE_PURE", "true")
            // Global .agents/.claude discovery can ignore the private HOME and
            // race the explicitly selected installed file with a same-name Skill.
            .env("OPENCODE_DISABLE_EXTERNAL_SKILLS", "true")
            .env("OPENCODE_DISABLE_DEFAULT_PLUGINS", "true")
            .env("OPENCODE_DISABLE_AUTOUPDATE", "true")
            .env("OPENCODE_DISABLE_MODELS_FETCH", "true")
            .env("OPENCODE_DISABLE_LSP_DOWNLOAD", "true")
            .env("OPENCODE_DISABLE_FFF", "true")
            .env("OPENCODE_PERMISSION", permission.to_string());
        for (key, path) in [
            ("XDG_CONFIG_HOME", &runtime.config_root),
            ("XDG_DATA_HOME", &runtime.data_root),
            ("XDG_CACHE_HOME", &runtime.cache_root),
            ("XDG_STATE_HOME", &runtime.state_root),
        ] {
            command.env(key, path.parent().ok_or_else(fail)?);
        }
        #[cfg(windows)]
        for (key, value) in
            super::windows_runtime::windows_native_system_environment().map_err(|_| fail())?
        {
            command.env(key, value);
        }
        #[cfg(unix)]
        let child = command
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(Stdio::null())
            .process_group(0)
            .spawn();
        #[cfg(windows)]
        let child = super::windows_runtime::spawn(
            &command,
            false,
            hiroute_platform::windows::job::Stream::Null,
            hiroute_platform::windows::job::Stream::File(&output),
            hiroute_platform::windows::job::Stream::Null,
        );
        let mut child =
            super::native_probe_process::NativeProbeProcess::new(child.map_err(|_| fail())?);
        let started = Instant::now();
        let result = (|| {
            loop {
                if started.elapsed() > Duration::from_secs(45) {
                    return Err(qoder_error("native check deadline"));
                }
                if observer.metadata().map_err(|_| fail())?.len() > 512 * 1024 {
                    return Err(qoder_error("native output bound"));
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        super::native_responses_probe::serve(&mut stream, &token, MODEL, |body| {
                            challenge.reply(body)
                        })
                        .map_err(qoder_error)?
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(fail()),
                }
                if let Some(status) = child.observe().map_err(|_| fail())? {
                    return Ok(status);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        if child.stop().is_err() {
            let _ = root.keep();
            return Err(qoder_error("OpenCode process scope"));
        }
        if !result? {
            return Err(qoder_error("native collaboration execution"));
        }
        if challenge.step != 3 {
            return Err(qoder_error("native collaboration incomplete"));
        }
        let (after, after_parts) = dependency_parts(&executable, native, &cli, &target)?;
        if before != after {
            for (index, name) in [
                "native-resource",
                "native-executable",
                "trusted-cli",
                "installed-skill",
            ]
            .into_iter()
            .enumerate()
            {
                if before_parts[index] != after_parts[index] {
                    eprintln!("native OpenCode collaboration dependency changed: {name}");
                }
            }
            return Err(qoder_error("native context changed"));
        }
        Ok(Evidence {
            cli,
            target,
            dependencies: before,
            observed: Instant::now(),
            observed_at: super::native_ingress_probe::now(),
        })
    }
}

fn probe_shell_command(
    cli: &Path,
    nonce: &str,
) -> Result<(Option<PathBuf>, String), QoderNativeError> {
    #[cfg(windows)]
    {
        // OpenCode's tool ID is `bash` even when it executes PowerShell/cmd.
        // Pin the system shell for this private probe, independently of PATH.
        let shell = hiroute_platform::windows::launcher::system_executable(Path::new(
            "WindowsPowerShell/v1.0/powershell.exe",
        ))
        .map_err(|_| qoder_error("OpenCode probe shell"))?;
        let cli = super::managed_launch::windows_executable_path(cli)
            .map_err(|_| qoder_error("trusted CLI path"))?;
        let path = cli
            .to_str()
            .ok_or_else(|| qoder_error("trusted CLI path"))?;
        let path = path.replace('\'', "''");
        Ok((
            Some(shell),
            format!(
                "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; & '{path}' schema list --output json; if ($LASTEXITCODE -ne 0) {{ exit $LASTEXITCODE }}; Write-Output '{nonce}'"
            ),
        ))
    }
    #[cfg(not(windows))]
    Ok((
        None,
        format!(
            "{} && printf '%s' '{nonce}'",
            super::qoder_probe::cli_command(cli)?
        ),
    ))
}
fn dependencies(
    executable: &Path,
    native: &OpenCodeNativeContext,
    cli: &Path,
    target: &Target,
) -> Result<CanonicalDigest, QoderNativeError> {
    dependency_parts(executable, native, cli, target).map(|(digest, _)| digest)
}
fn dependency_parts(
    executable: &Path,
    native: &OpenCodeNativeContext,
    cli: &Path,
    target: &Target,
) -> Result<(CanonicalDigest, [CanonicalDigest; 4]), QoderNativeError> {
    let skill = match target {
        Target::PreinstallCapability => None,
        Target::InstalledUserSkill { expected_content } => Some(CanonicalDigest::of_bytes(
            confirmed_skill(&native.skill_target(), expected_content)?.as_bytes(),
        )),
    };
    let resource = match &skill {
        Some(content) => super::observed_capabilities::content_bound_dependencies(
            Some(executable),
            &native.skill_target(),
            content,
        ),
        None => {
            super::observed_capabilities::dependencies(Some(executable), &native.skill_target())
        }
    }
    .map_err(|_| qoder_error("native resource identity"))?
    .0;
    let executable = super::native_ingress_probe::binary_identity(executable)
        .map_err(|_| qoder_error("native binary identity"))?;
    let cli = super::native_ingress_probe::binary_identity(cli)
        .map_err(|_| qoder_error("CLI identity"))?;
    let skill_digest =
        CanonicalDigest::of(&skill).map_err(|_| qoder_error("check dependencies"))?;
    let digest = CanonicalDigest::of(&(CONTRACT, native, &resource, &executable, &cli, skill))
        .map_err(|_| qoder_error("check dependencies"))?;
    Ok((digest, [resource, executable, cli, skill_digest]))
}
struct Challenge {
    name: String,
    directory: String,
    body: String,
    command: String,
    expected_cli: String,
    nonce: String,
    step: usize,
}
impl Challenge {
    fn reply(&mut self, body: &Value) -> Result<Value, &'static str> {
        let input = body["input"].as_array().ok_or("native input")?;
        let result = |id: &str| {
            input
                .iter()
                .find(|i| i["type"] == "function_call_output" && i["call_id"] == id)
                .and_then(|i| i["output"].as_str())
        };
        let item = match self.step {
            0 => {
                let tools = body["tools"].as_array().ok_or("native tools")?;
                if !body.to_string().contains(&self.name)
                    || !["skill", "bash"]
                        .iter()
                        .all(|name| tools.iter().any(|t| t["name"] == *name))
                {
                    return Err("native Skill discovery");
                }
                function("skill", "skill", json!({"name":self.name}))
            }
            1 => {
                let loaded = result("call_hiroute_skill").ok_or("native Skill receipt")?;
                if !loaded.starts_with(&format!("<skill_content name=\"{}\">", self.name))
                    || !loaded.contains(&format!("\n{}\n", self.body.trim()))
                    || !loaded.contains(&format!(
                        "\nBase directory for this skill: {}\n",
                        self.directory
                    ))
                {
                    return Err("actual native Skill source");
                }
                function(
                    "cli",
                    "bash",
                    json!({"command":self.command,"description":"Read HiRoute public command schema","timeout":10000}),
                )
            }
            2 => {
                let output = result("call_hiroute_cli").ok_or("native CLI receipt")?;
                if !output.contains(&self.expected_cli) || !output.trim_end().ends_with(&self.nonce)
                {
                    return Err("nonce-correlated CLI receipt");
                }
                json!({"id":"msg_probe","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"OK","annotations":[]}]})
            }
            _ => return Err("native request bound"),
        };
        self.step += 1;
        Ok(item)
    }
}
fn function(suffix: &str, name: &str, args: Value) -> Value {
    json!({"id":format!("fc_hiroute_{suffix}"),"type":"function_call","status":"completed","call_id":format!("call_hiroute_{suffix}"),"name":name,"arguments":args.to_string()})
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn opencode_probe_cli_shell_preserves_literal_paths_and_requires_success_for_nonce() {
        let root = tempfile::tempdir().unwrap();
        let cli = root.path().join("CLI ' space $(literal); & hiroute");
        let nonce = "0123456789abcdef";
        for exit in [0, 7] {
            std::fs::write(
                &cli,
                format!("#!/bin/sh\nprintf '%s' 'actual-cli-receipt'\nexit {exit}\n"),
            )
            .unwrap();
            std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
            let (shell, command) = probe_shell_command(&cli, nonce).unwrap();
            assert!(shell.is_none());
            let output = Command::new("/bin/sh")
                .args(["-c", &command])
                .output()
                .unwrap();
            assert_eq!(output.status.success(), exit == 0);
            let output = String::from_utf8(output.stdout).unwrap();
            assert!(output.starts_with("actual-cli-receipt"));
            assert_eq!(output.ends_with(nonce), exit == 0);
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn windows_opencode_probe_pins_system_powershell_and_parses_cli_as_literal_argv() {
        let root = super::super::probe_files::temporary("hiroute-opencode-shell-").unwrap();
        let cli = root.path().join("CLI 中文 ' $() & percent% literal.exe");
        std::fs::write(&cli, b"parser-only fixture").unwrap();
        let (shell, script) = probe_shell_command(&cli, "0123456789abcdef").unwrap();
        let shell = shell.unwrap();
        assert_eq!(
            shell,
            hiroute_platform::windows::launcher::system_executable(Path::new(
                "WindowsPowerShell/v1.0/powershell.exe"
            ))
            .unwrap()
        );
        let parser = r#"[Console]::OutputEncoding=[Text.Encoding]::UTF8; $tokens=$null; $errors=$null; $ast=[System.Management.Automation.Language.Parser]::ParseInput($env:HIROUTE_TEST_COMMAND,[ref]$tokens,[ref]$errors); if($errors.Count -ne 0){exit 1}; $calls=@($ast.FindAll({param($node) $node -is [System.Management.Automation.Language.CommandAst] -and $node.InvocationOperator -eq 'Ampersand'},$true)); if($calls.Count -ne 1){exit 2}; @($calls[0].CommandElements | ForEach-Object {if($_ -is [System.Management.Automation.Language.StringConstantExpressionAst]){$_.Value}else{$_.Extent.Text}}) | ConvertTo-Json -Compress"#;
        let payload = base64::engine::general_purpose::STANDARD.encode(
            parser
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let mut command = Command::new(shell);
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &payload,
            ])
            .env("HIROUTE_TEST_COMMAND", script);
        let output =
            super::super::windows_runtime::bounded(&command, true, Duration::from_secs(10), 8192)
                .unwrap();
        let text = std::str::from_utf8(&output)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let args: Vec<String> = serde_json::from_str(text).unwrap();
        let expected = super::super::managed_launch::windows_executable_path(&cli).unwrap();
        assert_eq!(
            args,
            [
                expected.to_str().unwrap(),
                "schema",
                "list",
                "--output",
                "json"
            ]
        );
    }
}
