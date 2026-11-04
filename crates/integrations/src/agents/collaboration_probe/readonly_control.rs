//! An independent negative control for the real Windows Codex shell sandbox.
//! A correlated native shell must enter the fixed script, attempt a real write,
//! catch only UnauthorizedAccessException, and leave the initially absent file absent.
use std::path::{Path, PathBuf};

#[cfg(test)]
mod clm_tests;
#[cfg(test)]
mod parser_probe;
#[cfg(any(windows, test))]
mod script;
#[cfg(all(test, windows))]
mod windows_tests;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ReceiptFacts {
    pub(super) attempt_matches: usize,
    pub(super) denial_matches: usize,
    pub(super) attempt_substring: bool,
    pub(super) denial_substring: bool,
    pub(super) script: ScriptFacts,
    pub(super) pipeline: PipelineFacts,
}

/// Observations from the nonce-bound command, never evidence of sandbox enforcement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LanguageMode {
    Unknown,
    FullLanguage,
    ConstrainedLanguage,
    RestrictedLanguage,
    NoLanguage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScriptPhase {
    Entered,
    ConsoleAttempt,
    FileWrite,
    ConsoleDenial,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ScriptFailure {
    pub(super) phase: ScriptPhase,
    pub(super) hresult: i32,
    pub(super) category: u8,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ScriptFacts {
    pub(super) entered_matches: usize,
    pub(super) mode_matches: usize,
    pub(super) mode: LanguageMode,
    pub(super) failure_matches: usize,
    pub(super) failure: Option<ScriptFailure>,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum ScriptErrorClass {
    Unknown,
    ConstrainedMethodInvocation,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct PipelineFacts {
    pub(super) entered_matches: usize,
    pub(super) mode_matches: usize,
    pub(super) mode: LanguageMode,
    pub(super) failure_matches: usize,
    pub(super) failure: Option<ScriptFailure>,
    pub(super) error_matches: usize,
    pub(super) error_class: ScriptErrorClass,
}

pub(super) struct ReadOnlyControl {
    path: PathBuf,
    command: String,
    attempted: String,
    denied: String,
    entered: String,
    mode_prefix: String,
    failure_prefix: String,
    pipeline_entered: String,
    pipeline_mode_prefix: String,
    pipeline_failure_prefix: String,
    pipeline_error_prefix: String,
    filesystem_denial: String,
    verified: bool,
}

impl ReadOnlyControl {
    #[cfg(any(windows, test))]
    pub(super) fn new(home: &Path, nonce: &str) -> Result<Self, &'static str> {
        let path = home.join(format!("hiroute-readonly-control-{nonce}.txt"));
        require_absent(&path)?;
        let literal = path
            .to_str()
            .ok_or("read-only control path encoding")?
            .replace('\'', "''");
        let attempted = format!("hiroute-readonly-attempt-{nonce}");
        let denied = format!("hiroute-readonly-denied-{nonce}");
        let entered = format!("hiroute-readonly-entered-{nonce}");
        let mode_prefix = format!("hiroute-readonly-mode-{nonce}:");
        let failure_prefix = format!("hiroute-readonly-failure-{nonce}:");
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).map_err(|_| "read-only pipeline entropy")?;
        let pipeline_nonce: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let pipeline_entered = format!("hiroute-readonly-pipeline-entered-{pipeline_nonce}");
        let pipeline_mode_prefix = format!("hiroute-readonly-pipeline-mode-{pipeline_nonce}:");
        let pipeline_failure_prefix =
            format!("hiroute-readonly-pipeline-failure-{pipeline_nonce}:");
        let pipeline_error_prefix = format!("hiroute-readonly-pipeline-error-{pipeline_nonce}:");
        let filesystem_denial = format!(
            "hiroute-readonly-filesystem-denied-{pipeline_nonce}:UnauthorizedAccess:-2147024891:18:GetContentWriterUnauthorizedAccessError"
        );
        // One physical command line prevents a rejected tool's echoed command
        // from containing a receipt as a standalone output line. Policy rejection,
        // invalid syntax and unrelated I/O failures must stay red.
        // Literal output works in CLM without suppressing the actual filesystem
        // operation. The command owner keeps FullLanguage's original .NET gate
        // and requires an independent typed filesystem receipt for CLM.
        let command = script::command(script::Frames {
            literal: &literal,
            attempted: &attempted,
            denied: &denied,
            entered: &entered,
            mode: &mode_prefix,
            failure: &failure_prefix,
            pipeline_entered: &pipeline_entered,
            pipeline_mode: &pipeline_mode_prefix,
            pipeline_failure: &pipeline_failure_prefix,
            pipeline_error: &pipeline_error_prefix,
            filesystem_denial: &filesystem_denial,
        });
        Ok(Self {
            path,
            command,
            attempted,
            denied,
            entered,
            mode_prefix,
            failure_prefix,
            pipeline_entered,
            pipeline_mode_prefix,
            pipeline_failure_prefix,
            pipeline_error_prefix,
            filesystem_denial,
            verified: false,
        })
    }

    pub(super) fn command(&self) -> &str {
        &self.command
    }

    #[cfg(test)]
    fn verified(&self) -> bool {
        self.verified
    }

    #[cfg(test)]
    pub(super) fn full_language_fixture_frames(&self) -> String {
        // Synthetic protocol fixtures use the current producer's exact frames;
        // this helper performs no shell execution or filesystem write.
        format!(
            "{}\n{}FullLanguage\n{}\n{}FullLanguage\n",
            self.pipeline_entered, self.pipeline_mode_prefix, self.entered, self.mode_prefix
        )
    }

    pub(super) fn receipt_facts(&self, output: &str) -> ReceiptFacts {
        ReceiptFacts {
            attempt_matches: matching_lines(output, &self.attempted).count(),
            denial_matches: matching_lines(output, &self.denied).count(),
            attempt_substring: output.contains(&self.attempted),
            denial_substring: output.contains(&self.denied),
            script: self.script_facts(output),
            pipeline: self.pipeline_facts(output),
        }
    }

    fn script_facts(&self, output: &str) -> ScriptFacts {
        let modes: Vec<_> = output
            .lines()
            .filter_map(|line| line.trim().strip_prefix(&self.mode_prefix))
            .collect();
        let mode = language_mode(&modes);
        let failures: Vec<_> = output
            .lines()
            .filter_map(|line| line.trim().strip_prefix(&self.failure_prefix))
            .collect();
        ScriptFacts {
            entered_matches: matching_lines(output, &self.entered).count(),
            mode_matches: modes.len(),
            mode,
            failure_matches: failures.len(),
            failure: match failures.as_slice() {
                [failure] => script_failure(failure),
                _ => None,
            },
        }
    }

    fn pipeline_facts(&self, output: &str) -> PipelineFacts {
        let suffixes = |prefix: &str| -> Vec<String> {
            output
                .lines()
                .filter_map(|line| line.trim().strip_prefix(prefix))
                .map(ToOwned::to_owned)
                .collect()
        };
        let modes = suffixes(&self.pipeline_mode_prefix);
        let failures = suffixes(&self.pipeline_failure_prefix);
        let errors = suffixes(&self.pipeline_error_prefix);
        PipelineFacts {
            entered_matches: matching_lines(output, &self.pipeline_entered).count(),
            mode_matches: modes.len(),
            mode: language_mode(&modes.iter().map(String::as_str).collect::<Vec<_>>()),
            failure_matches: failures.len(),
            failure: match failures.as_slice() {
                [failure] => script_failure(failure),
                _ => None,
            },
            error_matches: errors.len(),
            error_class: match errors.as_slice() {
                [error] if error == "ConstrainedMethodInvocation" => {
                    ScriptErrorClass::ConstrainedMethodInvocation
                }
                _ => ScriptErrorClass::Unknown,
            },
        }
    }

    pub(super) fn confirm(&mut self, output: &str) -> Result<(), &'static str> {
        let attempted = unique_line(output, &self.attempted);
        let denied = unique_line(output, &self.denied);
        let ordered = attempted
            .zip(denied)
            .is_some_and(|(attempt, denial)| attempt < denial);
        let absent = require_absent(&self.path).is_ok();
        let facts = self.receipt_facts(output);
        let supported = |mode_matches, mode: &LanguageMode| {
            mode_matches == 1
                && matches!(
                    mode,
                    LanguageMode::FullLanguage | LanguageMode::ConstrainedLanguage
                )
        };
        let diagnostics_valid = facts.script.entered_matches == 1
            && facts.pipeline.entered_matches == 1
            && supported(facts.script.mode_matches, &facts.script.mode)
            && supported(facts.pipeline.mode_matches, &facts.pipeline.mode)
            && facts.script.mode == facts.pipeline.mode
            && facts.script.failure_matches == 0
            && facts.pipeline.failure_matches == 0
            && facts.pipeline.error_matches == 0
            && matching_lines(output, "hiroute-readonly-write-succeeded")
                .next()
                .is_none();
        let clm = facts.script.mode == LanguageMode::ConstrainedLanguage
            || facts.pipeline.mode == LanguageMode::ConstrainedLanguage;
        let typed = unique_line(output, &self.filesystem_denial);
        let filesystem_prefix = self.filesystem_denial.split_once(':').unwrap().0;
        let typed_rows = output
            .lines()
            .filter(|line| {
                line.trim()
                    .strip_prefix(filesystem_prefix)
                    .is_some_and(|suffix| suffix.starts_with(':'))
            })
            .count();
        let clm_valid = if !clm {
            typed_rows == 0
        } else {
            facts.script.entered_matches == 1
                && facts.pipeline.entered_matches == 1
                && facts.script.mode_matches == 1
                && facts.pipeline.mode_matches == 1
                && typed_rows == 1
                && attempted
                    .zip(typed)
                    .zip(denied)
                    .is_some_and(|((attempt, typed), denial)| attempt < typed && typed < denial)
        };
        #[cfg(test)]
        eprintln!(
            "native collaboration readonly control: shell-attempted={} access-denied={} receipt-order={ordered} file-absent={absent}",
            attempted.is_some(),
            denied.is_some()
        );
        if !ordered || !absent || !diagnostics_valid || !clm_valid {
            return Err("native read-only write rejection");
        }
        self.verified = true;
        Ok(())
    }

    pub(super) fn verify_after_stop(&self) -> Result<(), &'static str> {
        if !self.verified {
            return Err("native read-only write rejection");
        }
        require_absent(&self.path)
    }
}

