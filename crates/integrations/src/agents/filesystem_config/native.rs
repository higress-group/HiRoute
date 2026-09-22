//! Native field rendering and ownership, shared by persistent and isolated file effects.
use super::*;

const CLAUDE_MANAGED_PATHS: [&str; 10] = [
    "apiKeyHelper",
    "hiroute.auth_environment",
    "env.ANTHROPIC_BASE_URL",
    "env.ANTHROPIC_MODEL",
    "env.ANTHROPIC_DEFAULT_OPUS_MODEL",
    "env.ANTHROPIC_DEFAULT_SONNET_MODEL",
    "env.ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "env.ANTHROPIC_SMALL_FAST_MODEL",
    "env.CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "env.CLAUDE_CODE_MAX_CONTEXT_TOKENS",
];

/// Renders one exact semantic change into Claude Code's user settings while preserving every
/// unowned field. Prior auth bytes and helper commands stay exclusively in the returned protected
/// byte buffer; callers must place the original file in an encrypted restore store before Apply.
pub(in crate::agents) fn render_claude_user_change(
    path: &Path,
    change: &AgentConfigChangeV1,
) -> Result<Zeroizing<Vec<u8>>, AgentFilesystemScanError> {
    let bytes = read_validated_config_bytes(path)?
        .map(|(bytes, _)| bytes)
        .unwrap_or_else(|| Zeroizing::new(b"{}".to_vec()));
    render_claude_change_bytes(&bytes, change)
}

pub(in crate::agents) fn render_claude_change_bytes(
    bytes: &[u8],
    change: &AgentConfigChangeV1,
) -> Result<Zeroizing<Vec<u8>>, AgentFilesystemScanError> {
    validate_native_bytes(bytes)?;
    validate_change(change)?;
    let mut root: Value =
        serde_json::from_slice(bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    if !root.is_object() {
        return Err(AgentFilesystemScanError::InvalidConfig);
    }

    for field in &change.fields {
        let before = claude_semantic_value(&root, &field.path)?;
        let before_digest = CanonicalDigest::of(&before)
            .map_err(|_| AgentFilesystemScanError::InvalidDescriptor)?;
        if before_digest != field.before_digest || before != field.before {
            return Err(AgentFilesystemScanError::SourceChanged);
        }
        apply_claude_semantic_value(&mut root, &field.path, field.after.as_ref())?;
    }
    if change
        .fields
        .iter()
        .any(|field| field.path == "apiKeyHelper" && field.after.is_some())
        && root
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| {
                CLAUDE_AUTH_ENVIRONMENT_FIELDS
                    .iter()
                    .any(|name| env.contains_key(*name))
            })
    {
        // A managed helper alongside a leftover user token creates Claude's dual-auth warning
        // and can silently select the wrong credential. Never stage that native file.
        return Err(AgentFilesystemScanError::InvalidDescriptor);
    }
    let mut rendered = Zeroizing::new(
        serde_json::to_vec_pretty(&root).map_err(|_| AgentFilesystemScanError::InvalidConfig)?,
    );
    rendered.push(b'\n');
    if rendered.len() as u64 > MAX_CONFIG_BYTES {
        return Err(AgentFilesystemScanError::ConfigTooLarge);
    }
    Ok(rendered)
}

