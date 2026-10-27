//! Publication-owned native semantics. Supported fixed bundles remain immutable,
//! distinct from editable defaults and from their complete question checksums.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use super::{BranchExecutionPolicyV1, JudgmentSettingsV1, MaterializedBranchV1};
use crate::{CanonicalDigest, DecisionDefinitionV1};

pub const SYSTEM_ONE_C_V1: &str = "hiroute.system-one-c/v1";
pub const SYSTEM_ONE_C_V2: &str = "hiroute.system-one-c/v2";
pub const SYSTEM_ONE_LEGACY: &str = "hiroute.system-one-legacy/v1";
pub const C_CONTEXT_LAYOUT: &str = "hiroute.partitioned-history/v1";
pub const C_FULL_CHECKSUM: &str =
    "4dbab2f53954eb2176b6aa8d31b97808c4fee15ebb2fae337012eba348aef3c7";
pub const C_V2_FULL_CHECKSUM: &str =
    "a545e497ae5478150790c29f17c041c02a77a29cd825ce2c5fd849b6e4b68efc";
pub const LEGACY_ASSESSMENT_WRAPPER: &str = "Evaluate only the completed stage from state.visible_conversation[state.assessment_from] through the end. state.latest_user may supply explicit feedback, but is not yet executed.";

pub fn stock_c_questions() -> Value {
    serde_json::from_str(include_str!("fixtures/system_one_c_v1.json"))
        .expect("frozen C v1 fixture")
}

pub fn stock_c_v2_questions() -> Value {
    serde_json::from_str(include_str!("fixtures/system_one_c_v2.json"))
        .expect("frozen C v2 fixture")
}

pub fn native_mode(mode: &super::ComplexityClassifierModeV1) -> bool {
    matches!(mode, super::ComplexityClassifierModeV1::DecisionService { service } if matches!(service.connection, super::DecisionConnectionV1::SystemOne { .. }))
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeFixedContentV1 {
    pub degree_prefix: String,
    pub assessment_wrapper: String,
    #[serde(
        serialize_with = "serialize_canonical_json",
        deserialize_with = "deserialize_canonical_json"
    )]
    pub applicability: Value,
}

// Publication digests preserve typed field order. Canonicalize only the generic
// JSON inside C so senders/receivers with different serde_json map features agree,
// without changing the serialization or digest algorithm of legacy publications.
fn serialize_canonical_json<S: serde::Serializer>(
    value: &Value,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    crate::canonicalize_json(value.clone()).serialize(serializer)
}

fn deserialize_canonical_json<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Value, D::Error> {
    Value::deserialize(deserializer).map(crate::canonicalize_json)
}

impl NativeFixedContentV1 {
    pub fn c_v1() -> Self {
        let stock = stock_c_questions();
        let instructions = stock["q1"]["instructions"].as_str().unwrap();
        let (wrapper, _) = instructions
            .split_once(" Distinguish those meanings")
            .unwrap();
        Self {
            degree_prefix: "Assess state.latest_user only, not historical competence.".into(),
            assessment_wrapper: wrapper.into(),
            applicability: crate::canonicalize_json(stock["q2"].clone()),
        }
    }

    pub fn c_v2() -> Self {
        let mut fixed = Self::c_v1();
        fixed.applicability = crate::canonicalize_json(stock_c_v2_questions()["q2"].clone());
        fixed
    }
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeProtocolV1 {
    pub schema: String,
    pub fixed: NativeFixedContentV1,
    pub prior_context_max_turns: u8,
}

impl Default for NativeProtocolV1 {
    fn default() -> Self {
        Self::c_v2()
    }
}

impl NativeProtocolV1 {
    pub fn c_v1() -> Self {
        Self {
            schema: SYSTEM_ONE_C_V1.into(),
            fixed: NativeFixedContentV1::c_v1(),
            prior_context_max_turns: 8,
        }
    }

    pub fn c_v2() -> Self {
        Self {
            schema: SYSTEM_ONE_C_V2.into(),
            fixed: NativeFixedContentV1::c_v2(),
            prior_context_max_turns: 8,
        }
    }

