//! Lossless native TOML edits. Callers own protected file IO and Journal persistence.
//! This module never opens a file, executes a helper, or accepts upstream credential material.
use hiroute_domain::{
    AgentAccessGrantMaterial, CanonicalDigest, ModelAlias, valid_client_model_name,
};
use toml_edit::{DocumentMut, Item, Table};
use zeroize::Zeroizing;

#[path = "codex_restore_record.rs"]
mod restore_record;

const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodexSelectionTarget {
    /// User config or a separately selected native profile file, as proven by discovery.
    Root,
    LegacyProfile(String),
}

/// Contains local credential material. No Debug/serde implementation or public diagnostics.
pub struct CodexNativeEdit {
    pub rendered: Zeroizing<String>,
    pub restore: CodexNativeRestore,
}

impl CodexNativeEdit {
    pub fn with_managed_aliases(mut self, aliases: &[String]) -> Result<Self, CodexNativeError> {
        if aliases.len() > 128
            || aliases
                .iter()
                .any(|alias| ModelAlias::parse(alias.clone()).is_err())
            || aliases
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != aliases.len()
        {
            return Err(CodexNativeError::InvalidIntent);
        }
        self.restore.managed_aliases = aliases.to_vec();
        Ok(self)
    }

    pub fn with_model_catalog(mut self, path: &std::path::Path) -> Result<Self, CodexNativeError> {
        let path_text = path.to_str().ok_or(CodexNativeError::InvalidIntent)?;
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            || path_text.contains(['\0', '\r', '\n'])
            || self.restore.fields.iter().any(|field| {
                field
                    .path
                    .last()
                    .is_some_and(|key| key == "model_catalog_json")
            })
        {
            return Err(CodexNativeError::InvalidIntent);
        }
        let selection = self
            .restore
            .fields
            .iter()
            .find(|field| field.path.last().is_some_and(|key| key == "model_provider"))
            .ok_or(CodexNativeError::UnsupportedLayout)?;
        let parent = selection.path[..selection.path.len() - 1].to_vec();
        let mut document = parse(&self.rendered)?;
        set(
            &mut document,
            &parent,
            "model_catalog_json",
            toml_edit::value(path_text),
            &mut self.restore,
        )?;
        self.rendered = render_bounded(&document)?;
        Ok(self)
    }
}

