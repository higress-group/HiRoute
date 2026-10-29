use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use serde_json::Value;
use tokio::time::{Instant, sleep};

use super::types::*;
use crate::p0::canonical::sha256_hex;

const MAX_CHANNEL_BYTES: u64 = 16 * 1024 * 1024;
pub(super) const SEALED_EXECUTION_ORDER: [&str; 14] = [
    "route_decision",
    "candidate_decision",
    "runtime_state",
    "credential_lease",
    "runtime_state",
    "runtime_state",
    "runtime_state",
    "runtime_state",
    "runtime_state",
    "attempt_started",
    "semantic_commit",
    "attempt_finished",
    "request_finished",
    "usage_and_cache",
];
pub(super) const CURRENT_EXECUTION_FACT_COUNT: usize = SEALED_EXECUTION_ORDER.len() + 1;

pub(crate) async fn wait_for_independent_terminals(
    directory: &Path,
    expected: &ExpectedObservation,
    publication_digest: &str,
    publication_revision: u64,
    freshness_challenge: &str,
    bound: Duration,
    current: bool,
) -> Result<(), ProductionError> {
    let deadline = Instant::now() + bound;
    loop {
        let last = match collect(
            directory,
            expected,
            publication_digest,
            publication_revision,
            freshness_challenge,
            current,
        ) {
            Ok(_) => return Ok(()),
            Err(error) => error.to_string(),
        };
        if Instant::now() >= deadline {
            return Err(ProductionError::Collector(format!(
                "independent observation terminals timed out: {last}"
            )));
        }
        sleep(Duration::from_millis(5)).await;
    }
}

pub(crate) fn collect(
    directory: &Path,
    expected: &ExpectedObservation,
    publication_digest: &str,
    publication_revision: u64,
    freshness_challenge: &str,
    current: bool,
) -> Result<CollectorEvidence, ProductionError> {
    let lifecycle = read_channel(directory, "lifecycle", "lifecycle.jsonl")?;
    let execution = read_channel(directory, "execution_fact", "execution-fact.jsonl")?;
    let content = read_channel(
        directory,
        "conversation_content",
        "conversation-content.jsonl",
    )?;
    let otel = read_channel(directory, "otel", "otel.jsonl")?;
    let request_ids = [&lifecycle, &execution, &content, &otel]
        .into_iter()
        .flat_map(|channel| channel.records.iter())
        .filter_map(|record| {
            record
                .pointer("/correlation/request_id")
                .and_then(Value::as_str)
        })
        .collect::<BTreeSet<_>>();
    if request_ids.len() != 1 {
        return Err(ProductionError::Collector(format!(
            "expected exactly one correlated request, found {}",
            request_ids.len()
        )));
    }
    let request_id = (*request_ids.first().expect("one request ID")).to_owned();
    let mut evidence = CollectorEvidence {
        schema_version: COLLECTOR_SCHEMA.into(),
        request_id,
        lifecycle,
        execution_fact: execution,
        conversation_content: content,
        otel,
        independent_streams: true,
        no_gap_or_loss: true,
        content_terminal_independent: true,
    };
    verify_collector_evidence_for(
        &evidence,
        expected,
        publication_digest,
        publication_revision,
        freshness_challenge,
        current,
    )?;
    evidence.lifecycle.terminal = expected.lifecycle_terminal.clone();
    evidence.execution_fact.terminal = expected.execution_terminal.clone();
    evidence.conversation_content.terminal = expected
        .content_terminals
        .iter()
        .map(|terminal| format!("{}:{}", terminal.direction, terminal.phase))
        .collect::<Vec<_>>()
        .join(",");
    evidence.otel.terminal = expected.required_otel_signal.clone();
    Ok(evidence)
}

pub fn verify_collector_evidence(
    evidence: &CollectorEvidence,
    expected: &ExpectedObservation,
    publication_digest: &str,
    publication_revision: u64,
    freshness_challenge: &str,
) -> Result<(), ProductionError> {
    verify_collector_evidence_for(
        evidence,
        expected,
        publication_digest,
        publication_revision,
        freshness_challenge,
        false,
    )
}