/// Compose a change planned against the currently managed file with the preceding protected
/// restore base. The returned change owns the complete base→next transition, so restoring the
/// next Operation cannot expose an older helper or leave an older managed endpoint behind.
pub(in crate::agents) fn rebase_claude_change_bytes(
    current: &[u8],
    base: &[u8],
    previous_change: &AgentConfigChangeV1,
    change: &AgentConfigChangeV1,
) -> Result<(Zeroizing<Vec<u8>>, AgentConfigChangeV1), AgentFilesystemScanError> {
    validate_change(previous_change)?;
    if !claude_change_bytes_are_applied(current, previous_change)? {
        return Err(AgentFilesystemScanError::SourceChanged);
    }
    // Check the accepted before-values before constructing the release transition.
    render_claude_change_bytes(current, change)?;
    let base = claude_document_from_bytes(base)?;
    let desired_change: BTreeMap<_, _> =
        change
            .fields
            .iter()
            .map(|field| {
                let after = if field.after.is_none()
                    && field.path.strip_prefix("env.").is_some_and(|key| {
                        hiroute_domain::CLAUDE_CONTEXT_ENVIRONMENT.contains(&key)
                    }) {
                    base.fields.get(&field.path).cloned()
                } else {
                    field.after.clone()
                };
                (field.path.clone(), after)
            })
            .collect();
    let adjusted = AgentConfigChangeV1::preview(
        &claude_document_from_bytes(current)?,
        desired_change.clone(),
    )
    .map_err(|_| AgentFilesystemScanError::InvalidDescriptor)?;
    let rendered = render_claude_change_bytes(current, &adjusted)?;
    let desired = CLAUDE_MANAGED_PATHS
        .into_iter()
        .map(|path| {
            let after = if let Some(after) = desired_change.get(path) {
                after.clone()
            } else if let Some(field) = previous_change
                .fields
                .iter()
                .find(|field| field.path == path)
            {
                field.after.clone()
            } else {
                base.fields.get(path).cloned()
            };
            (path.to_owned(), after)
        })
        .collect();
    let rebased = AgentConfigChangeV1::preview(&base, desired)
        .map_err(|_| AgentFilesystemScanError::InvalidDescriptor)?;
    Ok((rendered, rebased))
}

fn claude_document_from_bytes(
    bytes: &[u8],
) -> Result<AgentConfigDocumentV1, AgentFilesystemScanError> {
    validate_native_bytes(bytes)?;
    let root: Value =
        serde_json::from_slice(bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    if !root.is_object() {
        return Err(AgentFilesystemScanError::InvalidConfig);
    }
    let mut fields = BTreeMap::new();
    for path in CLAUDE_MANAGED_PATHS {
        if let Some(value) = claude_semantic_value(&root, path)? {
            fields.insert(path.to_owned(), value);
        }
    }
    Ok(AgentConfigDocumentV1 { fields })
}

/// Verifies only the fields owned by an applied Claude configuration change. Claude Code may
/// rewrite its settings file (including whitespace and unrelated fields) during startup, so a
/// whole-file byte digest is not a stable ownership signal after activation.
pub(in crate::agents) fn claude_user_change_is_applied(
    path: &Path,
    change: &AgentConfigChangeV1,
) -> Result<bool, AgentFilesystemScanError> {
    validate_change(change)?;
    let Some((bytes, _)) = read_validated_config_bytes(path)? else {
        return Ok(false);
    };
    claude_change_bytes_are_applied(&bytes, change)
}

pub(in crate::agents) fn claude_change_bytes_are_applied(
    bytes: &[u8],
    change: &AgentConfigChangeV1,
) -> Result<bool, AgentFilesystemScanError> {
    validate_native_bytes(bytes)?;
    validate_change(change)?;
    let root: Value =
        serde_json::from_slice(bytes).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    let object = root
        .as_object()
        .ok_or(AgentFilesystemScanError::InvalidConfig)?;

    for field in &change.fields {
        let applied = if field.path == "apiKeyHelper" {
            match field.after.as_ref() {
                Some(after) => {
                    object.get("apiKeyHelper").and_then(Value::as_str)
                        == Some(render_claude_helper_command(after)?.as_str())
                }
                None => !object.contains_key("apiKeyHelper"),
            }
        } else if field.path == "hiroute.auth_environment" {
            if field.after.is_some() {
                return Err(AgentFilesystemScanError::InvalidDescriptor);
            }
            !object
                .get("env")
                .and_then(Value::as_object)
                .is_some_and(|env| {
                    CLAUDE_AUTH_ENVIRONMENT_FIELDS
                        .iter()
                        .any(|name| env.contains_key(*name))
                })
        } else {
            let key = field
                .path
                .strip_prefix("env.")
                .ok_or(AgentFilesystemScanError::InvalidDescriptor)?;
            object
                .get("env")
                .and_then(Value::as_object)
                .and_then(|env| env.get(key))
                == field.after.as_ref()
        };
        if !applied {
            return Ok(false);
        }
    }
    // Absent original auth fields produce no semantic change field. If one is added later,
    // Claude can prefer it over our helper even though every changed field still matches.
    if change
        .fields
        .iter()
        .any(|field| field.path == "apiKeyHelper" && field.after.is_some())
        && object
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| {
                CLAUDE_AUTH_ENVIRONMENT_FIELDS
                    .iter()
                    .any(|name| env.contains_key(*name))
            })
    {
        return Ok(false);
    }
    Ok(true)
}

