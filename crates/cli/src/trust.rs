//! CLI input only; evidence and recommendation policy stay in Application.
use crate::request_input::{MAX_REQUEST_BYTES, RequestInput};
use hiroute_application_api::{ErrorCode, ObservationReadIntentV2, ObservationReadRequestV2};
use hiroute_application_api::{
    PlanTrustQueryV1, PlanTrustReplayQueryV1, TrustGroupByV1, TrustRevisionNameV1, TrustRevisionV1,
};
use serde_json::Value;

pub(super) fn payload(command: &str, options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = RequestInput::optional(options)? {
        let request: ObservationReadRequestV2 =
            serde_json::from_value(input.read(MAX_REQUEST_BYTES)?)
                .map_err(|_| ErrorCode::InvalidArguments)?;
        let matches = matches!(
            (command, &request.intent),
            (
                "observation.plan-trust",
                ObservationReadIntentV2::PlanTrust(_)
            ) | (
                "observation.plan-trust.replay",
                ObservationReadIntentV2::PlanTrustReplay(_)
            ) | (
                "observation.plan-trust.suggestions",
                ObservationReadIntentV2::PlanTrustSuggestions(_)
            )
        );
        if !matches {
            return Err(ErrorCode::InvalidArguments);
        }
        return serde_json::to_value(request).map_err(|_| ErrorCode::Internal);
    }
    if command == "observation.plan-trust.suggestions" {
        if !options.is_empty() {
            return Err(ErrorCode::InvalidArguments);
        }
        return serde_json::to_value(ObservationReadRequestV2::new(
            ObservationReadIntentV2::PlanTrustSuggestions(
                hiroute_application_api::ObservationStatusQueryV2 {},
            ),
        ))
        .map_err(|_| ErrorCode::Internal);
    }
    let mut query = PlanTrustQueryV1 {
        plan_id: String::new(),
        branch_id: None,
        revision: Default::default(),
        from_ms: None,
        to_ms: None,
        group_by: Default::default(),
    };
    let mut thresholds = Vec::new();
    let mut index = 0;
    while index < options.len() {
        match options[index].as_str() {
            "--plan-id" => query.plan_id = crate::next(options, &mut index)?,
            "--branch-id" => query.branch_id = Some(crate::next(options, &mut index)?),
            "--revision" => {
                query.revision = match crate::next(options, &mut index)?.as_str() {
                    "active" => TrustRevisionV1::Name(TrustRevisionNameV1::Active),
                    "all" => TrustRevisionV1::Name(TrustRevisionNameV1::All),
                    n => {
                        TrustRevisionV1::Number(n.parse().map_err(|_| ErrorCode::InvalidArguments)?)
                    }
                }
            }
            "--from-ms" => {
                query.from_ms = Some(
                    crate::next(options, &mut index)?
                        .parse()
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--to-ms" => {
                query.to_ms = Some(
                    crate::next(options, &mut index)?
                        .parse()
                        .map_err(|_| ErrorCode::InvalidArguments)?,
                )
            }
            "--group-by" => {
                query.group_by = match crate::next(options, &mut index)?.as_str() {
                    "model" => TrustGroupByV1::Model,
                    "executor" => TrustGroupByV1::Executor,
                    _ => return Err(ErrorCode::InvalidArguments),
                }
            }
            "--thresholds-millis" if command.ends_with(".replay") => {
                thresholds = crate::next(options, &mut index)?
                    .split(',')
                    .map(|s| s.parse::<u16>().map_err(|_| ErrorCode::InvalidArguments))
                    .collect::<Result<_, _>>()?
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    if query.plan_id.is_empty() {
        return Err(ErrorCode::InvalidArguments);
    }
    let intent = match command {
        "observation.plan-trust" => ObservationReadIntentV2::PlanTrust(query),
        "observation.plan-trust.replay"
            if !thresholds.is_empty()
                && thresholds.len() <= 8
                && thresholds.iter().all(|t| *t <= 1000) =>
        {
            ObservationReadIntentV2::PlanTrustReplay(PlanTrustReplayQueryV1 {
                scope: query,
                thresholds_millis: thresholds,
            })
        }
        _ => return Err(ErrorCode::InvalidArguments),
    };
    serde_json::to_value(ObservationReadRequestV2::new(intent)).map_err(|_| ErrorCode::Internal)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_keeps_exact_custom_thresholds_and_scopes() {
        let options = [
            "--plan-id",
            "plan/trial",
            "--revision",
            "17",
            "--thresholds-millis",
            "720,650",
        ]
        .map(str::to_owned);
        let value = payload("observation.plan-trust.replay", &options).unwrap();
        assert_eq!(value["intent"]["query"]["revision"], 17);
        assert_eq!(
            value["intent"]["query"]["thresholds_millis"],
            serde_json::json!([720, 650])
        );
        let roundtrip: ObservationReadRequestV2 = serde_json::from_value(value).unwrap();
        assert_eq!(roundtrip.operation(), "GetPlanTrustReplay");
        assert!(payload("observation.plan-trust", &options).is_err());
    }
}
