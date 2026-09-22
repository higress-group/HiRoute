use std::collections::VecDeque;
use std::sync::Arc;

use crate::server::core_runtime::adapters::{
    ChatToolProjection, ClientResponseRenderer, IncrementalClientSseRenderer,
    NativeResponseDecoder, NativeResponseProjector, RenderedClientResponse, ResponseDecodeStatus,
    ToolIdProjection,
};
use crate::server::core_runtime::model_ir::{ModelEvent, ModelStreamEventV1};
use crate::server::core_runtime::profiles::{CandidateProtocolProfile, ClientProtocolProfile};
use crate::server::request_plan::IngressProtocol;

use super::super::super::content::CanonicalFrameDelivery;
use super::CapturedNativeEvent;

const NATIVE_CAPTURE_PENDING_LIMIT: usize = 1024 * 1024;

pub(super) struct CanonicalResponseTracker {
    profile: Arc<CandidateProtocolProfile>,
    ingress: IngressProtocol,
    alias: String,
    streaming: bool,
    tool_id_projection: ToolIdProjection,
    chat_tool_projection: Option<ChatToolProjection>,
    decoder: Option<NativeResponseDecoder>,
    renderer: Option<IncrementalClientSseRenderer>,
    projector: Option<Box<NativeResponseProjector>>,
    native_output: bool,
    pending: VecDeque<CanonicalWireUnit>,
    pending_delivery: CanonicalFrameDelivery,
    native_pending: Vec<u8>,
    source_ended: bool,
}

struct CanonicalWireUnit {
    bytes: Vec<u8>,
    offset: usize,
    delivery: CanonicalFrameDelivery,
}

impl CanonicalResponseTracker {
    pub(super) fn new(
        profile: Arc<CandidateProtocolProfile>,
        tool_id_projection: ToolIdProjection,
        chat_tool_projection: Option<ChatToolProjection>,
        streaming: bool,
        alias: String,
    ) -> Self {
        Self {
            ingress: profile.ingress_protocol,
            native_output: profile.ingress_protocol == profile.capability.upstream_protocol,
            profile,
            alias,
            streaming,
            tool_id_projection,
            chat_tool_projection,
            decoder: None,
            renderer: None,
            projector: None,
            pending: VecDeque::new(),
            pending_delivery: CanonicalFrameDelivery::default(),
            native_pending: Vec::new(),
            source_ended: false,
        }
    }