fn claude_semantic_value(
    root: &Value,
    path: &str,
) -> Result<Option<Value>, AgentFilesystemScanError> {
    let object = root
        .as_object()
        .ok_or(AgentFilesystemScanError::InvalidConfig)?;
    if path == "apiKeyHelper" {
        return Ok(object
            .contains_key("apiKeyHelper")
            .then(|| json!({"configured": true})));
    }
    if path == "hiroute.auth_environment" {
        let present = object
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| {
                CLAUDE_AUTH_ENVIRONMENT_FIELDS
                    .iter()
                    .any(|name| env.contains_key(*name))
            });
        return Ok(present.then(|| json!({"configured": true})));
    }
    let key = path
        .strip_prefix("env.")
        .ok_or(AgentFilesystemScanError::InvalidDescriptor)?;
    Ok(object
        .get("env")
        .and_then(Value::as_object)
        .and_then(|env| env.get(key))
        .cloned())
}

fn apply_claude_semantic_value(
    root: &mut Value,
    path: &str,
    after: Option<&Value>,
) -> Result<(), AgentFilesystemScanError> {
    let object = root
        .as_object_mut()
        .ok_or(AgentFilesystemScanError::InvalidConfig)?;
    if path == "apiKeyHelper" {
        let Some(after) = after else {
            object.remove("apiKeyHelper");
            return Ok(());
        };
        object.insert(
            "apiKeyHelper".to_owned(),
            Value::String(render_claude_helper_command(after)?),
        );
        return Ok(());
    }
    if path == "hiroute.auth_environment" {
        if after.is_some() {
            return Err(AgentFilesystemScanError::InvalidDescriptor);
        }
        if let Some(env) = object.get_mut("env").and_then(Value::as_object_mut) {
            for name in CLAUDE_AUTH_ENVIRONMENT_FIELDS {
                env.remove(name);
            }
        }
        return Ok(());
    }
    let key = path
        .strip_prefix("env.")
        .ok_or(AgentFilesystemScanError::InvalidDescriptor)?;
    if let Some(after) = after {
        if !after.is_string() {
            return Err(AgentFilesystemScanError::InvalidDescriptor);
        }
        let env = object
            .entry("env")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or(AgentFilesystemScanError::InvalidConfig)?;
        env.insert(key.to_owned(), after.clone());
    } else if let Some(env) = object.get_mut("env").and_then(Value::as_object_mut) {
        env.remove(key);
    }
    Ok(())
}

fn render_claude_helper_command(after: &Value) -> Result<String, AgentFilesystemScanError> {
    let executable = after
        .get("executable")
        .and_then(Value::as_str)
        .filter(|value| Path::new(value).is_absolute())
        .ok_or(AgentFilesystemScanError::InvalidDescriptor)?;
    let argv = after
        .get("argv")
        .and_then(Value::as_array)
        .filter(|values| values.len() == 2)
        .ok_or(AgentFilesystemScanError::InvalidDescriptor)?;
    let verb = argv[0]
        .as_str()
        .filter(|value| *value == hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1)
        .ok_or(AgentFilesystemScanError::InvalidDescriptor)?;
    let connection_id = argv[1]
        .as_str()
        .filter(|value| value.starts_with("agent-connection/"))
        .ok_or(AgentFilesystemScanError::InvalidDescriptor)?;
    if after.as_object().is_none_or(|value| value.len() != 2) {
        return Err(AgentFilesystemScanError::InvalidDescriptor);
    }
    Ok([executable, verb, connection_id]
        .into_iter()
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" "))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn validate_native_bytes(bytes: &[u8]) -> Result<(), AgentFilesystemScanError> {
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(AgentFilesystemScanError::ConfigTooLarge);
    }
    // Use the same duplicate-key rejecting parser as discovery before serde_json::Value.
    serde_json::from_slice::<StrictJson>(bytes)
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    serde_json::from_slice::<ClaudeSettingsSubset>(bytes)
        .map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    Ok(())
}

