use std::collections::BTreeSet;

use serde::Serialize;
use unicase::UniCase;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use super::{
    BranchDecisionV1, COMPLEXITY_SCHEMA, COMPLEXITY_STRATEGY_ID, COMPLEXITY_THRESHOLD,
    CompiledClassifierKindV1, CompiledComplexPhraseV1, CompiledComplexityStrategyV1,
    ComplexityBranchV1, ComplexityDecisionSourceV1, ComplexityReasonCodeV1, ContinuationKindV1,
    CorrelatedBranchDecisionV1, PlannerError, SanitizedStructuralFactsV1, canonical_digest,
};

const STRATEGY_VERSION: u32 = 1;
const BUILTIN_RULE_REVISION: &str = "hiroute-complexity-v1/builtin-rules-2026-08";

const DEEP_ZH: &[&str] = &[
    "架构设计",
    "系统设计",
    "架构重构",
    "并发状态机",
    "并发安全",
    "线程安全",
    "竞态条件",
    "死锁分析",
    "一致性设计",
    "分布式一致性",
    "性能优化",
    "性能分析",
    "内存泄漏",
    "吞吐优化",
    "延迟优化",
    "安全审计",
    "威胁模型",
    "漏洞分析",
    "迁移方案",
    "兼容性方案",
    "灰度方案",
    "回滚方案",
    "根因分析",
    "多目标权衡",
    "取舍分析",
];

const DEEP_EN: &[&str] = &[
    "architecture design",
    "system design",
    "architectural refactor",
    "concurrent state machine",
    "concurrency safety",
    "thread safety",
    "race condition",
    "deadlock analysis",
    "consistency design",
    "distributed consistency",
    "performance optimization",
    "performance analysis",
    "memory leak",
    "throughput optimization",
    "latency optimization",
    "security audit",
    "threat model",
    "vulnerability analysis",
    "migration plan",
    "compatibility plan",
    "rollout plan",
    "rollback plan",
    "root cause analysis",
    "multi-objective tradeoff",
    "trade-off analysis",
];

const BROAD_ZH: &[&str] = &[
    "整个仓库",
    "全仓库",
    "整个项目",
    "全项目",
    "跨模块",
    "多个模块",
    "所有模块",
];

const BROAD_EN: &[&str] = &[
    "entire repository",
    "whole repository",
    "repository-wide",
    "entire codebase",
    "project-wide",
    "cross-module",
    "multiple modules",
    "all modules",
];

const ACTION_ZH: &[&str] = &[
    "实现",
    "编写",
    "修改",
    "改成",
    "改为",
    "修复",
    "调试",
    "重构",
    "编译",
    "测试",
    "审查",
    "打补丁",
    "提交补丁",
];

const ACTION_EN: &[&str] = &[
    "implement",
    "write code",
    "modify",
    "fix",
    "debug",
    "refactor",
    "compile",
    "test",
    "code review",
    "patch",
];

const PURE_CONTINUATIONS: &[&str] = &["继续", "按上面做", "continue", "go on"];

#[derive(Serialize)]
struct StrategyDigestPayload<'a> {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branch_ids: &'a Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_branch_id: &'a Option<String>,
    strategy_id: &'static str,
    schema_version: &'static str,
    strategy_version: u32,
    threshold: u8,
    failure_branch: ComplexityBranchV1,
    classifier_kind: CompiledClassifierKindV1,
    classifier_config_digest: &'a Option<String>,
    builtin_rule_revision: &'static str,
    deep_zh: &'static [&'static str],
    deep_en: &'static [&'static str],
    broad_zh: &'static [&'static str],
    broad_en: &'static [&'static str],
    action_zh: &'static [&'static str],
    action_en: &'static [&'static str],
    complex_phrases: &'a [CompiledComplexPhraseV1],
}

pub struct ComplexityV1;

impl ComplexityV1 {
    pub fn compile(
        phrases: impl IntoIterator<Item = (String, String)>,
    ) -> Result<CompiledComplexityStrategyV1, PlannerError> {
        Self::compile_with_classifier(phrases, CompiledClassifierKindV1::LocalRules, None)
    }