/// The Operation must store this in its protected restoration store, never the public Journal.
pub struct CodexNativeRestore {
    fields: Vec<FieldRestore>,
    created_tables: Vec<Vec<String>>,
    managed_aliases: Vec<String>,
}
struct FieldRestore {
    path: Vec<String>,
    before: Option<Item>,
    after: CanonicalDigest,
    // PreserveNative observes the user's default without writing it. Only a later
    // alias in the sealed grant is reverted; another native user choice survives.
    passive_model: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CodexNativeError {
    #[error("native TOML is invalid or exceeds the input limit")]
    InvalidToml,
    #[error("configuration changed after preview")]
    SourceChanged,
    #[error("the selected native configuration shape is not proven")]
    UnsupportedLayout,
    #[error("a user provider already occupies the proposed managed identity")]
    ProviderAlreadyExists,
    #[error("a managed field changed after application")]
    FieldConflict,
    #[error("the managed model or local endpoint is invalid")]
    InvalidIntent,
}

pub fn configure_codex_native(
    current: &str,
    expected: &CanonicalDigest,
    target: CodexSelectionTarget,
    provider_id: &str,
    endpoint: &str,
    model: Option<&str>,
    local_grant: &AgentAccessGrantMaterial,
) -> Result<CodexNativeEdit, CodexNativeError> {
    if &CanonicalDigest::of_bytes(current.as_bytes()) != expected {
        return Err(CodexNativeError::SourceChanged);
    }
    configure_codex_base(
        current,
        target,
        provider_id,
        endpoint,
        model,
        local_grant,
        false,
    )
}

/// Replace fields from a configuration whose protected restore record proves this provider is
/// already HiRoute-owned. The new restore record is rebased onto the original user fields, so a
/// later formal restore never leaves an older local bearer or managed model behind.
#[allow(clippy::too_many_arguments)]
pub(super) fn reconfigure_codex_native(
    current: &str,
    expected: &CanonicalDigest,
    previous: &CodexNativeRestore,
    target: CodexSelectionTarget,
    provider_id: &str,
    endpoint: &str,
    model: Option<&str>,
    local_grant: &AgentAccessGrantMaterial,
) -> Result<CodexNativeEdit, CodexNativeError> {
    if &CanonicalDigest::of_bytes(current.as_bytes()) != expected {
        return Err(CodexNativeError::SourceChanged);
    }
    let base = restore_codex_native(current, previous)?;
    configure_codex_base(
        &base,
        target,
        provider_id,
        endpoint,
        model,
        local_grant,
        previous.owns_provider(provider_id),
    )
}

#[allow(clippy::too_many_arguments)]
fn configure_codex_base(
    current: &str,
    target: CodexSelectionTarget,
    provider_id: &str,
    endpoint: &str,
    model: Option<&str>,
    local_grant: &AgentAccessGrantMaterial,
    owned_provider: bool,
) -> Result<CodexNativeEdit, CodexNativeError> {
    if provider_id != "hiroute"
        || !local_endpoint(endpoint)
        || model.is_some_and(|name| !valid_client_model_name(name))
    {
        return Err(CodexNativeError::InvalidIntent);
    }
    let mut document = parse(current)?;
    let selection = match target {
        CodexSelectionTarget::Root => Vec::new(),
        CodexSelectionTarget::LegacyProfile(name) if identifier(&name) => {
            let path = vec!["profiles".to_owned(), name];
            table_at(&document, &path)?;
            path
        }
        _ => return Err(CodexNativeError::UnsupportedLayout),
    };
    if let Some(providers) = document.get("model_providers") {
        let providers = providers
            .as_table()
            .ok_or(CodexNativeError::UnsupportedLayout)?;
        if providers.contains_key(provider_id) && !owned_provider {
            return Err(CodexNativeError::ProviderAlreadyExists);
        }
    }
    let mut restore = CodexNativeRestore {
        fields: Vec::new(),
        created_tables: Vec::new(),
        managed_aliases: Vec::new(),
    };
    if let Some(model) = model {
        set(
            &mut document,
            &selection,
            "model",
            toml_edit::value(model),
            &mut restore,
        )?;
    } else {
        let mut path = selection.clone();
        path.push("model".to_owned());
        let before = table_at(&document, &selection)?.get("model").cloned();
        if before.as_ref().is_some_and(|item| item.as_str().is_none()) {
            return Err(CodexNativeError::UnsupportedLayout);
        }
        restore.fields.push(FieldRestore {
            path,
            after: before
                .as_ref()
                .map(fingerprint)
                .unwrap_or_else(|| CanonicalDigest::of_bytes(b"codex.absent-model")),
            before,
            passive_model: true,
        });
    }
    set(
        &mut document,
        &selection,
        "model_provider",
        toml_edit::value(provider_id),
        &mut restore,
    )?;
    let provider = vec!["model_providers".to_owned(), provider_id.to_owned()];
    create_tables(&mut document, &provider, &mut restore.created_tables)?;
    let token =
        std::str::from_utf8(local_grant.expose()).map_err(|_| CodexNativeError::InvalidIntent)?;
    for (key, value) in [
        ("name", "HiRoute"),
        ("base_url", endpoint),
        ("wire_api", "responses"),
    ] {
        set(
            &mut document,
            &provider,
            key,
            toml_edit::value(value),
            &mut restore,
        )?;
    }
    for (key, value) in [
        ("requires_openai_auth", true),
        ("supports_websockets", false),
    ] {
        set(
            &mut document,
            &provider,
            key,
            toml_edit::value(value),
            &mut restore,
        )?;
    }
    let headers = vec![
        "model_providers".to_owned(),
        provider_id.to_owned(),
        "http_headers".to_owned(),
    ];
    create_tables(&mut document, &headers, &mut restore.created_tables)?;
    set(
        &mut document,
        &headers,
        "X-HiRoute-Token",
        toml_edit::value(token),
        &mut restore,
    )?;
    Ok(CodexNativeEdit {
        rendered: render_bounded(&document)?,
        restore,
    })
}

impl CodexNativeRestore {
    /// Status follows the fields HiRoute wrote. Other native settings and TOML formatting
    /// may change independently after the configuration Operation succeeds.
    pub fn managed_fields_are_applied(&self, current: &str) -> bool {
        let Ok(document) = parse(current) else {
            return false;
        };
        self.fields
            .iter()
            .filter(|field| !field.passive_model)
            .all(|field| {
                let Some((key, parent)) = field.path.split_last() else {
                    return false;
                };
                optional_table(&document, parent)
                    .ok()
                    .flatten()
                    .and_then(|table| table.get(key))
                    .is_some_and(|item| fingerprint(item) == field.after)
            })
    }

