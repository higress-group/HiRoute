//! Project only the production closed stage after bounded capture and secret checks.
#[path = "../../../integrations/src/agents/qoder_probe_cli_diagnostic.rs"]
mod cli_diagnostic;
#[path = "../../src/control/runtime/agent_connection_qoder_diagnostic.rs"]
mod diagnostic;

pub(super) fn project(bytes: &[u8]) {
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let native = line
            .strip_prefix(b"native collaboration check: native ingress probe could not establish ")
            .map(|reason| ("collaboration", reason))
            .or_else(|| {
                line.strip_prefix(
                    b"native authentication check: native ingress probe could not establish ",
                )
                .map(|reason| ("authentication", reason))
            });
        if let Some((scope, reason)) = native {
            let stage = match reason {
                b"trusted CLI baseline" => "cli-baseline",
                b"trusted CLI contract" | b"CLI contract" => "cli-contract",
                b"authenticated native completion" => "native-completion",
                b"native read-only write rejection" => "readonly-control",
                b"native Skill read and trusted CLI execution" => "tool-receipts",
                b"native Skill discovery" => "skill-discovery",
                b"native launch" => "native-launch",
                b"deadline" => "deadline",
                b"wrong authentication" => "authentication-header",
                b"wrong model" => "model-selection",
                b"stable executable" => "executable-changed",
                b"mode" => "http-socket-mode",
                b"header read" => "http-header-read",
                b"short header" => "http-header-eof",
                b"body read" => "http-body-read",
                b"short body" => "http-body-eof",
                b"request bound" => "http-request-bound",
                b"native wait" => "native-wait",
                b"native cleanup" => "native-cleanup",
                _ => "other-native-check",
            };
            eprintln!("Windows native check daemon failure: scope={scope} stage={stage}");
        }
        if let Some(facts) = cli_diagnostic::Facts::parse(line) {
            facts.emit();
        }
        if let Some(stage) = diagnostic::line(line) {
            eprintln!("{}{}", diagnostic::PREFIX, stage);
        }
    }
}

pub(super) fn collect(reader: &mut Option<std::thread::JoinHandle<Vec<u8>>>, stderr: bool) {
    if let Some(reader) = reader.take() {
        let bytes = reader
            .join()
            .expect("bounded daemon output collector failed");
        super::assert_no_secret(&bytes);
        if stderr {
            if std::thread::panicking()
                && let Some(root) = std::env::var_os("HIROUTE_VALIDATION_EVIDENCE_DIR")
            {
                // Preserve raw failure output privately; stdout/stderr is not
                // an allowed source for production structured diagnostics.
                let retain = || -> std::io::Result<()> {
                    use hiroute_platform::windows::security;
                    use std::io::Write;
                    let root = std::path::PathBuf::from(root);
                    security::open_identity(&root, true)?;
                    let mut file =
                        security::create_owner_only_file(&root.join("daemon-stderr.bin"))?;
                    file.write_all(&bytes)?;
                    file.sync_all()
                };
                if retain().is_err() {
                    eprintln!("Windows daemon failure output retention could not complete");
                }
            }
            project(&bytes);
        }
    }
}

/// Save the final writer bytes before the disposable installation is removed.
/// Periodic out-of-process collection can otherwise miss the failure itself.
pub(super) fn retain(root: &std::path::Path) {
    use hiroute_platform::windows::security;
    use std::io::Write;
    let Some(destination) = std::env::var_os("HIROUTE_VALIDATION_EVIDENCE_DIR") else {
        return;
    };
    let save = || -> std::io::Result<()> {
        let destination = std::path::PathBuf::from(destination);
        // The runner owns this existing directory; its inherited ACL need not
        // match the product's private-data ACL. Each captured file is private.
        security::open_identity(&destination, true)?;
        let mut index = 0;
        for role in ["daemon", "desktop"] {
            let directory = root.join(role);
            if !directory.exists() {
                continue;
            }
            security::open_owner_only(&directory, true)?;
            for entry in std::fs::read_dir(directory)? {
                let path = entry?.path();
                if path
                    .extension()
                    .is_none_or(|extension| extension != "jsonl")
                {
                    continue;
                }
                let mut input = security::open_owner_only(&path, false)?;
                if input.metadata()?.len() > 8 * 1024 * 1024 {
                    continue;
                }
                let path = destination.join(format!("diagnostics-final-{index}.jsonl"));
                let mut output = security::create_owner_only_file(&path)?;
                std::io::copy(&mut input, &mut output)?;
                output.flush()?;
                index += 1;
            }
        }
        Ok(())
    };
    if save().is_err() {
        eprintln!("Windows failure diagnostic retention could not complete");
    }
}
