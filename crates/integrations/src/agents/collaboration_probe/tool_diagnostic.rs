//! Closed observations, never authority, for Codex rust-v0.160.0 / a956835d.
//! The fixed header is core/src/tools/context.rs::ExecCommandToolOutput;
//! preflight captions are tools/handlers/{mod,unified_exec,unified_exec/exec_command}.rs.
//! No tool text, command, path, nonce or live session identifier is retained.
use super::{launch_diagnostic, readonly_control::ReceiptFacts};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SelectedTool {
    Unknown,
    ExecCommand,
    ShellCommand,
    Bash,
}

impl SelectedTool {
    pub(super) fn from_name(name: &str) -> Self {
        match name {
            "exec_command" => Self::ExecCommand,
            "shell_command" => Self::ShellCommand,
            "Bash" => Self::Bash,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum OutputShape {
    Missing,
    Null,
    Text,
    ContentItems,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReturnPhase {
    Unknown,
    HandlerPreflight,
    TerminalReturned,
    LiveSessionReturned,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum PreflightCause {
    Unknown,
    ArgumentsParse,
    UnsupportedPayload,
    EnvironmentUnavailable,
    TtyDisabled,
    LoginDisabled,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ToolHeader {
    pub(super) phase: ReturnPhase,
    pub(super) preflight_cause: PreflightCause,
    pub(super) wall_time_micros: Option<u64>,
    pub(super) terminal_code: Option<i32>,
    pub(super) live_session: bool,
    pub(super) header_malformed: bool,
}

impl ToolHeader {
    fn unknown() -> Self {
        Self {
            phase: ReturnPhase::Unknown,
            preflight_cause: PreflightCause::Unknown,
            wall_time_micros: None,
            terminal_code: None,
            live_session: false,
            header_malformed: false,
        }
    }

    fn capture(text: &str) -> Self {
        let mut facts = Self::unknown();
        let message = text.trim().strip_prefix("Error: ").unwrap_or(text.trim());
        // Only captions emitted before command launch establish this phase.
        // A generic exec_command failure may also occur after a process started.
        facts.preflight_cause = match message {
            "exec_command handler received unsupported payload" => {
                PreflightCause::UnsupportedPayload
            }
            "unified exec is unavailable in this session" => PreflightCause::EnvironmentUnavailable,
            "TTY execution is disabled by config; omit `tty` or set it to false." => {
                PreflightCause::TtyDisabled
            }
            "login shell is disabled by config; omit `login` or set it to false." => {
                PreflightCause::LoginDisabled
            }
            _ if message
                .strip_prefix("failed to parse function arguments: ")
                .is_some_and(|reason| !reason.is_empty()) =>
            {
                PreflightCause::ArgumentsParse
            }
            _ => PreflightCause::Unknown,
        };
        if facts.preflight_cause != PreflightCause::Unknown {
            facts.phase = ReturnPhase::HandlerPreflight;
            return facts;
        }
        let mut lines = text.lines();
        let mut line = lines.next();
        if line.is_some_and(|line| line.starts_with("Chunk ID: ")) {
            line = lines.next();
        }
        let Some(wall) = line.and_then(|line| line.strip_prefix("Wall time: ")) else {
            return facts;
        };
        // A partial/ambiguous header is unknown, not proof of launch or exit.
        facts.header_malformed = true;
        let Some(wall) = wall.strip_suffix(" seconds").and_then(wall_time_micros) else {
            return facts;
        };
        let Some(status) = lines.next() else {
            return facts;
        };
        let (phase, code, live) =
            if let Some(code) = status.strip_prefix("Process exited with code ") {
                let Some(code) = signed_decimal(code) else {
                    return facts;
                };
                (ReturnPhase::TerminalReturned, Some(code), false)
            } else if let Some(session) = status.strip_prefix("Process running with session ID ") {
                // Validate, then discard; this ID must never enter diagnostics.
                if unsigned_decimal(session).is_none_or(|id| id == 0 || id > i32::MAX as u32) {
                    return facts;
                }
                (ReturnPhase::LiveSessionReturned, None, true)
            } else {
                return facts;
            };
        let mut output = lines.next();
        if let Some(count) = output.and_then(|line| line.strip_prefix("Original token count: ")) {
            if unsigned_decimal(count).is_none() {
                return facts;
            }
            output = lines.next();
        }
        if output != Some("Output:") {
            return facts;
        }
        facts.phase = phase;
        facts.wall_time_micros = Some(wall);
        facts.terminal_code = code;
        facts.live_session = live;
        facts.header_malformed = false;
        facts
    }
}

fn unsigned_decimal(text: &str) -> Option<u32> {
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn signed_decimal(text: &str) -> Option<i32> {
    let digits = text.strip_prefix('-').unwrap_or(text);
    (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn wall_time_micros(text: &str) -> Option<u64> {
    let (seconds, fraction) = text.split_once('.')?;
    let seconds = unsigned_decimal(seconds)?;
    if seconds > 86_400 || fraction.len() != 4 {
        return None;
    }
    Some(u64::from(seconds) * 1_000_000 + u64::from(unsigned_decimal(fraction)?) * 100)
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ToolResultDiagnostic {
    pub(super) selected_tool: SelectedTool,
    pub(super) correlated: bool,
    pub(super) tool_error: bool,
    pub(super) output_shape: OutputShape,
    pub(super) content_items: usize,
    pub(super) text_items: usize,
    pub(super) output_bytes: usize,
    pub(super) expected_cli_bytes: usize,
    pub(super) skill_content: bool,
    pub(super) cli_output: bool,
    pub(super) truncated: bool,
    pub(super) process_launch: bool,
    pub(super) access_denied: bool,
    pub(super) sandbox: bool,
    pub(super) command_not_found: bool,
    pub(super) powershell_parser: bool,
    pub(super) dll_init: bool,
    pub(super) header: ToolHeader,
    pub(super) receipts: Option<ReceiptFacts>,
    pub(super) launch: launch_diagnostic::NativeLaunchDiagnostic,
}

impl ToolResultDiagnostic {
    pub(super) fn capture(
        selected_tool: SelectedTool,
        result: Option<&Value>,
        output: Option<&str>,
        contents: &str,
        expected_cli: &str,
        receipts: Option<ReceiptFacts>,
    ) -> Self {
        let text = output.unwrap_or_default();
        let value = result.and_then(|item| item.get("output"));
        let items = value.and_then(Value::as_array);
        Self {
            selected_tool,
            correlated: result.is_some(),
            tool_error: result.is_some_and(|result| result["is_error"] == true),
            output_shape: match value {
                None => OutputShape::Missing,
                Some(Value::Null) => OutputShape::Null,
                Some(Value::String(_)) => OutputShape::Text,
                Some(Value::Array(_)) => OutputShape::ContentItems,
                _ => OutputShape::Other,
            },
            content_items: items.map_or(0, Vec::len),
            text_items: items.map_or(0, |items| {
                items
                    .iter()
                    .filter(|item| item["type"] == "input_text" && item["text"].is_string())
                    .count()
            }),
            output_bytes: text.len(),
            expected_cli_bytes: expected_cli.len(),
            skill_content: output.is_some_and(|text| text.contains(contents)),
            cli_output: output.is_some_and(|text| text.contains(expected_cli)),
            truncated: text.contains("truncated"),
            process_launch: text.contains("CreateProcess") || text.contains("Failed to spawn"),
            access_denied: text.contains("Access is denied")
                || text.contains("os error 5")
                || text.contains("PermissionDenied"),
            sandbox: text.contains("sandbox") || text.contains("Sandbox"),
            command_not_found: text.contains("not recognized")
                || text.contains("command not found"),
            powershell_parser: text.contains("ParserError") || text.contains("UnexpectedToken"),
            dll_init: ["0xc0000142", "0xC0000142", "-1073741502", "1114"]
                .iter()
                .any(|marker| text.contains(marker)),
            header: ToolHeader::capture(text),
            receipts,
            launch: launch_diagnostic::native_launch_diagnostic(text),
        }
    }

    pub(super) fn emit(&self) {
        eprintln!("native collaboration closed tool result: {self:?}");
        // Keep the existing fixed captions available to evidence consumers.
        eprintln!(
            "native collaboration checkpoint: correlated={} content-items={} skill-content={} cli-output={} output-bytes={} expected-cli-bytes={} truncated={}",
            self.correlated,
            self.output_shape == OutputShape::ContentItems,
            self.skill_content,
            self.cli_output,
            self.output_bytes,
            self.expected_cli_bytes,
            self.truncated
        );
        eprintln!(
            "native collaboration failure flags: process-launch={} access-denied={} sandbox={} command-not-found={} powershell-parser={} dll-init={}",
            self.process_launch,
            self.access_denied,
            self.sandbox,
            self.command_not_found,
            self.powershell_parser,
            self.dll_init
        );
        eprintln!(
            "native collaboration closed launch diagnostic: {:?}",
            self.launch
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(text: &str) -> ToolHeader {
        ToolHeader::capture(text)
    }

    #[test]
    fn official_terminal_and_live_headers_keep_numbers_but_discard_session_and_native_text() {
        let terminal = header(
            "Chunk ID: opaque-private-id\nWall time: 0.0142 seconds\nProcess exited with code -1073741502\nOriginal token count: 29\nOutput:\nprivate-output-path-and-token",
        );
        assert_eq!(terminal.phase, ReturnPhase::TerminalReturned);
        assert_eq!(terminal.wall_time_micros, Some(14_200));
        assert_eq!(terminal.terminal_code, Some(-1073741502));
        assert!(!terminal.live_session);
        assert!(!terminal.header_malformed);
        let live = header(
            "Wall time: 10.0000 seconds\nProcess running with session ID 91234567\nOutput:\nprivate-output-path-and-token",
        );
        assert_eq!(live.phase, ReturnPhase::LiveSessionReturned);
        assert_eq!(live.wall_time_micros, Some(10_000_000));
        assert_eq!(live.terminal_code, None);
        assert!(live.live_session);
        for facts in [terminal, live] {
            let closed = format!("{facts:?}");
            for private in [
                "opaque-private-id",
                "91234567",
                "private-output-path-and-token",
            ] {
                assert!(!closed.contains(private));
            }
        }
    }

    #[test]
    fn header_rejects_partial_duplicate_overflow_and_embedded_receipts() {
        for text in [
            "Wall time: 1.0000 seconds\nProcess exited with code 0",
            "Wall time: 1.0000 seconds\nProcess exited with code 0\nProcess exited with code 1\nOutput:",
            "Wall time: NaN seconds\nProcess exited with code 0\nOutput:",
            "Wall time: -1.0000 seconds\nProcess exited with code 0\nOutput:",
            "Wall time: 86401.0000 seconds\nProcess exited with code 0\nOutput:",
            "Wall time: 1.0000 seconds\nProcess exited with code 2147483648\nOutput:",
            "Wall time: 1.0000 seconds\nProcess exited with code 0suffix\nOutput:",
            "Wall time: 1.0000 seconds\nProcess exited with code +5\nOutput:",
            "Wall time: 1.0000 seconds\nProcess running with session ID 0\nOutput:",
            "Wall time: 1.0000 seconds\nProcess running with session ID 4294967296\nOutput:",
            "Wall time: 1.0000 seconds\nProcess exited with code 0\nOriginal token count: secret\nOutput:",
        ] {
            let facts = header(text);
            assert_eq!(facts.phase, ReturnPhase::Unknown);
            assert!(facts.header_malformed);
            assert_eq!(facts.wall_time_micros, None);
            assert_eq!(facts.terminal_code, None);
            assert!(!facts.live_session);
        }
        let echoed = header(
            "exec_command failed: private-command-with-'Wall time: 1.0000 seconds'\nProcess exited with code 0\nOutput:",
        );
        assert_eq!(echoed.phase, ReturnPhase::Unknown);
        assert!(!echoed.header_malformed);
    }

    #[test]
    fn only_pinned_prelaunch_captions_establish_handler_preflight() {
        for (text, cause) in [
            (
                "failed to parse function arguments: private-json-line-1",
                PreflightCause::ArgumentsParse,
            ),
            (
                "exec_command handler received unsupported payload",
                PreflightCause::UnsupportedPayload,
            ),
            (
                "unified exec is unavailable in this session",
                PreflightCause::EnvironmentUnavailable,
            ),
            (
                "Error: TTY execution is disabled by config; omit `tty` or set it to false.",
                PreflightCause::TtyDisabled,
            ),
            (
                "login shell is disabled by config; omit `login` or set it to false.",
                PreflightCause::LoginDisabled,
            ),
        ] {
            let facts = header(text);
            assert_eq!(facts.phase, ReturnPhase::HandlerPreflight);
            assert_eq!(facts.preflight_cause, cause);
            assert_eq!(facts.terminal_code, None);
            assert!(!format!("{facts:?}").contains("private-json"));
        }
        for text in [
            "exec_command failed: unclassified reason",
            "private-command says unified exec is unavailable in this session",
            "failed to parse function arguments: ",
            "unknown text",
        ] {
            assert_eq!(header(text).phase, ReturnPhase::Unknown);
        }
    }

    #[test]
    fn facets_are_closed_and_shape_counts_do_not_invent_parsed_text() {
        let output = "Wall time: 0.0100 seconds\nProcess exited with code 1\nOutput:\nParserError private-command-path-token";
        let value = serde_json::json!({"output":[{"type":"input_image","text":"private-image"},{"type":"input_text","text":output},{"type":"input_text","text":7}]});
        let facts = ToolResultDiagnostic::capture(
            SelectedTool::ExecCommand,
            Some(&value),
            Some(output),
            "contents",
            "cli",
            None,
        );
        assert_eq!(facts.output_shape, OutputShape::ContentItems);
        assert_eq!(facts.content_items, 3);
        assert_eq!(facts.text_items, 1);
        assert_eq!(facts.output_bytes, output.len());
        assert!(facts.powershell_parser);
        assert_eq!(facts.header.phase, ReturnPhase::TerminalReturned);
        assert!(!format!("{facts:?}").contains("private-"));
        for (value, shape) in [
            (serde_json::json!({}), OutputShape::Missing),
            (serde_json::json!({"output":null}), OutputShape::Null),
            (
                serde_json::json!({"output":{ "text":"private" }}),
                OutputShape::Other,
            ),
        ] {
            let facts = ToolResultDiagnostic::capture(
                SelectedTool::Unknown,
                Some(&value),
                None,
                "contents",
                "cli",
                None,
            );
            assert_eq!(facts.output_shape, shape);
            assert_eq!(facts.output_bytes, 0);
            assert_eq!(facts.header.phase, ReturnPhase::Unknown);
        }
    }

    #[test]
    fn terminal_script_checkpoint_retains_failure_facts_without_proving_denial() {
        use super::super::readonly_control::{LanguageMode, ReadOnlyControl, ScriptPhase};
        let root = tempfile::tempdir().unwrap();
        let mut control = ReadOnlyControl::new(root.path(), "private-nonce").unwrap();
        let output = "Wall time: 3.7322 seconds\nProcess exited with code 1\nOutput:\nhiroute-readonly-entered-private-nonce\nhiroute-readonly-mode-private-nonce:ConstrainedLanguage\nhiroute-readonly-failure-private-nonce:console-attempt:-2146233087:7\nprivate-native-error-text";
        let result = serde_json::json!({"output":output});
        let diagnostic = ToolResultDiagnostic::capture(
            SelectedTool::ExecCommand,
            Some(&result),
            Some(output),
            "private-content-nonce",
            "private-cli-contract",
            Some(control.receipt_facts(output)),
        );
        assert_eq!(diagnostic.header.phase, ReturnPhase::TerminalReturned);
        assert_eq!(diagnostic.header.terminal_code, Some(1));
        let receipt = diagnostic.receipts.as_ref().unwrap();
        assert_eq!(receipt.attempt_matches, 0);
        assert_eq!(receipt.denial_matches, 0);
        assert_eq!(receipt.script.entered_matches, 1);
        assert_eq!(receipt.script.mode, LanguageMode::ConstrainedLanguage);
        let failure = receipt.script.failure.as_ref().unwrap();
        assert_eq!(failure.phase, ScriptPhase::ConsoleAttempt);
        assert_eq!(failure.hresult, -2146233087);
        assert_eq!(failure.category, 7);
        assert!(!format!("{diagnostic:?}").contains("private-"));
        assert!(control.confirm(output).is_err());
        assert!(!diagnostic.skill_content && !diagnostic.cli_output);
    }
}
