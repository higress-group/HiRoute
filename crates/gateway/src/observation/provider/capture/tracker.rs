use hiroute_gateway_core::runtime::body::{MemoryRole, Reservation, StreamBudget};
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

#[cfg(test)]
#[path = "memory_tests.rs"]
mod memory_tests;

pub(super) struct CanonicalResponseTracker {
    budget: StreamBudget,
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
    native_pending_reservation: Option<Reservation>,
    source_ended: bool,
}

struct CanonicalWireUnit {
    bytes: Vec<u8>,
    offset: usize,
    delivery: CanonicalFrameDelivery,
}

impl CanonicalResponseTracker {
    #[cfg(test)]
    pub(super) fn new(
        profile: Arc<CandidateProtocolProfile>,
        tool_id_projection: ToolIdProjection,
        chat_tool_projection: Option<ChatToolProjection>,
        streaming: bool,
        alias: String,
    ) -> Self {
        let budget =
            hiroute_gateway_core::runtime::body::BudgetTree::new(4 * 1024 * 1024, 4 * 1024 * 1024)
                .unwrap()
                .stream(4 * 1024 * 1024)
                .unwrap();
        Self::new_with_budget(
            profile,
            tool_id_projection,
            chat_tool_projection,
            streaming,
            alias,
            budget,
        )
    }