fn language_mode(modes: &[&str]) -> LanguageMode {
    match modes {
        ["FullLanguage"] => LanguageMode::FullLanguage,
        ["ConstrainedLanguage"] => LanguageMode::ConstrainedLanguage,
        ["RestrictedLanguage"] => LanguageMode::RestrictedLanguage,
        ["NoLanguage"] => LanguageMode::NoLanguage,
        _ => LanguageMode::Unknown,
    }
}

fn script_failure(text: &str) -> Option<ScriptFailure> {
    let mut parts = text.split(':');
    let phase = match parts.next()? {
        "entered" => ScriptPhase::Entered,
        "console-attempt" => ScriptPhase::ConsoleAttempt,
        "file-write" => ScriptPhase::FileWrite,
        "console-denial" => ScriptPhase::ConsoleDenial,
        _ => return None,
    };
    let code = parts.next()?;
    let hresult: i32 = code.parse().ok()?;
    if hresult.to_string() != code {
        return None;
    }
    let code = parts.next()?;
    // Microsoft ErrorCategory has exactly the fixed numeric range 0..=31.
    let category: u8 = code.parse().ok()?;
    if category > 31 || category.to_string() != code || parts.next().is_some() {
        return None;
    }
    Some(ScriptFailure {
        phase,
        hresult,
        category,
    })
}

