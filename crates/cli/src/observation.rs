//! Finite CLI codecs for observation and value reads.
mod analysis;

#[cfg(test)]
use std::io::Read;

use hiroute_application_api::{
    AgentPlanId, ErrorCode, ModelSwitchFilter, ObservationReadIntentV2, ObservationReadRequestV2,
    ObservationValueReportOptionsV2, SessionContentModeV1, SessionListRequestV1, SessionLookupV1,
    ValueGroupByV1, ValuePeriodV1,
};
use serde_json::Value;

use super::next;
#[cfg(test)]
use super::read_json;
use crate::request_input::{MAX_REQUEST_BYTES, RequestInput, valid_positional};

pub(super) fn plan_quality_samples(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        return observation_value_payload(
            ObservationCommand::PlanQuality,
            input.read(MAX_REQUEST_BYTES)?,
        );
    }
    let mut query = hiroute_application_api::PlanQualitySamplesQuery {
        execution_group: None,
        selection_reason: None,
        segment_ids: Vec::new(),
        executor: None,
        trust_row: None,
        include_routing_observations: true,
        competence: None,
        plan_id: None,
        session_id: None,
        segment_id: None,
        plan_revision: None,
        model_configuration_id: None,
        execution: None,
        unrated_only: false,
        from_ms: None,
        to_ms: None,
        score_gt: None,
        score_lt: None,
        limit: 50,
        cursor: None,
    };
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--plan-id" => query.plan_id = Some(next(options, &mut index)?),
            "--session-id" => query.session_id = Some(next(options, &mut index)?),
            "--segment-id" => query.segment_id = Some(next(options, &mut index)?),
            "--segment-ids" => {
                query.segment_ids = next(options, &mut index)?
                    .split(',')
                    .map(str::to_owned)
                    .collect()
            }
            "--executor" => query.executor = Some(next(options, &mut index)?),
            "--execution-group" => {
                query.execution_group = Some(
                    serde_json::from_value(Value::String(next(options, &mut index)?))
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--selection-reason" => {
                query.selection_reason = Some(
                    serde_json::from_value(Value::String(next(options, &mut index)?))
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--plan-revision" => query.plan_revision = Some(parse_u64(options, &mut index)?),
            "--model" => query.model_configuration_id = Some(next(options, &mut index)?),
            "--from-ms" => query.from_ms = Some(parse_i64(options, &mut index)?),
            "--to-ms" => query.to_ms = Some(parse_i64(options, &mut index)?),
            "--unrated" => query.unrated_only = true,
            "--competence" => {
                query.competence = Some(match next(options, &mut index)?.as_str() {
                    "below-floor" => hiroute_application_api::PlanCompetenceFilter::BelowFloor,
                    "meets-floor" => hiroute_application_api::PlanCompetenceFilter::MeetsFloor,
                    _ => return Err(ErrorCode::InvalidArguments),
                })
            }
            "--score-gt" => query.score_gt = Some(parse_score(options, &mut index)?),
            "--score-lt" => query.score_lt = Some(parse_score(options, &mut index)?),
            "--limit" => {
                query.limit = next(options, &mut index)?
                    .parse()
                    .map_err(|_| ErrorCode::InvalidArguments)?
            }
            "--cursor" => query.cursor = Some(next(options, &mut index)?),
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    if query.plan_id.is_none() && query.session_id.is_none() {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(ObservationReadRequestV2::new(
        ObservationReadIntentV2::PlanQuality(Box::new(query)),
    ))
    .map_err(|_| ErrorCode::Internal)
}

pub(super) fn sessions_list(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        return observation_value_payload(
            ObservationCommand::Sessions,
            input.read(MAX_REQUEST_BYTES)?,
        );
    }
    if options.iter().any(|flag| flag == "--view") {
        return analysis::sessions(options, None);
    }
    let mut query = SessionListRequestV1::default();
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--from-ms" => query.from_ms = Some(parse_i64(options, &mut index)?),
            "--to-ms" => query.to_ms = Some(parse_i64(options, &mut index)?),
            "--agent" => query.agent_id = Some(next(options, &mut index)?),
            "--query" => query.query = Some(next(options, &mut index)?),
            "--model-switch" => {
                query.model_switch = match next(options, &mut index)?.as_str() {
                    "any" => ModelSwitchFilter::Any,
                    "only" => ModelSwitchFilter::Only,
                    "exclude" => ModelSwitchFilter::Exclude,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--include-unlinked" => query.include_unlinked = true,
            "--limit" => {
                query.limit = Some(
                    next(options, &mut index)?
                        .parse::<u16>()
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--cursor" => query.cursor = Some(next(options, &mut index)?),
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    serde_json::to_value(query).map_err(|_| ErrorCode::Internal)
}

pub(super) fn session_show(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        return observation_value_payload(
            ObservationCommand::Timeline,
            input.read(MAX_REQUEST_BYTES)?,
        );
    }
    let Some(id) = options.first().filter(|value| valid_positional(value)) else {
        return Err(ErrorCode::InvalidArguments);
    };
    if options.iter().any(|flag| flag == "--view") {
        return analysis::sessions(&options[1..], Some(id));
    }
    let content = if options.len() == 1 {
        None
    } else if options.len() == 3 && options[1] == "--content" {
        Some(match options[2].as_str() {
            "none" => SessionContentModeV1::None,
            "messages" => SessionContentModeV1::Messages,
            "messages-and-tools" => SessionContentModeV1::MessagesAndTools,
            _ => return Err(ErrorCode::InvalidArguments),
        })
    } else {
        return Err(ErrorCode::InvalidArguments);
    };
    serde_json::to_value(SessionLookupV1 {
        id: id.clone(),
        content,
    })
    .map_err(|_| ErrorCode::Internal)
}

pub(super) fn session_receipt(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        return observation_value_payload(
            ObservationCommand::Facts,
            input.read(MAX_REQUEST_BYTES)?,
        );
    }
    let [id] = options else {
        return Err(ErrorCode::InvalidArguments);
    };
    if !valid_positional(id) {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(SessionLookupV1 {
        id: id.clone(),
        content: None,
    })
    .map_err(|_| ErrorCode::Internal)
}

pub(super) fn value_show(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        return observation_value_payload(
            ObservationCommand::Value,
            input.read(MAX_REQUEST_BYTES)?,
        );
    }
    if options
        .iter()
        .any(|flag| matches!(flag.as_str(), "--view" | "--analysis-scope" | "--order-by"))
        || options.windows(2).any(|pair| {
            pair[0] == "--group-by" && matches!(pair[1].as_str(), "session" | "request")
        })
    {
        return analysis::value(options);
    }
    let mut query = ObservationValueReportOptionsV2::default();
    let mut basis = None;
    let mut reference = None;
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--basis" if basis.is_none() => basis = Some(next(options, &mut index)?),
            "--reference-binding" if reference.is_none() => {
                reference = Some(next(options, &mut index)?)
            }
            "--routing" => {
                query.agent_plan_id = Some(
                    AgentPlanId::parse(next(options, &mut index)?)
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--from-ms" => query.from_ms = Some(parse_i64(options, &mut index)?),
            "--to-ms" => query.to_ms = Some(parse_i64(options, &mut index)?),
            "--currency" => query.currency = Some(next(options, &mut index)?),
            "--session" => {
                query.session_id = Some(
                    hiroute_application_api::SessionId::parse(next(options, &mut index)?)
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--group-by" => {
                query.group_by = match next(options, &mut index)?.as_str() {
                    "none" => ValueGroupByV1::None,
                    "day" => ValueGroupByV1::Day,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--period" => {
                query.period = Some(match next(options, &mut index)?.as_str() {
                    "today" => ValuePeriodV1::Today,
                    "7d" => ValuePeriodV1::SevenDays,
                    "30d" => ValuePeriodV1::ThirtyDays,
                    _ => return Err(ErrorCode::InvalidArguments),
                });
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let intent = match (basis.as_deref(), reference) {
        (None, None) => ObservationReadIntentV2::ValueReport(query),
        (Some("api-equivalent"), Some(reference_binding_id))
            if query.currency.is_some() && query.group_by == ValueGroupByV1::None =>
        {
            ObservationReadIntentV2::ApiEquivalentValue(
                hiroute_application_api::ApiEquivalentValueOptionsV1 {
                    report: query,
                    reference_binding_id,
                },
            )
        }
        _ => return Err(ErrorCode::InvalidArguments),
    };
    serde_json::to_value(ObservationReadRequestV2::new(intent)).map_err(|_| ErrorCode::Internal)
}

#[derive(Clone, Copy)]
enum ObservationCommand {
    Sessions,
    Timeline,
    Facts,
    Value,
    PlanQuality,
}

#[cfg(test)]
fn observation_read_payload(
    command: ObservationCommand,
    reader: impl Read,
) -> Result<Value, ErrorCode> {
    observation_value_payload(command, read_json(reader)?)
}

fn observation_value_payload(
    command: ObservationCommand,
    value: Value,
) -> Result<Value, ErrorCode> {
    let request: ObservationReadRequestV2 = crate::request_input::deserialize(value)?;
    if let ObservationReadIntentV2::Analysis(ref q) = request.intent {
        let allowed = match command {
            ObservationCommand::Sessions => matches!(
                q.view,
                hiroute_application_api::AnalysisViewV1::Conversations
                    | hiroute_application_api::AnalysisViewV1::Switches
            ),
            ObservationCommand::Timeline => {
                matches!(
                    q.view,
                    hiroute_application_api::AnalysisViewV1::Conversation
                        | hiroute_application_api::AnalysisViewV1::Routing
                ) && q.session_id.is_some()
            }
            ObservationCommand::Value => q.view.monetary(),
            _ => false,
        };
        if !allowed {
            return Err(ErrorCode::InvalidArguments);
        }
        return serde_json::to_value(request).map_err(|_| ErrorCode::Internal);
    }
    let matches_command = match (command, &request.intent) {
        (
            ObservationCommand::Sessions,
            ObservationReadIntentV2::Sessions(_)
            | ObservationReadIntentV2::Search(_)
            | ObservationReadIntentV2::SessionSearch(_),
        ) => true,
        (
            ObservationCommand::Timeline,
            ObservationReadIntentV2::Catalog(_)
            | ObservationReadIntentV2::Ancestry(_)
            | ObservationReadIntentV2::Content(_),
        ) => true,
        (ObservationCommand::Facts, ObservationReadIntentV2::Facts(_)) => true,
        (ObservationCommand::Timeline, ObservationReadIntentV2::Timeline(query)) => {
            query.session_id.is_some()
        }
        (
            ObservationCommand::Value,
            ObservationReadIntentV2::Value(_)
            | ObservationReadIntentV2::HomeValue(_)
            | ObservationReadIntentV2::ValueReport(_)
            | ObservationReadIntentV2::ApiEquivalentValue(_),
        ) => true,
        (ObservationCommand::PlanQuality, ObservationReadIntentV2::PlanQuality(query)) => {
            query.plan_id.is_some() || query.session_id.is_some()
        }
        _ => false,
    };
    if !matches_command {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(request).map_err(|_| ErrorCode::Internal)
}

fn parse_i64(options: &[String], index: &mut usize) -> Result<i64, ErrorCode> {
    next(options, index)?
        .parse()
        .map_err(|_| ErrorCode::InvalidArguments)
}

fn parse_u64(options: &[String], index: &mut usize) -> Result<u64, ErrorCode> {
    next(options, index)?
        .parse()
        .map_err(|_| ErrorCode::InvalidArguments)
}

fn parse_score(options: &[String], index: &mut usize) -> Result<f64, ErrorCode> {
    let value = next(options, index)?
        .parse::<f64>()
        .map_err(|_| ErrorCode::InvalidArguments)?;
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(ErrorCode::InvalidArguments)
    }
}

pub(super) fn capacity_get(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        let request: hiroute_application_api::ClientEmptyRequestV1 =
            serde_json::from_value(input.read(MAX_REQUEST_BYTES)?)
                .map_err(|_| ErrorCode::InvalidArguments)?;
        return serde_json::to_value(request).map_err(|_| ErrorCode::Internal);
    }
    if options.is_empty() {
        Ok(serde_json::json!({}))
    } else {
        Err(ErrorCode::InvalidArguments)
    }
}
pub(super) fn capacity_set(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        let request: hiroute_application_api::ObservationCapacityUpdateV1 =
            serde_json::from_value(input.read(MAX_REQUEST_BYTES)?)
                .map_err(|_| ErrorCode::InvalidArguments)?;
        return serde_json::to_value(request).map_err(|_| ErrorCode::Internal);
    }
    let value = match options {
        [flag] if flag == "--disable" => None,
        [flag, value] if flag == "--max-active-bytes" => Some(
            value
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or(ErrorCode::InvalidArguments)?,
        ),
        _ => return Err(ErrorCode::InvalidArguments),
    };
    Ok(serde_json::json!({"max_active_bytes":value}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn defaults_are_not_invented_by_cli_presentation() {
        let sessions: SessionListRequestV1 =
            serde_json::from_value(sessions_list(&[]).unwrap()).unwrap();
        assert_eq!(sessions.from_ms, None);
        assert_eq!(sessions.to_ms, None);
        assert_eq!(sessions.limit, None);
        assert_eq!(sessions.cursor, None);

        let request: ObservationReadRequestV2 =
            serde_json::from_value(value_show(&[]).unwrap()).unwrap();
        let ObservationReadIntentV2::ValueReport(value) = request.intent else {
            panic!("human value options must read current observation totals");
        };
        assert_eq!(value.agent_plan_id, None);
        assert_eq!(value.from_ms, None);
        assert_eq!(value.to_ms, None);
        assert_eq!(value.period, None);
        assert_eq!(value.currency, None);
    }

    #[test]
    fn human_value_flags_use_current_scoped_totals_and_day_groups() {
        let payload = value_show(
            &[
                "--routing",
                "plan/current",
                "--session",
                "observation-session-current",
                "--from-ms",
                "100",
                "--to-ms",
                "90000000",
                "--group-by",
                "day",
                "--currency",
                "CNY",
            ]
            .map(str::to_owned),
        )
        .unwrap();
        assert_eq!(payload["schema"], "hiroute.observation.query/v2");
        assert_eq!(payload["intent"]["view"], "value_report");
        let request: ObservationReadRequestV2 = serde_json::from_value(payload).unwrap();
        let ObservationReadIntentV2::ValueReport(q) = request.intent else {
            panic!()
        };
        assert_eq!(q.agent_plan_id.unwrap().as_str(), "plan/current");
        assert_eq!(
            q.session_id.unwrap().as_str(),
            "observation-session-current"
        );
        assert_eq!((q.from_ms, q.to_ms), (Some(100), Some(90_000_000)));
        assert_eq!(q.group_by, ValueGroupByV1::Day);
        assert_eq!(q.currency.as_deref(), Some("CNY"));
    }

    #[test]
    fn plan_quality_requires_scope_and_preserves_strict_score_filters() {
        assert_eq!(plan_quality_samples(&[]), Err(ErrorCode::InvalidArguments));
        let payload = plan_quality_samples(&[
            "--plan-id".into(),
            "plan/one".into(),
            "--model".into(),
            "model/config-a".into(),
            "--score-gt".into(),
            "0.2".into(),
            "--score-lt".into(),
            "0.8".into(),
        ])
        .unwrap();
        let request: ObservationReadRequestV2 = serde_json::from_value(payload).unwrap();
        let ObservationReadIntentV2::PlanQuality(query) = request.intent else {
            panic!("expected plan quality intent");
        };
        assert_eq!(query.plan_id.as_deref(), Some("plan/one"));
        assert_eq!(
            query.model_configuration_id.as_deref(),
            Some("model/config-a")
        );
        assert_eq!(query.score_gt, Some(0.2));
        assert_eq!(query.score_lt, Some(0.8));
        assert_eq!(
            plan_quality_samples(&[
                "--plan-id".into(),
                "plan/one".into(),
                "--score-gt".into(),
                "NaN".into(),
            ]),
            Err(ErrorCode::InvalidArguments)
        );
    }

    #[test]
    fn v2_stdin_is_typed_and_command_scoped() {
        let sessions = json!({
            "schema": "hiroute.observation.query/v2",
            "intent": {"view": "sessions", "query": {
                "from_ms": 1, "to_ms": 2, "session_id": null, "request_id": null, "limit": 1,
                "cursor": null, "agent_id": null, "plan_id": "plan/one",
                "native_model": null, "outcome": null, "only_model_switch": false
            }}
        });
        assert_eq!(
            observation_read_payload(
                ObservationCommand::Sessions,
                std::io::Cursor::new(serde_json::to_vec(&sessions).unwrap()),
            )
            .unwrap(),
            sessions
        );
        assert_eq!(
            observation_read_payload(
                ObservationCommand::Value,
                std::io::Cursor::new(serde_json::to_vec(&sessions).unwrap()),
            ),
            Err(ErrorCode::InvalidArguments)
        );

        let timeline = json!({
            "schema": "hiroute.observation.query/v2",
            "intent": {"view": "timeline", "query": {
                "from_ms": 1, "to_ms": 2, "session_id": "session/one", "request_id": null, "limit": 1,
                "cursor": null, "agent_id": null, "plan_id": "plan/one",
                "native_model": null, "outcome": null, "only_model_switch": false
            }}
        });
        assert_eq!(
            observation_read_payload(
                ObservationCommand::Timeline,
                std::io::Cursor::new(serde_json::to_vec(&timeline).unwrap()),
            )
            .unwrap(),
            timeline
        );
        assert_eq!(
            observation_read_payload(
                ObservationCommand::Sessions,
                std::io::Cursor::new(serde_json::to_vec(&timeline).unwrap()),
            ),
            Err(ErrorCode::InvalidArguments)
        );
        let mut unscoped_timeline = timeline.clone();
        unscoped_timeline["intent"]["query"]["session_id"] = serde_json::Value::Null;
        assert_eq!(
            observation_read_payload(
                ObservationCommand::Timeline,
                std::io::Cursor::new(serde_json::to_vec(&unscoped_timeline).unwrap()),
            ),
            Err(ErrorCode::InvalidArguments)
        );

        let value = json!({
            "schema": "hiroute.observation.query/v2",
            "intent": {"view": "value", "query": {
                "from_ms": 1, "to_ms": 2, "session_id": null,
                "plan_id": "plan/one", "currency": "USD"
            }}
        });
        assert_eq!(
            observation_read_payload(
                ObservationCommand::Value,
                std::io::Cursor::new(serde_json::to_vec(&value).unwrap()),
            )
            .unwrap(),
            value
        );
    }
    #[test]
    fn api_equivalent_is_explicit_and_old_value_flags_keep_their_intent() {
        let args = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(value_show(&[]).unwrap()["intent"]["view"], "value_report");
        let value = value_show(&args(
            "--basis api-equivalent --reference-binding binding/primary --currency CNY --period 7d",
        ))
        .unwrap();
        assert_eq!(value["intent"]["view"], "api_equivalent_value");
        for invalid in [
            "--basis api-equivalent",
            "--reference-binding binding/one",
            "--basis cash --reference-binding binding/one --currency USD",
            "--basis api-equivalent --reference-binding binding/one --currency USD --group-by day",
        ] {
            assert!(value_show(&args(invalid)).is_err());
        }
    }
}
