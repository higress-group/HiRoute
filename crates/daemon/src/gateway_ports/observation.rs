use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use hiroute_domain::{
    ConversationContentEnvelopeV1, ExecutionFactEnvelopeV1, LifecycleFactEnvelopeV2,
    LifecycleTelemetryReceiverV2, ObservationFeedback, ObservationGapHeartbeatV1,
    RunObservationLink,
};
use hiroute_gateway::server::composition::{
    ConversationContentSink, ExecutionFactSink, LifecycleTelemetrySink,
};
use hiroute_gateway::server::core_runtime::observation::{
    CONVERSATION_CONTENT_SCHEMA, EXECUTION_FACT_SCHEMA, LIFECYCLE_FACT_SCHEMA,
    OBSERVATION_NACK_SCHEMA, ObservationAck, ObservationEnvelopeViolationV1, ObservationNack,
    ObservationNackDetailV1, ObservationNackV1, ObservationRecord, ObservationRecordSink,
    accounted_acknowledgement,
};
use hiroute_integrations::gateway::{
    GatewayProjectionError, project_content_payload, project_execution_payload, project_feedback,
    project_gap_payload, project_lifecycle_payload,
};
use hiroute_observation::writer::{IngestOutcome, ObservationCommitPort, ObservationStoreError};
use hiroute_observation::{LocalObservationStore, query_v2::ObservationV2Error};

/// Operational lifecycle receiver. It shares neither a queue nor storage with the other sinks.
pub struct GatewayLifecycleTelemetrySink<R> {
    receiver: Arc<R>,
}

impl<R> GatewayLifecycleTelemetrySink<R> {
    pub fn new(receiver: Arc<R>) -> Self {
        Self { receiver }
    }
}

impl<R: LifecycleTelemetryReceiverV2> LifecycleTelemetrySink for GatewayLifecycleTelemetrySink<R> {}

impl<R: LifecycleTelemetryReceiverV2> ObservationRecordSink for GatewayLifecycleTelemetrySink<R> {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        isolate(record, || {
            let feedback = if record.is_gap_heartbeat() {
                let heartbeat = project_gap_payload(record.payload(), "lifecycle")?;
                ensure_gap_stamp(record, &heartbeat, "gateway-lifecycle")?;
                self.receiver
                    .receive_lifecycle_gap(&heartbeat)
                    .map_err(|_| DeliveryError::ReceiverUnavailable(true))?
            } else {
                let envelope = project_lifecycle_payload(record.payload())?;
                ensure_lifecycle_stamp(record, &envelope)?;
                self.receiver
                    .receive_lifecycle(&envelope)
                    .map_err(|_| DeliveryError::ReceiverUnavailable(true))?
            };
            project(feedback)
        })
    }
}

/// Immutable execution-fact receiver backed by the Product observation commit port.
pub struct GatewayExecutionFactSink<W> {
    writer: Arc<W>,
}

impl<W> GatewayExecutionFactSink<W> {
    pub fn new(writer: Arc<W>) -> Self {
        Self { writer }
    }
}

impl<W: ObservationCommitPort> GatewayExecutionFactSink<W> {
    fn ingest_envelope(
        &self,
        envelope: &ExecutionFactEnvelopeV1,
    ) -> Result<IngestOutcome, DeliveryError> {
        self.writer.ingest_fact(envelope, &[]).map_err(store_error)
    }

    /// Exercises the exact projection and commit path without constructing Gateway's private
    /// bounded-queue record. Production delivery performs the stamp check between these same two
    /// operations; this test seam deliberately does not become a second ingestion contract.
    #[cfg(test)]
    pub(super) fn project_and_ingest_payload_for_test(
        &self,
        payload: &[u8],
    ) -> Result<IngestOutcome, &'static str> {
        let envelope =
            project_execution_payload(payload).map_err(|_| "gateway projection rejected")?;
        self.ingest_envelope(&envelope)
            .map_err(|_| "product observation store rejected")
    }
}

impl<W: ObservationCommitPort> ExecutionFactSink for GatewayExecutionFactSink<W> {}

impl<W: ObservationCommitPort> ObservationRecordSink for GatewayExecutionFactSink<W> {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        isolate(record, || {
            let outcome = if record.is_gap_heartbeat() {
                let heartbeat = project_gap_payload(record.payload(), "execution_fact")?;
                ensure_gap_stamp(record, &heartbeat, "gateway-execution")?;
                self.writer
                    .ingest_gap_heartbeat(&heartbeat)
                    .map_err(store_error)?
            } else {
                let envelope = project_execution_payload(record.payload())?;
                ensure_execution_stamp(record, &envelope)?;
                self.ingest_envelope(&envelope)?
            };
            project(ingest_feedback(outcome))
        })
    }
}