pub(super) fn verify_collector_evidence_for(
    evidence: &CollectorEvidence,
    expected: &ExpectedObservation,
    publication_digest: &str,
    publication_revision: u64,
    freshness_challenge: &str,
    current: bool,
) -> Result<(), ProductionError> {
    if evidence.schema_version != COLLECTOR_SCHEMA {
        return Err(collector("collector schema version mismatch"));
    }
    let (execution_schema, execution_schema_digest) = if current {
        (CURRENT_EXECUTION_SCHEMA, CURRENT_EXECUTION_SCHEMA_DIGEST)
    } else {
        (LEGACY_EXECUTION_SCHEMA, LEGACY_EXECUTION_SCHEMA_DIGEST)
    };
    let channels = [
        (
            &evidence.lifecycle,
            "lifecycle",
            LIFECYCLE_SCHEMA,
            LIFECYCLE_SCHEMA_DIGEST,
        ),
        (
            &evidence.execution_fact,
            "execution_fact",
            execution_schema,
            execution_schema_digest,
        ),
        (
            &evidence.conversation_content,
            "conversation_content",
            CONTENT_SCHEMA,
            CONTENT_SCHEMA_DIGEST,
        ),
        (&evidence.otel, "otel", OTEL_SCHEMA, OTEL_SCHEMA_DIGEST),
    ];
    let correlation = correlation_key(
        evidence
            .lifecycle
            .records
            .first()
            .ok_or_else(|| collector("lifecycle observation channel is empty"))?,
    )?;
    let mut identities = BTreeSet::new();
    for (channel, expected_name, schema, schema_digest) in channels {
        super::record::validate_channel(
            channel,
            expected_name,
            schema,
            schema_digest,
            &evidence.request_id,
        )?;
        if channel
            .records
            .iter()
            .map(correlation_key)
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|actual| actual != &correlation)
        {
            return Err(collector(
                "observation channels do not share one exact correlation identity",
            ));
        }
        if !identities.insert((
            channel.producer_id.as_str(),
            channel.producer_epoch.as_str(),
            channel.stream_id.as_str(),
        )) {
            return Err(collector(
                "observation channels do not own independent streams",
            ));
        }
    }
    if identities.len() != 4
        || !evidence.independent_streams
        || !evidence.no_gap_or_loss
        || !evidence.content_terminal_independent
    {
        return Err(collector("independent stream proof is false"));
    }
    validate_fact_terminal(
        &evidence.lifecycle.records,
        &expected.lifecycle_terminal,
        "lifecycle",
        true,
    )?;
    validate_lifecycle(&evidence.lifecycle.records, current)?;
    validate_fact_terminal(
        &evidence.execution_fact.records,
        &expected.execution_terminal,
        "execution_fact",
        false,
    )?;
    validate_execution(
        &evidence.execution_fact.records,
        expected,
        publication_digest,
        publication_revision,
        current,
    )?;
    validate_content(
        &evidence.conversation_content.records,
        expected,
        freshness_challenge,
        current,
    )?;
    validate_otel(&evidence.otel.records, &expected.required_otel_signal)?;
    Ok(())
}