    pub fn validate(&self) -> Result<(), NativeProtocolError> {
        let expected = match self.schema.as_str() {
            SYSTEM_ONE_C_V1 => NativeFixedContentV1::c_v1(),
            SYSTEM_ONE_C_V2 => NativeFixedContentV1::c_v2(),
            _ => return Err(NativeProtocolError::Unsupported),
        };
        if self.fixed != expected {
            return Err(NativeProtocolError::FixedContent);
        }
        if self.prior_context_max_turns != 8 {
            return Err(NativeProtocolError::HistoryCap);
        }
        Ok(())
    }

    pub fn stock_assessment(&self, judgment: &mut JudgmentSettingsV1) {
        let stock = stock_c_questions();
        judgment.competence.instructions = stock["q1"]["instructions"]
            .as_str()
            .unwrap()
            .strip_prefix(&(self.fixed.assessment_wrapper.clone() + " "))
            .unwrap()
            .into();
        judgment.competence.criteria =
            std::array::from_fn(|i| stock["q1"]["criteria"][i].as_str().unwrap().into());
    }

    /// Current editable default; the v1 fixed bundle and its stock fixture remain unchanged.
    pub fn current_assessment(&self, judgment: &mut JudgmentSettingsV1) {
        self.stock_assessment(judgment);
        let guidance: Value = serde_json::from_str(include_str!(
            "../../../../assets/decision-services/default-assessment-guidance.json"
        ))
        .expect("default assessment guidance");
        judgment.competence.instructions.push(' ');
        judgment
            .competence
            .instructions
            .push_str(guidance["feedback"].as_str().expect("feedback guidance"));
    }

    pub(crate) fn is_stock_assessment(&self, judgment: &JudgmentSettingsV1) -> bool {
        let mut stock = judgment.clone();
        self.stock_assessment(&mut stock);
        judgment.competence.instructions == stock.competence.instructions
            && judgment.competence.criteria == stock.competence.criteria
    }
}

pub fn composed_assessment(
    protocol: Option<&NativeProtocolV1>,
    judgment: &JudgmentSettingsV1,
) -> Value {
    let wrapper = protocol.map_or(LEGACY_ASSESSMENT_WRAPPER, |p| {
        p.fixed.assessment_wrapper.as_str()
    });
    json!({"type":"score", "instructions":format!("{wrapper} {}", judgment.competence.instructions), "criteria":judgment.competence.criteria})
}

pub fn validate_assessment_scope(
    protocol: Option<&super::NativeProtocolSelectionV1>,
    scope: &str,
    judgment: &JudgmentSettingsV1,
) -> Result<(), NativeProtocolError> {
    if let Some(p) = protocol {
        p.validate()?;
    }
    let texts = [
        ("degree.instructions", &judgment.degree.instructions),
        ("degree.simple", &judgment.degree.simple),
        ("degree.complex", &judgment.degree.complex),
        ("competence.instructions", &judgment.competence.instructions),
        ("competence.criteria.0", &judgment.competence.criteria[0]),
        ("competence.criteria.1", &judgment.competence.criteria[1]),
        ("competence.criteria.2", &judgment.competence.criteria[2]),
    ];
    if (protocol.is_some() || judgment.competence.standard.is_some())
        && let Some((field, _)) = texts
            .iter()
            .find(|(_, t)| incompatible_native_reference(protocol.is_some(), t))
    {
        return Err(NativeProtocolError::StateReference(format!(
            "{scope}.{field}"
        )));
    }
    Ok(())
}

pub fn incompatible_native_reference(c: bool, text: &str) -> bool {
    let paths: &[&str] = if c {
        &["visible_conversation", "assessment_from"]
    } else {
        &["assessment_stage", "prior_context"]
    };
    paths.iter().any(|p| text.contains(p))
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenAssessmentScopeV1 {
    #[serde(
        serialize_with = "serialize_canonical_json",
        deserialize_with = "deserialize_canonical_json"
    )]
    pub assessment: Value,
    pub simple_threshold_millis: u16,
    pub competence_floor_millis: u16,
    pub standard: String,
}

#[derive(schemars::JsonSchema, Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenNativeProtocolV1 {
    pub protocol: NativeProtocolV1,
    pub context_layout: String,
    pub decision: DecisionDefinitionV1,
    pub scopes: BTreeMap<String, FrozenAssessmentScopeV1>,
    pub digest: CanonicalDigest,
}

