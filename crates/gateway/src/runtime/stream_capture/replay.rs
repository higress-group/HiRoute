use super::*;
use crate::server::core_runtime::adapters::{
    NativeResponseDecoder, ProtocolAdapterError, ResponseDecodeStatus,
};
use hiroute_gateway_core::runtime::body::BudgetTree;

/// Offline only. `chunk_bytes == 0` preserves captured reader blocks; other values
/// rechunk the same response entity. Never claims to reproduce socket timing.
pub async fn replay_capture(
    path: &Path,
    chunk_bytes: usize,
) -> Result<serde_json::Value, &'static str> {
    let bytes = read_private(path, MAX_FILE).map_err(|_| "capture unavailable or unsafe")?;
    let records = records(&bytes)?;
    let context: Context = serde_json::from_slice(
        records
            .first()
            .filter(|r| r.0 == 0)
            .ok_or("missing context")?
            .1,
    )
    .map_err(|_| "invalid context")?;
    if now() >= context.session.delete_after {
        return Err("capture retention expired");
    }
    if records.last().map(|r| r.0) != Some(8) {
        return Err("capture incomplete or over limit");
    }
    let request_bytes: usize = records.iter().filter(|r| r.0 == 1).map(|r| r.1.len()).sum();
    if request_bytes as u64 != context.request_bytes || !records.iter().any(|r| r.0 == 2) {
        return Err("request body capture incomplete");
    }
    let mut status = None;
    for (_, bytes) in records.iter().filter(|r| r.0 == 3) {
        let head = u16::from_le_bytes((*bytes).try_into().map_err(|_| "invalid status")?);
        if !(100..600).contains(&head) {
            return Err("invalid status");
        }
        if (100..200).contains(&head) {
            if status.is_some() {
                return Err("informational head after final status");
            }
            continue;
        }
        if status.replace(head).is_some() {
            return Err("multiple final statuses");
        }
    }
    let status = status.ok_or("missing final status")?;
    // Native successful delivery uses NativeResponseProjector, while retryable
    // HTTP failures can bypass decoding entirely. Do not substitute a decoder
    // verdict for a production path this offline reader cannot restore.
    if !(200..300).contains(&status)
        || context.profile.ingress_protocol == context.profile.capability.upstream_protocol
    {
        return Err("unsupported_capture_path");
    }
    let entity: Vec<u8> = records
        .iter()
        .filter(|r| r.0 == 4)
        .flat_map(|r| r.1.iter().copied())
        .collect();
    let chunks: Box<dyn Iterator<Item = &[u8]>> = if chunk_bytes == 0 {
        Box::new(records.iter().filter(|r| r.0 == 4).map(|r| r.1))
    } else {
        Box::new(entity.chunks(chunk_bytes))
    };
    let tree =
        BudgetTree::new(8 * 1024 * 1024, 8 * 1024 * 1024).map_err(|_| "budget unavailable")?;
    let budget = tree
        .stream(8 * 1024 * 1024)
        .map_err(|_| "budget unavailable")?;
    let delivery =
        adapters::ActiveResponseDelivery::new(context.profile.ingress_protocol, budget.clone());
    let result = adapters::with_active_response_delivery(delivery, async {
        let mut decoder = NativeResponseDecoder::new_for_attempt(
            &context.profile,
            status,
            context.streaming && (200..300).contains(&status),
            context.chat_tools,
            budget,
        )?;
        let mut event_count = 0u64;
        for chunk in chunks {
            drain(&mut decoder, chunk, false, &mut event_count)?;
        }
        drain(&mut decoder, &[], true, &mut event_count)?;
        let position = decoder.diagnostic_position();
        let decoded = decoder.finish()?;
        Ok::<_, ProtocolAdapterError>((event_count, position, decoded.response.tool_id_map.len()))
    })
    .await;
    let result = match result {
        Ok((events, (frame, received), tools)) => {
            serde_json::json!({"result":"accepted", "events":events, "frame_index":frame, "received_bytes":received, "tool_calls":tools})
        }
        Err(error) => {
            let (reason, field) = crate::runtime::driver::response_diagnostics::category(&error);
            serde_json::json!({"result":"rejected", "reason":reason, "field":field})
        }
    };
    Ok(
        serde_json::json!({"source_sha": context.session.source_sha, "request_bytes": request_bytes,
        "response_bytes": entity.len(), "chunk_bytes": chunk_bytes, "decoder": result,
        "captured_gateway_failure": records.iter().any(|r| r.0 == 6)}),
    )
}

fn drain(
    decoder: &mut NativeResponseDecoder,
    bytes: &[u8],
    eof: bool,
    events: &mut u64,
) -> Result<(), ProtocolAdapterError> {
    let mut status = decoder.feed(bytes, eof)?;
    loop {
        *events += decoder.take_events().len() as u64;
        if status != ResponseDecodeStatus::NeedDrain {
            break;
        }
        status = decoder.resume()?;
    }
    Ok(())
}

pub(super) fn records(mut bytes: &[u8]) -> Result<Vec<(u8, &[u8])>, &'static str> {
    let mut records = Vec::new();
    while !bytes.is_empty() {
        if bytes.len() < 9 || records.len() as u64 >= MAX_RECORDS {
            return Err("invalid record boundary");
        }
        let kind = bytes[0];
        let len = u64::from_le_bytes(bytes[1..9].try_into().map_err(|_| "invalid length")?);
        bytes = &bytes[9..];
        if kind > 8 || len > bytes.len() as u64 {
            return Err("invalid record");
        }
        records.push((kind, &bytes[..len as usize]));
        bytes = &bytes[len as usize..];
    }
    Ok(records)
}
