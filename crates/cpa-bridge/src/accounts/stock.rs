use std::collections::BTreeSet;
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{
    AccountDiscoveryError, AccountSnapshotRecord, CpaAccountKind, CpaControlPlane,
    MAX_MODELS_PER_ACCOUNT, ManagedAccountIdentity, validate_opaque_stock_field,
    validate_registered_id,
};
use crate::config::InstanceSecrets;
use crate::http::{LoopbackRequest, percent_encode_query, request};

const MAX_AUTH_FILES: usize = 10_000;
const PIN_POLL_INTERVAL: Duration = Duration::from_millis(20);

mod validation;

use validation::{persisted_controls_match, reject_secret_material, validate_management_response};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StockCpaControlPlane;

impl CpaControlPlane for StockCpaControlPlane {
    fn probe_ready(
        &self,
        address: SocketAddr,
        secrets: &InstanceSecrets,
        expected_version: &str,
        timeout: Duration,
    ) -> Result<(), AccountDiscoveryError> {
        let health = request(LoopbackRequest {
            address,
            method: "GET",
            path: "/healthz",
            authorization: None,
            body: &[],
            timeout,
        })?;
        if health.status != 200 {
            return Err(AccountDiscoveryError::NotReady);
        }
        let management = request(LoopbackRequest {
            address,
            method: "GET",
            path: "/v0/management/auth-files?name=.__hiroute_ready_probe__",
            authorization: Some(&secrets.management),
            body: &[],
            timeout,
        })?;
        validate_management_response(&management, expected_version)?;
        let downstream = request(LoopbackRequest {
            address,
            method: "GET",
            path: "/v1/models",
            authorization: Some(&secrets.downstream),
            body: &[],
            timeout,
        })?;
        if downstream.status != 200 {
            return Err(AccountDiscoveryError::DownstreamAuthentication);
        }
        Ok(())
    }

    fn discover_and_pin(
        &self,
        address: SocketAddr,
        auth_dir: &Path,
        managed_identities: &[ManagedAccountIdentity],
        secrets: &InstanceSecrets,
        expected_version: &str,
        timeout: Duration,
        refresh_models: bool,
    ) -> Result<Vec<AccountSnapshotRecord>, AccountDiscoveryError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or(AccountDiscoveryError::PinNotApplied)?;
        let mut names = fs::read_dir(auth_dir)
            .map_err(AccountDiscoveryError::AuthDirectory)?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                let name = entry.file_name().into_string().ok()?;
                (file_type.is_file() && name.to_ascii_lowercase().ends_with(".json"))
                    .then_some(name)
            })
            .collect::<Vec<_>>();
        names.sort();
        if names.len() > MAX_AUTH_FILES {
            return Err(AccountDiscoveryError::TooManyAccounts);
        }

        let mut managed_names = BTreeSet::new();
        for identity in managed_identities {
            identity.validate()?;
            if !managed_names.insert(identity.stock_file_name.as_str()) {
                return Err(AccountDiscoveryError::InvalidAccount);
            }
        }
        let mut matched_managed = BTreeSet::new();
        let mut snapshots = Vec::new();
        let mut seen_indexes = BTreeSet::new();
        for name in names {
            let Some(account) = get_oauth_account(
                address,
                secrets,
                expected_version,
                remaining_timeout(deadline)?,
                &name,
            )?
            else {
                continue;
            };
            if !seen_indexes.insert(account.auth_index.clone()) {
                return Err(AccountDiscoveryError::DuplicateAccount);
            }
            let managed = managed_identities.iter().find(|identity| {
                identity.account_kind == account.kind && identity.stock_file_name == name
            });
            if account.kind == CpaAccountKind::Codex && managed.is_none() {
                return Err(AccountDiscoveryError::UnexpectedManagedAccount);
            }
            let (digest, prefix, generation) = if let Some(identity) = managed {
                matched_managed.insert(identity.stock_file_name.as_str());
                (
                    identity.account_digest.clone(),
                    identity.prefix()?,
                    identity.generation,
                )
            } else {
                let digest = account_digest(account.kind, &account.auth_index);
                let prefix = format!("hiroute-{}", &digest[..24]);
                (digest, prefix, 1)
            };
            // PATCH also synchronously re-registers the account in CPA, including
            // remote model discovery. Ordinary compilation/preview is a read;
            // only repair changed controls or honor an explicit subscription check.
            if refresh_models
                || account.request_retry != Some(0)
                || !persisted_controls_match(auth_dir, &name, &prefix)?
            {
                patch_account_controls(
                    address,
                    secrets,
                    expected_version,
                    remaining_timeout(deadline)?,
                    &account.id,
                    &prefix,
                    managed,
                )?;
            }
            let models = wait_for_account_pin(
                address,
                auth_dir,
                secrets,
                expected_version,
                deadline,
                &name,
                &account,
                &prefix,
            )?;
            snapshots.push(AccountSnapshotRecord {
                account_kind: account.kind,
                stock_id: account.id,
                stock_auth_index: account.auth_index,
                stock_file_name: name,
                prefix,
                account_digest: digest,
                generation,
                observed_model_ids: models,
                active: true,
            });
        }
        if matched_managed.len() != managed_identities.len() {
            return Err(AccountDiscoveryError::AccountDisappeared);
        }
        wait_for_prefixed_catalog(address, secrets, deadline, &snapshots)?;
        snapshots.sort_by(|left, right| left.account_digest.cmp(&right.account_digest));
        Ok(snapshots)
    }
}

