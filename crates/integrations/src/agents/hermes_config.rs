//! One owned Hermes default/provider transition, with private conditional restore state.
use super::hermes_yaml::{HermesDocument, HermesNativeError, at, hermes_error};
use hiroute_domain::{AdditionalAgentModelV1, CanonicalDigest};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

pub const HERMES_PLUGIN_NAME: &str = "hiroute";
const COMPACT_PROVIDER_PREFIX: &str = "hiroute-main-";

// The native file already scopes this identity to one effective Hermes profile.
// Full context/grant authority stays in the sealed effect and protected restore record.
fn native_provider_id(alias: &str) -> String {
    let digest = CanonicalDigest::of_bytes(alias.as_bytes());
    format!("{COMPACT_PROVIDER_PREFIX}{}", &digest.as_str()[7..19])
}

fn compact_provider_key(key: &str) -> bool {
    key.strip_prefix("providers/hiroute-main-")
        .is_some_and(|suffix| {
            suffix.len() == 12
                && suffix
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
}
const SELECTORS: &[&str] = &[
    "default",
    "provider",
    "context_length",
    "base_url",
    "api_mode",
    "transport",
    "api_key",
    "api_key_env",
    "key_env",
    "key_cmd",
    "model",
    "url",
    "api",
];

/// This record contains native credentials; it belongs only in the protected restore port.
#[derive(Deserialize, Serialize)]
pub struct HermesConfigRestoreV1 {
    schema: String,
    provider_id: String,
    original: Vec<u8>,
    managed_file_digest: CanonicalDigest,
    before: BTreeMap<String, Option<Value>>,
    after: BTreeMap<String, Option<Value>>,
    original_enabled: bool,
}
impl Drop for HermesConfigRestoreV1 {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.original.zeroize();
        self.before
            .values_mut()
            .chain(self.after.values_mut())
            .filter_map(Option::as_mut)
            .for_each(super::pi_sources::clear_native_value);
    }
}

pub fn hermes_configure(
    current: &[u8],
    previous: Option<&HermesConfigRestoreV1>,
    provider_id: &str,
    endpoint: &str,
    model: &AdditionalAgentModelV1,
    token: &str,
) -> Result<(Zeroizing<Vec<u8>>, HermesConfigRestoreV1), HermesNativeError> {
    model
        .validate_pi()
        .map_err(|_| hermes_error("Plan budget"))?;
    if !provider_id.starts_with("hiroute-main-") || token.is_empty() {
        return Err(hermes_error("owned provider"));
    }
    let doc = HermesDocument::parse(current)?;
    if doc
        .value
        .pointer("/plugins/disabled")
        .and_then(Value::as_array)
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|v| v.as_str() == Some(HERMES_PLUGIN_NAME))
        })
    {
        return Err(hermes_error("native plugin explicitly disabled"));
    }
    let concrete_provider = native_provider_id(&model.alias);
    if let Some(previous) = previous
        && (previous.provider_id != provider_id || !previous.applied(current)?)
    {
        return Err(hermes_error("owned configuration changed"));
    }
    if doc
        .value
        .get("providers")
        .and_then(|p| p.get(&concrete_provider))
        .is_some()
        && previous.is_none_or(|p| {
            !p.after
                .get(&format!("providers/{concrete_provider}"))
                .is_some_and(Option::is_some)
                || p.after.get("model/default").and_then(Option::as_ref)
                    != Some(&json!(model.alias))
        })
        || previous.is_none() && doc.value.pointer("/plugins/entries/hiroute").is_some()
    {
        return Err(hermes_error("foreign provider or plugin entry"));
    }
    for key in ["model", "providers", "plugins"] {
        if doc.value.get(key).is_some_and(|v| !v.is_object()) {
            return Err(hermes_error("native mapping shape"));
        }
    }
    let mode = match model.protocol {
        hiroute_domain::AgentIngressProtocolV1::Responses => "codex_responses",
        hiroute_domain::AgentIngressProtocolV1::Messages => "anthropic_messages",
        hiroute_domain::AgentIngressProtocolV1::ChatCompletions => "chat_completions",
    };
    let base = if model.protocol == hiroute_domain::AgentIngressProtocolV1::Messages {
        endpoint
            .strip_suffix("/v1")
            .ok_or_else(|| hermes_error("Messages base"))?
    } else {
        endpoint
    };
    let mut changes = BTreeMap::new();
    for key in SELECTORS {
        changes.insert(vec!["model".into(), (*key).into()], None);
    }
    changes.insert(
        vec!["model".into(), "default".into()],
        Some(json!(model.alias)),
    );
    changes.insert(
        vec!["model".into(), "provider".into()],
        Some(json!(concrete_provider)),
    );
    changes.insert(
        vec!["model".into(), "context_length".into()],
        Some(json!(model.context_window_tokens)),
    );
    // Hermes auxiliary Chat calls bypass request middleware and use this
    // provider's standard API key. Reuse the same scoped model grant; global
    // model headers could send it to an unrelated auxiliary provider.
    changes.insert(vec!["providers".into(),concrete_provider.clone()],Some(json!({
        "name":"HiRoute","api":base,"transport":mode,"api_key":token,
        "default_model":model.alias,"models":{&model.alias:{"context_length":model.context_window_tokens}},
        "extra_headers":{"X-HiRoute-Token":token}
    })));
    changes.insert(
        vec![
            "plugins".into(),
            "entries".into(),
            HERMES_PLUGIN_NAME.into(),
        ],
        Some(json!({
            "provider_id":concrete_provider,"model":model.alias,"base_url":base,"api_mode":mode
        })),
    );
    // A provider alias cannot distinguish another user entry selecting the same route.
    if let Some(providers) = doc.value.get("providers").and_then(Value::as_object) {
        for (id, p) in providers {
            if id == &concrete_provider
                || previous.is_some_and(|p| p.after.contains_key(&format!("providers/{id}")))
            {
                continue;
            }
            let address = p
                .get("api")
                .or_else(|| p.get("url"))
                .or_else(|| p.get("base_url"))
                .and_then(Value::as_str);
            if address == Some(base) {
                return Err(hermes_error("route-matching provider ambiguity"));
            }
        }
    }
    let mut enabled = enabled_plugins(&doc.value)?;
    let original_enabled = previous.map_or(enabled.iter().any(|v| v == HERMES_PLUGIN_NAME), |p| {
        p.original_enabled
    });
    if !enabled.iter().any(|v| v == HERMES_PLUGIN_NAME) {
        enabled.push(HERMES_PLUGIN_NAME.into());
    }
    changes.insert(
        vec!["plugins".into(), "enabled".into()],
        Some(json!(enabled)),
    );
    if let Some(previous) = previous {
        // Reconfiguration must remove the prior per-Plan provider only after owned selectors
        // move, and only if no foreign native reference still uses it.
        for key in previous
            .after
            .keys()
            .filter(|k| k.starts_with("providers/"))
        {
            let path: Vec<_> = key.split('/').map(str::to_owned).collect();
            changes.entry(path).or_insert(None);
        }
    }
    let bytes = doc.edit(&changes)?;
    // A later native consumer may reference the previous Plan provider. Never remove
    // that provider while an unowned selector still references it.
    if let Some(previous) = previous {
        let next = HermesDocument::parse(&bytes)?;
        for key in previous
            .after
            .keys()
            .filter_map(|k| k.strip_prefix("providers/"))
        {
            if key != concrete_provider && refers_to(&next.value, key) {
                return Err(hermes_error("remaining native provider reference"));
            }
        }
    }
    let original = previous.map_or_else(|| current.to_vec(), |p| p.original.clone());
    let base_doc = HermesDocument::parse(&original)?;
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    for (path, value) in &changes {
        if path == &["plugins".to_owned(), "enabled".to_owned()] {
            continue;
        }
        before.insert(path.join("/"), at(&base_doc.value, path).cloned());
        after.insert(path.join("/"), value.clone());
    }
    let restore = HermesConfigRestoreV1 {
        schema: "hiroute.hermes-config-restore/v1".into(),
        provider_id: provider_id.into(),
        original,
        managed_file_digest: CanonicalDigest::of_bytes(&bytes),
        before,
        after,
        original_enabled,
    };
    Ok((bytes, restore))
}

