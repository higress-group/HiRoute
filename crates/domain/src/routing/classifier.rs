use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const COMPLEXITY_CLASSIFIER_REVISION_V1: &str = "hiroute-complexity-v1";
pub const DEFAULT_REST_CLASSIFIER_TIMEOUT_MS: u64 = 3_000;
pub const MAX_REST_CLASSIFIER_TIMEOUT_MS: u64 = 3_600_000;

const DEEP_REASONING_PHRASES: &[&str] = &[
    "architecture",
    "concurrency",
    "performance",
    "security",
    "migration",
    "root cause",
    "multi-objective",
    "tradeoff",
    "trade-off",
    "架构",
    "并发",
    "性能",
    "安全",
    "迁移",
    "根因",
    "多目标",
    "权衡",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComplexityClassifierModeV1 {
    LocalRules,
    DecisionService {
        service: Box<super::DecisionServiceV1>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifierAuthHeaderV1 {
    pub name: String,
    pub value_secret_ref: String,
}

impl ComplexityClassifierModeV1 {
    pub fn validate(&self) -> Result<(), ComplexityClassifierError> {
        match self {
            Self::LocalRules => Ok(()),
            Self::DecisionService { service } => {
                if service.validate() {
                    Ok(())
                } else {
                    Err(ComplexityClassifierError::UnsupportedClassifier)
                }
            }
        }
    }
}

pub(super) fn valid_classifier_transport(
    endpoint: &str,
    timeout_ms: u64,
    auth_header: Option<&ClassifierAuthHeaderV1>,
) -> bool {
    valid_bounded_text(endpoint, 2_048)
        && (1..=MAX_REST_CLASSIFIER_TIMEOUT_MS).contains(&timeout_ms)
        && auth_header.is_none_or(ClassifierAuthHeaderV1::validate)
}

impl ClassifierAuthHeaderV1 {
    fn validate(&self) -> bool {
        valid_header_name(&self.name)
            && !transport_owned_header(&self.name)
            && valid_bounded_text(&self.value_secret_ref, 256)
            && self.value_secret_ref.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
            })
    }
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn transport_owned_header(value: &str) -> bool {
    [
        "host",
        "content-length",
        "transfer-encoding",
        "connection",
        "content-type",
        "te",
        "trailer",
        "upgrade",
    ]
    .iter()
    .any(|owned| value.eq_ignore_ascii_case(owned))
}

fn valid_bounded_text(value: &str, max_chars: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.chars().count() <= max_chars
        && !value.chars().any(char::is_control)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComplexityClassifierV1 {
    pub revision: String,
    pub mode: ComplexityClassifierModeV1,
    #[serde(default)]
    pub user_keywords: Vec<String>,
}

impl ComplexityClassifierV1 {
    pub fn new(user_keywords: Vec<String>) -> Result<Self, ComplexityClassifierError> {
        let value = Self {
            revision: COMPLEXITY_CLASSIFIER_REVISION_V1.to_owned(),
            mode: ComplexityClassifierModeV1::LocalRules,
            user_keywords,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn with_mode(
        user_keywords: Vec<String>,
        mode: ComplexityClassifierModeV1,
    ) -> Result<Self, ComplexityClassifierError> {
        let value = Self {
            revision: COMPLEXITY_CLASSIFIER_REVISION_V1.to_owned(),
            mode,
            user_keywords,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ComplexityClassifierError> {
        if self.revision != COMPLEXITY_CLASSIFIER_REVISION_V1 || self.user_keywords.len() > 64 {
            return Err(ComplexityClassifierError::UnsupportedClassifier);
        }
        self.mode.validate()?;
        let mut seen = BTreeSet::new();
        for keyword in &self.user_keywords {
            let folded = keyword.to_lowercase();
            if !(1..=64).contains(&keyword.chars().count())
                || keyword.trim() != keyword
                || keyword.chars().any(char::is_control)
                || !seen.insert(folded)
            {
                return Err(ComplexityClassifierError::InvalidKeyword);
            }
        }
        Ok(())
    }

    /// Classifies only the latest normalized human task plus explicitly extracted structural
    /// facts. Provider/model facts, prompt history, Tool schema size, Vision, Context, price, and
    /// health are deliberately absent from this input contract.
    pub fn classify(
        &self,
        input: &ComplexityInputV1,
    ) -> Result<ComplexityDecisionV1, ComplexityClassifierError> {
        self.validate()?;
        input.validate()?;
        if let Some(branch) = input.continuation_branch {
            return Ok(ComplexityDecisionV1 {
                branch,
                score: 0,
                reason_codes: vec!["continuation_inherited".to_owned()],
            });
        }
        let Some(text) = input.human_text.as_deref() else {
            return Ok(ComplexityDecisionV1 {
                branch: ComplexityBranch::Complex,
                score: 3,
                reason_codes: vec!["missing_human_fail_safe".to_owned()],
            });
        };
        let normalized = normalize_human_text(text);
        if normalized.is_empty() {
            return Ok(ComplexityDecisionV1 {
                branch: ComplexityBranch::Complex,
                score: 3,
                reason_codes: vec!["missing_human_fail_safe".to_owned()],
            });
        }
        let folded = normalized.to_lowercase();
        if let Some(index) = self
            .user_keywords
            .iter()
            .position(|keyword| folded.contains(&keyword.to_lowercase()))
        {
            return Ok(ComplexityDecisionV1 {
                branch: ComplexityBranch::Complex,
                score: 3,
                reason_codes: vec![format!("user_complex_phrase:{index}")],
            });
        }

        let mut score = 0_u8;
        let mut reason_codes = Vec::new();
        if DEEP_REASONING_PHRASES
            .iter()
            .any(|phrase| folded.contains(phrase))
        {
            score += 2;
            reason_codes.push("deep_reasoning".to_owned());
        }
        if input.file_or_module_count >= 2 {
            score += 2;
            reason_codes.push("multi_file_or_module".to_owned());
        }
        if input.independent_deliverable_count >= 2 {
            score += 1;
            reason_codes.push("multiple_deliverables".to_owned());
        }
        if input.has_implementation_intent {
            score += 1;
            reason_codes.push("implementation_intent".to_owned());
        }
        if input.has_diagnostic_structure {
            score += 1;
            reason_codes.push("diagnostic_structure".to_owned());
        }
        match normalized.chars().count() {
            1_500.. => {
                score += 2;
                reason_codes.push("human_text_1500_plus".to_owned());
            }
            500..=1_499 => {
                score += 1;
                reason_codes.push("human_text_500_1499".to_owned());
            }
            _ => {}
        }
        if reason_codes.is_empty() {
            reason_codes.push("simple_default".to_owned());
        }
        Ok(ComplexityDecisionV1 {
            branch: if score >= 3 {
                ComplexityBranch::Complex
            } else {
                ComplexityBranch::Simple
            },
            score,
            reason_codes,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComplexityInputV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_text: Option<String>,
    #[serde(default)]
    pub file_or_module_count: u16,
    #[serde(default)]
    pub independent_deliverable_count: u16,
    #[serde(default)]
    pub has_implementation_intent: bool,
    #[serde(default)]
    pub has_diagnostic_structure: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_branch: Option<ComplexityBranch>,
}

impl ComplexityInputV1 {
    fn validate(&self) -> Result<(), ComplexityClassifierError> {
        if self.human_text.as_ref().is_some_and(|text| {
            text.chars()
                .any(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t'))
        }) {
            Err(ComplexityClassifierError::InvalidInput)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityBranch {
    Simple,
    Complex,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComplexityDecisionV1 {
    pub branch: ComplexityBranch,
    pub score: u8,
    pub reason_codes: Vec<String>,
}

fn normalize_human_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ComplexityClassifierError {
    #[error("complexity classifier revision is unsupported")]
    UnsupportedClassifier,
    #[error("complexity keyword is invalid")]
    InvalidKeyword,
    #[error("complexity input is outside bounded limits")]
    InvalidInput,
}

#[cfg(test)]
mod routing_classifier_tests {
    use super::*;

    fn input(text: Option<&str>) -> ComplexityInputV1 {
        ComplexityInputV1 {
            human_text: text.map(str::to_owned),
            ..ComplexityInputV1::default()
        }
    }

    #[test]
    fn routing_classifier_is_deterministic_for_score_and_user_phrase() {
        let classifier = ComplexityClassifierV1::new(vec!["性能回归".into()]).unwrap();
        assert_eq!(
            classifier
                .classify(&input(Some("rename a field")))
                .unwrap()
                .branch,
            ComplexityBranch::Simple
        );
        let mut structured = input(Some("review the architecture and implement the fix"));
        structured.has_implementation_intent = true;
        let decision = classifier.classify(&structured).unwrap();
        assert_eq!(decision.branch, ComplexityBranch::Complex);
        assert_eq!(decision.score, 3);
        assert_eq!(
            classifier
                .classify(&input(Some("排查性能回归")))
                .unwrap()
                .branch,
            ComplexityBranch::Complex
        );
        assert_eq!(
            classifier.classify(&structured).unwrap(),
            classifier.classify(&structured).unwrap()
        );
    }

    #[test]
    fn routing_classifier_inherits_continuations_and_fails_safe_without_human() {
        let classifier = ComplexityClassifierV1::new(Vec::new()).unwrap();
        assert_eq!(
            classifier.classify(&input(None)).unwrap().branch,
            ComplexityBranch::Complex
        );
        let continued = ComplexityInputV1 {
            continuation_branch: Some(ComplexityBranch::Simple),
            ..ComplexityInputV1::default()
        };
        assert_eq!(
            classifier.classify(&continued).unwrap().branch,
            ComplexityBranch::Simple
        );
    }

    #[test]
    fn saved_classifier_timeout_is_bounded_and_direct_rest_is_rejected() {
        let mode = |timeout_ms| ComplexityClassifierModeV1::DecisionService {
            service: Box::new(crate::DecisionServiceV1 {
                id: "decision-fixture".into(),
                revision: 1,
                name: "Fixture extension".into(),
                connection: crate::DecisionConnectionV1::Custom {
                    endpoint: "https://classifier.example/v1/decisions".into(),
                    timeout_ms,
                    auth_header: None,
                },
            }),
        };
        assert!(mode(1).validate().is_ok());
        assert!(mode(MAX_REST_CLASSIFIER_TIMEOUT_MS).validate().is_ok());
        assert!(mode(0).validate().is_err());
        assert!(mode(MAX_REST_CLASSIFIER_TIMEOUT_MS + 1).validate().is_err());
        assert!(
            serde_json::from_value::<ComplexityClassifierModeV1>(serde_json::json!({
                "kind":"rest",
                "endpoint":"https://classifier.example/v1/decisions",
                "timeout_ms":3000
            }))
            .is_err()
        );
    }
}