/// Content-v2 receiver. Each begin/append/finish/abort event is committed independently and its
/// rich ACK/NACK is returned without an adapter-owned checkpoint or acknowledgement cache.
pub struct GatewayConversationContentSink<W> {
    writer: Arc<W>,
    #[cfg(feature = "integration-test-hooks")]
    fail_once: std::sync::atomic::AtomicBool,
}

impl<W> GatewayConversationContentSink<W> {
    pub fn new(writer: Arc<W>) -> Self {
        Self {
            writer,
            #[cfg(feature = "integration-test-hooks")]
            fail_once: std::sync::atomic::AtomicBool::new(
                std::env::var("HIROUTE_TEST_OBSERVATION_CONTENT_SINK").as_deref()
                    == Ok("fail_once"),
            ),
        }
    }
}

impl<W: ObservationCommitPort> ConversationContentSink for GatewayConversationContentSink<W> {}

impl<W: ObservationCommitPort> ObservationRecordSink for GatewayConversationContentSink<W> {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        isolate(record, || {
            let outcome = if record.is_gap_heartbeat() {
                let heartbeat = project_gap_payload(record.payload(), "conversation_content")?;
                ensure_gap_stamp(record, &heartbeat, "gateway-content")?;
                self.writer.ingest_gap_heartbeat(&heartbeat)
            } else {
                let envelope = project_content_payload(record.payload())?;
                ensure_content_stamp(record, &envelope)?;
                // Exercise the actual Product persistence boundary, not the separate E2E
                // plaintext collector. Invalid records and recovery heartbeats do not
                // consume this process-local, one-shot injection.
                #[cfg(feature = "integration-test-hooks")]
                if self
                    .fail_once
                    .swap(false, std::sync::atomic::Ordering::AcqRel)
                {
                    return Err(DeliveryError::ReceiverUnavailable(true));
                }
                self.writer.ingest_content(&envelope, &[])
            }
            .map_err(store_error)?;
            project(ingest_feedback(outcome))
        })
    }
}

/// Persists only the relation projected by Gateway from a successfully
/// authenticated delegated-run authority. Delivery runs on Gateway's bounded
/// observation worker, never on the request thread.
pub struct GatewayRunRelationSink {
    store: Arc<LocalObservationStore>,
}

impl GatewayRunRelationSink {
    pub fn new(store: Arc<LocalObservationStore>) -> Self {
        Self { store }
    }
}

impl ObservationRecordSink for GatewayRunRelationSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        isolate(record, || {
            if record.is_gap_heartbeat() {
                let heartbeat = project_gap_payload(record.payload(), "run_relation")?;
                ensure_gap_stamp(record, &heartbeat, "gateway-run-relation")?;
                return Ok(accounted_acknowledgement(record));
            }
            let link: RunObservationLink = serde_json::from_slice(record.payload())
                .map_err(|_| DeliveryError::Projection(GatewayProjectionError::InvalidInput))?;
            ensure_run_relation_stamp(record, &link)?;
            self.store
                .link_observed_request(&link)
                .map_err(|error| match error {
                    ObservationV2Error::RelationshipConflict => {
                        DeliveryError::RelationshipConflict(link.request_id.to_string())
                    }
                    ObservationV2Error::Invalid => {
                        DeliveryError::Projection(GatewayProjectionError::InvalidOutput)
                    }
                    _ => DeliveryError::ReceiverUnavailable(true),
                })?;
            Ok(accounted_acknowledgement(record))
        })
    }
}

