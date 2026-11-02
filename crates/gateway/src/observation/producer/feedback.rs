use super::super::schema::{
    CONVERSATION_CONTENT_CHANNEL, ConversationContentEnvelopeV1, OBSERVATION_ACK_SCHEMA,
    OBSERVATION_NACK_SCHEMA, ObservationAckV2, ObservationBlobAcknowledgementV1,
    ObservationContentAcknowledgementV1, ObservationDigestSubjectV1,
    ObservationEnvelopeViolationV1, ObservationFeedbackIdentityV1,
    ObservationFeedbackValidationError, ObservationNackDetailV1, ObservationNackV1,
    ObservationSequenceRangeV1,
};
use super::bounded::{ObservationRecord, ObservationSequenceStamp};

impl ObservationFeedbackIdentityV1 {
    pub fn validate(&self) -> Result<(), ObservationFeedbackValidationError> {
        nonempty(&self.channel, "identity.channel")?;
        nonempty(&self.producer_id, "identity.producer_id")?;
        nonempty(&self.producer_epoch, "identity.producer_epoch")?;
        nonempty(&self.stream_id, "identity.stream_id")
    }
}

impl ObservationContentAcknowledgementV1 {
    pub fn validate(&self) -> Result<(), ObservationFeedbackValidationError> {
        nonempty(&self.request_id, "content_acknowledgement.request_id")?;
        nonempty(&self.fork_id, "content_acknowledgement.fork_id")?;
        optional_nonempty(
            self.transcript_root.as_deref(),
            "content_acknowledgement.transcript_root",
        )?;
        optional_nonempty(
            self.delta_parent_transcript_root.as_deref(),
            "content_acknowledgement.delta_parent_transcript_root",
        )?;
        validate_blobs(&self.acknowledged_blobs)
    }
}

impl ObservationAckV2 {
    pub fn validate(&self) -> Result<(), ObservationFeedbackValidationError> {
        if self.schema_version != OBSERVATION_ACK_SCHEMA {
            return Err(ObservationFeedbackValidationError::UnsupportedSchema);
        }
        self.identity.validate()?;
        if self.highest_contiguous_sequence > self.highest_accounted_sequence {
            return Err(ObservationFeedbackValidationError::InvalidFrontier);
        }
        if let Some(content) = &self.content_acknowledgement {
            if self.identity.channel != CONVERSATION_CONTENT_CHANNEL {
                return Err(ObservationFeedbackValidationError::InvalidContentChannel);
            }
            content.validate()?;
        }
        Ok(())
    }
}

impl ObservationNackDetailV1 {
    fn is_content_detail(&self) -> bool {
        matches!(
            self,
            Self::UnknownTranscriptRoot { .. }
                | Self::MissingBlob { .. }
                | Self::ChunkOrdinalConflict { .. }
                | Self::ContentStateConflict { .. }
        ) || matches!(
            self,
            Self::DigestMismatch {
                subject: ObservationDigestSubjectV1::Transcript
                    | ObservationDigestSubjectV1::ContentBlob,
                ..
            }
        )
    }

    fn validate(&self) -> Result<(), ObservationFeedbackValidationError> {
        match self {
            Self::ReceiverUnavailable { .. } => Ok(()),
            Self::UnsupportedSchema {
                rejected_schema_version,
                supported_schema_versions,
            } => {
                nonempty(rejected_schema_version, "detail.rejected_schema_version")?;
                if supported_schema_versions.is_empty() {
                    return Err(ObservationFeedbackValidationError::EmptyRepairDetail);
                }
                for schema in supported_schema_versions {
                    nonempty(schema, "detail.supported_schema_versions")?;
                }
                Ok(())
            }
            Self::InvalidEnvelope { violation, field } => {
                if !matches!(violation, ObservationEnvelopeViolationV1::Malformed)
                    && field.is_none()
                {
                    return Err(ObservationFeedbackValidationError::EmptyRepairDetail);
                }
                optional_nonempty(field.as_deref(), "detail.field")
            }
            Self::MissingSequenceRanges { ranges } => validate_ranges(ranges),
            Self::SequenceEventConflict {
                sequence,
                expected_event_id,
                rejected_event_id,
            } => {
                nonzero(*sequence)?;
                nonempty(expected_event_id, "detail.expected_event_id")?;
                nonempty(rejected_event_id, "detail.rejected_event_id")
            }
            Self::MissingPrerequisite {
                prerequisite_sequence,
                prerequisite_event_id,
            } => {
                nonzero(*prerequisite_sequence)?;
                optional_nonempty(
                    prerequisite_event_id.as_deref(),
                    "detail.prerequisite_event_id",
                )
            }
            Self::UnknownTranscriptRoot {
                request_id,
                fork_id,
                transcript_root,
                ..
            } => {
                validate_content_coordinates(request_id, fork_id)?;
                nonempty(transcript_root, "detail.transcript_root")
            }
            Self::MissingBlob {
                request_id,
                fork_id,
                blobs,
                ..
            } => {
                validate_content_coordinates(request_id, fork_id)?;
                if blobs.is_empty() {
                    return Err(ObservationFeedbackValidationError::EmptyRepairDetail);
                }
                validate_blobs(blobs)
            }
            Self::ChunkOrdinalConflict {
                request_id,
                fork_id,
                ..
            }
            | Self::ContentStateConflict {
                request_id,
                fork_id,
                ..
            } => validate_content_coordinates(request_id, fork_id),
            Self::DigestMismatch {
                subject_id,
                expected_digest,
                rejected_digest,
                ..
            } => {
                nonempty(subject_id, "detail.subject_id")?;
                nonempty(expected_digest, "detail.expected_digest")?;
                nonempty(rejected_digest, "detail.rejected_digest")
            }
            Self::ImmutableProjectionConflict {
                projection_key,
                existing_digest,
                rejected_digest,
            } => {
                nonempty(projection_key, "detail.projection_key")?;
                nonempty(existing_digest, "detail.existing_digest")?;
                nonempty(rejected_digest, "detail.rejected_digest")
            }
        }
    }
}