struct StockAccount {
    kind: CpaAccountKind,
    id: String,
    auth_index: String,
    request_retry: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
fn wait_for_account_pin(
    address: SocketAddr,
    auth_dir: &Path,
    secrets: &InstanceSecrets,
    expected_version: &str,
    deadline: Instant,
    name: &str,
    account: &StockAccount,
    prefix: &str,
) -> Result<BTreeSet<String>, AccountDiscoveryError> {
    loop {
        let patched = get_oauth_account(
            address,
            secrets,
            expected_version,
            remaining_timeout(deadline)?,
            name,
        )?
        .ok_or(AccountDiscoveryError::AccountDisappeared)?;
        if patched.auth_index == account.auth_index && patched.request_retry == Some(0) {
            let registered = get_account_models(
                address,
                secrets,
                expected_version,
                remaining_timeout(deadline)?,
                &account.id,
            )?;
            if let Ok(models) = strip_exact_prefix(&registered, prefix)
                && persisted_controls_match(auth_dir, name, prefix)?
            {
                return Ok(models);
            }
        }
        let remaining = remaining_timeout(deadline)?;
        thread::sleep(PIN_POLL_INTERVAL.min(remaining));
    }
}

fn wait_for_prefixed_catalog(
    address: SocketAddr,
    secrets: &InstanceSecrets,
    deadline: Instant,
    snapshots: &[AccountSnapshotRecord],
) -> Result<(), AccountDiscoveryError> {
    loop {
        match validate_prefixed_catalog(address, secrets, remaining_timeout(deadline)?, snapshots) {
            Ok(()) => return Ok(()),
            Err(AccountDiscoveryError::PinNotApplied) => {
                let remaining = remaining_timeout(deadline)?;
                thread::sleep(PIN_POLL_INTERVAL.min(remaining));
            }
            Err(error) => return Err(error),
        }
    }
}

fn remaining_timeout(deadline: Instant) -> Result<Duration, AccountDiscoveryError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(AccountDiscoveryError::PinNotApplied);
    }
    Ok(remaining)
}

fn strip_exact_prefix(
    registered: &BTreeSet<String>,
    prefix: &str,
) -> Result<BTreeSet<String>, AccountDiscoveryError> {
    if registered.is_empty() {
        return Err(AccountDiscoveryError::PinNotApplied);
    }
    let marker = format!("{prefix}/");
    let mut models = BTreeSet::new();
    for transport_model in registered {
        let model = transport_model
            .strip_prefix(&marker)
            .filter(|model| !model.is_empty())
            .ok_or(AccountDiscoveryError::PinNotApplied)?;
        validate_registered_id(model)?;
        if !models.insert(model.to_owned()) {
            return Err(AccountDiscoveryError::DuplicateModel);
        }
    }
    Ok(models)
}

fn get_oauth_account(
    address: SocketAddr,
    secrets: &InstanceSecrets,
    expected_version: &str,
    timeout: Duration,
    name: &str,
) -> Result<Option<StockAccount>, AccountDiscoveryError> {
    validate_opaque_stock_field(name)?;
    let path = format!(
        "/v0/management/auth-files?name={}",
        percent_encode_query(name)
    );
    let response = request(LoopbackRequest {
        address,
        method: "GET",
        path: &path,
        authorization: Some(&secrets.management),
        body: &[],
        timeout,
    })?;
    validate_management_response(&response, expected_version)?;
    parse_auth_file_response(&response.body, name)
}

