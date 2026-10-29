//! Presentation-only polling of the bounded Application timeline query.
use super::*;
use hiroute_application_api::{
    ObservationReadIntentV2, ObservationReadRequestV2, RoutingTimelinePageV1,
    RoutingTimelineQueryV1,
};
use std::collections::BTreeMap;

fn parse(options: &[String]) -> Result<RoutingTimelineQueryV1, ErrorCode> {
    let Some(session) = options
        .first()
        .filter(|s| crate::request_input::valid_positional(s))
    else {
        return Err(ErrorCode::InvalidArguments);
    };
    let mut query = RoutingTimelineQueryV1 {
        include_presentation: false,
        session_id: session.clone(),
        after_watermark: None,
        limit: 50,
    };
    let mut index = 1;
    let mut limit = false;
    let mut follow = false;
    while index < options.len() {
        match options[index].as_str() {
            "--follow" if !follow => follow = true,
            "--after-watermark" if query.after_watermark.is_none() => {
                query.after_watermark = Some(next(options, &mut index)?)
            }
            "--limit" if !limit => {
                query.limit = next(options, &mut index)?
                    .parse()
                    .map_err(|_| ErrorCode::InvalidArguments)?;
                limit = true;
            }
            _ => return Err(ErrorCode::InvalidArguments),
        }
        index += 1;
    }
    if !(1..=200).contains(&query.limit) {
        return Err(ErrorCode::InvalidArguments);
    }
    Ok(query)
}
pub(super) fn payload(options: &[String]) -> Result<Value, ErrorCode> {
    if let Some(input) = crate::request_input::RequestInput::optional(options)? {
        let request: ObservationReadRequestV2 =
            serde_json::from_value(input.read(crate::request_input::MAX_REQUEST_BYTES)?)
                .map_err(|_| ErrorCode::InvalidArguments)?;
        if !matches!(request.intent, ObservationReadIntentV2::RoutingTimeline(_)) {
            return Err(ErrorCode::InvalidArguments);
        }
        return serde_json::to_value(request).map_err(|_| ErrorCode::Internal);
    }
    serde_json::to_value(ObservationReadRequestV2::new(
        ObservationReadIntentV2::RoutingTimeline(parse(options)?),
    ))
    .map_err(|_| ErrorCode::Internal)
}