impl ObservationNackV1 {
    pub fn validate(&self) -> Result<(), ObservationFeedbackValidationError> {
        if self.schema_version != OBSERVATION_NACK_SCHEMA {
            return Err(ObservationFeedbackValidationError::UnsupportedSchema);
        }
        self.identity.validate()?;
        nonzero(self.rejected_sequence)?;
        nonzero(self.expected_sequence)?;
        if self.detail.is_content_detail() && self.identity.channel != CONVERSATION_CONTENT_CHANNEL
        {
            return Err(ObservationFeedbackValidationError::InvalidContentChannel);
        }
        self.detail.validate()
    }
}

impl ObservationRecord {
    pub fn feedback_identity(&self) -> ObservationFeedbackIdentityV1 {
        feedback_identity(self.stamp())
    }
}

pub fn accounted_acknowledgement(record: &ObservationRecord) -> ObservationAckV2 {
    let highest_accounted_sequence = record
        .stamp()
        .loss_watermark
        .map_or(record.stamp().sequence, |loss| loss.last_sequence);
    ObservationAckV2 {
        schema_version: OBSERVATION_ACK_SCHEMA.into(),
        identity: record.feedback_identity(),
        // The discard and E2E file adapters do not own a durable checkpoint.
        // They can confirm terminal accounting for this delivery, but must not
        // invent a contiguous frontier across an earlier producer Gap.
        highest_contiguous_sequence: 0,
        highest_accounted_sequence,
        content_acknowledgement: None,
    }
}

pub(super) fn receiver_unavailable_nack(
    record: &ObservationRecord,
    retryable: bool,
) -> Box<ObservationNackV1> {
    Box::new(ObservationNackV1 {
        schema_version: OBSERVATION_NACK_SCHEMA.into(),
        identity: record.feedback_identity(),
        rejected_sequence: record.stamp().sequence,
        expected_sequence: record.stamp().sequence,
        retryable,
        detail: ObservationNackDetailV1::ReceiverUnavailable {
            retry_after_millis: None,
        },
    })
}

pub(super) fn acknowledgement_covers_record(
    record: &ObservationRecord,
    acknowledgement: &ObservationAckV2,
    contiguous_ceiling: Option<u64>,
) -> bool {
    if acknowledgement.validate().is_err()
        || acknowledgement.identity != record.feedback_identity()
        || contiguous_ceiling
            .is_some_and(|ceiling| acknowledgement.highest_contiguous_sequence > ceiling)
    {
        return false;
    }
    if let Some(loss) = record.stamp().loss_watermark {
        return acknowledgement.content_acknowledgement.is_none()
            && acknowledgement.highest_accounted_sequence >= loss.last_sequence
            && acknowledgement.highest_contiguous_sequence < loss.first_sequence;
    }
    acknowledgement.highest_accounted_sequence >= record.stamp().sequence
        && acknowledgement
            .content_acknowledgement
            .as_ref()
            .is_none_or(|content| content_acknowledges_record(record, content))
}

pub(super) fn nack_matches_record(record: &ObservationRecord, nack: &ObservationNackV1) -> bool {
    nack.validate().is_ok()
        && nack.identity == record.feedback_identity()
        && nack.rejected_sequence == record.stamp().sequence
        && nack_detail_matches_record(record, &nack.detail)
}

fn feedback_identity(stamp: &ObservationSequenceStamp) -> ObservationFeedbackIdentityV1 {
    ObservationFeedbackIdentityV1 {
        channel: stamp.identity.channel.to_string(),
        producer_id: stamp.identity.producer_id.to_string(),
        producer_epoch: stamp.identity.producer_epoch.to_string(),
        stream_id: stamp.identity.stream_id.to_string(),
    }
}

