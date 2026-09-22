//! Projection of canonical events whose downstream frames were accepted.
//! Client transcripts can update tool outcomes, never assistant content.
use super::*;
use crate::agent_turn_history::ToolStatus;
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1, WebSearchStatus};

#[derive(Default)]
pub(super) struct AcceptedOutput {
    step: Option<usize>,
    blocks: BTreeMap<u32, usize>,
    tools: BTreeMap<String, Option<(usize, usize)>>,
}

impl AcceptedOutput {
    pub(super) fn next_step(&mut self) {
        self.step = None;
        self.blocks.clear();
    }

    pub(super) fn retained_bytes(&self) -> usize {
        // Include B-tree node slack, not only payloads.
        self.blocks.len() * 256
            + self
                .tools
                .keys()
                .map(|id| 256 + id.capacity())
                .sum::<usize>()
    }
}

pub(super) fn apply_tool_results(
    active: &mut ActiveTurn,
    results: &[(usize, String, ToolStatus)],
    previous_message_count: usize,
) {
    for (message_index, id, status) in results {
        if *message_index < previous_message_count {
            continue;
        }
        // Keep an ambiguous ID unknown for the rest of this turn: a late result
        // from either old call must never be attributed to a later reuse.
        let Some(Some((step, part))) = active.output.tools.get(id).copied() else {
            continue;
        };
        active.output.tools.remove(id);
        if let Some(VisibleContentPart::ToolActivity { status: target, .. }) = active
            .steps
            .get_mut(step)
            .and_then(|step| step.get_mut(part))
        {
            *target = *status;
        }
    }
}

// An exact request retry remains in the same routing execution round under the
// existing boundary rules. Move its sealed projection back before appending another delivery;
// never reconstruct it from the client's assistant messages.
pub(super) fn reopen_finalized(entry: &mut SessionEntry, user: &[VisibleContentPart]) {
    if entry
        .closed
        .back()
        .is_some_and(|turn| turn.completed.agent_turn_id == entry.active.agent_turn_id)
    {
        let turn = entry.closed.pop_back().expect("checked last turn");
        let wire = Arc::unwrap_or_clone(turn.wire);
        entry.active.user = wire.user;
        entry.active.steps = wire.steps;
    } else {
        entry.active.user = user.to_vec();
        entry.active.capture_partial = true;
        entry.history_partial = true;
    }
    entry.active.finalized = false;
}

impl AgentTurnHistoryStore {
    pub(crate) fn accept_output(&self, ticket: &AgentTurnTicket, events: &[ModelStreamEventV1]) {
        let mut inner = self.inner.lock();
        if matching_entry_mut(&mut inner, ticket).is_err() {
            return;
        }
        for event in events {
            let (index, part, append, tool_id) = match &event.event {
                ModelEvent::TextDelta { index, text }
                | ModelEvent::RefusalDelta { index, text } => {
                    (*index, OutputPart::Text(text), true, None)
                }
                ModelEvent::TextFinished { index, text, .. }
                | ModelEvent::RefusalFinished { index, text, .. } => {
                    (*index, OutputPart::Text(text), false, None)
                }
                ModelEvent::ToolCallStarted {
                    index,
                    logical_id,
                    namespace,
                    name,
                    ..
                } => (
                    *index,
                    OutputPart::Tool(namespace.as_deref(), name, ToolStatus::Unknown),
                    false,
                    Some(logical_id),
                ),
                ModelEvent::WebSearch { index, item, .. } => {
                    let status = match item.status {
                        WebSearchStatus::Completed => ToolStatus::Completed,
                        WebSearchStatus::Failed => ToolStatus::Failed,
                        _ => ToolStatus::Unknown,
                    };
                    (
                        *index,
                        OutputPart::Tool(None, "web_search", status),
                        false,
                        None,
                    )
                }
                _ => continue,
            };
            // Reserve against the shared store before copying any output. No separate
            // response buffer or unbounded capture pool; exhausted history is partial.
            let charge = part
                .bytes()
                .saturating_add(tool_id.map_or(0, |id| id.len()))
                .saturating_mul(2)
                .saturating_add(2048);
            inner.evict_to_budget(
                self.max_memory_bytes.saturating_sub(charge),
                Some(&ticket.key),
            );
            let available = inner.accounted_bytes.saturating_add(charge) <= self.max_memory_bytes;
            let Ok(entry) = matching_entry_mut(&mut inner, ticket) else {
                return;
            };
            if !available {
                entry.active.capture_partial = true;
                entry.history_partial = true;
                continue;
            }
            let active = &mut entry.active;
            let step_index = *active.output.step.get_or_insert_with(|| {
                active.steps.reserve_exact(1);
                active.steps.push(Vec::new());
                active.steps.len() - 1
            });
            let step = &mut active.steps[step_index];
            let part_index = *active.output.blocks.entry(index).or_insert_with(|| {
                step.reserve_exact(1);
                step.push(part.owned());
                step.len() - 1
            });
            match (&mut step[part_index], &part) {
                (VisibleContentPart::Text { text }, OutputPart::Text(value)) => {
                    // Deltas append, including the first chunk;
                    // final snapshots replace, so terminal text is not duplicated.
                    if append {
                        text.reserve_exact(value.len());
                        text.push_str(value);
                    } else {
                        text.clear();
                        text.reserve_exact(value.len());
                        text.push_str(value);
                    }
                }
                (
                    VisibleContentPart::ToolActivity { status, .. },
                    OutputPart::Tool(_, _, value),
                ) => *status = *value,
                _ => active.capture_partial = true,
            }
            if let Some(id) = tool_id {
                // Ambiguous concurrent reuse affects observation only, never forwarding.
                active
                    .output
                    .tools
                    .entry(id.clone())
                    .and_modify(|position| {
                        if *position != Some((step_index, part_index)) {
                            *position = None;
                        }
                    })
                    .or_insert(Some((step_index, part_index)));
            }
            let old = entry.accounted_bytes;
            entry.accounted_bytes = entry_bytes(&ticket.key, entry);
            let new = entry.accounted_bytes;
            inner.replace_accounting(old, new);
            inner.note_peak();
        }
    }

    pub(crate) fn mark_output_partial(&self, ticket: &AgentTurnTicket) {
        if let Ok(entry) = matching_entry_mut(&mut self.inner.lock(), ticket) {
            entry.active.capture_partial = true;
            entry.history_partial = true;
        }
    }
}

enum OutputPart<'a> {
    Text(&'a str),
    Tool(Option<&'a str>, &'a str, ToolStatus),
}

impl OutputPart<'_> {
    fn bytes(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Tool(namespace, name, _) => namespace.map_or(0, str::len) + name.len() + 1,
        }
    }

    fn owned(&self) -> VisibleContentPart {
        match self {
            // The common update below installs the first text chunk too.
            Self::Text(_) => VisibleContentPart::Text {
                text: String::new(),
            },
            Self::Tool(namespace, name, status) => VisibleContentPart::ToolActivity {
                tool: namespace
                    .map_or_else(|| (*name).into(), |namespace| format!("{namespace}.{name}")),
                status: *status,
            },
        }
    }
}
