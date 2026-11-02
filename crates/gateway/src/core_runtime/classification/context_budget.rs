//! One shared-state reservation satisfies total and every independent question envelope.
use super::{CallFailure, openai_decisions};
use crate::server::core_runtime::adapters::RequestedReplacement;
use hiroute_domain::{
    DecisionContextConstraintV1 as Constraint, DecisionContextNoFitV1 as NoFit,
    DecisionWireProtocolV2, ResolvedDecisionContextV1,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::io::Write;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BudgetRejection {
    pub reason: NoFit,
    pub limiting: Constraint,
    pub request_bytes: Option<u64>,
    pub question_bytes: Option<u64>,
}
impl BudgetRejection {
    pub fn shape() -> Self {
        Self {
            reason: NoFit::MandatoryShape,
            limiting: Constraint::JsonShape,
            request_bytes: None,
            question_bytes: None,
        }
    }
    pub fn accounting() -> Self {
        Self {
            reason: NoFit::Accounting,
            limiting: Constraint::Request,
            request_bytes: None,
            question_bytes: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Cost {
    pub bytes: usize,
    pub values: usize,
}
impl Cost {
    pub fn add(self, other: Self) -> Result<Self, CallFailure> {
        Ok(Self {
            bytes: self
                .bytes
                .checked_add(other.bytes)
                .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))?,
            values: self
                .values
                .checked_add(other.values)
                .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))?,
        })
    }
    pub fn comma(self, needed: bool) -> Result<Self, CallFailure> {
        self.add(Self {
            bytes: usize::from(needed),
            values: 0,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct Remaining {
    pub bytes: usize,
    pub values: usize,
    pub limiting: Constraint,
}
impl Remaining {
    pub fn fits(&self, cost: Cost) -> bool {
        cost.bytes <= self.bytes && cost.values <= self.values
    }
    pub fn admit(&mut self, cost: Cost) -> bool {
        if !self.fits(cost) {
            if cost.values > self.values {
                self.limiting = Constraint::JsonShape;
            }
            return false;
        }
        self.bytes -= cost.bytes;
        self.values -= cost.values;
        true
    }
}

pub(super) struct CoreMeasurement {
    pub allowance: Result<Remaining, BudgetRejection>,
    pub question_envelopes: BTreeMap<String, usize>,
    state_extra: usize,
    state_values: usize,
}

fn empty_envelope(
    codec: DecisionWireProtocolV2,
    model: &str,
    questions: &Map<String, Value>,
) -> Result<usize, CallFailure> {
    Ok(
        openai_decisions::prepare_native_state(codec, model, questions, &json!({}), vec![])?
            .wire_len,
    )
}

pub(super) fn measure_core(
    codec: DecisionWireProtocolV2,
    body: &Value,
    refs: Vec<RequestedReplacement>,
    context: &ResolvedDecisionContextV1,
) -> Result<CoreMeasurement, CallFailure> {
    let template = openai_decisions::prepare_template(codec, body, refs)
        .map_err(|_| CallFailure::BudgetRejected(BudgetRejection::shape()))?;
    let model = body["model"].as_str().ok_or(CallFailure::RejectedInput)?;
    let questions = body["questions"]
        .as_object()
        .ok_or(CallFailure::RejectedInput)?;
    let empty = empty_envelope(codec, model, questions)?;
    let state_extra = template
        .wire_len
        .checked_sub(empty)
        .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))?;
    let mut question_envelopes = BTreeMap::new();
    let mut largest = 0;
    for (name, question) in questions {
        let single = Map::from_iter([(name.clone(), question.clone())]);
        let envelope = empty_envelope(codec, model, &single)?;
        largest = largest.max(
            envelope
                .checked_add(state_extra)
                .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))?,
        );
        question_envelopes.insert(name.clone(), envelope);
    }
    let state_values = if codec == DecisionWireProtocolV2::OpenaiDecisions {
        openai_decisions::state_value_count(&body["state"])?
    } else {
        0
    };
    Ok(CoreMeasurement {
        allowance: allowance(context, template.wire_len, largest, state_values),
        question_envelopes,
        state_extra,
        state_values,
    })
}

fn allowance(
    context: &ResolvedDecisionContextV1,
    request: usize,
    question: usize,
    values: usize,
) -> Result<Remaining, BudgetRejection> {
    let total = context.request_usable_tokens() as usize;
    let per_question = context.per_question_usable_tokens() as usize;
    let limiting = if per_question.saturating_sub(question) <= total.saturating_sub(request) {
        Constraint::Question
    } else {
        Constraint::Request
    };
    if request > total || question > per_question {
        return Err(BudgetRejection {
            reason: NoFit::MandatoryCore,
            limiting,
            request_bytes: Some(request as u64),
            question_bytes: Some(question as u64),
        });
    }
    Ok(Remaining {
        bytes: (total - request).min(per_question - question),
        values: if values == 0 {
            usize::MAX
        } else {
            10_000usize.saturating_sub(values)
        },
        limiting,
    })
}

impl CoreMeasurement {
    pub fn mandatory_allowance(
        &self,
        codec: DecisionWireProtocolV2,
        model: &str,
        questions: &Map<String, Value>,
        context: &ResolvedDecisionContextV1,
    ) -> Result<Remaining, CallFailure> {
        let request = empty_envelope(codec, model, questions)?
            .checked_add(self.state_extra)
            .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))?;
        let question = questions.keys().try_fold(0usize, |largest, name| {
            self.question_envelopes
                .get(name)
                .and_then(|bytes| bytes.checked_add(self.state_extra))
                .map(|bytes| largest.max(bytes))
                .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))
        })?;
        allowance(context, request, question, self.state_values)
            .map_err(CallFailure::BudgetRejected)
    }
}

/// The final state is encoded once. Only small empty-state question envelopes are remeasured.
pub(super) fn final_question_bytes(
    codec: DecisionWireProtocolV2,
    model: &str,
    questions: &Map<String, Value>,
    wire_len: usize,
    envelopes: &BTreeMap<String, usize>,
) -> Result<usize, CallFailure> {
    let state_extra = wire_len
        .checked_sub(empty_envelope(codec, model, questions)?)
        .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))?;
    questions.keys().try_fold(0usize, |largest, name| {
        let envelope = envelopes.get(name).ok_or(CallFailure::RejectedInput)?;
        Ok(largest.max(
            envelope
                .checked_add(state_extra)
                .ok_or(CallFailure::BudgetRejected(BudgetRejection::accounting()))?,
        ))
    })
}

/// Measurement stops at the derived wire budget, without allocating a serialized fragment.
pub(super) fn measured_bytes(
    value: &impl Serialize,
    maximum: usize,
) -> Result<Option<usize>, CallFailure> {
    struct Counter {
        remaining: usize,
        used: usize,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.remaining {
                return Err(std::io::ErrorKind::FileTooLarge.into());
            }
            self.remaining -= bytes.len();
            self.used += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Counter {
        remaining: maximum,
        used: 0,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(Some(writer.used)),
        Err(error) if error.io_error_kind() == Some(std::io::ErrorKind::FileTooLarge) => Ok(None),
        Err(_) => Err(CallFailure::RejectedInput),
    }
}