fn validate_lifecycle(records: &[Value], current: bool) -> Result<(), ProductionError> {
    let kinds = records
        .iter()
        .filter_map(|record| record.pointer("/fact/kind").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let expected: &[&str] = if current {
        &[
            "request_accepted",
            "canonical_request_accepted",
            "attempt_started",
            "response_frame_accepted",
            "attempt_finished",
            "request_finished",
        ]
    } else {
        &[
            "request_accepted",
            "canonical_request_accepted",
            "attempt_started",
            "response_frame_accepted",
            "response_frame_accepted",
            "attempt_finished",
            "request_finished",
        ]
    };
    if kinds != expected
        || records
            .get(records.len().saturating_sub(2))
            .and_then(|record| record.pointer("/fact/outcome"))
            .and_then(Value::as_str)
            != Some("accepted")
        || records
            .last()
            .and_then(|record| record.pointer("/fact/outcome"))
            .and_then(Value::as_str)
            != Some("accepted")
    {
        return Err(collector(
            "lifecycle fact sequence/outcome is not the exact green smoke",
        ));
    }
    Ok(())
}

fn correlation_key(record: &Value) -> Result<[String; 4], ProductionError> {
    let field = |name: &str| {
        record
            .pointer(&format!("/correlation/{name}"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| collector("observation correlation identity is incomplete"))
    };
    Ok([
        field("workspace_id")?,
        field("conversation_id")?,
        field("turn_id")?,
        field("request_id")?,
    ])
}

fn read_channel(
    directory: &Path,
    name: &str,
    file_name: &str,
) -> Result<ChannelEvidence, ProductionError> {
    let path = directory.join(file_name);
    let metadata = std::fs::metadata(&path)
        .map_err(|error| ProductionError::Collector(format!("missing {name} channel: {error}")))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_CHANNEL_BYTES {
        return Err(ProductionError::Collector(format!(
            "{name} channel size {} is invalid",
            metadata.len()
        )));
    }
    let bytes = std::fs::read(&path)?;
    if bytes.last() != Some(&b'\n') {
        return Err(collector("observation JSONL is missing its final newline"));
    }
    let records = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_slice(line).map_err(|error| {
                ProductionError::Collector(format!("corrupt {name} JSONL: {error}"))
            })
        })
        .collect::<Result<Vec<Value>, _>>()?;
    let first = records
        .first()
        .ok_or_else(|| collector("observation channel is empty"))?;
    let producer = first
        .get("producer")
        .and_then(Value::as_object)
        .ok_or_else(|| collector("observation producer descriptor is missing"))?;
    let text = |field: &str| {
        producer
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| collector("observation producer identity is incomplete"))
    };
    let first_sequence = first
        .get("sequence")
        .and_then(Value::as_u64)
        .ok_or_else(|| collector("observation sequence is missing"))?;
    let last_sequence = records
        .last()
        .and_then(|record| record.get("sequence"))
        .and_then(Value::as_u64)
        .ok_or_else(|| collector("observation terminal sequence is missing"))?;
    Ok(ChannelEvidence {
        name: name.into(),
        file_sha256: sha256_hex(&bytes),
        records_digest: ChannelEvidence::digest_records(&records),
        producer_id: text("producer_id")?,
        producer_epoch: text("producer_epoch")?,
        stream_id: text("stream_id")?,
        first_sequence,
        last_sequence,
        record_count: records.len(),
        terminal: String::new(),
        records,
    })
}

