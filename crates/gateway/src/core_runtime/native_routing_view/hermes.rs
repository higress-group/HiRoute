//! Select an actual observed turn input without mistaking a trailing reminder for it.
use super::*;
use crate::context_hold::ObservedUserInput;

const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_SCAN_BYTES: usize = 1024 * 1024;

pub(super) fn project(
    request: &ModelRequestIRV1,
    replay: &ReplayStore,
    budget: &StreamBudget,
    hint: &ObservedUserInput,
) -> Option<(Box<ModelRequestIRV1>, Reservation)> {
    if request.messages.len() > 2048 {
        return None;
    }
    let mut counter = BoundedCounter(0);
    serde_json::to_writer(&mut counter, request).ok()?;
    let reservation = budget
        .reserve(
            MemoryRole::ModelIrBacking,
            counter.0.checked_mul(64)?.checked_add(4096)?,
        )
        .ok()?;
    let users = request
        .messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            (message.role == MessageRole::User
                && message.content.iter().any(|part| {
                    matches!(part, ContentPart::Text { .. } | ContentPart::Image { .. })
                }))
            .then_some(index)
        })
        .take(MAX_USER_MESSAGES + 1)
        .collect::<Vec<_>>();
    if users.len() > MAX_USER_MESSAGES {
        return None;
    }
    let selected = *users.iter().rev().nth(hint.reverse_user_index)?;
    let mut scanned = 0usize;
    let mut matches = Vec::new();
    for &index in &users {
        // Restrict the optional hint to the public adapter's exact single-text
        // shape. Other multimodal/batched requests keep ordinary routing.
        let [ContentPart::Text { text }] = request.messages[index].content.as_slice() else {
            return None;
        };
        let digest = exact_text_digest(text, replay, &mut scanned)?;
        if digest == hint.text_sha256 {
            matches.push(index);
        }
    }
    if matches.as_slice() != [selected] {
        return None;
    }
    let encoded = serde_json::to_vec(request).ok()?;
    let mut view: Box<ModelRequestIRV1> = Box::new(serde_json::from_slice(&encoded).ok()?);
    view.native_only = request.native_only;
    // Earlier context and all model/tool output retain their positions. Only
    // later user envelopes cease to open a task in this semantic view. Capture
    // can still see output after the selected input and mark a cold join partial.
    for index in users.into_iter().filter(|index| *index > selected) {
        view.messages[index].role = MessageRole::System;
    }
    Some((view, reservation))
}

fn exact_text_digest(text: &str, replay: &ReplayStore, scanned: &mut usize) -> Option<String> {
    let bytes = text
        .content_ref()
        .map_or(Some(text.len()), |r| usize::try_from(r.byte_len()).ok())?;
    *scanned = scanned.checked_add(bytes)?;
    if bytes == 0 || bytes > MAX_TEXT_BYTES || *scanned > MAX_SCAN_BYTES {
        return None;
    }
    let mut digest = Sha256::new();
    if let Some(reference) = text.content_ref() {
        let mut reader = replay.reader(&reference).ok()?;
        let mut buffer = [0; 4096];
        loop {
            let read = reader.read(&mut buffer).ok()?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        reader.verify_terminal().ok()?;
    } else {
        digest.update(text.as_bytes());
    }
    Some(format!("{:x}", digest.finalize()))
}