/// Independent additive observation channel. Old execution/receipt carriers and
/// the Worker relation protocol retain their exact serializers.
pub struct GatewayNativeContinuationSink {
    store: Arc<LocalObservationStore>,
}
impl GatewayNativeContinuationSink {
    pub fn new(store: Arc<LocalObservationStore>) -> Self {
        Self { store }
    }
}
impl ObservationRecordSink for GatewayNativeContinuationSink {
    fn deliver(&self, record: &ObservationRecord) -> Result<ObservationAck, ObservationNack> {
        isolate(record, || {
            if record.is_gap_heartbeat() {
                let heartbeat = project_gap_payload(record.payload(), "native_continuation")?;
                ensure_gap_stamp(record, &heartbeat, "gateway-native-continuation")?;
                return Ok(accounted_acknowledgement(record));
            }
            let e: hiroute_domain::NativeContinuationEvidenceV1 =
                serde_json::from_slice(record.payload())
                    .map_err(|_| DeliveryError::Projection(GatewayProjectionError::InvalidInput))?;
            let s = record.stamp();
            if s.identity.channel.as_ref() != "native_continuation"
                || s.identity.component.as_ref() != "gateway-native-continuation"
                || s.identity.producer_id.as_ref() != e.producer_id
                || s.identity.producer_epoch.as_ref() != e.producer_epoch
                || s.identity.stream_id.as_ref() != e.stream_id
                || s.sequence != e.sequence
            {
                return Err(DeliveryError::StampMismatch);
            }
            self.store
                .ingest_native_continuation(&e)
                .map_err(|error| match error {
                    ObservationV2Error::Invalid => {
                        DeliveryError::Projection(GatewayProjectionError::InvalidInput)
                    }
                    ObservationV2Error::RelationshipConflict => {
                        DeliveryError::RelationshipConflict(e.request_id.clone())
                    }
                    _ => DeliveryError::ReceiverUnavailable(true),
                })?;
            Ok(accounted_acknowledgement(record))
        })
    }
}

fn isolate(
    record: &ObservationRecord,
    deliver: impl FnOnce() -> Result<ObservationAck, DeliveryError>,
) -> Result<ObservationAck, ObservationNack> {
    match catch_unwind(AssertUnwindSafe(deliver)) {
        Ok(Ok(ack)) => Ok(ack),
        Ok(Err(error)) => Err(delivery_nack(record, error)),
        Err(_) => Err(delivery_nack(
            record,
            DeliveryError::ReceiverUnavailable(true),
        )),
    }
}

fn project(feedback: ObservationFeedback) -> Result<ObservationAck, DeliveryError> {
    project_feedback(feedback)
        .map_err(DeliveryError::Projection)?
        .map_err(DeliveryError::ReceiverNack)
}

fn ingest_feedback(outcome: IngestOutcome) -> ObservationFeedback {
    match outcome {
        IngestOutcome::Ack(ack) => ObservationFeedback::Ack(ack),
        IngestOutcome::Nack(nack) => ObservationFeedback::Nack(nack),
    }
}

fn store_error(error: ObservationStoreError) -> DeliveryError {
    DeliveryError::ReceiverUnavailable(!matches!(error, ObservationStoreError::Corrupt))
}

fn ensure_lifecycle_stamp(
    record: &ObservationRecord,
    envelope: &LifecycleFactEnvelopeV2,
) -> Result<(), DeliveryError> {
    ensure_stamp(
        record,
        "lifecycle",
        "gateway-lifecycle",
        &envelope.producer.revision,
        envelope.producer.stream.producer_id.as_str(),
        envelope.producer.stream.producer_epoch.as_str(),
        envelope.producer.stream.stream_id.as_str(),
        envelope.sequence,
        envelope
            .loss_watermark
            .as_ref()
            .map(|loss| (loss.first_sequence, loss.last_sequence)),
    )
}

fn ensure_execution_stamp(
    record: &ObservationRecord,
    envelope: &ExecutionFactEnvelopeV1,
) -> Result<(), DeliveryError> {
    ensure_stamp(
        record,
        "execution_fact",
        "gateway-execution",
        &envelope.producer.revision,
        envelope.producer.stream.producer_id.as_str(),
        envelope.producer.stream.producer_epoch.as_str(),
        envelope.producer.stream.stream_id.as_str(),
        envelope.sequence,
        envelope
            .loss_watermark
            .as_ref()
            .map(|loss| (loss.first_sequence, loss.last_sequence)),
    )
}

fn ensure_content_stamp(
    record: &ObservationRecord,
    envelope: &ConversationContentEnvelopeV1,
) -> Result<(), DeliveryError> {
    ensure_stamp(
        record,
        "conversation_content",
        "gateway-content",
        &envelope.producer.revision,
        envelope.producer.stream.producer_id.as_str(),
        envelope.producer.stream.producer_epoch.as_str(),
        envelope.producer.stream.stream_id.as_str(),
        envelope.sequence,
        envelope
            .loss_watermark
            .as_ref()
            .map(|loss| (loss.first_sequence, loss.last_sequence)),
    )
}

fn ensure_run_relation_stamp(
    record: &ObservationRecord,
    link: &RunObservationLink,
) -> Result<(), DeliveryError> {
    let stamp = record.stamp();
    if stamp.identity.channel.as_ref() != "run_relation"
        || stamp.identity.component.as_ref() != "gateway-run-relation"
        || stamp.identity.revision.as_ref() != "hiroute-gateway-observation/1"
        || stamp.identity.producer_epoch.as_ref() != link.producer_epoch
        || stamp.loss_watermark.is_some()
    {
        return Err(DeliveryError::StampMismatch);
    }
    Ok(())
}