    pub(super) fn observe(&mut self, event: CapturedNativeEvent) -> Result<(), &'static str> {
        match event {
            CapturedNativeEvent::Head(status) if !(100..200).contains(&status) => {
                self.begin(status)
            }
            CapturedNativeEvent::Head(_) => Ok(()),
            CapturedNativeEvent::Body(bytes) => self.feed(&bytes, false),
            CapturedNativeEvent::EndStream if self.source_ended => Ok(()),
            CapturedNativeEvent::EndStream => self.feed(&[], true),
        }
    }

    fn begin(&mut self, status: u16) -> Result<(), &'static str> {
        if self.decoder.is_some() {
            return Err("canonical_response_duplicate_head");
        }
        let streaming = self.streaming && (200..300).contains(&status);
        if self.native_output && (200..300).contains(&status) {
            self.projector = Some(Box::new(
                NativeResponseProjector::new_for_observation(
                    &self.profile,
                    streaming,
                    self.alias.clone(),
                    self.chat_tool_projection.clone(),
                    self.tool_id_projection.clone(),
                )
                .map_err(|_| "native_response_projector_unavailable")?,
            ));
        }
        self.decoder = Some(
            NativeResponseDecoder::new_for_observation(
                &self.profile,
                status,
                streaming,
                self.tool_id_projection.clone(),
                self.chat_tool_projection.take(),
            )
            .map_err(|_| "canonical_response_decoder_unavailable")?,
        );
        let client_profile = ClientProtocolProfile::for_candidate(&self.profile)
            .map_err(|_| "canonical_response_client_profile_unavailable")?;
        self.renderer = (streaming && !self.native_output)
            .then(|| IncrementalClientSseRenderer::new(client_profile, self.alias.clone()))
            .transpose()
            .map_err(|_| "canonical_response_renderer_unavailable")?;
        Ok(())
    }

    fn feed(&mut self, bytes: &[u8], end_stream: bool) -> Result<(), &'static str> {
        if self.native_output && self.streaming {
            return self.feed_native_stream(bytes, end_stream);
        }
        let mut status = self
            .decoder
            .as_mut()
            .ok_or("canonical_response_body_before_head")?
            .feed(bytes, end_stream)
            .map_err(|_| "canonical_response_decode_failed")?;
        loop {
            let events = self
                .decoder
                .as_mut()
                .ok_or("canonical_response_decoder_unavailable")?
                .take_events();
            self.observe_events(events)?;
            if status != ResponseDecodeStatus::NeedDrain {
                break;
            }
            status = self
                .decoder
                .as_mut()
                .ok_or("canonical_response_decoder_unavailable")?
                .resume()
                .map_err(|_| "canonical_response_decode_failed")?;
        }
        if end_stream {
            self.source_ended = true;
            let decoded = self
                .decoder
                .take()
                .ok_or("canonical_response_decoder_unavailable")?
                .finish()
                .map_err(|_| "canonical_response_terminal_malformed")?;
            self.observe_events(decoded.events)?;
            if !self.streaming && !self.native_output {
                let rendered = ClientResponseRenderer::render_nonstream(
                    self.ingress,
                    &self.alias,
                    &decoded.response,
                )
                .map_err(|_| "canonical_response_render_failed")?;
                let bytes = match rendered {
                    RenderedClientResponse::Json { bytes, .. } => bytes,
                    RenderedClientResponse::Sse { .. } => {
                        return Err("canonical_response_nonstream_rendered_sse");
                    }
                };
                self.push_wire(bytes);
            }
        }
        if let Some(projector) = self.projector.as_mut() {
            let projected = projector
                .feed(bytes, end_stream)
                .map_err(|_| "native_response_projection_failed")?;
            for unit in projected {
                self.push_wire(unit.bytes);
            }
        }
        Ok(())
    }

    fn feed_native_stream(&mut self, bytes: &[u8], end_stream: bool) -> Result<(), &'static str> {
        // Each projected SSE unit carries the source event's exact byte count.
        // Feed the capture-only decoder one complete source event at a time so
        // a later event in the same transport chunk cannot be counted when
        // only the first downstream unit was accepted.
        if self.native_pending.len().saturating_add(bytes.len()) > NATIVE_CAPTURE_PENDING_LIMIT {
            return Err("native_response_capture_budget_exceeded");
        }
        self.native_pending.extend_from_slice(bytes);
        let units = self
            .projector
            .as_mut()
            .ok_or("native_response_projector_unavailable")?
            .feed(bytes, end_stream)
            .map_err(|_| "native_response_projection_failed")?;
        for unit in units {
            if unit.source_bytes > self.native_pending.len() {
                return Err("native_response_event_alignment_failed");
            }
            let source = self
                .native_pending
                .drain(..unit.source_bytes)
                .collect::<Vec<_>>();
            self.feed_capture_decoder(&source, false)?;
            self.push_wire(unit.bytes);
        }
        if end_stream {
            if !self.native_pending.is_empty() {
                return Err("native_response_incomplete_tail");
            }
            self.feed_capture_decoder(&[], true)?;
            self.source_ended = true;
            self.decoder
                .take()
                .ok_or("canonical_response_decoder_unavailable")?
                .finish()
                .map_err(|_| "canonical_response_terminal_malformed")?;
        }
        Ok(())
    }

    fn feed_capture_decoder(&mut self, bytes: &[u8], end_stream: bool) -> Result<(), &'static str> {
        let mut status = self
            .decoder
            .as_mut()
            .ok_or("canonical_response_decoder_unavailable")?
            .feed(bytes, end_stream)
            .map_err(|_| "canonical_response_decode_failed")?;
        loop {
            let events = self
                .decoder
                .as_mut()
                .ok_or("canonical_response_decoder_unavailable")?
                .take_events();
            self.observe_events(events)?;
            if status != ResponseDecodeStatus::NeedDrain {
                return Ok(());
            }
            status = self
                .decoder
                .as_mut()
                .ok_or("canonical_response_decoder_unavailable")?
                .resume()
                .map_err(|_| "canonical_response_decode_failed")?;
        }
    }

    fn observe_events(&mut self, events: Vec<ModelStreamEventV1>) -> Result<(), &'static str> {
        for event in events {
            let rendered = self
                .renderer
                .as_mut()
                .map(|renderer| renderer.push(&event))
                .transpose()
                .map_err(|_| "canonical_response_render_failed")?
                .unwrap_or_default();
            // Same-protocol Native delivery records usage from the main
            // projector in provider completion. The strict decoder here is a
            // capture-only sidecar and must not create a second usage fact.
            if !self.native_output
                && let ModelEvent::UsageUpdated { usage } = &event.event
            {
                self.pending_delivery.merge_usage(usage);
            }
            if captures_conversation_content(&event.event) {
                self.pending_delivery.events.push(event);
            }
            if !rendered.is_empty() {
                let mut bytes = Vec::new();
                for frame in rendered {
                    bytes.extend_from_slice(
                        &frame
                            .wire_bytes()
                            .map_err(|_| "canonical_response_render_failed")?,
                    );
                }
                self.push_wire(bytes);
            }
        }
        Ok(())
    }

    fn push_wire(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.pending.push_back(CanonicalWireUnit {
                bytes,
                offset: 0,
                delivery: std::mem::take(&mut self.pending_delivery),
            });
        }
    }

    pub(super) fn correlate_accepted_output(
        &mut self,
        bytes: &[u8],
        end_stream: bool,
    ) -> Result<CanonicalFrameDelivery, &'static str> {
        let mut delivery = CanonicalFrameDelivery::default();
        let mut actual_offset = 0;
        while actual_offset < bytes.len() {
            let unit = self
                .pending
                .front_mut()
                .ok_or("canonical_response_unexpected_wire_bytes")?;
            let remaining = &unit.bytes[unit.offset..];
            let compared = remaining.len().min(bytes.len() - actual_offset);
            if remaining[..compared] != bytes[actual_offset..actual_offset + compared] {
                return Err("canonical_response_wire_mismatch");
            }
            unit.offset += compared;
            actual_offset += compared;
            if unit.offset == unit.bytes.len() {
                let unit = self
                    .pending
                    .pop_front()
                    .ok_or("canonical_response_pending_unit_lost")?;
                delivery.merge(unit.delivery);
            }
        }
        if end_stream {
            if !self.pending.is_empty() {
                return Err("canonical_response_wire_truncated");
            }
            if self.native_output && self.streaming && !self.native_pending.is_empty() {
                return Err("native_response_incomplete_tail");
            }
            delivery.merge(std::mem::take(&mut self.pending_delivery));
        }
        Ok(delivery)
    }
}

