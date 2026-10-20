//! A deterministic Responses peer: native Skill discovery, exact user path/body, then one CLI.
//! Requests stay in memory and are never forwarded or included in diagnostics.
#[path = "qoder_probe_cli_diagnostic.rs"]
mod cli_diagnostic;

use serde_json::{Value, json};
use std::net::TcpStream;

pub(super) struct Challenge {
    pub skill_name: String,
    #[cfg(any(test, not(windows)))]
    pub skill_directory: String,
    pub skill_body: String,
    pub command: String,
    pub expected_cli: String,
    pub step: usize,
    #[cfg(windows)]
    pub expected_skill: Option<super::skill_diagnostic::NativeSkill>,
    #[cfg(windows)]
    pub expected_workspace: Option<super::workspace::Receipt>,
}

impl Challenge {
    pub fn complete(&self) -> bool {
        self.step == 3
    }

    fn reply(&mut self, body: &Value) -> Result<Value, &'static str> {
        let input = body["input"].as_array().ok_or("native input contract")?;
        let tool_declared = |name| {
            body["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
        };
        let tool_result = |call: &str| {
            input
                .iter()
                .find(|item| item["type"] == "function_call_output" && item["call_id"] == call)
                .and_then(|item| item["output"].as_str())
        };
        let item = match self.step {
            0 => {
                if !body.to_string().contains(&self.skill_name)
                    || !tool_declared("Skill")
                    || !tool_declared("Bash")
                {
                    return Err("native Skill discovery");
                }
                function("skill", "Skill", json!({"skill": self.skill_name}))
            }
            1 => {
                if tool_result("call_hiroute_skill")
                    != Some(format!("Launching skill: {}", self.skill_name).as_str())
                {
                    return Err("native Skill call result");
                }
                #[cfg(not(windows))]
                let prefix = format!(
                    "Base directory for this skill: {}\n\n",
                    self.skill_directory
                );
                #[cfg(not(windows))]
                let loaded = input
                    .iter()
                    .filter(|item| item["type"] == "message" && item["role"] == "user")
                    .filter_map(|item| item["content"].as_array())
                    .flatten()
                    .filter_map(|item| item["text"].as_str())
                    .any(|text| {
                        text.strip_prefix(&prefix)
                            .is_some_and(|body| body.trim() == self.skill_body.trim())
                    });
                #[cfg(windows)]
                let loaded = self
                    .expected_skill
                    .as_mut()
                    .is_some_and(|binding| binding.matches_loaded_user(input, &self.skill_body));
                if !loaded {
                    #[cfg(test)]
                    super::skill_diagnostic::observe(
                        input,
                        &self.skill_directory,
                        &self.skill_body,
                        #[cfg(windows)]
                        self.expected_skill.as_mut(),
                    )
                    .emit();
                    return Err("actual native Skill path and contents");
                }
                function(
                    "cli",
                    "Bash",
                    json!({"command": self.command, "timeout": 10_000}),
                )
            }
            2 => {
                let output = tool_result("call_hiroute_cli").ok_or("trusted CLI call result")?;
                let exact = output.trim() == self.expected_cli.trim();
                #[cfg(windows)]
                let exact = exact
                    || self
                        .expected_workspace
                        .as_ref()
                        .is_some_and(|workspace| workspace.matches(output, &self.expected_cli));
                if !exact {
                    if let Some(facts) = cli_diagnostic::Facts::observe(output, &self.expected_cli)
                    {
                        facts.emit();
                    }
                    return Err("trusted CLI result mismatch");
                }
                json!({"id":"msg_hiroute_probe", "type":"message", "status":"completed", "role":"assistant", "content":[{"type":"output_text", "text":"OK", "annotations":[]}]})
            }
            _ => return Err("native request limit"),
        };
        self.step += 1;
        Ok(item)
    }
}

fn function(suffix: &str, name: &str, args: Value) -> Value {
    json!({"id":format!("fc_hiroute_{suffix}"), "type":"function_call", "status":"completed", "call_id":format!("call_hiroute_{suffix}"), "name":name, "arguments":args.to_string()})
}