    pub fn compile_with_classifier(
        phrases: impl IntoIterator<Item = (String, String)>,
        classifier_kind: CompiledClassifierKindV1,
        classifier_config_digest: Option<String>,
    ) -> Result<CompiledComplexityStrategyV1, PlannerError> {
        if (classifier_kind == CompiledClassifierKindV1::LocalRules)
            != classifier_config_digest.is_none()
            || classifier_config_digest
                .as_deref()
                .is_some_and(|digest| hiroute_domain::CanonicalDigest::parse(digest).is_err())
        {
            return Err(PlannerError::InvalidPolicy(
                "invalid classifier configuration identity",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut compiled = Vec::new();
        for (phrase_id, phrase) in phrases {
            let normalized = normalize_for_match(&phrase);
            if phrase_id.trim().is_empty()
                || normalized.is_empty()
                || !ids.insert(phrase_id.clone())
            {
                return Err(PlannerError::InvalidPolicy("invalid complex phrase"));
            }
            compiled.push(CompiledComplexPhraseV1 {
                phrase_id,
                phrase: normalized,
            });
        }
        let mut strategy = CompiledComplexityStrategyV1 {
            branch_ids: Vec::new(),
            default_branch_id: None,
            strategy_id: COMPLEXITY_STRATEGY_ID.into(),
            schema_version: COMPLEXITY_SCHEMA.into(),
            strategy_version: STRATEGY_VERSION,
            threshold: COMPLEXITY_THRESHOLD,
            failure_branch: ComplexityBranchV1::Complex,
            classifier_kind,
            classifier_config_digest,
            complex_phrases: compiled,
            payload_digest: String::new(),
        };
        strategy.payload_digest = strategy_digest(&strategy)?;
        Ok(strategy)
    }

    pub fn validate(strategy: &CompiledComplexityStrategyV1) -> Result<(), PlannerError> {
        if strategy.branch_ids.len() > 16
            || strategy.branch_ids.iter().collect::<BTreeSet<_>>().len()
                != strategy.branch_ids.len()
            || (strategy.branch_ids.is_empty() != strategy.default_branch_id.is_none())
            || strategy
                .default_branch_id
                .as_ref()
                .is_some_and(|id| !strategy.branch_ids.contains(id))
        {
            return Err(PlannerError::InvalidPolicy("invalid branch identities"));
        }
        if strategy.strategy_id != COMPLEXITY_STRATEGY_ID
            || strategy.schema_version != COMPLEXITY_SCHEMA
            || strategy.strategy_version != STRATEGY_VERSION
            || strategy.threshold != COMPLEXITY_THRESHOLD
            || strategy.failure_branch != ComplexityBranchV1::Complex
            || (strategy.classifier_kind == CompiledClassifierKindV1::LocalRules)
                != strategy.classifier_config_digest.is_none()
            || strategy
                .classifier_config_digest
                .as_deref()
                .is_some_and(|digest| hiroute_domain::CanonicalDigest::parse(digest).is_err())
        {
            return Err(PlannerError::InvalidPolicy(
                "unsupported complexity strategy identity",
            ));
        }
        let mut ids = BTreeSet::new();
        if strategy.complex_phrases.iter().any(|phrase| {
            phrase.phrase_id.trim().is_empty()
                || phrase.phrase.is_empty()
                || phrase.phrase != normalize_for_match(&phrase.phrase)
                || !ids.insert(phrase.phrase_id.as_str())
        }) {
            return Err(PlannerError::InvalidPolicy(
                "invalid compiled complex phrase",
            ));
        }
        if strategy.payload_digest != strategy_digest(strategy)? {
            return Err(PlannerError::ComplexityDigestMismatch);
        }
        Ok(())
    }

    pub fn with_branches(
        mut strategy: CompiledComplexityStrategyV1,
        branch_ids: Vec<String>,
        default_branch_id: String,
    ) -> Result<CompiledComplexityStrategyV1, PlannerError> {
        if !(2..=16).contains(&branch_ids.len())
            || !branch_ids.contains(&default_branch_id)
            || branch_ids.iter().collect::<BTreeSet<_>>().len() != branch_ids.len()
        {
            return Err(PlannerError::InvalidPolicy("invalid branch set"));
        }
        strategy.branch_ids = branch_ids;
        strategy.default_branch_id = Some(default_branch_id);
        strategy.payload_digest = strategy_digest(&strategy)?;
        Ok(strategy)
    }

    pub fn decide(
        latest_user: Option<&str>,
        correlated: Option<&CorrelatedBranchDecisionV1>,
        strategy: &CompiledComplexityStrategyV1,
    ) -> Result<(BranchDecisionV1, SanitizedStructuralFactsV1), PlannerError> {
        Self::validate(strategy)?;
        let projection = project_human(latest_user.unwrap_or(""));

        if let Some(correlated) = correlated
            && correlated.kind == ContinuationKindV1::TaskRoot
            && correlated_decision_matches(correlated, strategy)
        {
            return Ok((
                inherited_decision(
                    correlated,
                    strategy,
                    ComplexityReasonCodeV1::InheritedTaskRoot,
                )?,
                projection.facts,
            ));
        }

        if latest_user.is_some_and(is_pure_continuation) {
            return Ok((unresolved_decision(strategy), projection.facts));
        }

        if let Some(correlated) = correlated
            && correlated.kind == ContinuationKindV1::ToolContinuation
            && correlated_decision_matches(correlated, strategy)
        {
            return Ok((
                inherited_decision(
                    correlated,
                    strategy,
                    ComplexityReasonCodeV1::InheritedToolContinuation,
                )?,
                projection.facts,
            ));
        }

        if !strategy.branch_ids.is_empty() {
            return Ok((unresolved_decision(strategy), projection.facts));
        }
        let Some(_) = latest_user else {
            return Ok((unresolved_decision(strategy), projection.facts));
        };

        let matched_user_phrase_ids = strategy
            .complex_phrases
            .iter()
            .filter(|phrase| phrase_matches(&projection.semantic_text, &phrase.phrase))
            .map(|phrase| phrase.phrase_id.clone())
            .collect::<Vec<_>>();
        if !matched_user_phrase_ids.is_empty() {
            return Ok((
                BranchDecisionV1 {
                    policy: None,
                    competence_trigger: None,
                    execution_group: hiroute_domain::ExecutionGroupV1::Primary,
                    simple_probability: None,
                    simple_threshold_millis: None,
                    selection_reason: hiroute_domain::ModelGroupReasonV1::Heuristic,
                    strategy_id: strategy.strategy_id.clone(),
                    schema_version: strategy.schema_version.clone(),
                    payload_digest: strategy.payload_digest.clone(),
                    branch_id: hiroute_domain::SMART_SAVING_SCOPE_ID.to_owned(),
                    complexity_score: Some(strategy.threshold),
                    threshold: Some(strategy.threshold),
                    decision_source: ComplexityDecisionSourceV1::UserPhrase,
                    reason_codes: vec![ComplexityReasonCodeV1::UserComplexPhrase],
                    matched_user_phrase_ids,
                    fallback_used: false,
                    classification_duration_micros: None,
                    fallback_reason: None,
                },
                projection.facts,
            ));
        }

        let mut score = 0_u8;
        let mut reasons = Vec::new();
        if family_matches(&projection.semantic_text, DEEP_ZH, DEEP_EN) {
            score = score.saturating_add(2);
            reasons.push(ComplexityReasonCodeV1::DeepReasoning);
        }
        if projection.facts.distinct_file_or_module_refs >= 2
            || family_matches(&projection.semantic_text, BROAD_ZH, BROAD_EN)
        {
            score = score.saturating_add(2);
            reasons.push(ComplexityReasonCodeV1::MultiFileScope);
        }
        if projection.facts.numbered_requirement_count >= 2 {
            score = score.saturating_add(1);
            reasons.push(ComplexityReasonCodeV1::MultiConstraint);
        }
        if family_matches(&projection.semantic_text, ACTION_ZH, ACTION_EN) {
            score = score.saturating_add(1);
            reasons.push(ComplexityReasonCodeV1::ImplementationAction);
        }
        if projection.facts.diff_present || projection.facts.stack_trace_or_diagnostic {
            score = score.saturating_add(1);
            reasons.push(ComplexityReasonCodeV1::FailureOrDiffStructure);
        }
        match projection.facts.normalized_non_whitespace_scalar_count {
            0..=499 => {}
            500..=1499 => {
                score = score.saturating_add(1);
                reasons.push(ComplexityReasonCodeV1::HumanLengthMedium);
            }
            _ => {
                score = score.saturating_add(2);
                reasons.push(ComplexityReasonCodeV1::HumanLengthLarge);
            }
        }
        Ok((
            BranchDecisionV1 {
                policy: None,
                competence_trigger: None,
                execution_group: if score >= strategy.threshold {
                    hiroute_domain::ExecutionGroupV1::Primary
                } else {
                    hiroute_domain::ExecutionGroupV1::Regular
                },
                simple_probability: None,
                simple_threshold_millis: None,
                selection_reason: hiroute_domain::ModelGroupReasonV1::Heuristic,
                strategy_id: strategy.strategy_id.clone(),
                schema_version: strategy.schema_version.clone(),
                payload_digest: strategy.payload_digest.clone(),
                branch_id: hiroute_domain::SMART_SAVING_SCOPE_ID.to_owned(),
                complexity_score: Some(score),
                threshold: Some(strategy.threshold),
                decision_source: ComplexityDecisionSourceV1::BuiltinRules,
                reason_codes: reasons,
                matched_user_phrase_ids: Vec::new(),
                fallback_used: false,
                classification_duration_micros: None,
                fallback_reason: None,
            },
            projection.facts,
        ))
    }
}

fn strategy_digest(strategy: &CompiledComplexityStrategyV1) -> Result<String, PlannerError> {
    canonical_digest(&StrategyDigestPayload {
        branch_ids: &strategy.branch_ids,
        default_branch_id: &strategy.default_branch_id,
        strategy_id: COMPLEXITY_STRATEGY_ID,
        schema_version: COMPLEXITY_SCHEMA,
        strategy_version: STRATEGY_VERSION,
        threshold: COMPLEXITY_THRESHOLD,
        failure_branch: ComplexityBranchV1::Complex,
        classifier_kind: strategy.classifier_kind,
        classifier_config_digest: &strategy.classifier_config_digest,
        builtin_rule_revision: BUILTIN_RULE_REVISION,
        deep_zh: DEEP_ZH,
        deep_en: DEEP_EN,
        broad_zh: BROAD_ZH,
        broad_en: BROAD_EN,
        action_zh: ACTION_ZH,
        action_en: ACTION_EN,
        complex_phrases: &strategy.complex_phrases,
    })
}

fn unresolved_decision(strategy: &CompiledComplexityStrategyV1) -> BranchDecisionV1 {
    BranchDecisionV1 {
        policy: None,
        competence_trigger: None,
        execution_group: hiroute_domain::ExecutionGroupV1::Primary,
        simple_probability: None,
        simple_threshold_millis: None,
        selection_reason: hiroute_domain::ModelGroupReasonV1::Heuristic,
        strategy_id: strategy.strategy_id.clone(),
        schema_version: strategy.schema_version.clone(),
        payload_digest: strategy.payload_digest.clone(),
        branch_id: strategy
            .default_branch_id
            .clone()
            .unwrap_or_else(|| hiroute_domain::SMART_SAVING_SCOPE_ID.to_owned()),
        complexity_score: Some(strategy.threshold),
        threshold: Some(strategy.threshold),
        decision_source: ComplexityDecisionSourceV1::Unresolved,
        reason_codes: vec![ComplexityReasonCodeV1::TaskContextUnresolved],
        matched_user_phrase_ids: Vec::new(),
        fallback_used: true,
        classification_duration_micros: None,
        fallback_reason: None,
    }
}

fn correlated_decision_matches(
    correlated: &CorrelatedBranchDecisionV1,
    strategy: &CompiledComplexityStrategyV1,
) -> bool {
    let previous = &correlated.decision;
    previous.strategy_id == strategy.strategy_id
        && previous.schema_version == strategy.schema_version
        && previous.payload_digest == strategy.payload_digest
}

fn inherited_decision(
    correlated: &CorrelatedBranchDecisionV1,
    strategy: &CompiledComplexityStrategyV1,
    inheritance_reason: ComplexityReasonCodeV1,
) -> Result<BranchDecisionV1, PlannerError> {
    let previous = &correlated.decision;
    if !correlated_decision_matches(correlated, strategy) {
        return Err(PlannerError::CorrelatedDecisionMismatch);
    }
    let mut reasons = vec![inheritance_reason];
    reasons.extend(previous.reason_codes.iter().copied().filter(|reason| {
        !matches!(
            reason,
            ComplexityReasonCodeV1::InheritedToolContinuation
                | ComplexityReasonCodeV1::InheritedTaskRoot
        )
    }));
    Ok(BranchDecisionV1 {
        policy: previous.policy.clone(),
        competence_trigger: previous.competence_trigger.clone(),
        execution_group: previous.execution_group,
        simple_probability: previous.simple_probability.clone(),
        simple_threshold_millis: previous.simple_threshold_millis,
        selection_reason: previous.selection_reason,
        strategy_id: previous.strategy_id.clone(),
        schema_version: previous.schema_version.clone(),
        payload_digest: previous.payload_digest.clone(),
        branch_id: previous.branch_id.clone(),
        complexity_score: previous.complexity_score,
        threshold: previous.threshold,
        decision_source: ComplexityDecisionSourceV1::Inherited,
        reason_codes: reasons,
        matched_user_phrase_ids: previous.matched_user_phrase_ids.clone(),
        fallback_used: previous.fallback_used,
        classification_duration_micros: previous.classification_duration_micros,
        fallback_reason: previous.fallback_reason,
    })
}

struct HumanProjection {
    semantic_text: String,
    facts: SanitizedStructuralFactsV1,
}

fn project_human(text: &str) -> HumanProjection {
    let normalized_full = normalize_for_match(text);
    let diff_present = has_complete_diff(text);
    let stack_trace_or_diagnostic = has_registered_diagnostic(text);
    let mut semantic_lines = Vec::new();
    let mut in_fence = false;
    let mut in_diff_hunk = false;
    let mut code_block_count = 0_u32;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if !in_fence {
                code_block_count = code_block_count.saturating_add(1);
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if diff_present && is_diff_metadata_line(trimmed) {
            if trimmed.starts_with("@@") {
                in_diff_hunk = true;
            }
            continue;
        }
        if diff_present
            && in_diff_hunk
            && (line.starts_with('+')
                || line.starts_with('-')
                || line.starts_with(' ')
                || line.starts_with("\\ No newline"))
        {
            continue;
        }
        in_diff_hunk = false;
        if !is_diagnostic_line(trimmed) {
            semantic_lines.push(line);
        }
    }
    let semantic_text = normalize_for_match(&semantic_lines.join("\n"));
    HumanProjection {
        semantic_text,
        facts: SanitizedStructuralFactsV1 {
            distinct_file_or_module_refs: u32::try_from(file_and_module_refs(text).len())
                .unwrap_or(u32::MAX),
            code_block_count,
            diff_present,
            stack_trace_or_diagnostic,
            numbered_requirement_count: numbered_requirement_count(&semantic_lines),
            normalized_non_whitespace_scalar_count: normalized_full
                .chars()
                .filter(|character| !character.is_whitespace())
                .count()
                .try_into()
                .unwrap_or(u64::MAX),
        },
    }
}

fn normalize_for_match(text: &str) -> String {
    let normalized = text.nfkc().collect::<String>();
    let folded = UniCase::unicode(normalized).to_folded_case();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_pure_continuation(text: &str) -> bool {
    let normalized = normalize_for_match(text);
    let trimmed = normalized.trim_matches(|character: char| {
        matches!(
            character,
            '.' | ',' | '!' | '?' | ';' | ':' | '。' | '，' | '！' | '？' | '；' | '：'
        )
    });
    PURE_CONTINUATIONS.contains(&trimmed)
}

fn family_matches(text: &str, chinese: &[&str], english: &[&str]) -> bool {
    chinese.iter().any(|phrase| text.contains(phrase))
        || english
            .iter()
            .any(|phrase| token_phrase_matches(text, phrase))
}

fn phrase_matches(text: &str, phrase: &str) -> bool {
    if phrase.chars().any(is_cjk) {
        text.contains(phrase)
    } else {
        token_phrase_matches(text, phrase)
    }
}

fn token_phrase_matches(text: &str, phrase: &str) -> bool {
    let text_tokens = word_tokens(text);
    let phrase_tokens = word_tokens(phrase);
    !phrase_tokens.is_empty()
        && text_tokens
            .windows(phrase_tokens.len())
            .any(|window| window == phrase_tokens)
}

fn word_tokens(text: &str) -> Vec<&str> {
    text.unicode_words().collect()
}

fn is_cjk(character: char) -> bool {
    matches!(character as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff)
}

fn has_complete_diff(text: &str) -> bool {
    let has_hunk = text.lines().any(|line| line.trim_start().starts_with("@@"));
    let git_header = text
        .lines()
        .any(|line| line.trim_start().starts_with("diff --git "));
    let old_header = text
        .lines()
        .any(|line| line.trim_start().starts_with("--- "));
    let new_header = text
        .lines()
        .any(|line| line.trim_start().starts_with("+++ "));
    has_hunk && (git_header || (old_header && new_header))
}

fn is_diff_metadata_line(line: &str) -> bool {
    line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("@@")
}

fn has_registered_diagnostic(text: &str) -> bool {
    text.lines().any(is_diagnostic_line)
}

fn is_diagnostic_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    (lower.starts_with("error[") && lower.contains("]:"))
        || (lower.starts_with("error:") && lower.contains("-->"))
        || (lower.starts_with("-->") && has_line_column(lower.as_str()))
        || (lower.starts_with("file \"") && lower.contains("\", line "))
        || (lower.starts_with("at ") && has_line_column(lower.as_str()))
        || lower
            .split_once(": error:")
            .is_some_and(|(location, _)| has_line_column(location))
        || (lower.starts_with("thread '") && lower.contains("panicked at"))
}

fn has_line_column(text: &str) -> bool {
    let mut pieces = text.rsplit(':');
    let last = pieces
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| !c.is_ascii_digit());
    let prior = pieces
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| !c.is_ascii_digit());
    !last.is_empty()
        && !prior.is_empty()
        && last.bytes().all(|byte| byte.is_ascii_digit())
        && prior.bytes().all(|byte| byte.is_ascii_digit())
}

fn file_and_module_refs(text: &str) -> BTreeSet<String> {
    let mut references = BTreeSet::new();
    let mut in_fence = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        let diff_header = trimmed.starts_with("diff --git ")
            || trimmed.starts_with("--- ")
            || trimmed.starts_with("+++ ");
        if in_fence && !diff_header {
            continue;
        }
        references.extend(
            line.split_whitespace()
                .filter(|chunk| !chunk.contains('@') && !chunk.contains("://"))
                .flat_map(|chunk| {
                    chunk.split(|character: char| {
                        !(character.is_alphanumeric()
                            || matches!(character, '_' | '-' | '.' | '/' | '\\' | ':'))
                    })
                })
                .filter_map(|token| {
                    if diff_header {
                        normalize_diff_reference(token)
                    } else {
                        normalize_reference(token)
                    }
                }),
        );
    }
    references
}