    fn owns_provider(&self, provider_id: &str) -> bool {
        [
            "name",
            "base_url",
            "wire_api",
            "requires_openai_auth",
            "supports_websockets",
        ]
        .into_iter()
        .all(|key| {
            self.fields.iter().any(|field| {
                field.before.is_none()
                    && field.path
                        == [
                            "model_providers".to_owned(),
                            provider_id.to_owned(),
                            key.to_owned(),
                        ]
            })
        })
    }

    /// When this edit wrote the managed catalog pointer, the user's original pointer it
    /// replaced (None inside means the user had none). Absent when the edit never touched
    /// the pointer, so the scope's own configuration still carries the user's state.
    pub fn catalog_pointer_before(&self) -> Option<Option<String>> {
        self.fields
            .iter()
            .find(|field| {
                field
                    .path
                    .last()
                    .is_some_and(|key| key == "model_catalog_json")
            })
            .map(|field| {
                field
                    .before
                    .as_ref()
                    .and_then(|item| item.as_str().map(str::to_owned))
            })
    }
}

pub fn restore_codex_native(
    current: &str,
    restore: &CodexNativeRestore,
) -> Result<Zeroizing<String>, CodexNativeError> {
    restore_codex_native_with_model(current, restore, None)
}

pub fn codex_explicit_model(text: &str) -> Result<Option<String>, CodexNativeError> {
    Ok(parse(text)?
        .get("model")
        .and_then(Item::as_str)
        .map(str::to_owned))
}

pub fn codex_explicit_reasoning_effort(text: &str) -> Result<Option<String>, CodexNativeError> {
    parse(text)?
        .get("model_reasoning_effort")
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or(CodexNativeError::UnsupportedLayout)
        })
        .transpose()
}

/// Restore native ownership and, when explicitly chosen, set a model proven by the original
/// catalog in the caller's preview. The same choice is rechecked by the daemon at Apply.
pub fn restore_codex_native_with_model(
    current: &str,
    restore: &CodexNativeRestore,
    native_model: Option<&str>,
) -> Result<Zeroizing<String>, CodexNativeError> {
    if native_model.is_some_and(|model| !valid_client_model_name(model)) {
        return Err(CodexNativeError::InvalidIntent);
    }
    let mut document = parse(current)?;
    let mut pending = Vec::new();
    for field in &restore.fields {
        let (key, parent) = field
            .path
            .split_last()
            .ok_or(CodexNativeError::UnsupportedLayout)?;
        let item = optional_table(&document, parent)?.and_then(|table| table.get(key));
        if field.passive_model {
            // This field was observed, not written. Release only a route alias
            // authorized by this exact settings Operation; other user edits survive.
            if item
                .and_then(Item::as_str)
                .is_some_and(|model| restore.managed_aliases.iter().any(|alias| alias == model))
                && item.map(fingerprint) != field.before.as_ref().map(fingerprint)
            {
                pending.push(field);
            }
            continue;
        }
        if item.map(fingerprint) == field.before.as_ref().map(fingerprint) {
            continue;
        }
        if item.map(fingerprint).as_ref() != Some(&field.after) {
            return Err(CodexNativeError::FieldConflict);
        }
        pending.push(field);
    }
    for field in pending {
        let (key, parent) = field
            .path
            .split_last()
            .ok_or(CodexNativeError::UnsupportedLayout)?;
        let table = table_at_mut(&mut document, parent)?;
        match &field.before {
            Some(before) => {
                let mut restored = before.clone();
                if let (Some(current), Some(value)) = (
                    table.get(key).and_then(Item::as_value),
                    restored.as_value_mut(),
                ) {
                    *value.decor_mut() = current.decor().clone();
                }
                *table.get_mut(key).ok_or(CodexNativeError::FieldConflict)? = restored;
            }
            None => {
                table.remove(key);
            }
        }
    }
    // Newly added user keys keep their table alive. Only tables created by this edit can go.
    for path in restore.created_tables.iter().rev() {
        if optional_table(&document, path)?.is_some_and(Table::is_empty) {
            let (key, parent) = path
                .split_last()
                .ok_or(CodexNativeError::UnsupportedLayout)?;
            table_at_mut(&mut document, parent)?.remove(key);
        }
    }
    if let Some(native_model) = native_model {
        let field = restore
            .fields
            .iter()
            .find(|field| field.path.last().is_some_and(|key| key == "model"))
            .ok_or(CodexNativeError::UnsupportedLayout)?;
        let (_, parent) = field
            .path
            .split_last()
            .ok_or(CodexNativeError::UnsupportedLayout)?;
        let table = table_at_mut(&mut document, parent)?;
        table["model"] = toml_edit::value(native_model);
    }
    render_bounded(&document)
}

