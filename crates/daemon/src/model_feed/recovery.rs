//! Recover the mutable cache only from a trusted acceptance floor; never adopt orphan blobs.
use super::*;

pub(super) fn restore_state(
    store: &FeedStore,
    minimum_generation: u64,
    minimum_sequence: u64,
) -> Result<(FeedState, Option<String>, bool), String> {
    let checkpoint = store.read_acceptance();
    let loaded = store.read_state();
    let mut state = loaded.clone().unwrap_or_else(|_| FeedState {
        retrieval_enabled: false,
        ..Default::default()
    });
    state.sequence_floor = state.sequence_floor.max(minimum_sequence);
    if let Err(reason) = checkpoint {
        discard(&mut state, minimum_generation)?;
        return Ok((state, Some(reason), false));
    }
    let checkpoint = checkpoint.unwrap();
    match store.read_data_floor() {
        Ok(Some(floor)) => match &mut state.data_floor {
            Some(current) => {
                if let Err(reason) = current.advance(&floor) {
                    discard(&mut state, minimum_generation)?;
                    return Ok((state, Some(reason), false));
                }
            }
            None => state.data_floor = Some(floor),
        },
        Ok(None) => {}
        Err(reason) => {
            discard(&mut state, minimum_generation)?;
            return Ok((state, Some(reason), false));
        }
    }

    if let Err(reason) = loaded {
        // A corrupt state could have accepted more than the control database activated.
        // Only the independently committed high-water record authorizes an explicit reset.
        let recoverable = reason == "feed_state_invalid" && checkpoint.is_some();
        state.highest_accepted = checkpoint;
        discard(&mut state, minimum_generation)?;
        return Ok((state, Some(reason), recoverable));
    }
    let original_sequence = state.highest_accepted.as_ref().map_or(0, |h| h.sequence);
    if let (Some(high), Some(saved)) = (&state.highest_accepted, &checkpoint)
        && high.sequence == saved.sequence
        && high.payload_digest != saved.payload_digest
    {
        discard(&mut state, minimum_generation)?;
        return Ok((state, Some("feed_acceptance_conflict".into()), false));
    }
    if checkpoint
        .as_ref()
        .is_some_and(|saved| saved.sequence > original_sequence)
    {
        state.highest_accepted = checkpoint.clone();
    }
    if state.generation < minimum_generation
        || original_sequence < minimum_sequence
        || checkpoint
            .as_ref()
            .is_some_and(|saved| saved.sequence > original_sequence)
    {
        discard(&mut state, minimum_generation)?;
        state.last_attempt = Some(FeedAttempt {
            at_ms: now_ms(),
            result: "validation".into(),
            reason: Some("recovered_missing_or_rolled_back_state".into()),
        });
        store.commit(&state)?;
    } else if original_sequence > checkpoint.as_ref().map_or(0, |h| h.sequence) {
        // Upgrade an existing valid pre-checkpoint record without changing its acceptance.
        store.commit(&state)?;
    }
    Ok((state, None, false))
}

fn discard(state: &mut FeedState, minimum_generation: u64) -> Result<(), String> {
    state.held = None;
    state.etag = None;
    state.retrieval_enabled = false;
    state.generation = state
        .generation
        .max(minimum_generation)
        .checked_add(1)
        .ok_or("feed_generation_overflow")?;
    Ok(())
}