fn captures_conversation_content(event: &ModelEvent) -> bool {
    matches!(
        event,
        ModelEvent::ContentBlockStarted { .. }
            | ModelEvent::TextDelta { .. }
            | ModelEvent::ReasoningDelta { .. }
            | ModelEvent::RefusalDelta { .. }
            | ModelEvent::ToolCallStarted { .. }
            | ModelEvent::ToolArgumentsDelta { .. }
            | ModelEvent::ToolCallFinished { .. }
            | ModelEvent::TextFinished { .. }
            | ModelEvent::RefusalFinished { .. }
            | ModelEvent::WebSearch { .. }
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::server::core_runtime::adapters::ToolIdProjection;
    use crate::server::core_runtime::profiles::fixed_reasoning;

    fn projection() -> ToolIdProjection {
        ToolIdProjection::new(IngressProtocol::Responses)
    }

    #[test]
    fn native_capture_does_not_duplicate_main_projector_usage() {
        let profile = Arc::new(CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        ));
        let mut tracker =
            CanonicalResponseTracker::new(profile, projection(), None, false, "alias".into());
        let mut response = json!({
            "id": "native-response",
            "model": "physical",
            "status": "completed",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "ok"}]
            }],
            "usage": {"input_tokens": 3, "output_tokens": 2, "total_tokens": 5}
        });

        tracker.observe(CapturedNativeEvent::Head(200)).unwrap();
        tracker
            .observe(CapturedNativeEvent::Body(
                serde_json::to_vec(&response).unwrap(),
            ))
            .unwrap();
        tracker.observe(CapturedNativeEvent::EndStream).unwrap();

        response["model"] = "alias".into();
        let delivery = tracker
            .correlate_accepted_output(&serde_json::to_vec(&response).unwrap(), true)
            .unwrap();
        assert!(delivery.usage.is_none());
        assert_eq!(delivery.events.len(), 3);
        assert!(matches!(
            &delivery.events[0].event,
            ModelEvent::ContentBlockStarted { .. }
        ));
        assert!(matches!(
            &delivery.events[1].event,
            ModelEvent::TextDelta { text, .. } if text == "ok"
        ));
        assert!(matches!(
            &delivery.events[2].event,
            ModelEvent::TextFinished { text, .. } if text == "ok"
        ));
    }

    #[test]
    fn native_capture_attributes_only_the_accepted_sse_unit_from_a_shared_chunk() {
        let profile = Arc::new(CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        ));
        let mut wire_projector = NativeResponseProjector::new_for_observation(
            &profile,
            true,
            "alias".into(),
            None,
            projection(),
        )
        .unwrap();
        let mut tracker =
            CanonicalResponseTracker::new(profile, projection(), None, true, "alias".into());
        let created = b"event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"response\",\"model\":\"physical\"}}\n\n";
        let delta = b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"item_id\":\"item\",\"output_index\":0,\"content_index\":0,\"delta\":\"not-delivered-yet\"}\n\n";
        let terminal = b"event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"response\",\"model\":\"physical\",\"status\":\"completed\",\"output\":[{\"type\":\"message\",\"id\":\"item\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"not-delivered-yet\"}]}]}}\n\n";
        tracker.observe(CapturedNativeEvent::Head(200)).unwrap();
        tracker
            .observe(CapturedNativeEvent::Body(
                [created.as_slice(), delta.as_slice(), terminal.as_slice()].concat(),
            ))
            .unwrap();

        let projected_created = wire_projector.feed(created, false).unwrap().remove(0).bytes;
        let first = tracker
            .correlate_accepted_output(&projected_created, false)
            .unwrap();
        assert!(
            first.events.is_empty(),
            "later chunk events are not accepted yet"
        );
        let second = tracker.correlate_accepted_output(delta, false).unwrap();
        assert!(second.events.iter().any(|event| {
            matches!(&event.event, ModelEvent::TextDelta { text, .. } if text == "not-delivered-yet")
        }));
    }
}
