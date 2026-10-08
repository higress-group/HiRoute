//! Compile independent typed questions once, then reduce only the selected path.
use std::collections::BTreeMap;

use hiroute_domain::{DecisionDefinitionV1, OrdinalLevelV1, SMART_SAVING_SCOPE_ID};
use serde::Deserialize;
use serde_json::{Map, Value, json, value::RawValue};

use super::protocol::{
    UniqueMap, assessment_definition, latest_user_wire_value, normalize_probabilities,
};
use super::{CallFailure, ClassifierAssessment, ClassifierResponse, PreparedDecisionRequest};
use crate::agent_turn_history::AgentTurnHistorySnapshot;
use crate::server::core_runtime::adapters::prepare_replay_json_template;
use crate::server::core_runtime::model_ir::ModelRequestIRV1;
use crate::server::request_plan::RestBranchClassifierAuthorityV1;

// A bounded wire allocation, not a tokenizer or a business-model context limit.
// Providers enforce their own token limits (Bailian documents 65,536 tokens).
const MAX_REQUEST_BYTES: usize = 256 * 1024;

pub(super) struct Bindings {
    definition: DecisionDefinitionV1,
    choice: Option<String>,
    degrees: BTreeMap<String, String>,
    assessment: Option<String>,
}

pub(super) fn request_template(
    authority: &RestBranchClassifierAuthorityV1,
    request: &ModelRequestIRV1,
    history: &AgentTurnHistorySnapshot,
) -> Result<PreparedDecisionRequest, CallFailure> {
    let mut refs = Vec::new();
    let latest =
        latest_user_wire_value(request, &mut refs).map_err(|_| CallFailure::RejectedInput)?;
    let mut target = assessment_definition(history, authority);
    let mut questions = Map::new();
    let mut bindings = Bindings {
        definition: authority.decision.clone(),
        choice: None,
        degrees: BTreeMap::new(),
        assessment: None,
    };
    match &authority.decision {
        DecisionDefinitionV1::Ordinal {
            instructions,
            levels,
        } => {
            let id = add_degree(&mut questions, instructions, levels);
            bindings.degrees.insert(SMART_SAVING_SCOPE_ID.into(), id);
        }
        DecisionDefinitionV1::Categorical {
            instructions,
            options,
        } => {
            let id = question_id(&questions);
            let criteria: Map<String, Value> = options
                .iter()
                .map(|o| (o.id.clone(), Value::String(o.criterion.clone())))
                .collect();
            questions.insert(
                id.clone(),
                json!({"type":"choice","instructions":instructions,"criteria":criteria}),
            );
            bindings.choice = Some(id);
            for option in options {
                if let Some(degree) = &option.refinement {
                    let id = add_degree(&mut questions, &degree.instructions, &degree.levels);
                    bindings.degrees.insert(option.id.clone(), id);
                }
            }
        }
    }
    if let Some(target) = &target {
        let id = question_id(&questions);
        questions.insert(id.clone(), json!({
            "type":"score",
            "instructions":format!("Evaluate only the completed stage from state.visible_conversation[state.assessment_from] through the end. state.latest_user may supply explicit feedback, but is not yet executed. {}", target.instructions),
            "criteria":target.criteria.iter().map(|c| &c.criterion).collect::<Vec<_>>(),
        }));
        bindings.assessment = Some(id);
    }
    let mut body = json!({"model": authority.system_one_model, "questions":questions,
        "state":{"latest_user":latest,"visible_conversation":[],"history_partial":history.history_partial,"assessment_from":target.as_ref().map(|t| t.from)}});
    // Replay computes the full escaped current input length without truncating or materializing it.
    let base = prepare_replay_json_template(&body, refs.clone())
        .map_err(|_| CallFailure::RejectedInput)?;
    if base.wire_len > MAX_REQUEST_BYTES {
        return Err(CallFailure::RejectedInput);
    }
    let mut budget = ByteBudget(MAX_REQUEST_BYTES - base.wire_len);
    let mut removed = history.visible_conversation.len();
    for turn in history.visible_conversation.iter().rev() {
        if serde_json::to_writer(&mut budget, turn).is_err() {
            break;
        }
        budget.0 = budget.0.saturating_sub(1);
        removed -= 1;
    }
    let visible = &history.visible_conversation[removed..];
    let target_partial = target.as_ref().is_some_and(|t| removed > t.from);
    if visible.is_empty() {
        target = None;
        if let Some(id) = bindings.assessment.take() {
            body["questions"].as_object_mut().unwrap().remove(&id);
        }
    }
    body["state"]["visible_conversation"] =
        serde_json::to_value(visible).map_err(|_| CallFailure::RejectedInput)?;
    body["state"]["history_partial"] = json!(history.history_partial || removed > 0);
    body["state"]["assessment_from"] =
        json!(target.as_ref().map(|t| t.from.saturating_sub(removed)));
    let template =
        prepare_replay_json_template(&body, refs).map_err(|_| CallFailure::RejectedInput)?;
    Ok(PreparedDecisionRequest {
        template,
        has_target: target.is_some(),
        target_partial,
        bindings: Some(bindings),
    })
}

