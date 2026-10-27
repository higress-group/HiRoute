use super::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BodyDirection {
    LogicalRequest,
    AttemptRequest,
    AttemptResponsePrecommit,
    AcceptedResponse,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BodyPlan {
    PassThrough {
        max_chunk_bytes: usize,
    },
    StreamingReplay {
        max_chunk_bytes: usize,
        max_replay_bytes: usize,
    },
    BufferedTransform {
        max_body_bytes: usize,
    },
    SseFramedStreaming {
        max_event_bytes: usize,
        max_pending_bytes: usize,
        max_output_event_bytes: usize,
        expansion_ratio_numerator: usize,
        expansion_ratio_denominator: usize,
        expansion_slack_bytes: usize,
    },
}

impl BodyPlan {
    pub fn validate(&self) -> Result<(), BodyError> {
        let valid = match self {
            Self::PassThrough { max_chunk_bytes } => *max_chunk_bytes > 0,
            Self::StreamingReplay {
                max_chunk_bytes,
                max_replay_bytes,
            } => *max_chunk_bytes > 0 && *max_replay_bytes > 0,
            Self::BufferedTransform { max_body_bytes } => *max_body_bytes > 0,
            Self::SseFramedStreaming {
                max_event_bytes,
                max_pending_bytes,
                max_output_event_bytes,
                expansion_ratio_numerator,
                expansion_ratio_denominator,
                ..
            } => {
                *max_event_bytes > 0
                    && *max_pending_bytes > 0
                    && *max_pending_bytes >= *max_event_bytes
                    && *max_output_event_bytes > 0
                    && *expansion_ratio_numerator > 0
                    && *expansion_ratio_denominator > 0
            }
        };
        if valid {
            Ok(())
        } else {
            Err(BodyError::InvalidPlanLimit)
        }
    }

    pub fn max_chunk_bytes(&self) -> usize {
        match self {
            Self::PassThrough { max_chunk_bytes }
            | Self::StreamingReplay {
                max_chunk_bytes, ..
            } => *max_chunk_bytes,
            Self::BufferedTransform { max_body_bytes } => *max_body_bytes,
            Self::SseFramedStreaming {
                max_event_bytes, ..
            } => *max_event_bytes,
        }
    }

    pub fn max_retained_bytes(&self) -> Option<usize> {
        match self {
            Self::PassThrough { .. } => None,
            Self::StreamingReplay {
                max_replay_bytes, ..
            } => Some(*max_replay_bytes),
            Self::BufferedTransform { max_body_bytes } => Some(*max_body_bytes),
            Self::SseFramedStreaming {
                max_pending_bytes, ..
            } => Some(*max_pending_bytes),
        }
    }

    pub fn validate_chunk(&self, bytes: usize) -> Result<(), BodyError> {
        if bytes <= self.max_chunk_bytes() {
            Ok(())
        } else {
            Err(BodyError::BodyLimitExceeded)
        }
    }
}

/// Per-direction linear execution state for a compiled body plan. The owner
/// is intentionally not Clone: cumulative byte limits and EOS are properties
/// of one concrete body flow, not of individual transport chunks.
#[derive(Debug)]
pub struct BodyPlanExecutor {
    direction: BodyDirection,
    plan: BodyPlan,
    hard_total_limit: usize,
    observed_bytes: usize,
    declared_length: Option<usize>,
    eos: bool,
}

impl BodyPlanExecutor {
    pub fn new(
        direction: BodyDirection,
        plan: BodyPlan,
        hard_total_limit: usize,
    ) -> Result<Self, BodyError> {
        plan.validate()?;
        if hard_total_limit == 0 {
            return Err(BodyError::InvalidPlanLimit);
        }
        Ok(Self {
            direction,
            plan,
            hard_total_limit,
            observed_bytes: 0,
            declared_length: None,
            eos: false,
        })
    }

    pub fn direction(&self) -> BodyDirection {
        self.direction
    }

    pub fn plan(&self) -> &BodyPlan {
        &self.plan
    }

    pub fn observed_bytes(&self) -> usize {
        self.observed_bytes
    }

    pub fn preflight_content_length(&mut self, content_length: usize) -> Result<(), BodyError> {
        if content_length > self.total_limit() {
            return Err(BodyError::BodyLimitExceeded);
        }
        self.declared_length = Some(content_length);
        Ok(())
    }

    /// Admits one transport/body frame before it is copied or retained.
    pub fn admit_chunk(&mut self, bytes: usize) -> Result<(), BodyError> {
        self.admit_prefix(bytes, bytes)
    }

    /// Accounts for an already-spooled prefix without retaining its frame sizes.
    /// The total and largest original frame preserve both cumulative and frame limits.
    pub fn admit_prefix(
        &mut self,
        bytes: usize,
        largest_chunk_bytes: usize,
    ) -> Result<(), BodyError> {
        if self.eos {
            return Err(BodyError::BodyAfterEos);
        }
        if largest_chunk_bytes > bytes || (bytes > 0 && largest_chunk_bytes == 0) {
            return Err(BodyError::InvalidPlanLimit);
        }
        match &self.plan {
            BodyPlan::PassThrough { max_chunk_bytes }
            | BodyPlan::StreamingReplay {
                max_chunk_bytes, ..
            } => {
                if largest_chunk_bytes > *max_chunk_bytes {
                    return Err(BodyError::BodyLimitExceeded);
                }
            }
            // BufferedTransform has a cumulative bound. SSE framing applies
            // its event bound after incremental framing, not to arbitrary
            // transport chunk boundaries.
            BodyPlan::BufferedTransform { .. } | BodyPlan::SseFramedStreaming { .. } => {}
        }
        let next = self
            .observed_bytes
            .checked_add(bytes)
            .ok_or(BodyError::BodyLimitExceeded)?;
        if next > self.total_limit() {
            return Err(BodyError::BodyLimitExceeded);
        }
        self.observed_bytes = next;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<usize, BodyError> {
        if self.eos {
            return Err(BodyError::DuplicateBodyEos);
        }
        self.eos = true;
        if self
            .declared_length
            .is_some_and(|declared| declared != self.observed_bytes)
        {
            return Err(BodyError::ContentLengthMismatch);
        }
        Ok(self.observed_bytes)
    }

    pub fn is_buffered_transform(&self) -> bool {
        matches!(self.plan, BodyPlan::BufferedTransform { .. })
    }

    fn total_limit(&self) -> usize {
        let plan_limit = match &self.plan {
            BodyPlan::PassThrough { .. } => self.hard_total_limit,
            BodyPlan::StreamingReplay {
                max_replay_bytes, ..
            } => *max_replay_bytes,
            BodyPlan::BufferedTransform { max_body_bytes } => *max_body_bytes,
            // Pending/event/output bounds are enforced by SseFramer and its
            // charged emitter. The request-level hard limit remains a final
            // defense for a stream direction that is explicitly finite.
            BodyPlan::SseFramedStreaming { .. } => self.hard_total_limit,
        };
        plan_limit.min(self.hard_total_limit)
    }
}