fn content_acknowledges_record(
    record: &ObservationRecord,
    acknowledgement: &ObservationContentAcknowledgementV1,
) -> bool {
    let Ok(envelope) = serde_json::from_slice::<ConversationContentEnvelopeV1>(record.payload())
    else {
        return false;
    };
    if acknowledgement.request_id != envelope.correlation.request_id
        || acknowledgement.direction.as_str() != envelope.direction
        || acknowledgement.fork_id != envelope.fork_id
        || envelope
            .chunk_ordinal
            .is_some_and(|ordinal| acknowledgement.next_chunk_ordinal <= ordinal)
        || envelope
            .result_transcript_root
            .as_ref()
            .is_some_and(|root| {
                acknowledgement
                    .transcript_root
                    .as_ref()
                    .is_some_and(|acknowledged| acknowledged != root)
            })
        || acknowledgement
            .delta_parent_transcript_root
            .as_ref()
            .is_some_and(|root| envelope.parent_transcript_root.as_ref() != Some(root))
    {
        return false;
    }
    match (envelope.content_id, envelope.content_blob_digest) {
        // `acknowledged_blobs` reports blobs that became durable while applying this
        // append. That can be the immediately preceding content part when a stream
        // switches to a new part; the current chunk is independently acknowledged by
        // `next_chunk_ordinal`. If the receiver does name the current content ID, its
        // digest must still match exactly.
        (Some(content_id), Some(digest)) => acknowledgement
            .acknowledged_blobs
            .iter()
            .find(|blob| blob.content_id == content_id)
            .is_none_or(|blob| blob.digest == digest),
        (None, None) => true,
        _ => false,
    }
}

fn nack_detail_matches_record(
    record: &ObservationRecord,
    detail: &ObservationNackDetailV1,
) -> bool {
    if let ObservationNackDetailV1::SequenceEventConflict { sequence, .. } = detail {
        return *sequence == record.stamp().sequence;
    }
    let Some((request_id, direction, fork_id)) = content_coordinates(detail) else {
        return true;
    };
    let Ok(envelope) = serde_json::from_slice::<ConversationContentEnvelopeV1>(record.payload())
    else {
        return false;
    };
    request_id == envelope.correlation.request_id
        && direction.as_str() == envelope.direction
        && fork_id == envelope.fork_id
}

fn content_coordinates(
    detail: &ObservationNackDetailV1,
) -> Option<(
    &str,
    super::super::schema::ObservationContentDirectionV1,
    &str,
)> {
    match detail {
        ObservationNackDetailV1::UnknownTranscriptRoot {
            request_id,
            direction,
            fork_id,
            ..
        }
        | ObservationNackDetailV1::MissingBlob {
            request_id,
            direction,
            fork_id,
            ..
        }
        | ObservationNackDetailV1::ChunkOrdinalConflict {
            request_id,
            direction,
            fork_id,
            ..
        }
        | ObservationNackDetailV1::ContentStateConflict {
            request_id,
            direction,
            fork_id,
            ..
        } => Some((request_id, *direction, fork_id)),
        _ => None,
    }
}

fn nonempty(value: &str, field: &'static str) -> Result<(), ObservationFeedbackValidationError> {
    if value.trim().is_empty() {
        Err(ObservationFeedbackValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn optional_nonempty(
    value: Option<&str>,
    field: &'static str,
) -> Result<(), ObservationFeedbackValidationError> {
    value.map_or(Ok(()), |value| nonempty(value, field))
}

fn nonzero(value: u64) -> Result<(), ObservationFeedbackValidationError> {
    if value == 0 {
        Err(ObservationFeedbackValidationError::InvalidSequence)
    } else {
        Ok(())
    }
}

fn validate_content_coordinates(
    request_id: &str,
    fork_id: &str,
) -> Result<(), ObservationFeedbackValidationError> {
    nonempty(request_id, "detail.request_id")?;
    nonempty(fork_id, "detail.fork_id")
}

fn validate_ranges(
    ranges: &[ObservationSequenceRangeV1],
) -> Result<(), ObservationFeedbackValidationError> {
    if ranges.is_empty() {
        return Err(ObservationFeedbackValidationError::EmptyRepairDetail);
    }
    if ranges
        .iter()
        .any(|range| range.first_sequence == 0 || range.first_sequence > range.last_sequence)
        || ranges.windows(2).any(|pair| {
            pair[0]
                .last_sequence
                .checked_add(1)
                .is_none_or(|next| next >= pair[1].first_sequence)
        })
    {
        return Err(ObservationFeedbackValidationError::InvalidSequenceRange);
    }
    Ok(())
}

fn validate_blobs(
    blobs: &[ObservationBlobAcknowledgementV1],
) -> Result<(), ObservationFeedbackValidationError> {
    let mut identities = std::collections::BTreeSet::new();
    for blob in blobs {
        nonempty(&blob.content_id, "blob.content_id")?;
        nonempty(&blob.digest, "blob.digest")?;
        if !identities.insert(blob.content_id.as_str()) {
            return Err(ObservationFeedbackValidationError::DuplicateBlobAcknowledgement);
        }
    }
    Ok(())
}
