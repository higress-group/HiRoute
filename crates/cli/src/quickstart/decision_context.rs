//! Omitted edit flags retain saved intent; an explicit Automatic choice clears custom caps.
use super::*;
use hiroute_domain::{DecisionContextModeV1, DecisionContextPolicyV1, TypeSafeLongContextV1};

pub(super) fn policy(flags: &Flags, existing: Option<&Value>) -> Result<DecisionContextPolicyV1> {
    let mut policy: DecisionContextPolicyV1 = existing
        .and_then(|service| service.get("context"))
        .and_then(|context| context.get("policy"))
        .map(|policy| {
            serde_json::from_value(policy.clone()).map_err(|_| error(ErrorCode::InvalidArguments))
        })
        .transpose()?
        .unwrap_or_default();
    if flags.has("--context-window-auto") && flags.has("--context-window-tokens") {
        return Err(error(ErrorCode::InvalidArguments));
    }
    if flags.has("--context-window-auto") {
        policy.mode = DecisionContextModeV1::Automatic;
        policy.window_tokens = None;
        policy.per_question_tokens = None;
    }
    if let Some(value) = flags.get("--context-window-tokens") {
        policy.mode = DecisionContextModeV1::Custom;
        policy.window_tokens = Some(positive(value)?);
    }
    if let Some(value) = flags.get("--context-question-tokens") {
        policy.per_question_tokens = Some(positive(value)?);
    }
    if let Some(value) = flags.get("--typesafe-long-context") {
        policy.typesafe_long_context = match value {
            "auto" => TypeSafeLongContextV1::Automatic,
            "off" => TypeSafeLongContextV1::Disabled,
            _ => return Err(error(ErrorCode::InvalidArguments)),
        };
    }
    if !policy.validate() {
        return Err(error(ErrorCode::InvalidArguments));
    }
    Ok(policy)
}

fn positive(value: &str) -> Result<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(error(ErrorCode::InvalidArguments));
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| error(ErrorCode::InvalidArguments))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn flags(values: &[&str]) -> Flags {
        Flags::parse(
            &values.iter().map(|v| (*v).into()).collect::<Vec<_>>(),
            &[
                "--context-window-tokens",
                "--context-question-tokens",
                "--typesafe-long-context",
            ],
            &[],
            &["--context-window-auto"],
        )
        .unwrap()
    }
    #[test]
    fn preserves_saved_intent_and_restores_automatic_explicitly() {
        let saved = json!({"context":{"policy":{"mode":"custom","window_tokens":100000,"per_question_tokens":40000,"typesafe_long_context":"disabled"}}});
        let retained = policy(&flags(&[]), Some(&saved)).unwrap();
        assert_eq!(retained.window_tokens, Some(100000));
        assert_eq!(retained.per_question_tokens, Some(40000));
        let automatic = policy(&flags(&["--context-window-auto"]), Some(&saved)).unwrap();
        assert_eq!(automatic.mode, DecisionContextModeV1::Automatic);
        assert_eq!(automatic.window_tokens, None);
        assert_eq!(automatic.per_question_tokens, None);
        assert_eq!(
            automatic.typesafe_long_context,
            TypeSafeLongContextV1::Disabled
        );
        assert_eq!(
            policy(&flags(&[]), None).unwrap(),
            DecisionContextPolicyV1::default()
        );
    }
    #[test]
    fn rejects_invalid_unsigned_windows_caps_and_conflicting_modes() {
        for invalid in ["0", "-1", "1.5", "4294967296", "+32000"] {
            assert!(policy(&flags(&["--context-window-tokens", invalid]), None).is_err());
        }
        assert!(
            policy(
                &flags(&["--context-window-tokens", "32000", "--context-window-auto"]),
                None
            )
            .is_err()
        );
        assert!(policy(&flags(&["--context-question-tokens", "10000"]), None).is_err());
        assert!(
            policy(
                &flags(&[
                    "--context-window-tokens",
                    "32000",
                    "--context-question-tokens",
                    "32001"
                ]),
                None
            )
            .is_err()
        );
    }
}
