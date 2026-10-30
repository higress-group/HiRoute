//! OpenAI wire adaptation of the same frozen native questions and selected-path reducer.
use std::collections::{BTreeMap, BTreeSet};

use hiroute_domain::{DecisionDefinitionV1, DecisionWireProtocolV2, SMART_SAVING_SCOPE_ID};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json, value::RawValue};

use super::protocol::normalize_probabilities;
use super::system_one::{Bindings, parse_response as reduce_system_one};
use super::{CallFailure, ClassifierResponse};
use crate::server::core_runtime::adapters::{
    PreparedReplayTemplate, RequestedReplacement, prepare_replay_json_template,
};

#[derive(Serialize)]
struct OpenaiRequest<'a> {
    model: &'a str,
    input: [OpenaiMessage<'a>; 1],
    questions: Vec<Value>,
}

#[derive(Serialize)]
struct OpenaiMessage<'a> {
    role: &'static str,
    content: [OpenaiInputPart<'a>; 1],
}

#[derive(Serialize)]
struct OpenaiInputPart<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    data: OpenaiState<'a>,
}

#[derive(Serialize)]
struct OpenaiState<'a> {
    state: &'a Value,
}

/// Convert only the bounded questions. The state and Replay-owned text are borrowed.
pub(super) fn prepare_template(
    codec: DecisionWireProtocolV2,
    body: &Value,
    refs: Vec<RequestedReplacement>,
) -> Result<PreparedReplayTemplate, CallFailure> {
    if codec == DecisionWireProtocolV2::SystemOne {
        return prepare_replay_json_template(body, refs).map_err(|_| CallFailure::RejectedInput);
    }
    let object = body.as_object().ok_or(CallFailure::RejectedInput)?;
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or(CallFailure::RejectedInput)?;
    let state = object
        .get("state")
        .filter(|value| value.is_object())
        .ok_or(CallFailure::RejectedInput)?;
    let mut values = 1; // The outer input_json object contains the state.
    check_json_limits(state, 2, &mut values)?;
    let questions = object
        .get("questions")
        .and_then(Value::as_object)
        .ok_or(CallFailure::RejectedInput)?;
    if !(1..=400).contains(&questions.len()) {
        return Err(CallFailure::RejectedInput);
    }
    let mut encoded = Vec::with_capacity(questions.len());
    for (name, question) in questions {
        let q = question.as_object().ok_or(CallFailure::RejectedInput)?;
        let kind = q
            .get("type")
            .and_then(Value::as_str)
            .ok_or(CallFailure::RejectedInput)?;
        let instructions = q
            .get("instructions")
            .and_then(Value::as_str)
            .ok_or(CallFailure::RejectedInput)?;
        let criteria = q.get("criteria").ok_or(CallFailure::RejectedInput)?;
        let result = match kind {
            "choice" => {
                let options = criteria.as_object().ok_or(CallFailure::RejectedInput)?;
                if !(2..=255).contains(&options.len()) {
                    return Err(CallFailure::RejectedInput);
                }
                let choices = options
                    .iter()
                    .map(|(value, description)| {
                        description
                            .as_str()
                            .map(|description| json!({"value": value, "description": description}))
                            .ok_or(CallFailure::RejectedInput)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                json!({"type":"choice", "name":name, "instructions":instructions, "choices":choices})
            }
            "score" => {
                let criteria = criteria.as_array().ok_or(CallFailure::RejectedInput)?;
                if !(2..=10).contains(&criteria.len()) {
                    return Err(CallFailure::RejectedInput);
                }
                let levels = criteria.iter().enumerate().map(|(index, description)| {
                    description.as_str().map(|description| json!({"label":index.to_string(), "description":description}))
                        .ok_or(CallFailure::RejectedInput)
                }).collect::<Result<Vec<_>, _>>()?;
                json!({"type":"score", "name":name, "instructions":instructions, "levels":levels})
            }
            _ => return Err(CallFailure::RejectedInput),
        };
        encoded.push(result);
    }
    let request = OpenaiRequest {
        model,
        input: [OpenaiMessage {
            role: "user",
            content: [OpenaiInputPart {
                kind: "input_json",
                data: OpenaiState { state },
            }],
        }],
        questions: encoded,
    };
    prepare_replay_json_template(&request, refs).map_err(|_| CallFailure::RejectedInput)
}

fn check_json_limits(value: &Value, depth: usize, count: &mut usize) -> Result<(), CallFailure> {
    *count = count.checked_add(1).ok_or(CallFailure::RejectedInput)?;
    if *count > 10_000 || depth > 32 {
        return Err(CallFailure::RejectedInput);
    }
    match value {
        Value::Array(items) => {
            for item in items {
                check_json_limits(item, depth + 1, count)?;
            }
        }
        Value::Object(items) => {
            for item in items.values() {
                check_json_limits(item, depth + 1, count)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Deserialize)]
struct Output {
    answers: Vec<Box<RawValue>>,
}
#[derive(Deserialize)]
struct NamedAnswer {
    name: String,
}
#[derive(Deserialize)]
struct AnswerKind {
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct ChoiceAnswer {
    choice: String,
    probabilities: Vec<ChoiceProbability>,
}
#[derive(Deserialize)]
struct ChoiceProbability {
    value: String,
    probability: f64,
}
#[derive(Deserialize)]
struct ScoreAnswer {
    score: f64,
    probabilities: Vec<ScoreProbability>,
}
#[derive(Deserialize)]
struct ScoreProbability {
    value: usize,
    probability: f64,
    label: String,
}

pub(super) fn parse_response(
    body: &[u8],
    bindings: &Bindings,
) -> Result<ClassifierResponse, CallFailure> {
    let output: Output = serde_json::from_slice(body).map_err(|_| CallFailure::InvalidOutput)?;
    let known: BTreeSet<&str> = bindings
        .choice
        .iter()
        .chain(bindings.degrees.values())
        .chain(bindings.assessment.iter())
        .chain(bindings.applicability.iter())
        .map(String::as_str)
        .collect();
    let mut answers = Map::new();
    for raw in output.answers {
        let named: NamedAnswer =
            serde_json::from_str(raw.get()).map_err(|_| CallFailure::InvalidOutput)?;
        if !known.contains(named.name.as_str()) || answers.contains_key(&named.name) {
            return Err(CallFailure::InvalidOutput);
        }
        // An invalid or refused answer remains local to its question. The existing
        // reducer decides whether that question belongs to the selected path.
        let kind = serde_json::from_str::<AnswerKind>(raw.get()).ok();
        let converted = match kind.as_ref().map(|answer| answer.kind.as_str()) {
            Some("choice") => convert_choice(raw.get(), &named.name, bindings),
            Some("score") => convert_score(raw.get(), &named.name, bindings),
            Some("refusal") => Some(json!({"type":"refusal"})),
            _ => None,
        }
        .unwrap_or_else(|| json!({"type":"invalid"}));
        answers.insert(named.name, converted);
    }
    let encoded =
        serde_json::to_vec(&json!({"answers":answers})).map_err(|_| CallFailure::InvalidOutput)?;
    reduce_system_one(&encoded, bindings)
}

fn convert_choice(raw: &str, name: &str, bindings: &Bindings) -> Option<Value> {
    let expected: Vec<&str> = if bindings.applicability.as_deref() == Some(name) {
        vec!["applicable", "not_established"]
    } else if bindings.choice.as_deref() == Some(name) {
        match &bindings.definition {
            DecisionDefinitionV1::Categorical { options, .. } => {
                options.iter().map(|o| o.id.as_str()).collect()
            }
            _ => return None,
        }
    } else {
        return None;
    };
    let answer: ChoiceAnswer = serde_json::from_str(raw).ok()?;
    if !expected.contains(&answer.choice.as_str()) {
        return None;
    }
    let mut probabilities = BTreeMap::new();
    for p in answer.probabilities {
        if probabilities.insert(p.value, p.probability).is_some() {
            return None;
        }
    }
    let probabilities = normalize_probabilities(probabilities, expected.into_iter()).ok()?;
    Some(json!({"type":"choice", "choice":answer.choice, "probabilities":probabilities}))
}

fn convert_score(raw: &str, name: &str, bindings: &Bindings) -> Option<Value> {
    let count = if bindings.assessment.as_deref() == Some(name) {
        3
    } else {
        let branch = bindings
            .degrees
            .iter()
            .find_map(|(branch, id)| (id == name).then_some(branch.as_str()))?;
        match &bindings.definition {
            DecisionDefinitionV1::Ordinal { levels, .. } if branch == SMART_SAVING_SCOPE_ID => {
                levels.len()
            }
            DecisionDefinitionV1::Categorical { options, .. } => options
                .iter()
                .find(|o| o.id == branch)?
                .refinement
                .as_ref()?
                .levels
                .len(),
            _ => return None,
        }
    };
    let answer: ScoreAnswer = serde_json::from_str(raw).ok()?;
    if !answer.score.is_finite()
        || !(0.0..=(count.saturating_sub(1) as f64)).contains(&answer.score)
    {
        return None;
    }
    let mut probabilities = BTreeMap::new();
    for p in answer.probabilities {
        if p.label != p.value.to_string()
            || probabilities
                .insert(p.value.to_string(), p.probability)
                .is_some()
        {
            return None;
        }
    }
    let expected = (0..count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>();
    let probabilities =
        normalize_probabilities(probabilities, expected.iter().map(String::as_str)).ok()?;
    Some(json!({"type":"score", "score":answer.score, "probabilities":probabilities}))
}