fn parse_auth_file_response(
    bytes: &[u8],
    expected_name: &str,
) -> Result<Option<StockAccount>, AccountDiscoveryError> {
    let value: Value = serde_json::from_slice(bytes).map_err(AccountDiscoveryError::Json)?;
    reject_secret_material(&value)?;
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .ok_or(AccountDiscoveryError::InvalidResponse)?;
    if files.is_empty() {
        return Ok(None);
    }
    if files.len() != 1 {
        return Err(AccountDiscoveryError::DuplicateAccount);
    }
    let entry = files[0]
        .as_object()
        .ok_or(AccountDiscoveryError::InvalidResponse)?;
    let name = required_string(entry, "name")?;
    if name != expected_name
        || entry.get("runtime_only").and_then(Value::as_bool) != Some(false)
        || entry.get("source").and_then(Value::as_str) != Some("file")
        || entry.get("account_type").and_then(Value::as_str) != Some("oauth")
    {
        return Err(AccountDiscoveryError::NonSubscriptionAccount);
    }
    if entry.get("disabled").and_then(Value::as_bool) == Some(true)
        || entry.get("unavailable").and_then(Value::as_bool) == Some(true)
        || !entry
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|status| status.eq_ignore_ascii_case("active"))
    {
        return Ok(None);
    }
    let provider = required_string(entry, "provider")?;
    let kind = match provider.to_ascii_lowercase().as_str() {
        "codex" => CpaAccountKind::Codex,
        "claude" => CpaAccountKind::Claude,
        _ => return Ok(None),
    };
    let id = required_string(entry, "id")?.to_owned();
    let auth_index = required_string(entry, "auth_index")?.to_owned();
    validate_opaque_stock_field(&id)?;
    validate_opaque_stock_field(&auth_index)?;
    let request_retry = entry.get("request_retry").and_then(Value::as_i64);
    Ok(Some(StockAccount {
        kind,
        id,
        auth_index,
        request_retry,
    }))
}

#[allow(clippy::too_many_arguments)]
fn patch_account_controls(
    address: SocketAddr,
    secrets: &InstanceSecrets,
    expected_version: &str,
    timeout: Duration,
    id: &str,
    prefix: &str,
    managed: Option<&ManagedAccountIdentity>,
) -> Result<(), AccountDiscoveryError> {
    let body = account_controls_body(id, prefix, managed)?;
    let response = request(LoopbackRequest {
        address,
        method: "PATCH",
        path: "/v0/management/auth-files/fields",
        authorization: Some(&secrets.management),
        body: &body,
        timeout,
    })?;
    validate_management_response(&response, expected_version)
}

fn account_controls_body(
    id: &str,
    prefix: &str,
    managed: Option<&ManagedAccountIdentity>,
) -> Result<Vec<u8>, AccountDiscoveryError> {
    let mut body = serde_json::json!({
        "name": id,
        "prefix": prefix,
        "request_retry": 0,
        "disable_cooling": true
    });
    if let Some(identity) =
        managed.filter(|identity| identity.account_kind == CpaAccountKind::Codex)
    {
        identity.validate()?;
        // CPA merges this PATCH into its loaded Auth, persists it, then invokes
        // synchronous discovery. Its watcher may still hold the previous file.
        // Carry the validated lease's current version so stale metadata cannot
        // overwrite it. No token material enters this control request.
        body["hiroute_client_version"] = serde_json::json!(identity.client_version);
    }
    serde_json::to_vec(&body).map_err(AccountDiscoveryError::Json)
}

fn get_account_models(
    address: SocketAddr,
    secrets: &InstanceSecrets,
    expected_version: &str,
    timeout: Duration,
    id: &str,
) -> Result<BTreeSet<String>, AccountDiscoveryError> {
    let path = format!(
        "/v0/management/auth-files/models?name={}",
        percent_encode_query(id)
    );
    let response = request(LoopbackRequest {
        address,
        method: "GET",
        path: &path,
        authorization: Some(&secrets.management),
        body: &[],
        timeout,
    })?;
    validate_management_response(&response, expected_version)?;
    let value: Value =
        serde_json::from_slice(&response.body).map_err(AccountDiscoveryError::Json)?;
    reject_secret_material(&value)?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or(AccountDiscoveryError::InvalidResponse)?;
    if models.len() > MAX_MODELS_PER_ACCOUNT {
        return Err(AccountDiscoveryError::TooManyModels);
    }
    let mut result = BTreeSet::new();
    for model in models {
        let id = model
            .get("id")
            .and_then(Value::as_str)
            .ok_or(AccountDiscoveryError::InvalidResponse)?;
        validate_transport_model_id(id)?;
        if !result.insert(id.to_owned()) {
            return Err(AccountDiscoveryError::DuplicateModel);
        }
    }
    Ok(result)
}