/// Restore only the original values of fields still owned by the applied semantic change.
/// Original helper/token bytes remain in this protected caller-owned buffer, never the change.
pub(in crate::agents) fn restore_claude_change_bytes(
    current: &[u8],
    original: &[u8],
    change: &AgentConfigChangeV1,
) -> Result<Zeroizing<Vec<u8>>, AgentFilesystemScanError> {
    validate_native_bytes(original)?;
    if !claude_change_bytes_are_applied(current, change)? {
        return Err(AgentFilesystemScanError::SourceChanged);
    }
    let mut root: Value =
        serde_json::from_slice(current).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    let before: Value =
        serde_json::from_slice(original).map_err(|_| AgentFilesystemScanError::InvalidConfig)?;
    let object = root
        .as_object_mut()
        .ok_or(AgentFilesystemScanError::InvalidConfig)?;
    let before_object = before
        .as_object()
        .ok_or(AgentFilesystemScanError::InvalidConfig)?;
    for field in &change.fields {
        if field.path == "apiKeyHelper" {
            match before_object.get("apiKeyHelper") {
                Some(value) => {
                    object.insert("apiKeyHelper".into(), value.clone());
                }
                None => {
                    object.remove("apiKeyHelper");
                }
            }
            continue;
        }
        let names: Vec<&str> = if field.path == "hiroute.auth_environment" {
            CLAUDE_AUTH_ENVIRONMENT_FIELDS.to_vec()
        } else {
            vec![
                field
                    .path
                    .strip_prefix("env.")
                    .ok_or(AgentFilesystemScanError::InvalidDescriptor)?,
            ]
        };
        for name in names {
            let previous = before_object
                .get("env")
                .and_then(Value::as_object)
                .and_then(|env| env.get(name));
            if let Some(value) = previous {
                let env = object
                    .entry("env")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                    .ok_or(AgentFilesystemScanError::InvalidConfig)?;
                env.insert(name.into(), value.clone());
            } else if let Some(env) = object.get_mut("env").and_then(Value::as_object_mut) {
                env.remove(name);
            }
        }
    }
    if !before_object.contains_key("env")
        && object
            .get("env")
            .and_then(Value::as_object)
            .is_some_and(|env| env.is_empty())
    {
        object.remove("env");
    }
    let mut result = Zeroizing::new(
        serde_json::to_vec_pretty(&root).map_err(|_| AgentFilesystemScanError::InvalidConfig)?,
    );
    result.push(b'\n');
    if result.len() as u64 > MAX_CONFIG_BYTES {
        return Err(AgentFilesystemScanError::ConfigTooLarge);
    }
    Ok(result)
}

fn validate_change(change: &AgentConfigChangeV1) -> Result<(), AgentFilesystemScanError> {
    change
        .validate()
        .map_err(|_| AgentFilesystemScanError::InvalidDescriptor)?;
    if change
        .fields
        .iter()
        .any(|field| !CLAUDE_MANAGED_PATHS.contains(&field.path.as_str()))
    {
        return Err(AgentFilesystemScanError::InvalidDescriptor);
    }
    Ok(())
}

