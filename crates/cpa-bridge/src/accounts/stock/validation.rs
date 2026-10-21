use std::fs;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use zeroize::Zeroizing;

use crate::accounts::AccountDiscoveryError;
use crate::config::validate_private_file;

const MAX_AUTH_CONTROL_FILE_BYTES: u64 = 256 * 1024;

#[derive(Deserialize)]
struct PersistedControls<'a> {
    #[serde(borrow)]
    prefix: Option<&'a str>,
    request_retry: Option<i64>,
    disable_cooling: Option<bool>,
}

pub(super) fn observe_persisted_controls(
    auth_dir: &Path,
    name: &str,
    prefix: &str,
    checks: &mut hiroute_diagnostics::event::CpaPinChecks,
) -> Result<bool, AccountDiscoveryError> {
    if Path::new(name).file_name().and_then(|value| value.to_str()) != Some(name) {
        return Err(AccountDiscoveryError::InvalidAccount);
    }
    let path = auth_dir.join(name);
    validate_private_file(&path).map_err(|error| {
        AccountDiscoveryError::AuthDirectory(std::io::Error::other(error.to_string()))
    })?;
    let file = fs::File::open(&path).map_err(AccountDiscoveryError::AuthDirectory)?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_AUTH_CONTROL_FILE_BYTES + 1)
        .read_to_end(bytes.as_mut())
        .map_err(AccountDiscoveryError::AuthDirectory)?;
    if bytes.len() as u64 > MAX_AUTH_CONTROL_FILE_BYTES {
        return Err(AccountDiscoveryError::InvalidAccount);
    }
    let controls: PersistedControls<'_> =
        serde_json::from_slice(bytes.as_slice()).map_err(AccountDiscoveryError::Json)?;
    checks.persisted_prefix_matches = Some(controls.prefix == Some(prefix));
    checks.persisted_retry_zero = Some(controls.request_retry == Some(0));
    checks.persisted_cooling_disabled = Some(controls.disable_cooling == Some(true));
    Ok(checks.persisted_prefix_matches == Some(true)
        && checks.persisted_retry_zero == Some(true)
        && checks.persisted_cooling_disabled == Some(true))
}

pub(super) fn validate_management_response(
    response: &crate::http::LoopbackResponse,
    expected_version: &str,
) -> Result<(), AccountDiscoveryError> {
    if response.status != 200 {
        return Err(AccountDiscoveryError::ManagementAuthentication);
    }
    let actual = response
        .header("x-cpa-version")
        .ok_or(AccountDiscoveryError::MissingVersionHeader)?
        .trim()
        .strip_prefix('v')
        .unwrap_or_else(|| {
            response
                .header("x-cpa-version")
                .expect("header was present")
                .trim()
        });
    if actual != expected_version.trim_start_matches('v') {
        return Err(AccountDiscoveryError::RunningVersionMismatch);
    }
    let value: Value =
        serde_json::from_slice(&response.body).map_err(AccountDiscoveryError::Json)?;
    reject_secret_material(&value)
}

pub(super) fn reject_secret_material(value: &Value) -> Result<(), AccountDiscoveryError> {
    match value {
        Value::Object(object) => {
            if object.get("account_type").and_then(Value::as_str) == Some("api_key")
                && object
                    .get("account")
                    .is_some_and(|account| !account.is_null())
            {
                return Err(AccountDiscoveryError::SecretBearingResponse);
            }
            for (key, child) in object {
                let normalized = key.to_ascii_lowercase().replace('-', "_");
                if normalized == "id_token" {
                    validate_stock_id_token_claims(child)?;
                    continue;
                }
                let secret_key = matches!(
                    normalized.as_str(),
                    "access_token"
                        | "refresh_token"
                        | "api_key"
                        | "authorization"
                        | "cookie"
                        | "client_secret"
                        | "secret"
                ) || normalized == "token";
                if secret_key && !child.is_null() {
                    return Err(AccountDiscoveryError::SecretBearingResponse);
                }
                reject_secret_material(child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                reject_secret_material(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_stock_id_token_claims(value: &Value) -> Result<(), AccountDiscoveryError> {
    let claims = value
        .as_object()
        .filter(|claims| !claims.is_empty())
        .ok_or(AccountDiscoveryError::SecretBearingResponse)?;
    for (key, value) in claims {
        match key.as_str() {
            "chatgpt_account_id" | "plan_type" => {
                let text = value
                    .as_str()
                    .filter(|text| !text.is_empty() && text.len() <= 4_096)
                    .ok_or(AccountDiscoveryError::SecretBearingResponse)?;
                if text.contains(['\r', '\n', '\0']) {
                    return Err(AccountDiscoveryError::SecretBearingResponse);
                }
            }
            "chatgpt_subscription_active_start" | "chatgpt_subscription_active_until" => {
                let safe_scalar = value.is_boolean()
                    || value.is_number()
                    || value.as_str().is_some_and(|text| {
                        !text.is_empty()
                            && text.len() <= 4_096
                            && !text.contains(['\r', '\n', '\0'])
                    });
                if !safe_scalar {
                    return Err(AccountDiscoveryError::SecretBearingResponse);
                }
            }
            _ => return Err(AccountDiscoveryError::SecretBearingResponse),
        }
    }
    Ok(())
}
