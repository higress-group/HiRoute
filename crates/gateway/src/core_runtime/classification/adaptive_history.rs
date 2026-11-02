//! Borrowed whole-turn/whole-step suffixes. No source history is mutated or text split.
use super::{
    CallFailure,
    context_budget::{Cost, Remaining, measured_bytes},
};
use crate::agent_turn_history::{AgentTurnStatus, VisibleAgentTurn, VisibleContentPart};
use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};
use std::sync::Arc;

#[derive(Serialize)]
pub(super) struct TurnView<'a> {
    user: &'a [VisibleContentPart],
    status: AgentTurnStatus,
    steps: Steps<'a>,
}
struct Steps<'a> {
    values: &'a [Vec<VisibleContentPart>],
    partial: bool,
}
#[derive(Serialize)]
struct Marker {
    kind: &'static str,
    source_kind: &'static str,
}
fn marker() -> [Marker; 1] {
    [Marker {
        kind: "unavailable",
        source_kind: "context_budget",
    }]
}
impl Serialize for Steps<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq =
            serializer.serialize_seq(Some(self.values.len() + usize::from(self.partial)))?;
        if self.partial {
            seq.serialize_element(&marker())?;
        }
        for step in self.values {
            seq.serialize_element(step)?;
        }
        seq.end()
    }
}

pub(super) struct Selection<'a> {
    pub turns: Vec<TurnView<'a>>,
    pub partial: bool,
    pub steps_retained: usize,
    pub steps_removed: Option<usize>,
    pub execution_evidence: bool,
}
impl<'a> Selection<'a> {
    pub fn empty(removed_turns: usize) -> Self {
        Self {
            turns: vec![],
            partial: removed_turns > 0,
            steps_retained: 0,
            steps_removed: (removed_turns == 0).then_some(0),
            execution_evidence: false,
        }
    }
}

fn part_values(part: &VisibleContentPart) -> usize {
    match part {
        VisibleContentPart::ToolActivity { .. } => 4,
        _ => 3,
    }
}
fn step_values(step: &[VisibleContentPart]) -> usize {
    1 + step.iter().map(part_values).sum::<usize>()
}
fn evidence(steps: &[Vec<VisibleContentPart>]) -> bool {
    steps.iter().flatten().any(|part| match part {
        VisibleContentPart::Text { text } => !text.is_empty(),
        VisibleContentPart::ToolActivity { .. } => true,
        VisibleContentPart::Unavailable { .. } => false,
    })
}

fn turn<'a>(
    value: &'a VisibleAgentTurn,
    remaining: &mut Remaining,
    partial_allowed: bool,
) -> Result<Option<(TurnView<'a>, Cost, usize)>, CallFailure> {
    let empty = TurnView {
        user: &value.user,
        status: value.status,
        steps: Steps {
            values: &[],
            partial: false,
        },
    };
    let Some(frame_bytes) = measured_bytes(&empty, remaining.bytes)? else {
        return Ok(None);
    };
    let frame = Cost {
        bytes: frame_bytes,
        values: 4 + value.user.iter().map(part_values).sum::<usize>(),
    };
    if !remaining.fits(frame) {
        if frame.values > remaining.values {
            remaining.limiting = hiroute_domain::DecisionContextConstraintV1::JsonShape;
        }
        return Ok(None);
    }
    let mut cost = frame;
    let mut measured = Vec::new();
    for step in value.steps.iter().rev() {
        let Some(bytes) = measured_bytes(step, remaining.bytes.saturating_sub(cost.bytes))? else {
            break;
        };
        let added = Cost {
            bytes,
            values: step_values(step),
        }
        .comma(!measured.is_empty())?;
        let candidate = cost.add(added)?;
        if !remaining.fits(candidate) {
            if candidate.values > remaining.values {
                remaining.limiting = hiroute_domain::DecisionContextConstraintV1::JsonShape;
            }
            break;
        }
        measured.push(added);
        cost = candidate;
    }
    let partial = measured.len() != value.steps.len();
    if partial {
        if !partial_allowed {
            return Ok(None);
        }
        let marker_cost = Cost {
            bytes: measured_bytes(&marker(), remaining.bytes)?
                .unwrap_or(remaining.bytes.saturating_add(1)),
            values: 4,
        };
        // Keep a real whole step after the marker. User-only remnants are never execution.
        while !measured.is_empty() {
            let candidate = cost.add(marker_cost.comma(true)?)?;
            if remaining.fits(candidate) {
                cost = candidate;
                break;
            }
            let removed = measured.pop().expect("nonempty suffix");
            cost.bytes -= removed.bytes;
            cost.values -= removed.values;
        }
        if measured.is_empty() {
            return Ok(None);
        }
    }
    let retained = measured.len();
    Ok(Some((
        TurnView {
            user: &value.user,
            status: value.status,
            steps: Steps {
                values: &value.steps[value.steps.len() - retained..],
                partial,
            },
        },
        cost,
        retained,
    )))
}

pub(super) fn admit<'a>(
    turns: &'a [Arc<VisibleAgentTurn>],
    cap: usize,
    partial_allowed: bool,
    budget: &mut Remaining,
) -> Result<Selection<'a>, CallFailure> {
    let mut result = Selection::empty(turns.len());
    for (index, value) in turns.iter().enumerate().rev().take(cap) {
        let comma = usize::from(!result.turns.is_empty());
        let Some(available) = budget.bytes.checked_sub(comma) else {
            break;
        };
        let mut available = Remaining {
            bytes: available,
            ..*budget
        };
        let candidate = turn(value, &mut available, partial_allowed)?;
        budget.limiting = available.limiting;
        let Some((view, cost, steps)) = candidate else {
            result.steps_removed = (index == 0).then_some(value.steps.len());
            break;
        };
        if !budget.admit(cost.comma(comma > 0)?) {
            break;
        }
        result.steps_retained += steps;
        result.execution_evidence |= evidence(view.steps.values);
        let partial = view.steps.partial;
        result.turns.push(view);
        if partial {
            result.steps_removed = (index == 0).then_some(value.steps.len() - steps);
            break;
        }
    }
    result.turns.reverse();
    result.partial =
        result.turns.len() != turns.len() || result.turns.iter().any(|turn| turn.steps.partial);
    if !result.partial {
        result.steps_removed = Some(0);
    }
    Ok(result)
}
