//! A real Qoder print call selects the persisted provider; no route or credential overlay.
use super::*;

impl LocalControlAdapter {
    #[cfg(unix)]
    pub(super) fn execute_qoder_live_attempt(
        &self,
        context_id: &str,
        model: &str,
        trust: &FrozenExecutionTrustV1,
        deadline: Instant,
    ) -> Result<CanonicalDigest, AttemptFailure> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::process::CommandExt;
        let unavailable = || AttemptFailure {
            executed: false,
            reason: CLIENT_FAILED,
        };
        let executable = self
            .scanner
            .qoder_executable_target()
            .ok_or_else(unavailable)?;
        let context = self
            .scanner
            .qoder_native_context()
            .map_err(|_| unavailable())?;
        let working = tempfile::tempdir().map_err(|_| unavailable())?;
        let safety = working.path().join("safety.json");
        let safety_bytes = serde_json::to_vec(&serde_json::json!({
            "disableAllHooks":true,"general":{"enableAutoUpdate":false,"sessionRetention":{"enabled":false}},
            "plugins":{"autoUpdate":false},"autoMemoryEnabled":false,"autoMemoryUserScopeEnabled":false,
            "dream":{"enabled":false},"promptSuggestionEnabled":false
        })).map_err(|_| unavailable())?;
        use std::io::Write;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&safety)
            .and_then(|mut file| file.write_all(&safety_bytes))
            .map_err(|_| unavailable())?;
        let output_path = working.path().join("native-output.private");
        let output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&output_path)
            .map_err(|_| unavailable())?;
        let native_model = format!(
            "{}/{}",
            hiroute_application::agent_connection::qoder_model_provider_id(context_id),
            model
        );
        let mut command = Command::new(executable);
        command
            .arg("--cwd")
            .arg(working.path())
            .arg("--config-dir")
            .arg(&context.config_root)
            .args(["--setting-sources", "user", "--settings"])
            .arg(&safety)
            .args([
                "--model",
                &native_model,
                "--print",
                "--no-session-persistence",
                "--output-format",
                "stream-json",
                "--strict-mcp-config",
                "--mcp-config",
                "{\"mcpServers\":{}}",
                "--tools",
                "",
                "--permission-mode",
                "dont_ask",
                "--max-model-request-retries",
                "0",
                "-p",
                LIVE_PROMPT,
            ])
            .env_clear()
            .env("HOME", &context.home)
            .env("QODER_CONFIG_DIR", &context.config_root)
            .env("TMPDIR", working.path())
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::null())
            .current_dir(working.path())
            .process_group(0);
        let child = command.spawn().map_err(|_| unavailable())?;
        let mut child = hiroute_integrations::NativeProbeProcess::new(child);
        let failure = |reason| AttemptFailure {
            executed: true,
            reason,
        };
        let success = loop {
            if Instant::now() >= deadline {
                return Err(failure(CHECK_TIMEOUT));
            }
            if std::fs::metadata(&output_path)
                .map_err(|_| failure(CLIENT_OUTPUT_INVALID))?
                .len()
                > OUTPUT_LIMIT as u64
            {
                return Err(failure(CLIENT_OUTPUT_INVALID));
            }
            if let Some(success) = child.observe().map_err(|_| failure(CLIENT_FAILED))? {
                break success;
            }
            thread::sleep(Duration::from_millis(20));
        };
        child.stop().map_err(|_| failure(CLIENT_FAILED))?;
        if !success {
            return Err(failure(CLIENT_FAILED));
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        std::fs::File::open(&output_path)
            .map_err(|_| failure(CLIENT_OUTPUT_INVALID))?
            .take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| failure(CLIENT_OUTPUT_INVALID))?;
        if bytes.len() > OUTPUT_LIMIT {
            return Err(failure(CLIENT_OUTPUT_INVALID));
        }
        let identity = parse_qoder_output(&bytes).ok_or_else(|| failure(CLIENT_OUTPUT_INVALID))?;
        let response_digest = CanonicalDigest::of_bytes(&bytes);
        self.verify_live_receipt(trust, &[identity], &response_digest, deadline)
    }

    #[cfg(not(unix))]
    pub(super) fn execute_qoder_live_attempt(
        &self,
        _context_id: &str,
        _model: &str,
        _trust: &FrozenExecutionTrustV1,
        _deadline: Instant,
    ) -> Result<CanonicalDigest, AttemptFailure> {
        Err(AttemptFailure {
            executed: false,
            reason: CLIENT_FAILED,
        })
    }
}

fn parse_qoder_output(bytes: &[u8]) -> Option<NativeAgentObservationIdentityV1> {
    let mut result = None;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.iter().all(u8::is_ascii_whitespace))
    {
        let value: serde_json::Value = serde_json::from_slice(line).ok()?;
        if value["type"] == "result" {
            if result.is_some()
                || value["subtype"] != "success"
                || value["is_error"] != false
                || value["result"].as_str()?.trim() != LIVE_PROMPT_RESPONSE
            {
                return None;
            }
            let session_id = value["session_id"].as_str()?;
            if !valid_native_identity(session_id) {
                return None;
            }
            result = Some(NativeAgentObservationIdentityV1::QoderSession {
                session_id: session_id.into(),
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qoder_live_requires_successful_terminal_marker_and_exact_native_identity() {
        let valid = br#"{"type":"result","subtype":"success","is_error":false,"result":"HIROUTE_LIVE_CHECK_OK","session_id":"native-session"}"#;
        assert!(
            matches!(parse_qoder_output(valid),Some(NativeAgentObservationIdentityV1::QoderSession{session_id}) if session_id=="native-session")
        );
        for changed in [
            String::from_utf8(valid.to_vec())
                .unwrap()
                .replace("\"is_error\":false", "\"is_error\":true"),
            String::from_utf8(valid.to_vec())
                .unwrap()
                .replace("HIROUTE_LIVE_CHECK_OK", "echo only"),
            format!(
                "{}\n{}",
                std::str::from_utf8(valid).unwrap(),
                std::str::from_utf8(valid).unwrap()
            ),
        ] {
            assert!(parse_qoder_output(changed.as_bytes()).is_none());
        }
    }
}