impl FrozenNativeProtocolV1 {
    pub fn freeze(
        protocol: &NativeProtocolV1,
        decision: DecisionDefinitionV1,
        judgments: impl IntoIterator<Item = (String, JudgmentSettingsV1)>,
    ) -> Result<Self, NativeProtocolError> {
        protocol.validate()?;
        let mut scopes = BTreeMap::new();
        let selection = super::NativeProtocolSelectionV1 {
            schema: protocol.schema.clone(),
            prior_context_max_turns: protocol.prior_context_max_turns,
        };
        for (id, judgment) in judgments {
            // Freezing is also used directly by compiler/diagnostic callers. Resolve
            // default intent here rather than relying on an editor to replace text.
            // Existing v1 publications carry the stock editable question as Default.
            // Validate and rebuild those bytes as written, without replacing them with
            // today's default. New editor publications resolve before reaching here.
            let judgment = if protocol.schema == SYSTEM_ONE_C_V1
                && judgment.competence.standard == Some(super::AssessmentStandardV1::Default)
                && protocol.is_stock_assessment(&judgment)
            {
                judgment
            } else {
                judgment.resolve_assessment(Some(&selection))?
            };
            if !judgment.validate() {
                return Err(NativeProtocolError::ContentMismatch);
            }
            scopes.insert(
                id,
                FrozenAssessmentScopeV1 {
                    assessment: crate::canonicalize_json(composed_assessment(
                        Some(protocol),
                        &judgment,
                    )),
                    simple_threshold_millis: judgment.degree.simple_threshold_millis,
                    competence_floor_millis: judgment.competence.floor_millis,
                    standard: match judgment.competence.standard {
                        Some(super::AssessmentStandardV1::Default) => "default",
                        _ => "custom",
                    }
                    .into(),
                },
            );
        }
        let mut value = Self {
            protocol: protocol.clone(),
            context_layout: C_CONTEXT_LAYOUT.into(),
            decision,
            scopes,
            digest: CanonicalDigest::of_bytes(&[]),
        };
        value.digest = value.content_digest()?;
        Ok(value)
    }
    fn content_digest(&self) -> Result<CanonicalDigest, NativeProtocolError> {
        CanonicalDigest::of(&(
            &self.protocol,
            &self.context_layout,
            &self.decision,
            &self.scopes,
        ))
        .map_err(|_| NativeProtocolError::Encoding)
    }
    pub fn validate(&self) -> Result<(), NativeProtocolError> {
        self.protocol.validate()?;
        if self.content_digest()? != self.digest {
            return Err(NativeProtocolError::DigestMismatch);
        }
        if self.context_layout != C_CONTEXT_LAYOUT
            || self.scopes.is_empty()
            || self.scopes.len() > 16
        {
            return Err(NativeProtocolError::ContentMismatch);
        }
        let score = |question: &Value| {
            question.as_object().is_some_and(|o| o.len() == 3)
                && question["type"] == "score"
                && question["instructions"].as_str().is_some_and(|s| {
                    s.starts_with(&(self.protocol.fixed.assessment_wrapper.clone() + " "))
                        && !s.trim().is_empty()
                })
                && question["criteria"].as_array().is_some_and(|v| {
                    v.len() == 3
                        && v.iter()
                            .all(|s| s.as_str().is_some_and(|s| !s.trim().is_empty()))
                })
        };
        if self.scopes.values().any(|s| {
            !score(&s.assessment)
                || s.simple_threshold_millis > 1000
                || s.competence_floor_millis > 1000
                || !matches!(s.standard.as_str(), "default" | "custom")
        }) {
            return Err(NativeProtocolError::ContentMismatch);
        }
        let degree = |instructions: &str, levels: &[super::OrdinalLevelV1]| {
            !instructions.trim().is_empty()
                && levels.len() == 2
                && levels[0].id == "simple"
                && levels[1].id == "complex"
                && levels.iter().all(|l| !l.criterion.trim().is_empty())
        };
        let keys: std::collections::BTreeSet<_> = self.scopes.keys().map(String::as_str).collect();
        match &self.decision {
            DecisionDefinitionV1::Ordinal {
                instructions,
                levels,
            } if degree(instructions, levels)
                && keys == std::collections::BTreeSet::from([super::SMART_SAVING_SCOPE_ID]) => {}
            DecisionDefinitionV1::Categorical {
                instructions,
                options,
            } if !instructions.trim().is_empty()
                && (2..=16).contains(&options.len())
                && options.len() == keys.len()
                && options
                    .iter()
                    .map(|o| o.id.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    == keys
                && options.iter().all(|o| {
                    super::valid_task_category_id(&o.id)
                        && !o.criterion.trim().is_empty()
                        && o.refinement
                            .as_ref()
                            .is_none_or(|d| degree(&d.instructions, &d.levels))
                }) => {}
            _ => return Err(NativeProtocolError::ContentMismatch),
        }
        Ok(())
    }
    pub fn validate_smart(&self, judgment: &JudgmentSettingsV1) -> Result<(), NativeProtocolError> {
        self.validate()?;
        let expected = Self::freeze(
            &self.protocol,
            judgment.degree.definition().into(),
            [(super::SMART_SAVING_SCOPE_ID.into(), judgment.clone())],
        )?;
        if *self != expected {
            return Err(NativeProtocolError::ContentMismatch);
        }
        Ok(())
    }
    pub fn validate_branches(
        &self,
        branches: &[MaterializedBranchV1],
        default_id: &str,
    ) -> Result<(), NativeProtocolError> {
        self.validate()?;
        if branches.iter().any(|branch| branch.judgment.is_none()) {
            return Err(NativeProtocolError::ContentMismatch);
        }
        let expected = Self::freeze(
            &self.protocol,
            c_v1_branch_decision_definition(branches, default_id),
            branches
                .iter()
                .map(|b| (b.id.clone(), b.judgment.clone().unwrap_or_default())),
        )?;
        if *self != expected {
            return Err(NativeProtocolError::ContentMismatch);
        }
        Ok(())
    }
    pub fn execution_policy(
        &self,
        scope: &str,
        judgment: &JudgmentSettingsV1,
        name: &str,
    ) -> BranchExecutionPolicyV1 {
        let mut policy = judgment.execution_policy(name);
        policy.criteria_digest = self.scopes.get(scope).and_then(|s| {
            CanonicalDigest::of(&(&self.protocol.schema, &self.protocol.fixed, &s.assessment)).ok()
        });
        policy
    }
}

pub fn branch_decision_definition(
    branches: &[MaterializedBranchV1],
    default_id: &str,
) -> DecisionDefinitionV1 {
    super::legacy_category::branch_decision_definition(branches, default_id)
}

/// Composition fixed by SYSTEM_ONE_C_V1, independently of future legacy defaults.
pub fn c_v1_branch_decision_definition(
    branches: &[MaterializedBranchV1],
    default_id: &str,
) -> DecisionDefinitionV1 {
    let template = serde_json::from_str(include_str!("fixtures/native_category_v1.json"))
        .expect("immutable native category v1 template");
    super::legacy_category::compose(branches, default_id, &template)
}

#[derive(schemars::JsonSchema, Clone, Debug, Error, Eq, PartialEq)]
pub enum NativeProtocolError {
    #[error("native protocol semantic version is unsupported")]
    Unsupported,
    #[error("native protocol fixed bundle is unsupported; select a supported bundle")]
    FixedContent,
    #[error("native prior context cap must be between 0 and 8")]
    HistoryCap,
    #[error("native state reference is incompatible in scope {0}")]
    StateReference(String),
    #[error("native protocol digest does not match frozen content")]
    DigestMismatch,
    #[error("native protocol frozen content differs from effective authoring")]
    ContentMismatch,
    #[error("native protocol encoding failed")]
    Encoding,
}

#[cfg(test)]
#[path = "system_one_c_tests.rs"]
mod tests;

impl NativeProtocolError {
    pub const fn diagnostic_key(&self) -> &'static str {
        match self {
            Self::Unsupported => "native.protocol.unsupported",
            Self::FixedContent => "native.protocol.fixed_content_invalid",
            Self::HistoryCap => "native.protocol.history_cap_invalid",
            Self::StateReference(_) => "native.protocol.state_reference_invalid",
            Self::DigestMismatch => "native.protocol.digest_mismatch",
            Self::ContentMismatch => "native.protocol.content_mismatch",
            Self::Encoding => "native.protocol.encoding_failed",
        }
    }
}