fn question_id(questions: &Map<String, Value>) -> String {
    format!("q{}", questions.len())
}
fn add_degree(
    questions: &mut Map<String, Value>,
    instructions: &str,
    levels: &[OrdinalLevelV1],
) -> String {
    let id = question_id(questions);
    questions.insert(id.clone(), json!({"type":"score","instructions":format!("Assess state.latest_user only, not historical competence. {instructions}"),"criteria":levels.iter().map(|l| &l.criterion).collect::<Vec<_>>()}));
    id
}
struct ByteBudget(usize);
impl std::io::Write for ByteBudget {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.0 {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        self.0 -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Deserialize)]
struct Output {
    answers: UniqueMap<Box<RawValue>>,
}
#[derive(Deserialize)]
struct Choice {
    #[serde(rename = "type")]
    kind: String,
    choice: String,
}
#[derive(Deserialize)]
struct Degree {
    #[serde(rename = "type")]
    kind: String,
    probabilities: UniqueMap<f64>,
}
#[derive(Deserialize)]
struct Competence {
    #[serde(rename = "type")]
    kind: String,
    score: f64,
}

pub(super) fn parse_response(
    body: &[u8],
    bindings: &Bindings,
) -> Result<ClassifierResponse, CallFailure> {
    let output: Output = serde_json::from_slice(body).map_err(|_| CallFailure::InvalidOutput)?;
    let answers = output.answers.0;
    let (branch_id, levels) = match &bindings.definition {
        DecisionDefinitionV1::Ordinal { levels, .. } => {
            (SMART_SAVING_SCOPE_ID.to_owned(), Some(levels))
        }
        DecisionDefinitionV1::Categorical { options, .. } => {
            let raw = bindings
                .choice
                .as_ref()
                .and_then(|id| answers.get(id))
                .ok_or(CallFailure::InvalidOutput)?;
            let choice: Choice =
                serde_json::from_str(raw.get()).map_err(|_| CallFailure::InvalidOutput)?;
            if choice.kind != "choice" {
                return Err(CallFailure::InvalidOutput);
            }
            let option = options
                .iter()
                .find(|o| o.id == choice.choice)
                .ok_or(CallFailure::InvalidOutput)?;
            (choice.choice, option.refinement.as_ref().map(|d| &d.levels))
        }
    };
    let parsed_degree = levels.map(|levels| {
        let raw = bindings
            .degrees
            .get(&branch_id)
            .and_then(|id| answers.get(id))
            .ok_or(CallFailure::InvalidOutput)?;
        let degree: Degree =
            serde_json::from_str(raw.get()).map_err(|_| CallFailure::InvalidOutput)?;
        if degree.kind != "score" {
            return Err(CallFailure::InvalidOutput);
        }
        let indexes = (0..levels.len()).map(|i| i.to_string()).collect::<Vec<_>>();
        let probabilities =
            normalize_probabilities(degree.probabilities.0, indexes.iter().map(String::as_str))?;
        Ok(levels
            .iter()
            .zip(indexes)
            .map(|(level, index)| (level.id.clone(), probabilities[&index]))
            .collect::<BTreeMap<_, _>>())
    });
    let degree_failed = parsed_degree.as_ref().is_some_and(Result::is_err);
    let probabilities = parsed_degree.and_then(Result::ok);
    let raw_score = bindings.assessment.as_ref().and_then(|id| answers.get(id));
    let score = raw_score
        .and_then(|raw| serde_json::from_str::<Competence>(raw.get()).ok())
        .filter(|s| s.kind == "score" && s.score.is_finite() && (0.0..=2.0).contains(&s.score));
    Ok(ClassifierResponse {
        branch_id,
        probabilities,
        degree_failed,
        invalid_assessment: raw_score.is_some() && score.is_none(),
        assessment: score.map(|s| ClassifierAssessment {
            score: s.score / 2.0,
            partial: false,
            reason: None,
        }),
    })
}

#[cfg(test)]
#[path = "system_one_tests.rs"]
mod tests;