fn enabled_plugins(root: &Value) -> Result<Vec<String>, HermesNativeError> {
    match root.pointer("/plugins/enabled") {
        None => Ok(Vec::new()),
        Some(Value::Array(a)) if a.len() <= 256 => a
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| hermes_error("enabled plugin list"))
            })
            .collect(),
        _ => Err(hermes_error("enabled plugin list")),
    }
}

impl HermesConfigRestoreV1 {
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, HermesNativeError> {
        self.validate()?;
        serde_json::to_vec(self)
            .map(Zeroizing::new)
            .map_err(|_| hermes_error("restore encode"))
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, HermesNativeError> {
        let value: Self =
            serde_json::from_slice(bytes).map_err(|_| hermes_error("restore decode"))?;
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), HermesNativeError> {
        if self.schema != "hiroute.hermes-config-restore/v1"
            || !self.provider_id.starts_with("hiroute-main-")
            || self.before.len() != self.after.len()
            || self.before.keys().ne(self.after.keys())
        {
            return Err(hermes_error("restore record"));
        }
        for key in self.after.keys() {
            let compact = compact_provider_key(key)
                && self.before[key].is_none()
                && (self.after[key].is_none()
                    || self
                        .after
                        .get("model/default")
                        .and_then(Option::as_ref)
                        .and_then(Value::as_str)
                        .is_some_and(|alias| {
                            key == &format!("providers/{}", native_provider_id(alias))
                        }));
            if !(key.starts_with("model/") && SELECTORS.contains(&&key[6..]))
                && !key.starts_with(&format!("providers/{}-", self.provider_id))
                && !compact
                && key != "plugins/entries/hiroute"
            {
                return Err(hermes_error("restore ownership"));
            }
        }
        Ok(())
    }
    pub fn applied(&self, current: &[u8]) -> Result<bool, HermesNativeError> {
        self.validate()?;
        let doc = HermesDocument::parse(current)?;
        Ok(self.after.iter().all(|(key, value)| {
            at(
                &doc.value,
                &key.split('/').map(str::to_owned).collect::<Vec<_>>(),
            ) == value.as_ref()
        }) && enabled_plugins(&doc.value)?
            .iter()
            .any(|v| v == HERMES_PLUGIN_NAME))
    }
    pub fn restore(&self, current: &[u8]) -> Result<Zeroizing<Vec<u8>>, HermesNativeError> {
        if !self.applied(current)? {
            return Err(hermes_error("owned configuration changed"));
        }
        if CanonicalDigest::of_bytes(current) == self.managed_file_digest {
            return Ok(Zeroizing::new(self.original.clone()));
        }
        let doc = HermesDocument::parse(current)?;
        let mut changes: BTreeMap<Vec<String>, Option<Value>> = self
            .before
            .iter()
            .map(|(key, value)| (key.split('/').map(str::to_owned).collect(), value.clone()))
            .collect();
        let mut enabled = enabled_plugins(&doc.value)?;
        if !self.original_enabled {
            enabled.retain(|v| v != HERMES_PLUGIN_NAME);
        }
        let original = HermesDocument::parse(&self.original)?;
        changes.insert(
            vec!["plugins".into(), "enabled".into()],
            if enabled.is_empty() && original.value.pointer("/plugins/enabled").is_none() {
                None
            } else {
                Some(json!(enabled))
            },
        );
        let mut next = doc.edit(&changes)?;
        // Removing the final owned member must not leave a YAML null overriding a native
        // default. Only empty containers introduced by this adapter can be released.
        for path in [
            vec!["model".into()],
            vec!["providers".into()],
            vec!["plugins".into(), "entries".into()],
            vec!["plugins".into()],
        ] {
            let restored = HermesDocument::parse(&next)?;
            let value = at(&restored.value, &path);
            let before = at(&original.value, &path);
            if value.is_some_and(|v| v.is_null() || v.as_object().is_some_and(|m| m.is_empty()))
                && before.is_none_or(|v| v.as_object().is_some_and(|m| m.is_empty()))
            {
                next = restored.edit(&BTreeMap::from([(path, before.cloned())]))?;
            }
        }
        let next_doc = HermesDocument::parse(&next)?;
        for key in self
            .after
            .keys()
            .filter_map(|k| k.strip_prefix("providers/"))
        {
            if self.before[&format!("providers/{key}")].is_none() && refers_to(&next_doc.value, key)
            {
                return Err(hermes_error("remaining native provider reference"));
            }
        }
        Ok(next)
    }
}

fn refers_to(value: &Value, id: &str) -> bool {
    match value {
        Value::String(s) => s == id || s.starts_with(&format!("{id}/")),
        Value::Array(a) => a.iter().any(|v| refers_to(v, id)),
        Value::Object(o) => o.values().any(|v| refers_to(v, id)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_model(alias: &str) -> AdditionalAgentModelV1 {
        AdditionalAgentModelV1 {
            alias: alias.into(),
            protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
            context_window_tokens: 64000,
            max_output_tokens: 8192,
        }
    }

    #[test]
    fn prior_long_name_restore_bytes_reconfigure_compactly_and_keep_original_baseline() {
        // Captured with the unchanged 80269c5 production library, not this producer.
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/hermes-config-restore-80269c5.json"
        ))
        .unwrap();
        let original = fixture["original"].as_str().unwrap().as_bytes();
        let legacy = fixture["managed"].as_str().unwrap().as_bytes();
        let record: HermesConfigRestoreV1 =
            serde_json::from_value(fixture["restore"].clone()).unwrap();
        assert!(record.applied(legacy).unwrap());
        assert_eq!(record.restore(legacy).unwrap().as_slice(), original);
        let model = fixture_model("hiroute-fixture");
        let (compact, updated) = hermes_configure(
            legacy,
            Some(&record),
            &record.provider_id,
            "http://127.0.0.1:8317/v1",
            &model,
            "synthetic-local-grant-token",
        )
        .unwrap();
        let doc = HermesDocument::parse(&compact).unwrap();
        let provider = doc.value["model"]["provider"].as_str().unwrap();
        assert_eq!(provider.len(), 25);
        assert_eq!(provider, native_provider_id(&model.alias));
        assert_eq!(doc.value["providers"][provider]["name"], "HiRoute");
        assert_eq!(
            doc.value["providers"][provider]["api_key"],
            "synthetic-local-grant-token"
        );
        assert_eq!(doc.value["providers"].as_object().unwrap().len(), 1);
        let reopened = HermesConfigRestoreV1::decode(&updated.encode().unwrap()).unwrap();
        assert!(reopened.applied(&compact).unwrap());
        assert_eq!(reopened.restore(&compact).unwrap().as_slice(), original);
        let (again, next) = hermes_configure(
            &compact,
            Some(&reopened),
            &record.provider_id,
            "http://127.0.0.1:8317/v1",
            &fixture_model("hiroute-second"),
            "synthetic-local-grant-token",
        )
        .unwrap();
        assert_ne!(
            HermesDocument::parse(&again).unwrap().value["model"]["provider"],
            provider
        );
        assert_eq!(next.restore(&again).unwrap().as_slice(), original);
    }

    #[test]
    fn chat_auxiliary_key_is_provider_scoped_and_the_complete_before_image_survives() {
        let original = br#"model:
  default: user-model
  provider: user-provider
  default_headers: {X-User-Header: preserve}
providers:
  user-provider: {api: 'https://user.invalid/v1', api_key: unrelated-user-key}
auxiliary:
  title: {provider: user-provider, model: user-title-model}
"#;
        let mut model = fixture_model("hiroute-auxiliary-chat");
        model.protocol = hiroute_domain::AgentIngressProtocolV1::ChatCompletions;
        let (managed, record) = hermes_configure(
            original,
            None,
            "hiroute-main-owner",
            "http://127.0.0.1:8317/v1",
            &model,
            "synthetic-local-grant-token",
        )
        .unwrap();
        let doc = HermesDocument::parse(&managed).unwrap();
        let provider = doc.value["model"]["provider"].as_str().unwrap();
        assert_eq!(
            doc.value["providers"][provider]["api_key"],
            "synthetic-local-grant-token"
        );
        assert_eq!(
            doc.value["providers"][provider]["extra_headers"]["X-HiRoute-Token"],
            "synthetic-local-grant-token"
        );
        assert!(doc.value.pointer("/model/extra_headers").is_none());
        let prior = HermesDocument::parse(original).unwrap();
        for path in [
            "/model/default_headers",
            "/providers/user-provider",
            "/auxiliary",
        ] {
            assert_eq!(doc.value.pointer(path), prior.value.pointer(path));
        }
        let reopened = HermesConfigRestoreV1::decode(&record.encode().unwrap()).unwrap();
        assert_eq!(reopened.restore(&managed).unwrap().as_slice(), original);
    }

    #[test]
    fn compact_name_never_adopts_foreign_destination_or_drops_native_references() {
        let namespace = "hiroute-main-owner";
        let first = fixture_model("hiroute-first");
        let second = fixture_model("hiroute-second");
        let (managed, record) = hermes_configure(
            b"user_note: preserve\n",
            None,
            namespace,
            "http://127.0.0.1:8317/v1",
            &first,
            "synthetic-local-grant-token",
        )
        .unwrap();
        let doc = HermesDocument::parse(&managed).unwrap();
        let foreign = doc
            .edit(&BTreeMap::from([(
                vec!["providers".into(), native_provider_id(&second.alias)],
                Some(json!({"api":"https://foreign.invalid/v1"})),
            )]))
            .unwrap();
        assert!(record.applied(&foreign).unwrap());
        assert!(
            hermes_configure(
                &foreign,
                Some(&record),
                namespace,
                "http://127.0.0.1:8317/v1",
                &second,
                "synthetic-local-grant-token"
            )
            .is_err()
        );
        let referenced = doc
            .edit(&BTreeMap::from([(
                vec!["auxiliary".into(), "provider".into()],
                Some(json!(native_provider_id(&first.alias))),
            )]))
            .unwrap();
        assert!(record.applied(&referenced).unwrap());
        assert!(
            hermes_configure(
                &referenced,
                Some(&record),
                namespace,
                "http://127.0.0.1:8317/v1",
                &second,
                "synthetic-local-grant-token"
            )
            .is_err()
        );
        assert!(record.restore(&referenced).is_err());
    }

    #[test]
    fn selected_transport_uses_native_sdk_base_and_restores_exact_original() {
        use hiroute_domain::AgentIngressProtocolV1::*;
        let original = b"user_note: preserve\nplugins:\n  enabled: [foreign]\n  entries:\n    foreign: {user_setting: retained}\n";
        for (protocol, transport, base) in [
            (Responses, "codex_responses", "http://127.0.0.1:8317/v1"),
            (Messages, "anthropic_messages", "http://127.0.0.1:8317"),
            (
                ChatCompletions,
                "chat_completions",
                "http://127.0.0.1:8317/v1",
            ),
        ] {
            let model = AdditionalAgentModelV1 {
                alias: "hiroute_test".into(),
                protocol,
                context_window_tokens: 64000,
                max_output_tokens: 8192,
            };
            let (managed, record) = hermes_configure(
                original,
                None,
                "hiroute-main-test",
                "http://127.0.0.1:8317/v1",
                &model,
                "local-token",
            )
            .unwrap();
            let doc = HermesDocument::parse(&managed).unwrap();
            let provider = doc.value["model"]["provider"].as_str().unwrap();
            assert_eq!(doc.value["providers"][provider]["transport"], transport);
            assert_eq!(doc.value["providers"][provider]["api"], base);
            assert_eq!(
                doc.value["plugins"]["enabled"],
                json!(["foreign", "hiroute"])
            );
            assert_eq!(
                doc.value["plugins"]["entries"]["hiroute"],
                json!({
                    "provider_id":provider,"model":model.alias,"base_url":base,"api_mode":transport
                })
            );
            assert_eq!(
                doc.value["plugins"]["entries"]["foreign"]["user_setting"],
                "retained"
            );
            assert_eq!(doc.value["user_note"], "preserve");
            let reopened = HermesConfigRestoreV1::decode(&record.encode().unwrap()).unwrap();
            assert!(reopened.applied(&managed).unwrap());
            assert_eq!(reopened.restore(&managed).unwrap().as_slice(), original);
        }
    }

    #[test]
    fn conditional_restore_releases_only_introduced_empty_containers() {
        let model = AdditionalAgentModelV1 {
            alias: "hiroute_test".into(),
            protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
            context_window_tokens: 64000,
            max_output_tokens: 8192,
        };
        for original in [
            b"unrelated: true\n".as_slice(),
            b"model: {}\nproviders: {}\nunrelated: true\n".as_slice(),
        ] {
            let (managed, record) = hermes_configure(
                original,
                None,
                "hiroute-main-test",
                "http://127.0.0.1:8317/v1",
                &model,
                "local-token",
            )
            .unwrap();
            let mut edited = managed.to_vec();
            edited.extend_from_slice(b"later: retained\n");
            let restored = HermesDocument::parse(&record.restore(&edited).unwrap()).unwrap();
            let mut expected = HermesDocument::parse(original).unwrap().value.clone();
            expected["later"] = json!("retained");
            assert_eq!(restored.value, expected);
        }
    }
    #[test]
    fn scoped_default_restore_keeps_foreign_edits_and_plugin_members() {
        let original=b"model:\n  default: glm-5.3\n  provider: glm\n  base_url: https://stale.example\nproviders:\n  glm:\n    api: https://coding.example\nplugins:\n  enabled: [other]\nauxiliary:\n  title: native # unchanged\n";
        let model = AdditionalAgentModelV1 {
            alias: "hiroute_test".into(),
            protocol: hiroute_domain::AgentIngressProtocolV1::Responses,
            context_window_tokens: 64000,
            max_output_tokens: 8192,
        };
        let (managed, record) = hermes_configure(
            original,
            None,
            "hiroute-main-test",
            "http://127.0.0.1:8317/v1",
            &model,
            "local-token",
        )
        .unwrap();
        assert!(record.applied(&managed).unwrap());
        assert_eq!(record.restore(&managed).unwrap().as_slice(), original);
        let mut edited = managed.to_vec();
        edited.extend_from_slice(b"user_note: preserve\n");
        let restored = record.restore(&edited).unwrap();
        let doc = HermesDocument::parse(&restored).unwrap();
        assert_eq!(doc.value["model"]["default"], "glm-5.3");
        assert_eq!(doc.value["user_note"], "preserve");
        assert_eq!(doc.value["plugins"]["enabled"], json!(["other"]));
        let changed = HermesDocument::parse(&managed)
            .unwrap()
            .edit(&BTreeMap::from([(
                vec!["model".into(), "default".into()],
                Some(json!("foreign")),
            )]))
            .unwrap();
        assert!(record.restore(&changed).is_err());
    }
}