// Reject duplicate keys even inside unknown objects that must survive a Value re-render.
struct StrictJson;
impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictJson;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("JSON with unique object keys")
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<StrictJson, E> {
                Ok(StrictJson)
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<StrictJson, E> {
                Ok(StrictJson)
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<StrictJson, E> {
                Ok(StrictJson)
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<StrictJson, E> {
                Ok(StrictJson)
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<StrictJson, E> {
                Ok(StrictJson)
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<StrictJson, E> {
                Ok(StrictJson)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<StrictJson, A::Error> {
                while seq.next_element::<StrictJson>()?.is_some() {}
                Ok(StrictJson)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<StrictJson, A::Error> {
                let mut seen = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key) {
                        return Err(serde::de::Error::custom("duplicate JSON key"));
                    }
                    map.next_value::<StrictJson>()?;
                }
                Ok(StrictJson)
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn managed_values(alias: &str) -> BTreeMap<String, Option<Value>> {
        BTreeMap::from([
            (
                "apiKeyHelper".to_owned(),
                Some(json!({
                    "executable": "/opt/hiroute/bin/hiroute",
                    "argv": [
                        hiroute_domain::HIDDEN_AGENT_GRANT_HELPER_VERB_V1,
                        "agent-connection/agent-context/claude/test"
                    ]
                })),
            ),
            ("hiroute.auth_environment".to_owned(), None),
            (
                "env.ANTHROPIC_BASE_URL".to_owned(),
                Some(json!("http://127.0.0.1:5837")),
            ),
            ("env.ANTHROPIC_MODEL".to_owned(), Some(json!(alias))),
            (
                "env.ANTHROPIC_DEFAULT_OPUS_MODEL".to_owned(),
                Some(json!(alias)),
            ),
            (
                "env.ANTHROPIC_DEFAULT_SONNET_MODEL".to_owned(),
                Some(json!(alias)),
            ),
            (
                "env.ANTHROPIC_DEFAULT_HAIKU_MODEL".to_owned(),
                Some(json!(alias)),
            ),
            (
                "env.ANTHROPIC_SMALL_FAST_MODEL".to_owned(),
                Some(json!(alias)),
            ),
        ])
    }

    #[test]
    fn claude_reconfiguration_keeps_opaque_user_helper_in_the_formal_restore_base() {
        let original = serde_json::to_vec(&json!({
            "theme": "dark",
            "apiKeyHelper": "user-owned-helper --do-not-run",
            "env": {
                "ANTHROPIC_BASE_URL": "https://provider.example/anthropic",
                "ANTHROPIC_MODEL": "provider-model",
                "ANTHROPIC_AUTH_TOKEN": "user-secret",
                "UNRELATED": "keep-me"
            }
        }))
        .unwrap();
        let original_document = claude_document_from_bytes(&original).unwrap();
        let first_change =
            AgentConfigChangeV1::preview(&original_document, managed_values("hiroute/first"))
                .unwrap();
        let configured = render_claude_change_bytes(&original, &first_change).unwrap();
        let configured_document = claude_document_from_bytes(&configured).unwrap();
        let update_change =
            AgentConfigChangeV1::preview(&configured_document, managed_values("hiroute/second"))
                .unwrap();

        let (updated, rebased) =
            rebase_claude_change_bytes(&configured, &original, &first_change, &update_change)
                .unwrap();
        let helper = rebased
            .fields
            .iter()
            .find(|field| field.path == "apiKeyHelper")
            .expect("opaque user helper must remain owned by the rebased restore change");
        assert_eq!(
            helper.after,
            managed_values("hiroute/second")["apiKeyHelper"]
        );
        assert!(
            !serde_json::to_string(&rebased)
                .unwrap()
                .contains("user-secret")
        );
        assert!(
            !serde_json::to_string(&rebased)
                .unwrap()
                .contains("do-not-run")
        );

        let restored = restore_claude_change_bytes(&updated, &original, &rebased).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&restored).unwrap(),
            serde_json::from_slice::<Value>(&original).unwrap()
        );
    }

    #[test]
    fn managed_claude_helper_cannot_leave_an_original_token_in_the_native_file() {
        let original = br#"{"env":{"ANTHROPIC_AUTH_TOKEN":"must-not-survive"}}"#;
        let document = claude_document_from_bytes(original).unwrap();
        let mut desired = managed_values("hiroute/selected");
        desired.remove("hiroute.auth_environment");
        let unsafe_change = AgentConfigChangeV1::preview(&document, desired).unwrap();
        assert!(render_claude_change_bytes(original, &unsafe_change).is_err());
    }
}
