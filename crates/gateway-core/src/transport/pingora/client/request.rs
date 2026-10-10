use super::*;

pub(super) fn build_request_header(
    head: &PreparedRequestHead,
) -> Result<RequestHeader, AttemptError> {
    let mut request = RequestHeader::build(
        head.method.as_str(),
        head.path_and_query.as_bytes(),
        Some(head.headers.len()),
    )
    .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
    for (name, value) in &head.headers {
        request
            .append_header(name.clone(), value.clone())
            .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
    }
    Ok(request)
}

pub(super) fn build_h2_request_header(
    head: &PreparedRequestHead,
    tls: bool,
) -> Result<RequestHeader, AttemptError> {
    let mut request = build_request_header(head)?;
    let authority = request.remove_header(&http::header::HOST).ok_or_else(|| {
        AttemptError::Transport("HTTP/2 request requires an authorized authority".into())
    })?;
    let uri = http::Uri::builder()
        .scheme(if tls { "https" } else { "http" })
        .authority(authority.as_bytes())
        .path_and_query(head.path_and_query.as_ref())
        .build()
        .map_err(|_| AttemptError::Transport("invalid HTTP/2 request authority".into()))?;
    // Pingora otherwise derives :authority from Host while retaining Host on
    // the wire. Some upstream gateways reject that duplicate authority.
    request.set_uri(uri);
    Ok(request)
}

pub(super) async fn run_upstream_reader(
    mut session: ClientSession,
    sender: mpsc::Sender<Result<TransportPrecommitReceipt, AttemptError>>,
    local_read_suppression: Arc<Mutex<ReaderSuppressionClock>>,
) -> ClientSession {
    loop {
        let suppression_started = Instant::now();
        local_read_suppression
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(suppression_started);
        let permit = match sender.reserve().await {
            Ok(permit) => permit,
            Err(_) => {
                local_read_suppression
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .finish();
                return session;
            }
        };
        let (suppressed, suppression_total_at_receipt) = {
            let mut clock = local_read_suppression
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let suppressed = clock.finish();
            (suppressed, clock.total())
        };
        if let Err(error) = session.read_response_header().await {
            permit.send(Err(AttemptError::Transport(error.to_string().into())));
            return session;
        }
        let received_at = Instant::now();
        let Some(response) = session.response_header() else {
            permit.send(Err(AttemptError::Transport(
                "Pingora reader completed without response head".into(),
            )));
            return session;
        };
        let informational = response.status.is_informational();
        permit.send(Ok(TransportPrecommitReceipt {
            event: TransportPrecommitEvent::ResponseHead {
                status: response.status,
                headers: response.headers.clone(),
            },
            received_at,
            local_read_suppressed: suppressed,
            local_read_suppression_total_at_receipt: Some(suppression_total_at_receipt),
        }));
        if !informational {
            break;
        }
    }

    loop {
        let suppression_started = Instant::now();
        local_read_suppression
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .begin(suppression_started);
        let permit = match sender.reserve().await {
            Ok(permit) => permit,
            Err(_) => {
                local_read_suppression
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .finish();
                return session;
            }
        };
        let (suppressed, suppression_total_at_receipt) = {
            let mut clock = local_read_suppression
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let suppressed = clock.finish();
            (suppressed, clock.total())
        };
        match session.read_response_body().await {
            Ok(Some(body)) => {
                permit.send(Ok(TransportPrecommitReceipt {
                    event: TransportPrecommitEvent::Body(body),
                    received_at: Instant::now(),
                    local_read_suppressed: suppressed,
                    local_read_suppression_total_at_receipt: Some(suppression_total_at_receipt),
                }));
            }
            Ok(None) => {
                permit.send(Ok(TransportPrecommitReceipt {
                    event: TransportPrecommitEvent::EndStream,
                    received_at: Instant::now(),
                    local_read_suppressed: suppressed,
                    local_read_suppression_total_at_receipt: Some(suppression_total_at_receipt),
                }));
                return session;
            }
            Err(error) => {
                permit.send(Err(AttemptError::Transport(error.to_string().into())));
                return session;
            }
        }
    }
}