fn require_absent(path: &Path) -> Result<(), &'static str> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => Err("read-only control file unexpectedly present or unreadable"),
    }
}

fn unique_line(output: &str, marker: &str) -> Option<usize> {
    let mut matching = matching_lines(output, marker);
    let first = matching.next()?;
    matching.next().is_none().then_some(first)
}

fn matching_lines<'a>(output: &'a str, marker: &'a str) -> impl Iterator<Item = usize> + 'a {
    output
        .lines()
        .enumerate()
        .filter_map(move |(index, line)| (line.trim() == marker).then_some(index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_control_requires_actual_attempt_and_access_denial_not_just_absence() {
        let root = tempfile::tempdir().unwrap();
        for output in [
            "",
            "hiroute-readonly-attempt-nonce",
            "hiroute-readonly-denied-nonce",
        ] {
            let mut control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
            assert!(control.confirm(output).is_err());
            assert!(!control.verified());
            assert!(control.verify_after_stop().is_err());
        }
        let mut control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
        control.confirm(&format!("{}Final output:\nhiroute-readonly-attempt-nonce\r\nhiroute-readonly-denied-nonce\r\n", control.full_language_fixture_frames())).unwrap();
        control.verify_after_stop().unwrap();
    }

    #[test]
    fn readonly_control_rejects_echoed_policy_error_and_unrelated_receipts() {
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
        let echoed = format!(
            "CreateProcess {{ message: Rejected(\"`{}` rejected: blocked by policy\") }}",
            control.command()
        );
        assert!(control.confirm(&echoed).is_err());
        assert!(
            control
                .confirm("hiroute-readonly-attempt-other\nhiroute-readonly-denied-other")
                .is_err()
        );
        assert!(!control.verified());
    }

    #[test]
    fn readonly_control_rejects_reversed_or_repeated_receipt_lines() {
        let root = tempfile::tempdir().unwrap();
        for output in [
            "hiroute-readonly-denied-nonce\nhiroute-readonly-attempt-nonce",
            "hiroute-readonly-attempt-nonce\nhiroute-readonly-attempt-nonce\nhiroute-readonly-denied-nonce",
            "hiroute-readonly-attempt-nonce\nhiroute-readonly-denied-nonce\nhiroute-readonly-denied-nonce",
        ] {
            let mut control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
            assert!(control.confirm(output).is_err());
            assert!(!control.verified());
        }
    }

    #[test]
    fn readonly_control_rejects_an_existing_file_and_any_write_before_or_after_receipt() {
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
        std::fs::write(&control.path, b"unexpected write").unwrap();
        assert!(ReadOnlyControl::new(root.path(), "nonce").is_err());
        assert!(
            control
                .confirm("hiroute-readonly-attempt-nonce\nhiroute-readonly-denied-nonce")
                .is_err()
        );
        assert!(!control.verified());
        std::fs::remove_file(&control.path).unwrap();
        control
            .confirm(&format!(
                "{}hiroute-readonly-attempt-nonce\nhiroute-readonly-denied-nonce",
                control.full_language_fixture_frames()
            ))
            .unwrap();
        std::fs::write(&control.path, b"late unexpected write").unwrap();
        assert!(control.verify_after_stop().is_err());
    }

    #[test]
    fn readonly_control_preserves_a_literal_path_and_only_catches_access_denial() {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home 中文 % ! ' &");
        std::fs::create_dir(&home).unwrap();
        let control = ReadOnlyControl::new(&home, "nonce").unwrap();
        assert!(!control.command().contains('\n'));
        assert!(control.command().contains("home 中文 % ! '' &"));
        assert!(
            control
                .command()
                .contains("[System.IO.File]::WriteAllText(")
        );
        assert!(
            control
                .command()
                .contains("catch [System.UnauthorizedAccessException]")
        );
        assert!(control.command().ends_with("; throw }"));
    }

    #[test]
    fn readonly_control_script_facts_are_closed_nonce_bound_and_never_satisfy_denial() {
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "private-nonce").unwrap();
        assert!(
            control
                .command()
                .contains("'hiroute-readonly-entered-private-nonce';")
        );
        for (caption, mode) in [
            ("FullLanguage", LanguageMode::FullLanguage),
            ("ConstrainedLanguage", LanguageMode::ConstrainedLanguage),
            ("RestrictedLanguage", LanguageMode::RestrictedLanguage),
            ("NoLanguage", LanguageMode::NoLanguage),
        ] {
            for (caption_phase, phase) in [
                ("entered", ScriptPhase::Entered),
                ("console-attempt", ScriptPhase::ConsoleAttempt),
                ("file-write", ScriptPhase::FileWrite),
                ("console-denial", ScriptPhase::ConsoleDenial),
            ] {
                let output = format!(
                    "hiroute-readonly-entered-private-nonce\nhiroute-readonly-mode-private-nonce:{caption}\nhiroute-readonly-failure-private-nonce:{caption_phase}:-2146233087:7\nprivate-output-path-token"
                );
                let facts = control.receipt_facts(&output);
                assert_eq!(facts.script.entered_matches, 1);
                assert_eq!(facts.script.mode_matches, 1);
                assert_eq!(facts.script.mode, mode);
                assert_eq!(facts.script.failure_matches, 1);
                assert_eq!(
                    facts.script.failure,
                    Some(ScriptFailure {
                        phase,
                        hresult: -2146233087,
                        category: 7
                    })
                );
                assert_eq!(facts.attempt_matches, 0);
                assert_eq!(facts.denial_matches, 0);
                assert!(!format!("{facts:?}").contains("private-"));
                assert!(control.confirm(&output).is_err());
                assert!(!control.verified());
            }
        }
    }

    #[test]
    fn readonly_control_console_diagnostics_preserve_exact_frames_and_cannot_replace_denial() {
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "console-nonce").unwrap();
        assert!(!control.command().contains("Write-Output"));
        assert!(!control.command().contains('\n'));
        assert!(
            control
                .command()
                .contains("'hiroute-readonly-entered-console-nonce';")
        );
        assert!(
            control
                .command()
                .contains("'hiroute-readonly-mode-console-nonce:FullLanguage'")
        );
        assert!(
            control
                .command()
                .contains("'hiroute-readonly-failure-console-nonce:' +")
        );
        let echo = control.receipt_facts(control.command());
        assert_eq!(echo.script.entered_matches, 0);
        assert_eq!(echo.script.mode_matches, 0);
        assert_eq!(echo.script.failure_matches, 0);
        assert_eq!(echo.attempt_matches, 0);
        assert_eq!(echo.denial_matches, 0);
        let diagnostics = "hiroute-readonly-entered-console-nonce\r\nhiroute-readonly-mode-console-nonce:FullLanguage\r\n";
        let facts = control.receipt_facts(diagnostics);
        assert_eq!(facts.script.entered_matches, 1);
        assert_eq!(facts.script.mode, LanguageMode::FullLanguage);
        assert!(control.confirm(diagnostics).is_err());
        assert!(!control.verified());
        let receipts = format!(
            "{}hiroute-readonly-attempt-console-nonce\r\nhiroute-readonly-denied-console-nonce\r\n",
            control.full_language_fixture_frames()
        );
        control.confirm(&receipts).unwrap();
        control.verify_after_stop().unwrap();
    }

    #[test]
    fn readonly_control_script_facts_reject_duplicate_unknown_and_malformed_failure_rows() {
        let root = tempfile::tempdir().unwrap();
        let control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
        for failure in [
            "unknown:-1:7",
            "console-attempt:+1:7",
            "console-attempt:-01:7",
            "console-attempt:2147483648:7",
            "console-attempt:-1:32",
            "console-attempt:-1:-1",
            "console-attempt:-1:07",
            "console-attempt:-1:7:private-extra",
            "console-attempt:private-error:7",
            "console-attempt:-1:private-category",
        ] {
            let facts = control.receipt_facts(&format!("hiroute-readonly-failure-nonce:{failure}"));
            assert_eq!(facts.script.failure_matches, 1);
            assert_eq!(facts.script.failure, None);
            assert!(!format!("{facts:?}").contains("private-"));
        }
        let facts = control.receipt_facts("hiroute-readonly-mode-nonce:FullLanguage\nhiroute-readonly-mode-nonce:FullLanguage\nhiroute-readonly-failure-nonce:file-write:-1:18\nhiroute-readonly-failure-nonce:file-write:-1:18");
        assert_eq!(facts.script.mode_matches, 2);
        assert_eq!(facts.script.mode, LanguageMode::Unknown);
        assert_eq!(facts.script.failure_matches, 2);
        assert_eq!(facts.script.failure, None);
        for output in [
            "hiroute-readonly-entered-other\nhiroute-readonly-mode-other:FullLanguage\nhiroute-readonly-failure-other:file-write:-1:18",
            "rejected echoed 'hiroute-readonly-entered-nonce'; 'hiroute-readonly-mode-nonce:FullLanguage'; 'hiroute-readonly-failure-nonce:file-write:-1:18'",
        ] {
            let facts = control.receipt_facts(output);
            assert_eq!(facts.script.entered_matches, 0);
            assert_eq!(facts.script.mode_matches, 0);
            assert_eq!(facts.script.failure_matches, 0);
            assert_eq!(facts.script.mode, LanguageMode::Unknown);
        }
    }

    fn pipeline_output(control: &ReadOnlyControl) -> String {
        format!(
            "{}\n{}ConstrainedLanguage\n{}entered:-2146233087:7\n{}ConstrainedMethodInvocation\n",
            control.pipeline_entered,
            control.pipeline_mode_prefix,
            control.pipeline_failure_prefix,
            control.pipeline_error_prefix
        )
    }

    #[test]
    fn readonly_pipeline_captures_before_console_failure_without_authorizing_denial() {
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "old-console-nonce").unwrap();
        let output = pipeline_output(&control);
        let facts = control.receipt_facts(&output);
        // The old Console-only observation loses every checkpoint in this failure.
        assert_eq!(facts.script.entered_matches, 0);
        assert_eq!(facts.script.failure_matches, 0);
        assert_eq!(facts.pipeline.entered_matches, 1);
        assert_eq!(facts.pipeline.mode, LanguageMode::ConstrainedLanguage);
        assert_eq!(
            facts.pipeline.failure,
            Some(ScriptFailure {
                phase: ScriptPhase::Entered,
                hresult: -2146233087,
                category: 7
            })
        );
        assert_eq!(
            facts.pipeline.error_class,
            ScriptErrorClass::ConstrainedMethodInvocation
        );
        assert_eq!(facts.attempt_matches, 0);
        assert_eq!(facts.denial_matches, 0);
        assert!(control.confirm(&output).is_err());
        assert!(!control.verified());
        assert!(!control.pipeline_entered.ends_with("old-console-nonce"));
        let command = control.command();
        assert!(
            command
                .find(&format!("'{}';", control.pipeline_entered))
                .unwrap()
                < command.find("[System.IO.File]::WriteAllText(").unwrap()
        );
        assert!(
            command
                .find(&format!("'{}' +", control.pipeline_failure_prefix))
                .unwrap()
                < command
                    .find(&format!("'{}' +", control.failure_prefix))
                    .unwrap()
        );
        assert!(command.contains("[System.IO.File]::WriteAllText("));
        assert!(command.contains("catch [System.UnauthorizedAccessException]"));
        assert!(command.ends_with("; throw }"));
    }

    #[test]
    fn readonly_pipeline_rejects_echo_wrong_nonce_and_duplicate_frames() {
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
        let other = ReadOnlyControl::new(root.path(), "other").unwrap();
        for output in [control.command().to_owned(), pipeline_output(&other)] {
            let facts = control.receipt_facts(&output);
            assert_eq!(facts.pipeline.entered_matches, 0);
            assert_eq!(facts.pipeline.mode_matches, 0);
            assert_eq!(facts.pipeline.failure_matches, 0);
            assert_eq!(facts.pipeline.error_matches, 0);
            assert!(control.confirm(&output).is_err());
        }
        let output = pipeline_output(&control).repeat(2);
        let facts = control.receipt_facts(&output);
        assert_eq!(facts.pipeline.entered_matches, 2);
        assert_eq!(facts.pipeline.mode_matches, 2);
        assert_eq!(facts.pipeline.mode, LanguageMode::Unknown);
        assert_eq!(facts.pipeline.failure_matches, 2);
        assert_eq!(facts.pipeline.failure, None);
        assert_eq!(facts.pipeline.error_matches, 2);
        assert_eq!(facts.pipeline.error_class, ScriptErrorClass::Unknown);
        assert!(control.confirm(&output).is_err());
        assert!(!control.verified());
    }

    #[test]
    fn readonly_pipeline_rejects_unknown_and_noncanonical_failure_facts() {
        let root = tempfile::tempdir().unwrap();
        let control = ReadOnlyControl::new(root.path(), "nonce").unwrap();
        for failure in [
            "unknown:-1:7",
            "entered:+1:7",
            "entered:-01:7",
            "entered:2147483648:7",
            "entered:-2147483649:7",
            "entered:-1:32",
            "entered:-1:-1",
            "entered:-1:07",
            "entered:-1:7:private-extra",
        ] {
            let output = format!(
                "{}private-mode\n{}{failure}\n{}private-error\n",
                control.pipeline_mode_prefix,
                control.pipeline_failure_prefix,
                control.pipeline_error_prefix
            );
            let facts = control.receipt_facts(&output);
            assert_eq!(facts.pipeline.mode, LanguageMode::Unknown);
            assert_eq!(facts.pipeline.failure_matches, 1);
            assert_eq!(facts.pipeline.failure, None);
            assert_eq!(facts.pipeline.error_class, ScriptErrorClass::Unknown);
            assert!(!format!("{facts:?}").contains("private-"));
        }
        for (code, category) in [(i32::MIN, 0), (i32::MAX, 31)] {
            let facts = control.receipt_facts(&format!(
                "{}file-write:{code}:{category}",
                control.pipeline_failure_prefix
            ));
            assert_eq!(
                facts.pipeline.failure,
                Some(ScriptFailure {
                    phase: ScriptPhase::FileWrite,
                    hresult: code,
                    category
                })
            );
        }
        for error in [
            "constrainedmethodinvocation",
            "ConstrainedMethodInvocation:private-extra",
            "MethodInvocationNotSupportedInConstrainedLanguage",
        ] {
            let facts = control.receipt_facts(&format!("{}{error}", control.pipeline_error_prefix));
            assert_eq!(facts.pipeline.error_class, ScriptErrorClass::Unknown);
        }
    }

    #[test]
    fn readonly_pipeline_terminal_facts_survive_the_original_console_early_failure() {
        use super::super::tool_diagnostic::{ReturnPhase, SelectedTool, ToolResultDiagnostic};
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "private-nonce").unwrap();
        let output = format!(
            "Wall time: 0.9514 seconds\nProcess exited with code 1\nOutput:\n{}private-native-error-path",
            pipeline_output(&control)
        );
        let value = serde_json::json!({"output":output});
        let facts = ToolResultDiagnostic::capture(
            SelectedTool::ExecCommand,
            Some(&value),
            Some(&output),
            "private-skill",
            "private-cli",
            Some(control.receipt_facts(&output)),
        );
        assert!(facts.correlated);
        assert_eq!(facts.header.phase, ReturnPhase::TerminalReturned);
        assert_eq!(facts.header.terminal_code, Some(1));
        assert_eq!(facts.header.wall_time_micros, Some(951400));
        let receipt = facts.receipts.as_ref().unwrap();
        assert_eq!(receipt.script.entered_matches, 0);
        assert_eq!(receipt.pipeline.entered_matches, 1);
        assert_eq!(receipt.pipeline.mode, LanguageMode::ConstrainedLanguage);
        assert_eq!(
            receipt.pipeline.error_class,
            ScriptErrorClass::ConstrainedMethodInvocation
        );
        assert!(!format!("{facts:?}").contains("private-"));
        assert!(!facts.skill_content && !facts.cli_output);
        assert!(control.confirm(&output).is_err());
        assert!(control.verify_after_stop().is_err());
        assert!(!control.verified());
    }

    #[cfg(windows)]
    #[test]
    fn readonly_control_windows_parser_checks_actual_command_without_execution() {
        use std::{process::Command, time::Duration};
        let root = super::super::super::probe_files::temporary("hiroute-readonly-parser-").unwrap();
        let control = ReadOnlyControl::new(root.path(), "parse-only").unwrap();
        let program = hiroute_platform::windows::launcher::system_executable(Path::new(
            "WindowsPowerShell/v1.0/powershell.exe",
        ))
        .unwrap();
        let broken = control.command().strip_suffix('}').unwrap();
        for (text, expected, arm) in [
            (control.command(), "accepted", "complete"),
            (broken, "rejected", "incomplete"),
        ] {
            let text = text.replace('\'', "''");
            let script = parser_probe::script(&text, arm);
            let mut command = Command::new(&program);
            command
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .env_clear()
                .current_dir(root.path());
            parser_probe::check(&command, Duration::from_secs(30), arm, expected).unwrap();
            require_absent(&control.path).unwrap();
        }
        // The actual command was only a string input to Parser.ParseInput.
        assert!(!control.verified());
    }
}
