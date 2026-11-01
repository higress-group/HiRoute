use std::io::Read;

use serde_json::Value;

use crate::content_ref::{ContentRef, ContentValueExt, JsonValueExt};
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::{
    CanonicalMessage, ContentPart, ImageSource, MessageRole, ModelRequestIRV1, ToolOutput,
    ToolResultStatusV1,
};

use super::super::crypto::WorkspaceHmac;
use super::super::otel::emit_content_ref;
use super::super::schema::{ContentRefV1, LifecycleFactV1};
use super::{CANONICALIZATION_VERSION, CONTENT_CHUNK_BYTES, ContentChunk, RequestObservation};
use hiroute_diagnostics::event::{DiagnosticEvent, SpillBegin, StorageMode};

#[path = "request_json.rs"]
mod json;

/// One coalescing read: how many bytes were filled and what the reads cost.
#[derive(Debug)]
struct ChunkRead {
    filled: usize,
    read_calls: u64,
    short_reads: u64,
}

fn read_content_chunk(reader: &mut impl Read, bytes: &mut [u8]) -> std::io::Result<ChunkRead> {
    let mut filled = 0;
    let mut read_calls = 0_u64;
    let mut short_reads = 0_u64;
    while filled < bytes.len() {
        read_calls = read_calls.saturating_add(1);
        match reader.read(&mut bytes[filled..]) {
            Ok(0) => break,
            Ok(count) => {
                if count < bytes.len() - filled {
                    short_reads = short_reads.saturating_add(1);
                }
                filled += count;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(ChunkRead {
        filled,
        read_calls,
        short_reads,
    })
}

impl RequestObservation {
    pub(crate) fn record_input_selection(
        &self,
        selection: hiroute_domain::RequestInputSelectionV1,
    ) {
        self.emit_execution(
            super::super::schema::ExecutionFactV1::RequestInputSelection(selection),
            None,
        );
    }

    pub(crate) fn canonical_request_accepted(&self) {
        if !self.is_enabled() {
            return;
        }
        let already = {
            let mut state = self.lock_state();
            std::mem::replace(&mut state.canonical_request_recorded, true)
        };
        if !already {
            self.emit_lifecycle(LifecycleFactV1::CanonicalRequestAccepted {
                canonicalization_version: CANONICALIZATION_VERSION.into(),
            });
        }
    }

    /// Records selected semantic activity after the existing continuity owner
    /// has selected its boundary. This does not snapshot setup or tool schemas.
    pub fn capture_request(&self, request: &ModelRequestIRV1, replay: &ReplayStore) {
        if !self.is_enabled() {
            return;
        }
        self.canonical_request_accepted();
        if !self.captures_content() {
            return;
        }
        self.inner.content_spool.bind(replay.overflow_factory());
        if self.inner.input_capture.get().is_none() {
            self.select_input_capture(
                request,
                None,
                request
                    .messages
                    .iter()
                    .rposition(|m| m.role == MessageRole::User),
                None,
            );
        }
        let start = self.inner.input_capture.get().map_or(0, |s| s.start);
        super::super::content_work::inline(
            self.capture_request_payload_run(&request.messages[start..], replay),
        );
    }

    pub(in crate::server::core_runtime::observation) async fn capture_request_payload_run(
        &self,
        messages: &[CanonicalMessage],
        replay: &ReplayStore,
    ) -> bool {
        let snapshot = replay.snapshot();
        self.emit_diagnostic(DiagnosticEvent::SpillBegin(SpillBegin {
            mode: if snapshot.disk_backed {
                StorageMode::Disk
            } else {
                StorageMode::Memory
            },
        }));
        self.begin_content_at_run("request_input", None).await;
        let result = self.capture_request_fields_run(messages, replay).await;
        match result {
            Ok(()) => {
                self.terminal_content_run("request_input", "finish", None)
                    .await
            }
            Err(()) => {
                self.terminal_content_run(
                    "request_input",
                    "abort",
                    Some("canonical_content_unavailable"),
                )
                .await
            }
        }
        result.is_ok()
    }

    async fn capture_request_fields_run(
        &self,
        messages: &[CanonicalMessage],
        replay: &ReplayStore,
    ) -> Result<(), ()> {
        let mut message_ordinal = self
            .inner
            .input_capture
            .get()
            .and_then(|s| u32::try_from(s.ordinal_base).ok())
            .unwrap_or(0);
        for message in messages {
            if matches!(message.role, MessageRole::System | MessageRole::Developer) {
                message_ordinal = message_ordinal.saturating_add(1);
                continue;
            }
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "system",
                MessageRole::Developer => "developer",
            };
            let mut part_ordinal = 0_u32;
            if let Some(name) = &message.name {
                self.append_value_run(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    role,
                    "message_name",
                    "text/plain; charset=utf-8",
                    name,
                )
                .await?;
                part_ordinal = part_ordinal.saturating_add(1);
            }
            self.capture_parts_from_run(
                replay,
                message_ordinal,
                role,
                part_ordinal,
                &message.content,
            )
            .await?;
            message_ordinal = message_ordinal.saturating_add(1);
        }
        Ok(())
    }

    async fn capture_parts_from_run(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        role: &str,
        mut part_ordinal: u32,
        parts: &[ContentPart],
    ) -> Result<(), ()> {
        for part in parts {
            if self.content_capture_failed() {
                return Err(());
            }
            match part {
                ContentPart::Text { text } => {
                    self.append_value_run(
                        replay,
                        message_ordinal,
                        part_ordinal,
                        role,
                        "text",
                        "text/plain; charset=utf-8",
                        text,
                    )
                    .await?
                }
                ContentPart::Image {
                    source: ImageSource::Url { url },
                } => {
                    self.append_value_run(
                        replay,
                        message_ordinal,
                        part_ordinal,
                        role,
                        "image_url",
                        "text/uri-list; charset=utf-8",
                        url,
                    )
                    .await?
                }
                ContentPart::Image {
                    source: ImageSource::Base64 { media_type, data },
                } => {
                    self.append_value_run(
                        replay,
                        message_ordinal,
                        part_ordinal,
                        role,
                        "image_base64",
                        media_type,
                        data,
                    )
                    .await?
                }
                ContentPart::ToolCall {
                    logical_id,
                    namespace,
                    name,
                    arguments,
                    raw_arguments,
                    ..
                } => {
                    self.append_inline_run(
                        message_ordinal,
                        part_ordinal,
                        role,
                        "tool_call_id",
                        "text/plain; charset=utf-8",
                        logical_id.as_bytes(),
                    )
                    .await;
                    part_ordinal = part_ordinal.saturating_add(1);
                    if let Some(namespace) = namespace {
                        self.append_value_run(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_call_namespace",
                            "text/plain; charset=utf-8",
                            namespace,
                        )
                        .await?;
                        part_ordinal = part_ordinal.saturating_add(1);
                    }
                    self.append_value_run(
                        replay,
                        message_ordinal,
                        part_ordinal,
                        role,
                        "tool_call_name",
                        "text/plain; charset=utf-8",
                        name,
                    )
                    .await?;
                    part_ordinal = part_ordinal.saturating_add(1);
                    if let Some(raw) = raw_arguments {
                        self.append_value_run(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_call_arguments",
                            "text/plain; charset=utf-8",
                            raw,
                        )
                        .await?;
                    } else {
                        self.append_json_run(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_call_arguments",
                            arguments,
                        )
                        .await?;
                    }
                }
                ContentPart::ToolResult {
                    logical_id,
                    output,
                    status,
                    ..
                } => {
                    self.append_inline_run(
                        message_ordinal,
                        part_ordinal,
                        role,
                        "tool_result_id",
                        "text/plain; charset=utf-8",
                        logical_id.as_bytes(),
                    )
                    .await;
                    part_ordinal = part_ordinal.saturating_add(1);
                    if *status == ToolResultStatusV1::Failed {
                        self.append_inline_run(
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_result_error",
                            "text/plain; charset=utf-8",
                            b"true",
                        )
                        .await;
                        part_ordinal = part_ordinal.saturating_add(1);
                    }
                    match output {
                        ToolOutput::Text(text) => {
                            self.append_value_run(
                                replay,
                                message_ordinal,
                                part_ordinal,
                                role,
                                "tool_result_text",
                                "text/plain; charset=utf-8",
                                text,
                            )
                            .await?
                        }
                        ToolOutput::Json(value) => {
                            self.append_json_run(
                                replay,
                                message_ordinal,
                                part_ordinal,
                                role,
                                "tool_result_json",
                                value,
                            )
                            .await?
                        }
                    }
                }
                ContentPart::ProviderState { state } => {
                    if state.kind == "reasoning_content" {
                        self.append_json_run(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "reasoning",
                            &state.value,
                        )
                        .await?;
                    } else if let Some(text) =
                        crate::content_ref::unsigned_messages_thinking_text(state)
                    {
                        self.append_value_run(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "reasoning",
                            "text/plain; charset=utf-8",
                            text,
                        )
                        .await?;
                    }
                    // Opaque provider state is forwarding material, not history.
                }
            }
            part_ordinal = part_ordinal.saturating_add(1);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn append_value_run(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        media_type: &str,
        value: &str,
    ) -> Result<(), ()> {
        if let Some(reference) = value.content_ref() {
            self.append_reference_run(
                replay,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                &reference,
            )
            .await
        } else {
            self.append_inline_run(
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                value.as_bytes(),
            )
            .await;
            Ok(())
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn append_json_run(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        value: &Value,
    ) -> Result<(), ()> {
        if let Some(reference) = value.content_ref() {
            self.append_reference_run(
                replay,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                "application/json",
                &reference,
            )
            .await
        } else {
            self.append_replay_json_run(replay, message_ordinal, part_ordinal, role, kind, value)
                .await
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn append_reference_run(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        media_type: &str,
        reference: &ContentRef,
    ) -> Result<(), ()> {
        if self.content_capture_failed() {
            return Err(());
        }
        let _drain = super::PartDrain::new(self, false);
        let mut digest = WorkspaceHmac::new(&self.inner.key, b"conversation-content-blob");
        digest.update(CANONICALIZATION_VERSION.as_bytes());
        digest.update(media_type.as_bytes());
        let mut reader = replay.reader(reference).map_err(|_| ())?;
        let mut bytes = vec![0_u8; CONTENT_CHUNK_BYTES];
        let mut read_calls = 0_u64;
        let mut short_reads = 0_u64;
        loop {
            let read = reader.read(&mut bytes).map_err(|_| ())?;
            read_calls = read_calls.saturating_add(1);
            if read == 0 {
                break;
            }
            if read < bytes.len() {
                short_reads = short_reads.saturating_add(1);
            }
            digest.update_stream(&bytes[..read]);
            self.content_checkpoint(read, false).await;
        }
        let whole_digest = format!("blob-{}", digest.finish().trim_start_matches("sha256:"));
        let message_instance_id = self.message_instance_id("request_input", message_ordinal, role);
        let content_id = self.content_id(&message_instance_id, part_ordinal, &whole_digest);
        let public_ref = self.public_content_ref(
            reference,
            media_type,
            content_id.clone(),
            whole_digest.clone(),
        );
        emit_content_ref(self, "request_input", public_ref.clone());
        let mut reader = replay.reader(reference).map_err(|_| ())?;
        let mut emitted = false;
        loop {
            let read = read_content_chunk(&mut reader, &mut bytes).map_err(|_| ())?;
            read_calls = read_calls.saturating_add(read.read_calls);
            short_reads = short_reads.saturating_add(read.short_reads);
            if read.filled == 0 {
                break;
            }
            self.append_content_chunk_run(ContentChunk {
                direction: "request_input",
                attempt_id: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes: &bytes[..read.filled],
                public_ref: Some(public_ref.clone()),
                whole_digest: &whole_digest,
                message_instance_id: &message_instance_id,
                content_id: &content_id,
                starts_content: !emitted,
                transport_frame_id: None,
                downstream_delivery: None,
            })
            .await;
            emitted = true;
        }
        self.note_content_reads("request_input", read_calls, short_reads);
        if !emitted {
            self.append_content_chunk_run(ContentChunk {
                direction: "request_input",
                attempt_id: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes: &[],
                public_ref: Some(public_ref.clone()),
                whole_digest: &whole_digest,
                message_instance_id: &message_instance_id,
                content_id: &content_id,
                starts_content: true,
                transport_frame_id: None,
                downstream_delivery: None,
            })
            .await;
        }
        Ok(())
    }

    async fn append_inline_run(
        &self,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        media_type: &str,
        bytes: &[u8],
    ) {
        if self.content_capture_failed() {
            return;
        }
        let _drain = super::PartDrain::new(self, false);
        let mut digest = WorkspaceHmac::new(&self.inner.key, b"conversation-content-blob");
        digest.update(CANONICALIZATION_VERSION.as_bytes());
        digest.update(media_type.as_bytes());
        for chunk in bytes.chunks(CONTENT_CHUNK_BYTES) {
            digest.update_stream(chunk);
            self.content_checkpoint(chunk.len(), false).await;
        }
        let whole_digest = format!("blob-{}", digest.finish().trim_start_matches("sha256:"));
        let message_instance_id = self.message_instance_id("request_input", message_ordinal, role);
        let content_id = self.content_id(&message_instance_id, part_ordinal, &whole_digest);
        emit_content_ref(
            self,
            "request_input",
            ContentRefV1 {
                content_id: content_id.clone(),
                digest: whole_digest.clone(),
                byte_count: bytes.len() as u64,
                media_type: media_type.into(),
            },
        );
        if bytes.is_empty() {
            self.append_content_chunk_run(ContentChunk {
                direction: "request_input",
                attempt_id: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes,
                public_ref: None,
                whole_digest: &whole_digest,
                message_instance_id: &message_instance_id,
                content_id: &content_id,
                starts_content: true,
                transport_frame_id: None,
                downstream_delivery: None,
            })
            .await;
            return;
        }
        for (index, chunk) in bytes.chunks(CONTENT_CHUNK_BYTES).enumerate() {
            self.append_content_chunk_run(ContentChunk {
                direction: "request_input",
                attempt_id: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes: chunk,
                public_ref: None,
                whole_digest: &whole_digest,
                message_instance_id: &message_instance_id,
                content_id: &content_id,
                starts_content: index == 0,
                transport_frame_id: None,
                downstream_delivery: None,
            })
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ShortReads<'a>(&'a [u8]);
    impl Read for ShortReads<'_> {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            let count = bytes.len().min(1);
            self.0.read(&mut bytes[..count])
        }
    }

    #[test]
    fn request_content_chunks_do_not_follow_replay_segment_boundaries() {
        let input = vec![b'x'; CONTENT_CHUNK_BYTES + 3];
        let mut reader = ShortReads(&input);
        let mut bytes = [0; CONTENT_CHUNK_BYTES];
        let first = read_content_chunk(&mut reader, &mut bytes).unwrap();
        assert_eq!(first.filled, CONTENT_CHUNK_BYTES);
        // The source delivers one byte per read: every read but the last one, which
        // already asked for a single byte, was short.
        assert_eq!(first.read_calls, CONTENT_CHUNK_BYTES as u64);
        assert_eq!(first.short_reads, CONTENT_CHUNK_BYTES as u64 - 1);
        assert_eq!(bytes.as_slice(), &input[..CONTENT_CHUNK_BYTES]);
        let second = read_content_chunk(&mut reader, &mut bytes).unwrap();
        assert_eq!(second.filled, 3);
        assert_eq!(second.read_calls, 4);
        assert_eq!(second.short_reads, 3);
        assert_eq!(bytes[..3], input[CONTENT_CHUNK_BYTES..]);
        let eof = read_content_chunk(&mut reader, &mut bytes).unwrap();
        assert_eq!(eof.filled, 0);
        assert_eq!(eof.read_calls, 1);
        assert_eq!(eof.short_reads, 0);
    }

    #[test]
    fn request_content_chunk_rejects_incomplete_integrity_reads() {
        struct Broken(bool);
        impl Read for Broken {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                if self.0 {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
                self.0 = true;
                bytes[0] = b'x';
                Ok(1)
            }
        }
        assert_eq!(
            read_content_chunk(&mut Broken(false), &mut [0; 8])
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}
