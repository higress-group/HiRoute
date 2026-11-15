//! An explicitly requested native compatibility check. Configuration overrides exist only in
//! the child process; no managed or daily configuration file is created or restored.
use super::*;
use hiroute_platform::desktop_engine::DesktopEngineLocation;

impl CodexNativeIngressProbe {
    pub fn run_ephemeral(
        self,
        executable: &Path,
    ) -> Result<CodexIngressEvidence, NativeIngressProbeError> {
        self.run_isolated(&resolve(executable)?, None)
    }

    pub(in crate::agents) fn run_resolved_ephemeral(
        self,
        executable: &DesktopEngineLocation,
    ) -> Result<CodexIngressEvidence, NativeIngressProbeError> {
        #[cfg(windows)]
        let _hold = match executable {
            DesktopEngineLocation::Package(package) => Some(
                package
                    .reopen()
                    .map_err(|_| fail("registered package executable"))?,
            ),
            DesktopEngineLocation::File(_) => None,
        };
        self.run_isolated(executable.path(), None)
    }

    pub fn run_collaboration(
        self,
        executable: &Path,
        cli: &Path,
    ) -> Result<CodexIngressEvidence, NativeIngressProbeError> {
        self.run_isolated(&resolve(executable)?, Some(cli))
    }

    fn run_isolated(
        self,
        binary: &Path,
        cli: Option<&Path>,
    ) -> Result<CodexIngressEvidence, NativeIngressProbeError> {
        let collaboration = cli.is_some();
        if super::super::codex_profile::codex_has_system_configuration()
            .map_err(|_| fail("organization configuration isolation"))?
        {
            return Err(fail("organization configuration isolation"));
        }
        let root = super::super::probe_files::temporary("hiroute-native-auth-")
            .map_err(|_| fail("private check directory"))?;
        let root_path =
            fs::canonicalize(root.path()).map_err(|_| fail("private check directory"))?;
        private_directory(&root_path)?;
        let home = root_path.join("home");
        let codex_home = home.join(".codex");
        let workspace = root_path.join("workspace");
        for path in [&home, &codex_home, &workspace] {
            super::super::probe_files::directory(path)
                .map_err(|_| fail("private check directory"))?;
            private_directory(path)?;
        }
        let mut challenge = cli
            .map(|cli| http::CollaborationChallenge::prepare(&home, &codex_home, cli))
            .transpose()
            .map_err(fail)?;
        // No shell interpolation, secret argv, inherited auth, project rules or managed file.
        // The authentication override tests native HTTP header consumption independently from
        // the separately observed managed-file rendering/atomic-replace capabilities.
        // This public test marker has no product/upstream authority. It exercises the SAME
        // experimental_bearer_token field emitted by the native managed configuration renderer.
        const PUBLIC_MARKER: &str = "hiroute-native-probe-no-authority";
        let provider = format!(
            "model_providers.hiroute_native_probe={{name=\"HiRoute native check\",base_url={},wire_api=\"responses\",experimental_bearer_token=\"hiroute-native-probe-no-authority\",requires_openai_auth=false}}",
            serde_json::to_string(&self.endpoint).map_err(|_| fail("endpoint encoding"))?
        );
        let mut command = Command::new(binary);
        command.arg("exec").arg("--ignore-rules");
        #[cfg(windows)]
        if collaboration {
            // Codex 0.160.0 / a956835d core/exec_policy.rs rejects unmatched
            // commands under readonly + Never when the Windows backend is Disabled.
            // Select the actual restricted-token backend, retaining both restrictions.
            // Never use an execpolicy allow rule: all explicit matches bypass sandboxing.
            command.args(["-c", "windows.sandbox=\"unelevated\""]);
        }
        command
            .args([
                "--ephemeral",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "--json",
                "-c",
                "model_provider=\"hiroute_native_probe\"",
                "-c",
                "model=\"hiroute/0011223344556677\"",
                "-c",
                &provider,
                if cli.is_some() {
                    "Use $hiroute-probe for the read-only local compatibility check."
                } else {
                    "Reply with OK only. Do not use tools."
                },
            ])
            .env_clear()
            .env("HOME", &home)
            .env("CODEX_HOME", &codex_home)
            .env("TMPDIR", &workspace)
            .env("PATH", native_probe_path())
            .current_dir(&workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        let child = command
            .process_group(0)
            .spawn()
            .map_err(|_| fail("native launch"))?;
        #[cfg(windows)]
        let child = super::super::windows_runtime::spawn(
            &command,
            false,
            hiroute_platform::windows::job::Stream::Null,
            hiroute_platform::windows::job::Stream::Null,
            hiroute_platform::windows::job::Stream::Null,
        )
        .map_err(|_| fail("native launch"))?;
        let mut child = super::super::NativeProbeProcess::new(child);
        let started = Instant::now();
        let mut requests = 0;
        let result = (|| {
            loop {
                match self.listener.accept() {
                    Ok((mut stream, _)) => {
                        requests += 1;
                        if requests > 4 {
                            return Err(fail("request bound"));
                        }
                        http::serve_challenge(
                            &mut stream,
                            PUBLIC_MARKER.as_bytes(),
                            self.model.as_str(),
                            challenge.as_mut(),
                        )
                        .map_err(fail)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => return Err(fail("loopback accept")),
                }
                if let Some(success) = child.observe().map_err(|_| fail("native wait"))? {
                    return if success
                        && requests > 0
                        && challenge.as_ref().is_none_or(|check| check.complete())
                    {
                        Ok(())
                    } else {
                        Err(fail("authenticated native completion"))
                    };
                }
                if started.elapsed() > Duration::from_secs(45) {
                    return Err(fail("deadline"));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        // Reap the process before the private directory is reclaimed on either outcome.
        if child.stop().is_err() {
            let _ = root.keep();
            return Err(fail("native cleanup"));
        }
        result?;
        if let Some(challenge) = &challenge {
            challenge.verify_after_stop().map_err(fail)?;
        }
        Ok(CodexIngressEvidence {
            observed: Instant::now(),
            observed_at: now(),
            collaboration,
        })
    }
}

fn resolve(executable: &Path) -> Result<std::path::PathBuf, NativeIngressProbeError> {
    super::super::executable::resolve(executable)
        .map_err(|_| fail("trusted executable"))?
        .ok_or_else(|| fail("executable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires explicitly selected trusted native Codex binary"]
    fn real_codex_ephemeral_authentication_without_managed_configuration() {
        let executable = std::path::PathBuf::from(
            std::env::var_os("HIROUTE_NATIVE_CODEX").expect("native executable"),
        );
        CodexNativeIngressProbe::bind()
            .unwrap()
            .run_ephemeral(&executable)
            .unwrap();
    }
}