pub(super) fn serve(
    stream: &mut TcpStream,
    token: &str,
    model: &str,
    challenge: &mut Challenge,
) -> Result<(), &'static str> {
    super::super::native_responses_probe::serve(stream, token, model, |body| challenge.reply(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge() -> Challenge {
        #[cfg(windows)]
        let (directory, expected_skill) = {
            let root = tempfile::tempdir().unwrap();
            let directory = root.path().join("hiroute-collaboration");
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), "confirmed Skill body").unwrap();
            let mut binding = super::super::skill_diagnostic::NativeSkill::retain(
                &directory,
                b"confirmed Skill body",
                false,
            )
            .unwrap();
            let canonical = std::fs::canonicalize(&directory)
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned();
            binding.keep_fixture_root(root);
            (canonical, Some(binding))
        };
        Challenge {
            skill_name: "hiroute-collaboration".into(),
            #[cfg(not(windows))]
            skill_directory: "/user/.qoder/skills/hiroute-collaboration".into(),
            #[cfg(windows)]
            skill_directory: directory,
            skill_body: "confirmed Skill body".into(),
            command: "trusted cli".into(),
            expected_cli: "trusted result".into(),
            step: 1,
            #[cfg(windows)]
            expected_skill,
            #[cfg(windows)]
            expected_workspace: None,
        }
    }

    #[test]
    fn installed_skill_requires_correlated_call_and_actual_user_source() {
        let body = |directory: &str, call: &str| {
            json!({"input":[
                {"type":"function_call_output","call_id":call,"output":"Launching skill: hiroute-collaboration"},
                {"type":"message","role":"user","content":[{"type":"input_text","text":format!("Base directory for this skill: {directory}\n\nconfirmed Skill body")}]}]})
        };
        let mut check = challenge();
        let directory = check.skill_directory.clone();
        assert!(
            check
                .reply(&body(
                    "/project/.qoder/skills/hiroute-collaboration",
                    "call_hiroute_skill"
                ))
                .is_err()
        );
        assert!(check.reply(&body(&directory, "unrelated_call")).is_err());
        assert!(check.reply(&body(&directory, "call_hiroute_skill")).is_ok());
    }

    #[test]
    fn model_claim_cannot_replace_cli_tool_evidence() {
        let mut check = challenge();
        check.step = 2;
        assert!(check.reply(&json!({"input":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"trusted result"}]}]})).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn native_cwd_envelope_keeps_the_correlated_exact_cli_proof() {
        let root = super::super::super::probe_files::temporary("hiroute-qoder-envelope-").unwrap();
        let (path, _held) = super::super::workspace::retain(root.path()).unwrap();
        let mut check = challenge();
        check.step = 2;
        check.expected_workspace = Some(super::super::workspace::Receipt::retain(&path).unwrap());
        let output = format!(
            "{}\n\nShell cwd was reset to {}",
            check.expected_cli,
            path.display()
        );
        assert!(check.reply(&json!({"input":[{"type":"function_call_output","call_id":"unrelated","output":output}]})).is_err());
        assert_eq!(check.step, 2);
        assert!(check.reply(&json!({"input":[{"type":"function_call_output","call_id":"call_hiroute_cli","output":output}]})).is_ok());
        assert!(check.complete());
    }

    #[test]
    fn trusted_cli_still_rejects_semantic_only_framed_and_uncorrelated_outputs() {
        let mut check = challenge();
        check.step = 2;
        check.expected_cli = "{\n\"status\":\"succeeded\"\n}".into();
        for output in [
            "{\"status\":\"succeeded\"}".to_owned(),
            check.expected_cli.replace('\n', "\r\n"),
            format!("stdout:\n{}\nstderr:\n(empty)", check.expected_cli),
            format!("Exit code 2\n{}", check.expected_cli),
        ] {
            assert_eq!(
                check.reply(&json!({"input":[{"type":"function_call_output","call_id":"call_hiroute_cli","output":output}]})).unwrap_err(),
                "trusted CLI result mismatch"
            );
            assert_eq!(check.step, 2);
        }
        let exact = check.expected_cli.clone();
        assert_eq!(
            check.reply(&json!({"input":[{"type":"function_call_output","call_id":"unrelated","output":exact}]})).unwrap_err(),
            "trusted CLI call result"
        );
        assert!(
            check
                .reply(&json!({"input":[{"type":"function_call_output","call_id":"call_hiroute_cli","output":exact}]}))
                .is_ok()
        );
        assert!(check.complete());
    }

    #[test]
    fn skill_receipt_diagnostics_do_not_admit_alternate_path_shape_or_newlines() {
        let request = |content| {
            json!({"input":[
                {"type":"function_call_output","call_id":"call_hiroute_skill","output":"Launching skill: hiroute-collaboration"},
                {"type":"message","role":"user","content":content}
            ]})
        };
        let mut check = challenge();
        let exact = format!(
            "Base directory for this skill: {}\n\nconfirmed Skill body",
            check.skill_directory
        );
        for content in [
            json!([{"type":"input_text","text":format!("Base directory for this skill: /project/foreign\n\nconfirmed Skill body")}]),
            json!(exact.clone()),
            json!([{"type":"input_text","text":exact.replace('\n', "\r\n")}]),
        ] {
            assert_eq!(
                check.reply(&request(content)).unwrap_err(),
                "actual native Skill path and contents"
            );
        }
        assert!(
            check
                .reply(&request(json!([{"type":"input_text","text":exact}])))
                .is_ok()
        );
    }
    #[cfg(windows)]
    #[test]
    fn windows_skill_receipt_requires_held_binding_and_same_candidate_exact_body() {
        let request = |directory: &str, body: &str| {
            json!({"input":[
            {"type":"function_call_output","call_id":"call_hiroute_skill","output":"Launching skill: hiroute-collaboration"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":format!("Base directory for this skill: {directory}\n\n{body}")}]}]})
        };
        let mut check = challenge();
        let directory = check.skill_directory.clone();
        let plain = directory.strip_prefix(r"\\?\").unwrap_or(&directory);
        assert_ne!(plain, directory);
        assert!(check.reply(&request(plain, "wrong body")).is_err());
        assert!(check.reply(&request(plain, "confirmed Skill body")).is_ok());
        let mut missing = challenge();
        let directory = missing.skill_directory.clone();
        missing.expected_skill = None;
        assert!(
            missing
                .reply(&request(&directory, "confirmed Skill body"))
                .is_err()
        );
        let mut changed = challenge();
        let directory = changed.skill_directory.clone();
        std::fs::write(
            std::path::Path::new(&directory).join("SKILL.md"),
            "changed body",
        )
        .unwrap();
        assert!(
            changed
                .reply(&request(&directory, "confirmed Skill body"))
                .is_err()
        );
    }
}
