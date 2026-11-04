//! Human flags for opt-in analysis; Application owns defaults and evidence.
use super::*;
use hiroute_application_api::{
    AnalysisCostBasisV1, AnalysisCostGroupV1, AnalysisPeriodV1, AnalysisViewV1,
    ObservationAnalysisQueryV1,
};
use std::collections::BTreeSet;

pub(super) fn sessions(options: &[String], session: Option<&str>) -> Result<Value, ErrorCode> {
    let mut q = ObservationAnalysisQueryV1::new(if session.is_some() {
        AnalysisViewV1::Conversation
    } else {
        AnalysisViewV1::Conversations
    });
    q.session_id = session.map(str::to_owned);
    parse(options, q, false)
}

pub(super) fn value(options: &[String]) -> Result<Value, ErrorCode> {
    parse(
        options,
        ObservationAnalysisQueryV1::new(AnalysisViewV1::CostRanking),
        true,
    )
}

fn parse(
    options: &[String],
    mut q: ObservationAnalysisQueryV1,
    monetary: bool,
) -> Result<Value, ErrorCode> {
    let showing_session = q.session_id.is_some();
    let mut seen = BTreeSet::new();
    let mut index = 0;
    while index < options.len() {
        let flag = options[index].as_str();
        if !seen.insert(flag) {
            return Err(ErrorCode::InvalidArguments);
        }
        match flag {
            "--view" => {
                q.view = match (
                    monetary,
                    next(options, &mut index)?.as_str(),
                    showing_session,
                ) {
                    (false, "conversations", false) => AnalysisViewV1::Conversations,
                    (false, "switches", false) => AnalysisViewV1::Switches,
                    (false, "conversation", true) => AnalysisViewV1::Conversation,
                    (false, "routing", true) => AnalysisViewV1::Routing,
                    (true, "summary", _) => AnalysisViewV1::SavingsSummary,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--period" => {
                q.time.period = Some(match next(options, &mut index)?.as_str() {
                    "today" => AnalysisPeriodV1::Today,
                    "yesterday" => AnalysisPeriodV1::Yesterday,
                    "7d" => AnalysisPeriodV1::SevenDays,
                    "30d" => AnalysisPeriodV1::ThirtyDays,
                    _ => return Err(ErrorCode::InvalidArguments),
                })
            }
            "--day" => q.time.day = Some(next(options, &mut index)?),
            "--from-ms" => q.time.from_ms = Some(parse_i64(options, &mut index)?),
            "--to-ms" => q.time.to_ms = Some(parse_i64(options, &mut index)?),
            "--analysis-scope" => q.analysis_scope = Some(next(options, &mut index)?),
            "--include-decision-failure" => q.include_decision_failure = true,
            "--session" if q.session_id.is_none() => {
                q.session_id = Some(next(options, &mut index)?)
            }
            "--request" => q.request_id = Some(next(options, &mut index)?),
            "--routing" => {
                let id = next(options, &mut index)?;
                AgentPlanId::parse(&id).map_err(|_| ErrorCode::InvalidArguments)?;
                q.plan_id = Some(id);
            }
            "--agent" => q.agent_id = Some(next(options, &mut index)?),
            "--query" if !monetary => q.query = Some(next(options, &mut index)?),
            "--limit" => {
                q.limit = next(options, &mut index)?
                    .parse()
                    .map_err(|_| ErrorCode::InvalidArguments)?
            }
            "--cursor" => q.cursor = Some(next(options, &mut index)?),
            "--basis" if monetary => {
                q.basis = Some(match next(options, &mut index)?.as_str() {
                    "recorded" => AnalysisCostBasisV1::Recorded,
                    "api-equivalent" => AnalysisCostBasisV1::ApiEquivalent,
                    _ => return Err(ErrorCode::InvalidArguments),
                })
            }
            "--group-by" if monetary => {
                q.group_by = Some(match next(options, &mut index)?.as_str() {
                    "session" => AnalysisCostGroupV1::Session,
                    "request" => AnalysisCostGroupV1::Request,
                    _ => return Err(ErrorCode::InvalidArguments),
                })
            }
            "--order-by" if monetary => {
                if next(options, &mut index)? != "cost-desc" {
                    return Err(ErrorCode::InvalidArguments);
                }
            }
            "--currency" if monetary => q.currency = Some(next(options, &mut index)?),
            "--reference-binding" if monetary => {
                q.reference_binding_id = Some(next(options, &mut index)?)
            }
            "--valuation-kind" if monetary => {
                q.valuation_kind = Some(match next(options, &mut index)?.as_str() {
                    "usage-estimate" => {
                        hiroute_application_api::PriceValuationKindV1::UsageEstimate
                    }
                    "api-equivalent" => {
                        hiroute_application_api::PriceValuationKindV1::ApiEquivalent
                    }
                    _ => return Err(ErrorCode::InvalidArguments),
                })
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    let absolute = q.time.from_ms.is_some() || q.time.to_ms.is_some();
    if q.limit == 0
        || q.limit > 200
        || u8::from(q.time.period.is_some()) + u8::from(q.time.day.is_some()) + u8::from(absolute)
            > 1
        || (q.analysis_scope.is_some() || q.cursor.is_some()) && !q.time.is_empty()
        || q.view == AnalysisViewV1::SavingsSummary
            && (q.group_by.is_some() || seen.contains("--order-by"))
        || q.query.is_some() && q.view != AnalysisViewV1::Conversations
        || q.include_decision_failure && q.view != AnalysisViewV1::Routing
    {
        return Err(ErrorCode::InvalidArguments);
    }
    serde_json::to_value(ObservationReadRequestV2::new(
        ObservationReadIntentV2::Analysis(q),
    ))
    .map_err(|_| ErrorCode::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).into()).collect()
    }

    #[test]
    fn new_views_preserve_family_scope_and_reject_legacy_switch_filter() {
        assert!(
            sessions(
                &args(&["--view", "conversations", "--model-switch", "only"]),
                None
            )
            .is_err()
        );
        assert!(sessions(&args(&["--view", "routing"]), None).is_err());
        assert!(sessions(&args(&["--view", "switches"]), Some("session-one")).is_err());
        let payload = sessions(
            &args(&[
                "--session",
                "session-one",
                "--view",
                "conversations",
                "--period",
                "yesterday",
            ]),
            None,
        )
        .unwrap();
        assert_eq!(payload["intent"]["query"]["session_id"], "session-one");
        assert_eq!(payload["intent"]["query"]["time"]["period"], "yesterday");
    }
    #[test]
    fn continuation_inherits_time_and_cannot_mix_time_selections() {
        assert!(
            sessions(
                &args(&[
                    "--view",
                    "conversation",
                    "--analysis-scope",
                    "scope",
                    "--period",
                    "7d"
                ]),
                Some("session-one")
            )
            .is_err()
        );
        assert!(
            sessions(
                &args(&[
                    "--view",
                    "conversations",
                    "--day",
                    "2026-10-16",
                    "--from-ms",
                    "1"
                ]),
                None
            )
            .is_err()
        );
        assert!(
            sessions(
                &args(&[
                    "--view",
                    "conversation",
                    "--analysis-scope",
                    "scope",
                    "--limit",
                    "5"
                ]),
                Some("session-one")
            )
            .is_ok()
        );
    }
    #[test]
    fn cost_ranking_and_summary_have_distinct_explicit_inputs() {
        let ranked = value(&args(&[
            "--group-by",
            "session",
            "--order-by",
            "cost-desc",
            "--basis",
            "recorded",
            "--valuation-kind",
            "usage-estimate",
        ]))
        .unwrap();
        assert_eq!(
            ranked["intent"]["query"]["valuation_kind"],
            "usage_estimate"
        );
        let summary = value(&args(&["--view", "summary", "--period", "yesterday"])).unwrap();
        assert_eq!(summary["intent"]["query"]["view"], "savings_summary");
        assert!(value(&args(&["--view", "summary", "--group-by", "request"])).is_err());
        assert!(value(&args(&["--group-by", "request", "--order-by", "name"])).is_err());
    }
}