    pub(super) fn new_with_budget(
        profile: Arc<CandidateProtocolProfile>,
        tool_id_projection: ToolIdProjection,
        chat_tool_projection: Option<ChatToolProjection>,
        streaming: bool,
        alias: String,
        budget: StreamBudget,
    ) -> Self {
        Self {
            budget,
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
            native_pending_reservation: None,
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
                NativeResponseProjector::new_for_observation_with_budget(
                    &self.profile,
                    streaming,
                    self.alias.clone(),
                    self.chat_tool_projection.clone(),
                    self.tool_id_projection.clone(),
                    self.budget.clone(),
                )
                .map_err(|_| "native_response_projector_unavailable")?,
            ));
        }
        self.decoder = Some(
            NativeResponseDecoder::new_for_observation_with_budget(
                &self.profile,
                status,
                streaming,
                self.tool_id_projection.clone(),
                self.chat_tool_projection.take(),
                self.budget.clone(),
            )
            .map_err(|_| "canonical_response_decoder_unavailable")?,
        );
        let client_profile = ClientProtocolProfile::for_candidate(&self.profile)
            .map_err(|_| "canonical_response_client_profile_unavailable")?;
        self.renderer = (streaming && !self.native_output)
            .then(|| IncrementalClientSseRenderer::new(client_profile, self.alias.clone()))
            .transpose()
            .map_err(|_| "canonical_response_renderer_unavailable")?
            .map(|renderer| renderer.with_budget(self.budget.clone()));
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
                self.push_wire(bytes)?;
            }
        }
        if let Some(projector) = self.projector.as_mut() {
            let projected = projector
                .feed(bytes, end_stream)
                .map_err(|_| "native_response_projection_failed")?;
            for unit in projected {
                self.push_wire(unit.bytes)?;
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
        let needed = self.native_pending.len() + bytes.len();
        if needed > self.native_pending.capacity() {
            let capacity = needed.max(self.native_pending.capacity().saturating_mul(2));
            let reservation = self
                .budget
                .reserve(MemoryRole::SseFrame, capacity)
                .map_err(|_| "canonical_capture_budget_exceeded")?;
            let mut pending = Vec::with_capacity(capacity);
            pending.extend_from_slice(&self.native_pending);
            self.native_pending = pending;
            self.native_pending_reservation = Some(reservation);
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
            self.push_wire(unit.bytes)?;
        }
        if self.native_pending.is_empty() {
            self.native_pending = Vec::new();
            self.native_pending_reservation = None;
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
                // Count without allocating a serialized copy. The native job
                // covers decode temporaries; this charge survives until the
                // exact projected wire unit is accepted and published.
                let mut size = SerializedSize(0);
                serde_json::to_writer(&mut size, &event)
                    .map_err(|_| "canonical_response_serialization_failed")?;
                let bytes =
                    super::capture_charge(size.0, 4).ok_or("canonical_capture_budget_overflow")?;
                self.pending_delivery.reservations.push(
                    self.budget
                        .reserve(MemoryRole::OutputQueue, bytes)
                        .map_err(|_| "canonical_capture_budget_exceeded")?,
                );
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
                self.push_wire(bytes)?;
            }
        }
        Ok(())
    }

    fn push_wire(&mut self, bytes: Vec<u8>) -> Result<(), &'static str> {
        if !bytes.is_empty() {
            let charge = super::capture_charge(bytes.capacity(), 1)
                .ok_or("canonical_capture_budget_overflow")?;
            self.pending_delivery.reservations.push(
                self.budget
                    .reserve(MemoryRole::OutputQueue, charge)
                    .map_err(|_| "canonical_capture_budget_exceeded")?,
            );
            self.pending.push_back(CanonicalWireUnit {
                bytes,
                offset: 0,
                delivery: std::mem::take(&mut self.pending_delivery),
            });
        }
        Ok(())
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
        // Do not retain a formerly large deque after its charged units drain.
        // A tiny reusable allocation is covered by the capture's base charge.
        if self.pending.capacity() > self.pending.len().saturating_mul(2).max(8) {
            self.pending.shrink_to_fit();
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

struct SerializedSize(usize);

impl std::io::Write for SerializedSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .ok_or_else(|| std::io::Error::other("capture size overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
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
    fn native_parallel_tools_capture_distinct_arguments_and_reject_conflicting_deltas() {
        // A real Bailian response repeated call 0's delta for call 1, while
        // call 1's done/snapshot contained different arguments. Native clients
        // can execute the snapshot, but capture must not certify that stream.
        for conflicting in [false, true] {
            let profile = Arc::new(CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                IngressProtocol::Responses,
                "physical",
                fixed_reasoning("fixed"),
            ));
            let mut events = vec![json!({"type":"response.created",
                "response":{"id":"response","model":"physical","status":"in_progress"}})];
            let arguments = [r#"{"path":"alpha.txt"}"#, r#"{"path":"beta.txt"}"#];
            let mut items = Vec::new();
            for (index, args) in arguments.iter().enumerate() {
                let item_id = format!("item-{index}");
                let mut item = json!({"type":"function_call","id":item_id,
                    "call_id":format!("call-{index}"),"name":"read_file",
                    "arguments":"","status":"in_progress"});
                events.push(json!({"type":"response.output_item.added",
                    "output_index":index,"item":item}));
                let delta = if conflicting && index == 1 {
                    arguments[0]
                } else {
                    args
                };
                events.push(json!({"type":"response.function_call_arguments.delta",
                    "output_index":index,"item_id":item_id,"delta":delta}));
                events.push(json!({"type":"response.function_call_arguments.done",
                    "output_index":index,"item_id":item_id,"name":"read_file","arguments":args}));
                item["arguments"] = json!(args);
                item["status"] = json!("completed");
                events.push(json!({"type":"response.output_item.done",
                    "output_index":index,"item":item}));
                items.push(item);
            }
            events.push(json!({"type":"response.completed","response":{
                "id":"response","model":"physical","status":"completed","output":items}}));
            let wire = events
                .iter()
                .flat_map(|event| format!("data:{event}\n\n").into_bytes())
                .collect::<Vec<_>>();
            let mut decoder =
                NativeResponseDecoder::new_for_observation(&profile, 200, true, projection(), None)
                    .unwrap();
            if conflicting {
                let error = decoder.feed(&wire, true).unwrap_err().to_string();
                assert!(
                    error.contains("native final tool arguments disagree with deltas"),
                    "{error}"
                );
            } else {
                decoder.feed(&wire, true).unwrap();
                decoder.finish().unwrap();
            }

            let mut wire_projector = NativeResponseProjector::new_for_observation(
                &profile,
                true,
                "physical".into(),
                None,
                projection(),
            )
            .unwrap();
            let mut tracker =
                CanonicalResponseTracker::new(profile, projection(), None, true, "physical".into());
            tracker.observe(CapturedNativeEvent::Head(200)).unwrap();
            let mut captured = CanonicalFrameDelivery::default();
            let mut failure = None;
            let mut projected = Vec::new();
            for event in &events {
                let bytes = format!("data:{event}\n\n").into_bytes();
                for chunk in bytes.chunks(31) {
                    if failure.is_none() {
                        failure = tracker
                            .observe(CapturedNativeEvent::Body(chunk.to_vec()))
                            .err();
                    }
                    for unit in wire_projector.feed(chunk, false).unwrap() {
                        if failure.is_none() {
                            captured.merge(
                                tracker
                                    .correlate_accepted_output(&unit.bytes, false)
                                    .unwrap(),
                            );
                        }
                        projected.extend(unit.bytes);
                    }
                }
            }
            if !conflicting {
                tracker.observe(CapturedNativeEvent::EndStream).unwrap();
            }
            for unit in wire_projector.feed(&[], true).unwrap() {
                if !conflicting {
                    captured.merge(
                        tracker
                            .correlate_accepted_output(&unit.bytes, false)
                            .unwrap(),
                    );
                }
                projected.extend(unit.bytes);
            }
            if !conflicting {
                captured.merge(tracker.correlate_accepted_output(&[], true).unwrap());
            }
            assert_eq!(
                failure,
                conflicting.then_some("canonical_response_decode_failed")
            );
            let projected_events = String::from_utf8(projected)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| serde_json::from_str::<serde_json::Value>(data).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                projected_events, events,
                "capture must not rewrite delivered provider arguments"
            );
            let finished = captured
                .events
                .iter()
                .filter_map(|event| match &event.event {
                    ModelEvent::ToolCallFinished { logical_id, .. } => Some(logical_id.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                finished,
                if conflicting {
                    vec!["call-0"]
                } else {
                    vec!["call-0", "call-1"]
                }
            );
        }
    }

    #[test]
    fn native_capture_preserves_reasoning_with_nullable_content_and_final_answer() {
        // Bailian emits reasoning_text deltas, then a summary snapshot with
        // content:null/status:null. Both stream and batch capture must retain
        // the following answer rather than abort after the reasoning item.
        for streaming in [false, true] {
            let profile = Arc::new(CandidateProtocolProfile::exact_portable_path(
                IngressProtocol::Responses,
                IngressProtocol::Responses,
                "physical",
                fixed_reasoning("fixed"),
            ));
            let reasoning = json!({"type":"reasoning","id":"reasoning","status":null,
                "summary":[{"type":"summary_text","text":"fixture thought"}],
                "content":null,"encrypted_content":null});
            let message = json!({"type":"message","id":"answer","role":"assistant",
                "status":"completed","content":[{"type":"output_text","text":"DONE"}]});
            let response = json!({"id":"response","model":"physical","status":"completed",
                "output":[reasoning,message],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}});
            let wire = if streaming {
                [
                    json!({"type":"response.created","response":{"id":"response","model":"physical","status":"in_progress"}}),
                    json!({"type":"response.output_item.added","output_index":0,"item":{
                        "type":"reasoning","id":"reasoning","summary":[],"content":null,"status":null}}),
                    json!({"type":"response.reasoning_text.delta","output_index":0,"item_id":"reasoning","content_index":0,"delta":"fixture thought"}),
                    json!({"type":"response.reasoning_text.done","output_index":0,"item_id":"reasoning","content_index":0,"text":"fixture thought"}),
                    json!({"type":"response.output_item.done","output_index":0,"item":reasoning}),
                    json!({"type":"response.output_text.delta","output_index":1,"item_id":"answer","content_index":0,"delta":"DONE"}),
                    json!({"type":"response.completed","response":response}),
                ].into_iter().flat_map(|event| format!("data:{event}\n\n").into_bytes()).collect::<Vec<_>>()
            } else {
                serde_json::to_vec(&response).unwrap()
            };
            let mut wire_projector = NativeResponseProjector::new_for_observation(
                &profile,
                streaming,
                "alias".into(),
                None,
                projection(),
            )
            .unwrap();
            let mut tracker = CanonicalResponseTracker::new(
                profile,
                projection(),
                None,
                streaming,
                "alias".into(),
            );
            let mut delivered = CanonicalFrameDelivery::default();
            tracker.observe(CapturedNativeEvent::Head(200)).unwrap();
            for chunk in wire.chunks(73) {
                tracker
                    .observe(CapturedNativeEvent::Body(chunk.to_vec()))
                    .unwrap();
                for unit in wire_projector.feed(chunk, false).unwrap() {
                    delivered.merge(
                        tracker
                            .correlate_accepted_output(&unit.bytes, false)
                            .unwrap(),
                    );
                }
            }
            tracker.observe(CapturedNativeEvent::EndStream).unwrap();
            for unit in wire_projector.feed(&[], true).unwrap() {
                delivered.merge(
                    tracker
                        .correlate_accepted_output(&unit.bytes, false)
                        .unwrap(),
                );
            }
            delivered.merge(tracker.correlate_accepted_output(&[], true).unwrap());
            let text = delivered
                .events
                .iter()
                .filter_map(|event| match &event.event {
                    ModelEvent::TextDelta { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();
            assert_eq!(text, "DONE");
            assert!(delivered.events.iter().any(|event| matches!(
                &event.event, ModelEvent::ReasoningDelta { text, .. } if text == "fixture thought"
            )));
            assert!(
                delivered.usage.is_none(),
                "main projector owns native usage"
            );
        }
    }

    #[test]
    fn native_capture_rejects_malformed_non_null_reasoning_content() {
        let profile = Arc::new(CandidateProtocolProfile::exact_portable_path(
            IngressProtocol::Responses,
            IngressProtocol::Responses,
            "physical",
            fixed_reasoning("fixed"),
        ));
        for content in [json!({}), json!(7), json!("malformed")] {
            let mut tracker = CanonicalResponseTracker::new(
                profile.clone(),
                projection(),
                None,
                false,
                "alias".into(),
            );
            let response = json!({"id":"response","model":"physical","status":"completed",
                "output":[{"type":"reasoning","id":"reasoning","summary":[],"content":content}]});
            tracker.observe(CapturedNativeEvent::Head(200)).unwrap();
            tracker
                .observe(CapturedNativeEvent::Body(
                    serde_json::to_vec(&response).unwrap(),
                ))
                .unwrap();
            assert!(tracker.observe(CapturedNativeEvent::EndStream).is_err());
        }
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