fn validate_fact_terminal(
    records: &[Value],
    terminal: &str,
    channel: &str,
    must_be_last: bool,
) -> Result<(), ProductionError> {
    let positions = records
        .iter()
        .enumerate()
        .filter(|(_, record)| {
            record.pointer("/fact/kind").and_then(Value::as_str) == Some(terminal)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if positions.len() != 1
        || (must_be_last && positions[0] != records.len() - 1)
        || (!must_be_last
            && records[positions[0] + 1..].iter().any(|record| {
                record.pointer("/fact/kind").and_then(Value::as_str) != Some("usage_and_cache")
            }))
    {
        return Err(collector(match channel {
            "lifecycle" => "lifecycle terminal is missing, duplicated, or out of order",
            _ => "execution terminal is missing, duplicated, or out of order",
        }));
    }
    Ok(())
}

fn validate_execution(
    records: &[Value],
    expected: &ExpectedObservation,
    publication_digest: &str,
    publication_revision: u64,
    current: bool,
) -> Result<(), ProductionError> {
    let kinds = records
        .iter()
        .filter_map(|record| record.pointer("/fact/kind").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if !valid_execution_kind_order(&kinds, &SEALED_EXECUTION_ORDER, current) {
        return Err(collector(
            "execution fact sequence is not the exact green smoke",
        ));
    }
    for required in &expected.required_execution_facts {
        if !kinds.iter().any(|kind| *kind == required) {
            return Err(ProductionError::Collector(format!(
                "required execution fact is missing: {required}"
            )));
        }
    }
    for record in records {
        if record
            .get("gateway_publication_digest")
            .and_then(Value::as_str)
            != Some(publication_digest)
            || record
                .get("gateway_publication_revision")
                .and_then(Value::as_str)
                != Some(publication_revision.to_string().as_str())
            || record.get("served_model_id").and_then(Value::as_str) != Some("oracle-smoke")
            || record.get("grant_id").and_then(Value::as_str) != Some("oracle-grant")
            || record.get("ingress_protocol").and_then(Value::as_str) != Some("responses")
        {
            return Err(collector("execution fact authority context is not exact"));
        }
    }
    let finished = records
        .iter()
        .find(|record| {
            record.pointer("/fact/kind").and_then(Value::as_str)
                == Some(expected.execution_terminal.as_str())
        })
        .and_then(|record| record.get("fact"))
        .ok_or_else(|| collector("execution terminal fact is missing"))?;
    if finished.get("outcome").and_then(Value::as_str) != Some("accepted")
        || finished.get("attempts_started").and_then(Value::as_u64) != Some(1)
        || finished.get("attempts_finished").and_then(Value::as_u64) != Some(1)
        || finished
            .get("accepted_attempt_ordinal")
            .and_then(Value::as_u64)
            != Some(1)
    {
        return Err(collector(
            "execution terminal outcome is not the green smoke",
        ));
    }
    let fact = |kind: &str| {
        records
            .iter()
            .find(|record| record.pointer("/fact/kind").and_then(Value::as_str) == Some(kind))
            .and_then(|record| record.get("fact"))
            .ok_or_else(|| collector("required execution fact is missing"))
    };
    if current && !valid_smoke_input_selection(fact("request_input_selection")?) {
        return Err(collector(
            "execution input selection is not the observed unverified smoke input",
        ));
    }
    let route = fact("route_decision")?;
    let candidate = fact("candidate_decision")?;
    let attempt = fact("attempt_started")?;
    let commit = fact("semantic_commit")?;
    let attempt_finished = fact("attempt_finished")?;
    let usage = fact("usage_and_cache")?;
    let native_output = candidate.get("ingress_protocol").and_then(Value::as_str)
        == candidate.get("upstream_protocol").and_then(Value::as_str);
    let expected_usage_source = if current && native_output {
        "provider_completion"
    } else {
        "accepted_canonical_model_event"
    };
    if route.get("outcome").and_then(Value::as_str) != Some("ready")
        || route.get("max_attempts").and_then(Value::as_u64) != Some(1)
        || candidate.get("stable_binding_id").and_then(Value::as_str)
            != Some("oracle-native-provider")
        || candidate.get("ingress_protocol").and_then(Value::as_str) != Some("responses")
        || candidate.get("upstream_protocol").and_then(Value::as_str) != Some("responses")
        || attempt.get("ordinal").and_then(Value::as_u64) != Some(1)
        || attempt.get("stable_binding_id").and_then(Value::as_str)
            != Some("oracle-native-provider")
        || attempt.get("request_model").and_then(Value::as_str) != Some("oracle-native-provider")
        || commit.get("ordinal").and_then(Value::as_u64) != Some(1)
        || commit.get("boundary").and_then(Value::as_str) != Some("full_frame_transport_accepted")
        || attempt_finished.get("ordinal").and_then(Value::as_u64) != Some(1)
        || attempt_finished.get("outcome").and_then(Value::as_str) != Some("accepted")
        || attempt_finished
            .get("provider_http_status")
            .and_then(Value::as_u64)
            != Some(200)
        || usage.get("ordinal").and_then(Value::as_u64) != Some(1)
        || usage.get("source").and_then(Value::as_str) != Some(expected_usage_source)
        || usage.get("input_tokens").and_then(Value::as_u64) != Some(3)
        || usage.get("output_tokens").and_then(Value::as_u64) != Some(2)
    {
        return Err(collector(
            "execution route/attempt/commit/usage evidence is not exact",
        ));
    }
    Ok(())
}

fn valid_execution_kind_order(kinds: &[&str], sealed_order: &[&str; 14], current: bool) -> bool {
    if !current {
        return kinds == sealed_order;
    }
    // The current Gateway records the request-local input selection before
    // routing. It is not evidence of human authorship.
    if kinds.len() != CURRENT_EXECUTION_FACT_COUNT
        || kinds.first().copied() != Some("request_input_selection")
    {
        return false;
    }
    let kinds = &kinds[1..];
    // Current canonical delivery can report usage immediately before or after
    // the request terminal. Same-protocol Native delivery owns usage in the
    // main projector, so provider completion records it before attempt finish.
    kinds.len() == sealed_order.len()
        && kinds[..11] == sealed_order[..11]
        && matches!(
            &kinds[11..],
            ["attempt_finished", "request_finished", "usage_and_cache"]
                | ["attempt_finished", "usage_and_cache", "request_finished"]
                | ["usage_and_cache", "attempt_finished", "request_finished"]
        )
}

fn valid_smoke_input_selection(fact: &Value) -> bool {
    fact.get("version").and_then(Value::as_u64) == Some(1)
        && fact.get("evidence").and_then(Value::as_str) == Some("wire_observed")
        && fact.get("message_ordinal").and_then(Value::as_u64) == Some(0)
        && fact.get("source_unverified").and_then(Value::as_bool) == Some(true)
}

fn validate_content(
    records: &[Value],
    expected: &ExpectedObservation,
    freshness_challenge: &str,
    current: bool,
) -> Result<(), ProductionError> {
    let expected_record_count = if current { 8 } else { 7 };
    if records.len() != expected_record_count
        || records.iter().any(|record| {
            !matches!(
                record.get("direction").and_then(Value::as_str),
                Some("request_input" | "response_delivered")
            )
        })
    {
        return Err(collector(
            "ConversationContent count or direction does not match the accepted response",
        ));
    }
    for terminal in &expected.content_terminals {
        let direction = records
            .iter()
            .filter(|record| {
                record.get("direction").and_then(Value::as_str) == Some(&terminal.direction)
            })
            .collect::<Vec<_>>();
        let phases = direction
            .iter()
            .filter_map(|record| record.get("phase").and_then(Value::as_str))
            .collect::<Vec<_>>();
        let expected_phases = match terminal.direction.as_str() {
            "request_input" => vec!["begin", "append", "finish"],
            "response_delivered" if current => {
                vec!["begin", "append", "append", "append", "finish"]
            }
            "response_delivered" => vec!["begin", "append", "append", "finish"],
            _ => return Err(collector("unknown ConversationContent direction")),
        };
        if direction.is_empty()
            || phases != expected_phases
            || direction
                .first()
                .and_then(|record| record.get("phase"))
                .and_then(Value::as_str)
                != Some("begin")
            || direction
                .last()
                .and_then(|record| record.get("phase"))
                .and_then(Value::as_str)
                != Some(terminal.phase.as_str())
            || direction
                .iter()
                .any(|record| record.get("phase").and_then(Value::as_str) == Some("abort"))
        {
            return Err(ProductionError::Collector(format!(
                "content terminal/order failed independently for {}",
                terminal.direction
            )));
        }
    }
    if records.iter().any(|record| {
        record.pointer("/fact/kind").and_then(Value::as_str) == Some("request_finished")
    }) {
        return Err(collector(
            "request_finished was incorrectly used as a ConversationContent barrier",
        ));
    }
    validate_content_freshness(records, freshness_challenge, current)?;
    Ok(())
}

fn validate_content_freshness(
    records: &[Value],
    freshness_challenge: &str,
    current: bool,
) -> Result<(), ProductionError> {
    if freshness_challenge.len() < 32 {
        return Err(collector("freshness challenge is missing"));
    }
    let request = decoded_content(records, "request_input")?;
    let request_append = records.iter().find(|record| {
        record.get("direction").and_then(Value::as_str) == Some("request_input")
            && record.get("phase").and_then(Value::as_str) == Some("append")
    });
    if request != vec![freshness_challenge.as_bytes().to_vec()]
        || request_append.and_then(|record| record.get("content_kind").and_then(Value::as_str))
            != Some("text")
    {
        return Err(collector(
            "request ConversationContent does not bind the fresh challenge exactly",
        ));
    }
    let response = decoded_content(records, "response_delivered")?;
    let response_events = response
        .iter()
        .map(|bytes| {
            serde_json::from_slice::<Value>(bytes)
                .map_err(|_| collector("response ConversationContent is not canonical JSON"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let response_appends = records
        .iter()
        .filter(|record| {
            record.get("direction").and_then(Value::as_str) == Some("response_delivered")
                && record.get("phase").and_then(Value::as_str) == Some("append")
        })
        .collect::<Vec<_>>();
    let expected_kinds = if current {
        vec![
            Some("content_block_started"),
            Some("text_delta"),
            Some("text_finished"),
        ]
    } else {
        vec![Some("content_block_started"), Some("text_delta")]
    };
    if response_events.len() != expected_kinds.len()
        || response_appends
            .iter()
            .map(|record| record.get("content_kind").and_then(Value::as_str))
            .collect::<Vec<_>>()
            != expected_kinds
        || response_appends.iter().any(|record| {
            record.get("downstream_delivery").and_then(Value::as_str)
                != Some("full_frame_transport_accepted")
        })
        || response_events[0]
            .pointer("/event/kind")
            .and_then(Value::as_str)
            != Some("content_block_started")
        || response_events[1]
            .pointer("/event/kind")
            .and_then(Value::as_str)
            != Some("text_delta")
        || response_events[1]
            .pointer("/event/text")
            .and_then(Value::as_str)
            != Some(freshness_challenge)
        || (current
            && (response_events[2]
                .pointer("/event/kind")
                .and_then(Value::as_str)
                != Some("text_finished")
                || response_events[2]
                    .pointer("/event/text")
                    .and_then(Value::as_str)
                    != Some(freshness_challenge)))
    {
        return Err(collector(
            "response ConversationContent does not bind the fresh challenge exactly",
        ));
    }
    Ok(())
}

fn decoded_content(records: &[Value], direction: &str) -> Result<Vec<Vec<u8>>, ProductionError> {
    records
        .iter()
        .filter(|record| {
            record.get("direction").and_then(Value::as_str) == Some(direction)
                && record.get("phase").and_then(Value::as_str) == Some("append")
        })
        .map(|record| {
            let encoded = record
                .get("canonical_bytes_base64")
                .and_then(Value::as_str)
                .ok_or_else(|| collector("ConversationContent append bytes are missing"))?;
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| collector("ConversationContent append bytes are corrupt"))
        })
        .collect()
}

fn validate_otel(records: &[Value], required: &str) -> Result<(), ProductionError> {
    let signals = records
        .iter()
        .filter_map(|record| record.get("signal"))
        .collect::<Vec<_>>();
    if signals.len() != 2
        || signals
            .iter()
            .any(|signal| signal.get("kind").and_then(Value::as_str) != Some(required))
        || signals
            .iter()
            .any(|signal| signal.get("status").and_then(Value::as_str) != Some("ok"))
        || signals
            .iter()
            .filter_map(|signal| signal.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>()
            != ["chat oracle-native-provider", "hiroute.gateway.request"]
    {
        return Err(collector(
            "required exact OTel signals are missing or corrupt",
        ));
    }
    let rendered = serde_json::to_string(records)?;
    if rendered.contains("canonical_bytes_base64") || rendered.contains("authorization") {
        return Err(collector(
            "OTel evidence contains forbidden content or credentials",
        ));
    }
    Ok(())
}

fn collector(message: &'static str) -> ProductionError {
    ProductionError::Collector(message.into())
}

#[cfg(test)]
mod execution_order_tests {
    use super::{
        SEALED_EXECUTION_ORDER as SEALED, valid_execution_kind_order, valid_smoke_input_selection,
    };
    use serde_json::json;

    #[test]
    fn current_order_accepts_each_single_usage_authority_position() {
        let current = |order: &[&'static str]| {
            std::iter::once("request_input_selection")
                .chain(order.iter().copied())
                .collect::<Vec<_>>()
        };
        assert!(valid_execution_kind_order(&current(&SEALED), &SEALED, true));
        let mut before_completion = SEALED;
        before_completion.swap(12, 13);
        assert!(valid_execution_kind_order(
            &current(&before_completion),
            &SEALED,
            true
        ));
        let mut native_provider_completion = SEALED;
        native_provider_completion[11..].rotate_right(1);
        assert!(valid_execution_kind_order(
            &current(&native_provider_completion),
            &SEALED,
            true
        ));
        assert!(!valid_execution_kind_order(&SEALED, &SEALED, true));
        let mut duplicated_selection = current(&SEALED);
        duplicated_selection.insert(0, "request_input_selection");
        assert!(!valid_execution_kind_order(
            &duplicated_selection,
            &SEALED,
            true
        ));
    }

    #[test]
    fn sealed_order_stays_exact_and_attempt_completion_cannot_be_late() {
        let mut usage_before_completion = SEALED;
        usage_before_completion.swap(12, 13);
        assert!(!valid_execution_kind_order(
            &usage_before_completion,
            &SEALED,
            false
        ));
        let mut attempt_after_request = SEALED;
        attempt_after_request.swap(11, 12);
        let current = std::iter::once("request_input_selection")
            .chain(attempt_after_request)
            .collect::<Vec<_>>();
        assert!(!valid_execution_kind_order(&current, &SEALED, true));
    }

    #[test]
    fn current_smoke_input_selection_preserves_source_uncertainty() {
        let mut selection = json!({
            "version": 1,
            "evidence": "wire_observed",
            "message_ordinal": 0,
            "source_unverified": true,
        });
        assert!(valid_smoke_input_selection(&selection));
        selection["source_unverified"] = json!(false);
        assert!(!valid_smoke_input_selection(&selection));
        selection["source_unverified"] = json!(true);
        selection["evidence"] = json!("native_matched");
        assert!(!valid_smoke_input_selection(&selection));
        selection["evidence"] = json!("wire_observed");
        selection["message_ordinal"] = json!(1);
        assert!(!valid_smoke_input_selection(&selection));
    }
}