fn ensure_gap_stamp(
    record: &ObservationRecord,
    heartbeat: &ObservationGapHeartbeatV1,
    component: &str,
) -> Result<(), DeliveryError> {
    ensure_stamp(
        record,
        &heartbeat.channel,
        component,
        &heartbeat.producer.revision,
        heartbeat.producer.stream.producer_id.as_str(),
        heartbeat.producer.stream.producer_epoch.as_str(),
        heartbeat.producer.stream.stream_id.as_str(),
        heartbeat.sequence,
        Some((
            heartbeat.loss_watermark.first_sequence,
            heartbeat.loss_watermark.last_sequence,
        )),
    )
}

#[allow(clippy::too_many_arguments)]
fn ensure_stamp(
    record: &ObservationRecord,
    channel: &str,
    component: &str,
    revision: &str,
    producer_id: &str,
    producer_epoch: &str,
    stream_id: &str,
    sequence: u64,
    loss: Option<(u64, u64)>,
) -> Result<(), DeliveryError> {
    let stamp = record.stamp();
    let stamped_loss = stamp
        .loss_watermark
        .map(|value| (value.first_sequence, value.last_sequence));
    if stamp.identity.channel.as_ref() != channel
        || stamp.identity.component.as_ref() != component
        || stamp.identity.revision.as_ref() != revision
        || stamp.identity.producer_id.as_ref() != producer_id
        || stamp.identity.producer_epoch.as_ref() != producer_epoch
        || stamp.identity.stream_id.as_ref() != stream_id
        || stamp.sequence != sequence
        || stamped_loss != loss
    {
        return Err(DeliveryError::StampMismatch);
    }
    Ok(())
}

fn delivery_nack(record: &ObservationRecord, error: DeliveryError) -> ObservationNack {
    let (retryable, detail) = match error {
        DeliveryError::Projection(GatewayProjectionError::UnsupportedSchema) => (
            false,
            ObservationNackDetailV1::UnsupportedSchema {
                rejected_schema_version: rejected_schema(record.payload()),
                supported_schema_versions: vec![supported_schema(record)],
            },
        ),
        DeliveryError::Projection(_) | DeliveryError::StampMismatch => (
            false,
            ObservationNackDetailV1::InvalidEnvelope {
                violation: ObservationEnvelopeViolationV1::Malformed,
                field: None,
            },
        ),
        DeliveryError::ReceiverUnavailable(retryable) => (
            retryable,
            ObservationNackDetailV1::ReceiverUnavailable {
                retry_after_millis: None,
            },
        ),
        DeliveryError::RelationshipConflict(request_id) => (
            false,
            ObservationNackDetailV1::ImmutableProjectionConflict {
                projection_key: request_id,
                existing_digest: "existing_verified_relation".into(),
                rejected_digest: "conflicting_verified_relation".into(),
            },
        ),
        DeliveryError::ReceiverNack(nack) => return nack,
    };
    Box::new(ObservationNackV1 {
        schema_version: OBSERVATION_NACK_SCHEMA.into(),
        identity: record.feedback_identity(),
        rejected_sequence: record.stamp().sequence,
        expected_sequence: record.stamp().sequence,
        retryable,
        detail,
    })
}

fn rejected_schema(payload: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|value| value.get("schema_version")?.as_str().map(str::to_owned))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "malformed".into())
}

fn supported_schema(record: &ObservationRecord) -> String {
    match record.stamp().identity.channel.as_ref() {
        "lifecycle" => LIFECYCLE_FACT_SCHEMA,
        "execution_fact" => EXECUTION_FACT_SCHEMA,
        "conversation_content" => CONVERSATION_CONTENT_SCHEMA,
        "run_relation" => "hiroute.observation.run-link/v1",
        "native_continuation" => hiroute_domain::NATIVE_CONTINUATION_SCHEMA_V1,
        _ => "unsupported-channel",
    }
    .into()
}

enum DeliveryError {
    Projection(GatewayProjectionError),
    ReceiverUnavailable(bool),
    ReceiverNack(ObservationNack),
    RelationshipConflict(String),
    StampMismatch,
}

impl From<GatewayProjectionError> for DeliveryError {
    fn from(value: GatewayProjectionError) -> Self {
        Self::Projection(value)
    }
}

impl From<ObservationNack> for DeliveryError {
    fn from(value: ObservationNack) -> Self {
        Self::ReceiverNack(value)
    }
}
