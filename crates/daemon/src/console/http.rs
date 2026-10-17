use super::{
    auth::{AuthError, valid_context},
    server::Server,
};
use bytes::Bytes;
use http::{Method, Request, Response, StatusCode, header};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};
use zeroize::Zeroizing;
const PREFIX: &str = "/api/console/v1";
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'";
type Reply = Response<Full<Bytes>>;
pub(super) async fn handle(
    server: Arc<Server>,
    request: Request<Incoming>,
    peer: SocketAddr,
    local: SocketAddr,
) -> Result<Reply, Infallible> {
    let reply = match checked(&server, request, peer, local).await {
        Ok(reply) => reply,
        Err((status, code)) => failure(status, code),
    };
    Ok(headers(reply))
}
/// Every request, including data-free assets, is checked before body collection or dispatch.
pub(super) fn valid_origin(headers: &http::HeaderMap, local: SocketAddr) -> bool {
    let mut hosts = headers.get_all(header::HOST).iter();
    let Some(host) = hosts.next().and_then(|h| h.to_str().ok()) else {
        return false;
    };
    if hosts.next().is_some() {
        return false;
    }
    let mut allowed = vec![local.to_string()];
    if local.ip().is_loopback() {
        allowed.extend([
            format!("127.0.0.1:{}", local.port()),
            format!("localhost:{}", local.port()),
            format!("[::1]:{}", local.port()),
        ]);
    }
    if !allowed.iter().any(|a| a == host) {
        return false;
    }
    let mut origins = headers.get_all(header::ORIGIN).iter();
    if let Some(origin) = origins.next()
        && (origin.to_str().ok() != Some(format!("http://{host}").as_str())
            || origins.next().is_some())
    {
        return false;
    }
    let mut sites = headers.get_all("sec-fetch-site").iter();
    if let Some(site) = sites.next()
        && (!matches!(site.to_str().ok(), Some("same-origin" | "none")) || sites.next().is_some())
    {
        return false;
    }
    true
}
async fn checked(
    server: &Arc<Server>,
    request: Request<Incoming>,
    peer: SocketAddr,
    local: SocketAddr,
) -> Result<Reply, (StatusCode, &'static str)> {
    if !valid_origin(request.headers(), local) {
        return Err((StatusCode::FORBIDDEN, "CONSOLE_ORIGIN_DENIED"));
    }
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    if !path.starts_with(PREFIX) {
        if method != Method::GET {
            return Err((StatusCode::NOT_FOUND, "COMMAND_UNKNOWN"));
        }
        if let Some(assets) = &server.assets {
            if let Some((mime, bytes)) = assets.get(&path) {
                return Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, mime)
                    .body(Full::new(Bytes::copy_from_slice(&bytes)))
                    .expect("static response"));
            }
            return Err((StatusCode::NOT_FOUND, "COMMAND_UNKNOWN"));
        }
        return Ok(Response::builder().status(StatusCode::SERVICE_UNAVAILABLE).header(header::CONTENT_TYPE,"text/html; charset=utf-8").body(Full::new(Bytes::from_static(b"<!doctype html><html><head><meta charset=\"utf-8\"><title>HiRoute</title></head><body><h1>HiRoute</h1><p>CONSOLE_ASSETS_UNAVAILABLE</p><p>Install the complete HiRoute package to open the console.</p></body></html>"))).expect("static response"));
    }
    let endpoint = path
        .strip_prefix(PREFIX)
        .ok_or((StatusCode::NOT_FOUND, "COMMAND_UNKNOWN"))?;
    let public = method == Method::POST && matches!(endpoint, "/session/launch" | "/session/token");
    let identity = if public {
        None
    } else {
        Some(
            server
                .control
                .auth
                .lock()
                .map_err(|_| auth_failure(AuthError::Unavailable))?
                .authenticate(request.headers(), peer.ip())
                .map_err(auth_failure)?,
        )
    };
    if endpoint == "/session" {
        let identity = identity.ok_or(auth_failure(AuthError::Required))?;
        if method == Method::GET {
            return Ok(json_reply(
                StatusCode::OK,
                json!({"session":identity,"transport":if peer.ip().is_loopback(){"loopback"}else{"network_plaintext"},"assets_available":server.assets.is_some(),"capabilities":{"host":"console","platform":std::env::consts::OS,"updates":false,"cli_entry":false,"quit":false,"titlebar":false,"directory_reveal":"path","diagnostic_directory":server.layout.diagnostics_root(),"recovery_directory":server.layout.state_root,"clipboard":"browser","save_file":true,"open_external_url":true}}),
            ));
        }
        if method == Method::DELETE {
            server
                .control
                .auth
                .lock()
                .map_err(|_| auth_failure(AuthError::Unavailable))?
                .logout(request.headers(), peer.ip())
                .map_err(auth_failure)?;
            for context in server.contexts.retire(&server.control.auth, false) {
                tokio::spawn(async move {
                    context.close().await;
                });
            }
            return Ok(json_reply(StatusCode::OK, Value::Null));
        }
        return Err((StatusCode::NOT_FOUND, "COMMAND_UNKNOWN"));
    }
    if method != Method::POST {
        return Err((StatusCode::NOT_FOUND, "COMMAND_UNKNOWN"));
    }
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        != Some("application/json")
    {
        return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "REQUEST_INVALID"));
    }
    let context_id = if public {
        None
    } else {
        let mut contexts = request.headers().get_all("x-hiroute-context").iter();
        let value = contexts
            .next()
            .and_then(|v| v.to_str().ok())
            .filter(|v| valid_context(v))
            .ok_or((StatusCode::BAD_REQUEST, "CLIENT_CONTEXT_INVALID"))?;
        if contexts.next().is_some() {
            return Err((StatusCode::BAD_REQUEST, "CLIENT_CONTEXT_INVALID"));
        }
        Some(value.to_owned())
    };
    let limit = hiroute_application_api::LOCAL_CONTROL_MAX_FRAME_BYTES;
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > limit as u64)
    {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "REQUEST_TOO_LARGE"));
    }
    let mut body = request.into_body();
    let bytes = tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| (StatusCode::BAD_REQUEST, "REQUEST_INVALID"))?;
            if let Ok(data) = frame.into_data() {
                if bytes.len() + data.len() > limit {
                    return Err((StatusCode::PAYLOAD_TOO_LARGE, "REQUEST_TOO_LARGE"));
                }
                bytes.extend_from_slice(&data);
            }
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| (StatusCode::REQUEST_TIMEOUT, "REQUEST_INVALID"))??;
    if public {
        let result = if endpoint == "/session/launch" {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                code: String,
            }
            let input: Input = serde_json::from_slice(&bytes)
                .map_err(|_| (StatusCode::BAD_REQUEST, "REQUEST_INVALID"))?;
            let code = Zeroizing::new(input.code);
            server
                .control
                .auth
                .lock()
                .map_err(|_| auth_failure(AuthError::Unavailable))?
                .exchange_launch(&code, peer.ip())
        } else {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Input {
                token: String,
            }
            let input: Input = serde_json::from_slice(&bytes)
                .map_err(|_| (StatusCode::BAD_REQUEST, "REQUEST_INVALID"))?;
            let token = Zeroizing::new(input.token);
            server
                .control
                .auth
                .lock()
                .map_err(|_| auth_failure(AuthError::Unavailable))?
                .exchange_token(&token, peer.ip())
        }
        .map_err(auth_failure)?;
        return Ok(json_reply(
            StatusCode::OK,
            serde_json::to_value(result).map_err(|_| auth_failure(AuthError::Unavailable))?,
        ));
    }
    let name = endpoint
        .strip_prefix("/command/")
        .filter(|n| !n.contains('/'))
        .ok_or((StatusCode::NOT_FOUND, "COMMAND_UNKNOWN"))?;
    let command = hiroute_product_session::catalog::product_command(name)
        .ok_or((StatusCode::NOT_FOUND, "COMMAND_UNKNOWN"))?;
    let args: Value =
        serde_json::from_slice(&bytes).map_err(|_| (StatusCode::BAD_REQUEST, "REQUEST_INVALID"))?;
    let identity = identity.ok_or(auth_failure(AuthError::Required))?;
    let context_id = context_id.ok_or((StatusCode::BAD_REQUEST, "CLIENT_CONTEXT_INVALID"))?;
    for context in server.contexts.retire(&server.control.auth, false) {
        tokio::spawn(async move {
            context.close().await;
        });
    }
    let context = server
        .contexts
        .acquire(
            &identity.id,
            &context_id,
            server.client.clone(),
            server.authority.clone(),
            server.diagnostics.clone(),
        )
        .map_err(auth_failure)?;
    let audited = audited(&command.name);
    if audited {
        server
            .control
            .auth
            .lock()
            .map_err(|_| auth_failure(AuthError::Unavailable))?
            .audit_command(&identity, &context_id, name, None)
            .map_err(auth_failure)?;
    }
    let result = if super::host_settings::is_host_product(name) {
        super::host_settings::command(server, name, args).await
    } else {
        context.command(name, args, &server.control.auth).await
    };
    match result {
        Ok(value) => {
            if audited && let Some(operation) = operation_id(&value) {
                server
                    .control
                    .auth
                    .lock()
                    .map_err(|_| auth_failure(AuthError::Unavailable))?
                    .audit_command(&identity, &context_id, name, Some(operation))
                    .map_err(auth_failure)?;
            }
            Ok(json_reply(StatusCode::OK, value))
        }
        Err(error) => Ok(json_reply(
            StatusCode::BAD_REQUEST,
            serde_json::to_value(error)
                .unwrap_or_else(|_| json!({"kind":"native","code":"RESPONSE_DATA_INVALID"})),
        )),
    }
}
fn operation_id(value: &Value) -> Option<&str> {
    value
        .get("operation_id")
        .or_else(|| value.get("operation").and_then(|v| v.get("operation_id")))
        .or_else(|| {
            value
                .get("approval_operation")
                .and_then(|v| v.get("operation_id"))
        })
        .or_else(|| {
            value
                .get("mutation")
                .and_then(|v| v.get("operation"))
                .and_then(|v| v.get("operation_id"))
        })
        .and_then(|v| v.as_str())
}
fn audited(name: &str) -> bool {
    !matches!(
        name,
        "desktop_snapshot"
            | "agent_snapshot"
            | "decision_services"
            | "compute_snapshot"
            | "compute_key_masks"
            | "compute_scan"
            | "compute_subscriptions"
            | "compute_connection_options"
            | "get_subscription_check_result"
            | "get_model_connection_check"
            | "get_compute_save_result"
            | "model_reference_query"
            | "effective_price_query"
            | "plan_editor_options"
            | "worker_settings_get"
            | "worker_executor_availability"
            | "worker_plans"
            | "worker_dependencies_discover"
            | "worker_list"
            | "worker_status"
            | "worker_result"
            | "worker_read"
            | "worker_wait"
            | "observe_operation"
            | "web_confirmation_snapshot"
            | "gateway_listener_status"
            | "subscription_proxy_status"
            | "diagnostic_status"
    )
}
fn auth_failure(error: AuthError) -> (StatusCode, &'static str) {
    (
        match error {
            AuthError::Required | AuthError::Invalid => StatusCode::UNAUTHORIZED,
            AuthError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            AuthError::Capacity => StatusCode::CONFLICT,
            AuthError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        },
        error.code(),
    )
}
fn failure(status: StatusCode, code: &str) -> Reply {
    json_reply(
        status,
        serde_json::to_value(hiroute_product_session::failure::DesktopFailure::from(
            code.to_owned(),
        ))
        .unwrap_or(Value::Null),
    )
}
fn json_reply(status: StatusCode, value: Value) -> Reply {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(
            serde_json::to_vec(&value).unwrap_or_default(),
        )))
        .expect("fixed response")
}
fn headers(mut response: Reply) -> Reply {
    for (name, value) in [
        ("content-security-policy", CSP),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("cache-control", "no-store"),
    ] {
        response.headers_mut().insert(
            http::header::HeaderName::from_static(name),
            http::HeaderValue::from_static(value),
        );
    }
    response
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_origin_and_fetch_site_are_port_exact() {
        let local = "127.0.0.1:5838".parse().unwrap();
        let mut h = http::HeaderMap::new();
        h.insert(header::HOST, "127.0.0.1:5838".parse().unwrap());
        assert!(valid_origin(&h, local));
        h.insert(header::ORIGIN, "http://127.0.0.1:5839".parse().unwrap());
        assert!(!valid_origin(&h, local));
        h.insert(header::ORIGIN, "http://127.0.0.1:5838".parse().unwrap());
        assert!(valid_origin(&h, local));
        h.insert("sec-fetch-site", "same-site".parse().unwrap());
        assert!(!valid_origin(&h, local));
        h.remove("sec-fetch-site");
        h.insert(header::HOST, "evil.example:5838".parse().unwrap());
        assert!(!valid_origin(&h, local));
    }
}