pub(super) fn follow(options: &[String], globals: Globals) -> CliExecution {
    if globals.agent.is_some() || globals.has_capability() {
        return failure(ErrorCode::InvalidArguments, globals.request_id);
    }
    let mut query = match parse(options) {
        Ok(q) => q,
        Err(e) => return failure(e, globals.request_id),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => return failure(ErrorCode::Internal, globals.request_id),
    };
    runtime.block_on(async {
        let interrupt = tokio::signal::ctrl_c();
        tokio::pin!(interrupt);
        let mut seen = BTreeMap::new();
        let mut wait = std::time::Duration::ZERO;
        let mut transient_failures = 0;
        let mut reset_once = false;
        loop {
            if !wait.is_zero() {
                tokio::select! { _ = &mut interrupt => break, _ = tokio::time::sleep(wait) => {} }
            }
            let request_id = next_request_id();
            let request = LocalControlWireRequestV2 {
                schema_version: LOCAL_CONTROL_SCHEMA_V2,
                request_id: request_id.clone(),
                operation_id: "GetRoutingTimeline".into(),
                protected_grant: None,
                payload: serde_json::to_value(ObservationReadRequestV2::new(
                    ObservationReadIntentV2::RoutingTimeline(query.clone()),
                ))
                .expect("serializable query"),
            };
            let timeout = globals.timeout.unwrap_or(std::time::Duration::from_secs(5));
            let call = tokio::task::spawn_blocking(move || {
                LocalControlClient::cli_from_environment()
                    .and_then(|client| client.with_timeout(timeout).call(request))
            });
            let response =
                tokio::select! { _ = &mut interrupt => break, response = call => response };
            let response = match response {
                Ok(Ok(r)) => r,
                _ if transient_failures < 3 => {
                    transient_failures += 1;
                    wait = std::time::Duration::from_secs(2);
                    continue;
                }
                _ => return failure(ErrorCode::DaemonUnavailable, Some(request_id)),
            };
            if response.status != hiroute_application_api::MachineStatus::Succeeded {
                if response
                    .error
                    .as_ref()
                    .is_some_and(|e| e.code == ErrorCode::ChangePreviewStale)
                    && !reset_once
                {
                    query.after_watermark = None;
                    reset_once = true;
                    wait = std::time::Duration::ZERO;
                    continue;
                }
                if let Some(delay) = retry_delay(response.error.as_ref(), &mut transient_failures) {
                    wait = delay;
                    continue;
                }
                return render(response);
            }
            let Some(page) = response
                .data
                .and_then(|v| serde_json::from_value::<RoutingTimelinePageV1>(v).ok())
            else {
                return failure(ErrorCode::Internal, Some(request_id));
            };
            reset_once = false;
            transient_failures = 0;
            if emit(&page, &mut seen, &mut std::io::stdout().lock()).is_err() {
                break;
            }
            query.after_watermark = Some(page.watermark);
            wait = if page.complete {
                std::time::Duration::from_secs(2)
            } else {
                std::time::Duration::ZERO
            };
        }
        CliExecution {
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    })
}

fn retry_delay(
    error: Option<&hiroute_application_api::ErrorV1>,
    failures: &mut u32,
) -> Option<std::time::Duration> {
    let error = error?;
    if !error.retryable {
        return None;
    }
    if error.observation_read.as_ref().is_some_and(|r| {
        r.reason == hiroute_application_api::ObservationReadFailureReasonV1::ReaderBusy
    }) {
        return Some(std::time::Duration::from_millis(u64::from(
            error
                .observation_read
                .as_ref()?
                .retry_after_ms
                .clamp(100, 1000),
        )));
    }
    if matches!(
        error.code,
        ErrorCode::DaemonUnavailable | ErrorCode::ObservationUnavailable
    ) && *failures < 3
    {
        *failures += 1;
        return Some(std::time::Duration::from_secs(2));
    }
    None
}

fn emit(
    page: &RoutingTimelinePageV1,
    seen: &mut BTreeMap<String, String>,
    output: &mut impl Write,
) -> std::io::Result<()> {
    for turn in &page.turns {
        if seen.get(&turn.turn_id) == Some(&turn.content_digest) {
            continue;
        }
        if seen.len() >= 10_000 && !seen.contains_key(&turn.turn_id) {
            return Err(std::io::Error::other("timeline scope exceeded"));
        }
        serde_json::to_writer(
            &mut *output,
            &json!({"schema":"hiroute.routing-timeline-change/v1","turn":turn,"watermark":page.watermark}),
        )?;
        output.write_all(b"\n")?;
        seen.insert(turn.turn_id.clone(), turn.content_digest.clone());
    }
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn follow_retries_busy_and_bounded_transients_but_stops_for_scope_and_authorization() {
        let mut count = 0;
        let mut busy = hiroute_application_api::ErrorV1::new(ErrorCode::ObservationUnavailable);
        busy.retryable = true;
        busy.observation_read = Some(hiroute_application_api::ObservationReadFailureV1 {
            reason: hiroute_application_api::ObservationReadFailureReasonV1::ReaderBusy,
            retry_after_ms: 100,
            action: "retry".into(),
        });
        assert_eq!(
            retry_delay(Some(&busy), &mut count),
            Some(std::time::Duration::from_millis(100))
        );
        let mut transient =
            hiroute_application_api::ErrorV1::new(ErrorCode::ObservationUnavailable);
        transient.retryable = true;
        for _ in 0..3 {
            assert!(retry_delay(Some(&transient), &mut count).is_some());
        }
        assert!(retry_delay(Some(&transient), &mut count).is_none());
        // A successful page resets the budget in follow; a nonretryable storage
        // failure must never use that budget, even with the same public code.
        count = 0;
        assert!(retry_delay(Some(&transient), &mut count).is_some());
        transient.retryable = false;
        assert!(retry_delay(Some(&transient), &mut 0).is_none());
        for code in [
            ErrorCode::ScopeTooLarge,
            ErrorCode::CapabilityDenied,
            ErrorCode::InvalidArguments,
        ] {
            assert!(
                retry_delay(Some(&hiroute_application_api::ErrorV1::new(code)), &mut 0).is_none()
            );
        }
    }

    #[test]
    fn watch_requires_a_session_and_bounded_scope() {
        let args = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
        assert!(parse(&args("--follow")).is_err());
        assert!(parse(&args("session --follow --limit 201")).is_err());
        assert!(parse(&args("session --follow --follow")).is_err());
        assert_eq!(
            parse(&args(
                "session --follow --limit 10 --after-watermark cursor"
            ))
            .unwrap()
            .limit,
            10
        );
    }
}