fn normalize_diff_reference(token: &str) -> Option<String> {
    let token = token
        .strip_prefix("a/")
        .or_else(|| token.strip_prefix("b/"))
        .unwrap_or(token);
    (token != "/dev/null" && token != "dev/null")
        .then(|| normalize_reference(token))
        .flatten()
}

fn normalize_reference(token: &str) -> Option<String> {
    let trimmed = token.trim_matches(|character| matches!(character, '.' | ':' | '/' | '\\'));
    if trimmed.is_empty()
        || trimmed.contains("://")
        || trimmed.contains('@')
        || trimmed.starts_with('.')
    {
        return None;
    }
    let path_like = trimmed.contains('/') || trimmed.contains('\\');
    let module_like = trimmed.contains("::");
    let filename_like = trimmed.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && !extension.is_empty()
            && extension.len() <= 8
            && extension
                .chars()
                .all(|character| character.is_ascii_alphanumeric())
    });
    (path_like || module_like || filename_like).then(|| trimmed.replace('\\', "/"))
}

fn numbered_requirement_count(lines: &[&str]) -> u32 {
    let bullet_count = lines
        .iter()
        .filter(|line| is_bullet_or_numbered(line.trim_start()))
        .count();
    let mut inline_count = 0_usize;
    let joined = lines.join(" ");
    let characters = joined.chars().collect::<Vec<_>>();
    for index in 0..characters.len() {
        if !characters[index].is_ascii_digit()
            || (index > 0 && characters[index - 1].is_ascii_digit())
        {
            continue;
        }
        let mut end = index + 1;
        while end < characters.len() && characters[end].is_ascii_digit() {
            end += 1;
        }
        if end < characters.len()
            && matches!(characters[end], '.' | ')' | '、')
            && (index == 0
                || characters[index - 1].is_whitespace()
                || matches!(characters[index - 1], ':' | '：' | ';' | '；'))
        {
            inline_count = inline_count.saturating_add(1);
        }
    }
    u32::try_from(bullet_count.max(inline_count)).unwrap_or(u32::MAX)
}

fn is_bullet_or_numbered(line: &str) -> bool {
    if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        return true;
    }
    let digits = line
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    digits > 0
        && line
            .chars()
            .nth(digits)
            .is_some_and(|character| matches!(character, '.' | ')' | '、'))
}