fn render_bounded(document: &DocumentMut) -> Result<Zeroizing<String>, CodexNativeError> {
    let rendered = Zeroizing::new(document.to_string());
    if rendered.len() > MAX_BYTES {
        return Err(CodexNativeError::InvalidToml);
    }
    Ok(rendered)
}

fn parse(text: &str) -> Result<DocumentMut, CodexNativeError> {
    if text.len() > MAX_BYTES || text.contains('\0') {
        return Err(CodexNativeError::InvalidToml);
    }
    text.parse().map_err(|_| CodexNativeError::InvalidToml)
}
fn identifier(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}
fn local_endpoint(text: &str) -> bool {
    let Some(port) = text
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| text.strip_prefix("http://[::1]:"))
        .and_then(|value| value.strip_suffix("/v1"))
    else {
        return false;
    };
    !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}
fn fingerprint(item: &Item) -> CanonicalDigest {
    match (item.as_str(), item.as_bool()) {
        (Some(value), _) => CanonicalDigest::of_bytes(format!("string\0{value}").as_bytes()),
        (_, Some(value)) => CanonicalDigest::of_bytes(format!("boolean\0{value}").as_bytes()),
        _ => CanonicalDigest::of_bytes(format!("other\0{item}").as_bytes()),
    }
}
fn optional_table<'a>(
    document: &'a DocumentMut,
    path: &[String],
) -> Result<Option<&'a Table>, CodexNativeError> {
    let mut table = document.as_table();
    for key in path {
        let Some(item) = table.get(key) else {
            return Ok(None);
        };
        table = item.as_table().ok_or(CodexNativeError::UnsupportedLayout)?;
    }
    Ok(Some(table))
}
fn table_at<'a>(document: &'a DocumentMut, path: &[String]) -> Result<&'a Table, CodexNativeError> {
    let mut table = document.as_table();
    for key in path {
        table = table
            .get(key)
            .and_then(Item::as_table)
            .ok_or(CodexNativeError::UnsupportedLayout)?;
    }
    Ok(table)
}
fn table_at_mut<'a>(
    document: &'a mut DocumentMut,
    path: &[String],
) -> Result<&'a mut Table, CodexNativeError> {
    let mut table = document.as_table_mut();
    for key in path {
        table = table
            .get_mut(key)
            .and_then(Item::as_table_mut)
            .ok_or(CodexNativeError::UnsupportedLayout)?;
    }
    Ok(table)
}
fn create_tables(
    document: &mut DocumentMut,
    path: &[String],
    created: &mut Vec<Vec<String>>,
) -> Result<(), CodexNativeError> {
    for index in 0..path.len() {
        let parent = table_at_mut(document, &path[..index])?;
        if !parent.contains_key(&path[index]) {
            let mut table = Table::new();
            table.set_implicit(true);
            parent.insert(&path[index], Item::Table(table));
            created.push(path[..=index].to_vec());
        }
        table_at(document, &path[..=index])?;
    }
    Ok(())
}
fn set(
    document: &mut DocumentMut,
    parent: &[String],
    key: &str,
    mut after: Item,
    restore: &mut CodexNativeRestore,
) -> Result<(), CodexNativeError> {
    let table = table_at_mut(document, parent)?;
    let before = table.get(key).cloned();
    if let Some(before) = &before {
        let previous = before
            .as_value()
            .ok_or(CodexNativeError::UnsupportedLayout)?;
        if !previous.is_str() {
            return Err(CodexNativeError::UnsupportedLayout);
        }
        if let Some(value) = after.as_value_mut() {
            *value.decor_mut() = previous.decor().clone();
        }
    }
    let mut path = parent.to_vec();
    path.push(key.to_owned());
    restore.fields.push(FieldRestore {
        path,
        before,
        after: fingerprint(&after),
        passive_model: false,
    });
    if let Some(current) = table.get_mut(key) {
        // Replacing via insert would also replace the key's decoration (leading comments).
        *current = after;
    } else {
        table.insert(key, after);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
