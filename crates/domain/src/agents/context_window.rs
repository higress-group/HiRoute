//! Client-specific adaptation of a Plan's context budget.
use std::collections::BTreeMap;

pub const CLAUDE_CONTEXT_ENVIRONMENT: [&str; 2] = [
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
];
pub const CLAUDE_CONTEXT_CONFLICT_ENVIRONMENT: [&str; 7] = [
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE",
    "CLAUDE_CODE_DISABLE_1M_CONTEXT",
    "CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT",
    "DISABLE_COMPACT",
    "DISABLE_AUTO_COMPACT",
];

/// Claude's documented auto-compact range is 100K..1M. Never round a small Plan up.
pub fn claude_context_window(tokens: u64) -> Option<u64> {
    (tokens >= 100_000).then_some(tokens.min(1_000_000))
}

pub fn claude_context_environment(tokens: u64) -> Option<BTreeMap<String, String>> {
    let window = claude_context_window(tokens)?.to_string();
    Some(
        CLAUDE_CONTEXT_ENVIRONMENT
            .into_iter()
            .map(|key| (key.to_owned(), window.clone()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn claude_window_never_expands_a_small_plan() {
        assert_eq!(claude_context_window(99_999), None);
        assert_eq!(claude_context_window(100_000), Some(100_000));
        assert_eq!(claude_context_window(272_000), Some(272_000));
        assert_eq!(claude_context_window(1_050_000), Some(1_000_000));
        let env = claude_context_environment(272_000).unwrap();
        assert_eq!(env["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "272000");
        assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "272000");
    }
}
