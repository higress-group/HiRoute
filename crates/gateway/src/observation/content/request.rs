use std::io::Read;

use serde_json::Value;

use crate::content_ref::{ContentRef, ContentValueExt, JsonValueExt};
use crate::replay::ReplayStore;
use crate::server::core_runtime::model_ir::{
    ContentPart, ImageSource, InstructionRole, MessageRole, ModelRequestIRV1, ToolOutput,
    ToolResultStatusV1,
};

use super::super::crypto::WorkspaceHmac;
use super::super::otel::emit_content_ref;
use super::super::schema::{ContentRefV1, LifecycleFactV1};
use super::{CANONICALIZATION_VERSION, CONTENT_CHUNK_BYTES, ContentChunk, RequestObservation};
use hiroute_diagnostics::event::{DiagnosticEvent, SpillBegin, StorageMode};

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
    /// Copies canonical request fields into the content producer before core
    /// execution starts. Replay references are opened and dropped inside this
    /// call; no observation task or queued envelope retains ReplayStore.
    pub fn capture_request(&self, request: &ModelRequestIRV1, replay: &ReplayStore) {
        if !self.is_enabled() {
            return;
        }
        self.emit_lifecycle(LifecycleFactV1::CanonicalRequestAccepted {
            canonicalization_version: CANONICALIZATION_VERSION.into(),
        });
        if !self.captures_content() {
            return;
        }
        let snapshot = replay.snapshot();
        self.emit_diagnostic(DiagnosticEvent::SpillBegin(SpillBegin {
            mode: if snapshot.disk_backed {
                StorageMode::Disk
            } else {
                StorageMode::Memory
            },
        }));
        self.begin_content("request_input", None);
        let result = self.capture_request_fields(request, replay);
        match result {
            Ok(()) => self.terminal_content("request_input", "finish", None),
            Err(()) => self.terminal_content(
                "request_input",
                "abort",
                Some("canonical_content_unavailable"),
            ),
        }
    }

    fn capture_request_fields(
        &self,
        request: &ModelRequestIRV1,
        replay: &ReplayStore,
    ) -> Result<(), ()> {
        let mut message_ordinal = 0_u32;
        for instruction in &request.instructions {
            let role = match instruction.role {
                InstructionRole::System => "system",
                InstructionRole::Developer => "developer",
            };
            self.capture_parts(replay, message_ordinal, role, &instruction.content)?;
            message_ordinal = message_ordinal.saturating_add(1);
        }
        for message in &request.messages {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::System => "system",
                MessageRole::Developer => "developer",
            };
            let mut part_ordinal = 0_u32;
            if let Some(name) = &message.name {
                self.append_value(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    role,
                    "message_name",
                    "text/plain; charset=utf-8",
                    name,
                )?;
                part_ordinal = part_ordinal.saturating_add(1);
            }
            self.capture_parts_from(
                replay,
                message_ordinal,
                role,
                part_ordinal,
                &message.content,
            )?;
            message_ordinal = message_ordinal.saturating_add(1);
        }
        for tool in &request.tools {
            let mut part_ordinal = 0_u32;
            self.append_value(
                replay,
                message_ordinal,
                part_ordinal,
                "tool_definition",
                "tool_name",
                "text/plain; charset=utf-8",
                &tool.name,
            )?;
            part_ordinal = part_ordinal.saturating_add(1);
            if let Some(description) = &tool.description {
                self.append_value(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    "tool_definition",
                    "tool_description",
                    "text/plain; charset=utf-8",
                    description,
                )?;
                part_ordinal = part_ordinal.saturating_add(1);
            }
            if let Some(schema) = &tool.input_schema {
                self.append_json(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    "tool_definition",
                    "tool_schema",
                    schema,
                )?;
            }
            message_ordinal = message_ordinal.saturating_add(1);
        }
        Ok(())
    }

    fn capture_parts(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        role: &str,
        parts: &[ContentPart],
    ) -> Result<(), ()> {
        self.capture_parts_from(replay, message_ordinal, role, 0, parts)
    }

    fn capture_parts_from(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        role: &str,
        mut part_ordinal: u32,
        parts: &[ContentPart],
    ) -> Result<(), ()> {
        for part in parts {
            match part {
                ContentPart::Text { text } => self.append_value(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    role,
                    "text",
                    "text/plain; charset=utf-8",
                    text,
                )?,
                ContentPart::Image {
                    source: ImageSource::Url { url },
                } => self.append_value(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    role,
                    "image_url",
                    "text/uri-list; charset=utf-8",
                    url,
                )?,
                ContentPart::Image {
                    source: ImageSource::Base64 { media_type, data },
                } => self.append_value(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    role,
                    "image_base64",
                    media_type,
                    data,
                )?,
                ContentPart::ToolCall {
                    logical_id,
                    namespace,
                    name,
                    arguments,
                    ..
                } => {
                    self.append_inline(
                        message_ordinal,
                        part_ordinal,
                        role,
                        "tool_call_id",
                        "text/plain; charset=utf-8",
                        logical_id.as_bytes(),
                    );
                    part_ordinal = part_ordinal.saturating_add(1);
                    if let Some(namespace) = namespace {
                        self.append_value(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_call_namespace",
                            "text/plain; charset=utf-8",
                            namespace,
                        )?;
                        part_ordinal = part_ordinal.saturating_add(1);
                    }
                    self.append_value(
                        replay,
                        message_ordinal,
                        part_ordinal,
                        role,
                        "tool_call_name",
                        "text/plain; charset=utf-8",
                        name,
                    )?;
                    part_ordinal = part_ordinal.saturating_add(1);
                    self.append_json(
                        replay,
                        message_ordinal,
                        part_ordinal,
                        role,
                        "tool_call_arguments",
                        arguments,
                    )?;
                }
                ContentPart::ToolResult {
                    logical_id,
                    output,
                    status,
                    ..
                } => {
                    self.append_inline(
                        message_ordinal,
                        part_ordinal,
                        role,
                        "tool_result_id",
                        "text/plain; charset=utf-8",
                        logical_id.as_bytes(),
                    );
                    part_ordinal = part_ordinal.saturating_add(1);
                    if *status == ToolResultStatusV1::Failed {
                        self.append_inline(
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_result_error",
                            "text/plain; charset=utf-8",
                            b"true",
                        );
                        part_ordinal = part_ordinal.saturating_add(1);
                    }
                    match output {
                        ToolOutput::Text(text) => self.append_value(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_result_text",
                            "text/plain; charset=utf-8",
                            text,
                        )?,
                        ToolOutput::Json(value) => self.append_json(
                            replay,
                            message_ordinal,
                            part_ordinal,
                            role,
                            "tool_result_json",
                            value,
                        )?,
                    }
                }
                ContentPart::ProviderState { state } => self.append_json(
                    replay,
                    message_ordinal,
                    part_ordinal,
                    role,
                    "provider_state",
                    &state.value,
                )?,
            }
            part_ordinal = part_ordinal.saturating_add(1);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn append_value(
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
            self.append_reference(
                replay,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                &reference,
            )
        } else {
            self.append_inline(
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                value.as_bytes(),
            );
            Ok(())
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn append_json(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        value: &Value,
    ) -> Result<(), ()> {
        if let Some(reference) = value.content_ref() {
            self.append_reference(
                replay,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                "application/json",
                &reference,
            )
        } else {
            let bytes = serde_json::to_vec(value).map_err(|_| ())?;
            self.append_inline(
                message_ordinal,
                part_ordinal,
                role,
                kind,
                "application/json",
                &bytes,
            );
            Ok(())
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn append_reference(
        &self,
        replay: &ReplayStore,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        media_type: &str,
        reference: &ContentRef,
    ) -> Result<(), ()> {
        let mut digest = WorkspaceHmac::new(&self.inner.key, b"conversation-content-blob");
        digest.update(CANONICALIZATION_VERSION.as_bytes());
        digest.update(media_type.as_bytes());
        let mut reader = replay.reader(reference).map_err(|_| ())?;
        let mut bytes = [0_u8; CONTENT_CHUNK_BYTES];
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
        }
        let whole_digest = format!("blob-{}", digest.finish().trim_start_matches("sha256:"));
        let message_instance_id = self.message_instance_id("request_input", message_ordinal, role);
        let content_id = self.content_id(&message_instance_id, part_ordinal, &whole_digest);
        let public_ref =
            self.public_content_ref(reference, media_type, content_id, whole_digest.clone());
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
            self.append_content_chunk(ContentChunk {
                direction: "request_input",
                attempt: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes: &bytes[..read.filled],
                public_ref: Some(public_ref.clone()),
                whole_digest: &whole_digest,
                starts_content: !emitted,
                transport_frame_id: None,
                downstream_delivery: None,
            });
            emitted = true;
        }
        self.note_content_reads("request_input", read_calls, short_reads);
        if !emitted {
            self.append_content_chunk(ContentChunk {
                direction: "request_input",
                attempt: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes: &[],
                public_ref: Some(public_ref),
                whole_digest: &whole_digest,
                starts_content: true,
                transport_frame_id: None,
                downstream_delivery: None,
            });
        }
        Ok(())
    }

    fn append_inline(
        &self,
        message_ordinal: u32,
        part_ordinal: u32,
        role: &str,
        kind: &str,
        media_type: &str,
        bytes: &[u8],
    ) {
        let whole_digest = self.content_blob_digest(media_type, bytes);
        let message_instance_id = self.message_instance_id("request_input", message_ordinal, role);
        emit_content_ref(
            self,
            "request_input",
            ContentRefV1 {
                content_id: self.content_id(&message_instance_id, part_ordinal, &whole_digest),
                digest: whole_digest.clone(),
                byte_count: bytes.len() as u64,
                media_type: media_type.into(),
            },
        );
        if bytes.is_empty() {
            self.append_content_chunk(ContentChunk {
                direction: "request_input",
                attempt: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes,
                public_ref: None,
                whole_digest: &whole_digest,
                starts_content: true,
                transport_frame_id: None,
                downstream_delivery: None,
            });
            return;
        }
        for (index, chunk) in bytes.chunks(CONTENT_CHUNK_BYTES).enumerate() {
            self.append_content_chunk(ContentChunk {
                direction: "request_input",
                attempt: None,
                message_ordinal,
                part_ordinal,
                role,
                kind,
                media_type,
                bytes: chunk,
                public_ref: None,
                whole_digest: &whole_digest,
                starts_content: index == 0,
                transport_frame_id: None,
                downstream_delivery: None,
            });
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