fn validate_prefixed_catalog(
    address: SocketAddr,
    secrets: &InstanceSecrets,
    timeout: Duration,
    snapshots: &[AccountSnapshotRecord],
) -> Result<(), AccountDiscoveryError> {
    let response = request(LoopbackRequest {
        address,
        method: "GET",
        path: "/v1/models",
        authorization: Some(&secrets.downstream),
        body: &[],
        timeout,
    })?;
    if response.status != 200 {
        return Err(AccountDiscoveryError::DownstreamAuthentication);
    }
    let value: Value =
        serde_json::from_slice(&response.body).map_err(AccountDiscoveryError::Json)?;
    reject_secret_material(&value)?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or(AccountDiscoveryError::InvalidResponse)?;
    let available = data
        .iter()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    for snapshot in snapshots {
        for model in &snapshot.observed_model_ids {
            if !available.contains(format!("{}/{model}", snapshot.prefix).as_str()) {
                return Err(AccountDiscoveryError::PinNotApplied);
            }
        }
    }
    Ok(())
}

fn account_digest(kind: CpaAccountKind, auth_index: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"hiroute.cpa-account/v1\0");
    hasher.update(kind.stock_provider().as_bytes());
    hasher.update(b"\0");
    hasher.update(auth_index.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn required_string<'a>(
    entry: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, AccountDiscoveryError> {
    entry
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(AccountDiscoveryError::InvalidResponse)
}

fn validate_transport_model_id(value: &str) -> Result<(), AccountDiscoveryError> {
    if value.is_empty()
        || value.len() > 512
        || value.contains("//")
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(AccountDiscoveryError::InvalidAccount);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn oauth_response(extra: &str) -> Vec<u8> {
        format!(
            r#"{{"files":[{{"id":"codex-a.json","auth_index":"idx-a","name":"codex-a.json","type":"codex","provider":"codex","status":"active","disabled":false,"unavailable":false,"runtime_only":false,"source":"file","account_type":"oauth","request_retry":0{extra}}}]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn parses_only_file_backed_oauth_without_exposing_account_label() {
        let account = parse_auth_file_response(
            &oauth_response(",\"account\":\"person@example.test\""),
            "codex-a.json",
        )
        .unwrap()
        .unwrap();
        assert_eq!(account.kind, CpaAccountKind::Codex);
        assert_eq!(account.auth_index, "idx-a");
        let digest = account_digest(account.kind, &account.auth_index);
        assert!(!digest.contains("person"));
    }

    #[test]
    fn rejects_any_raw_token_field_and_native_api_key_account() {
        let mut raw: Value = serde_json::from_slice(&oauth_response("")).unwrap();
        raw["files"][0][["access", "token"].join("_")] = serde_json::json!("forbidden");
        let raw = serde_json::to_vec(&raw).unwrap();
        assert!(matches!(
            parse_auth_file_response(&raw, "codex-a.json"),
            Err(AccountDiscoveryError::SecretBearingResponse)
        ));
        let api_key = String::from_utf8(oauth_response(",\"account\":\"native-material\""))
            .unwrap()
            .replace("\"account_type\":\"oauth\"", "\"account_type\":\"api_key\"");
        assert!(matches!(
            parse_auth_file_response(api_key.as_bytes(), "codex-a.json"),
            Err(AccountDiscoveryError::SecretBearingResponse)
        ));
    }

    #[test]
    fn accepts_only_v7_2_140_codex_id_token_claim_projection() {
        let allowed = oauth_response(
            ",\"id_token\":{\"chatgpt_account_id\":\"acct-fixture\",\"plan_type\":\"plus\",\"chatgpt_subscription_active_start\":1,\"chatgpt_subscription_active_until\":2}",
        );
        assert!(
            parse_auth_file_response(&allowed, "codex-a.json")
                .unwrap()
                .is_some()
        );
        for extra in [
            ",\"id_token\":\"raw-jwt-is-forbidden\"",
            ",\"id_token\":{\"email\":\"person@example.test\"}",
            ",\"id_token\":{\"chatgpt_subscription_active_until\":{\"token\":\"nested\"}}",
        ] {
            assert!(matches!(
                parse_auth_file_response(&oauth_response(extra), "codex-a.json"),
                Err(AccountDiscoveryError::SecretBearingResponse)
            ));
        }
    }

    #[test]
    fn exact_control_patch_and_prefix_stripping_are_fail_closed() {
        let prefix = format!("hiroute-{}", "a".repeat(24));
        let body: Value =
            serde_json::from_slice(&account_controls_body("stock-a", &prefix, None).unwrap())
                .unwrap();
        assert_eq!(body["request_retry"], 0);
        assert_eq!(body["disable_cooling"], true);
        let models =
            strip_exact_prefix(&BTreeSet::from([format!("{prefix}/codex-model")]), &prefix)
                .unwrap();
        assert_eq!(models, BTreeSet::from(["codex-model".into()]));
        assert!(strip_exact_prefix(&BTreeSet::from(["codex-model".into()]), &prefix).is_err());
    }
}

#[cfg(test)]
mod pin_tests;

#[cfg(all(test, unix))]
mod version_sync_tests;
