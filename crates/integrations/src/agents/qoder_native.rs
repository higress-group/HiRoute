//! Additional main-Agent providers. No native catalog import or default/purpose takeover.
use std::collections::BTreeSet;

use hiroute_domain::{AgentAccessGrantMaterial, CanonicalDigest, QoderAdditionalModelV1};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use super::{QoderNativeError, qoder_error, qoder_provider};
#[path = "qoder_jsonc.rs"]
mod jsonc;
use jsonc::Document;
use jsonc::LIMIT;

// This structure is serialized only into NativeAgentArtifactPort's encrypted restore envelope.
// It deliberately has no Debug implementation or public JSON conversion.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Restore {
    schema: String,
    provider_id: String,
    provider_digest: CanonicalDigest,
    rendered_digest: CanonicalDigest,
    original_exists: bool,
    #[serde(skip)]
    original: Vec<u8>,
}

impl Drop for Restore {
    fn drop(&mut self) {
        self.original.zeroize();
    }
}

pub(super) struct Edit {
    pub bytes: Zeroizing<Vec<u8>>,
    pub restore: Restore,
}

pub(super) fn validate_declaration(
    provider_id: &str,
    endpoint: &str,
    models: &[QoderAdditionalModelV1],
) -> Result<(), QoderNativeError> {
    qoder_provider::validate_model_route(provider_id, endpoint)?;
    if provider_id
        .strip_prefix("hiroute-main-")
        .is_none_or(|suffix| {
            suffix.len() != 64
                || !suffix
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return Err(qoder_error("main provider identity"));
    }
    let mut aliases = BTreeSet::new();
    if models.is_empty() {
        return Err(qoder_error("additional models"));
    }
    for model in models {
        model
            .validate()
            .map_err(|_| qoder_error("additional model"))?;
        if !aliases.insert(&model.alias) {
            return Err(qoder_error("additional model alias"));
        }
    }
    Ok(())
}

fn document(current: Option<&[u8]>) -> Result<Document<'_>, QoderNativeError> {
    Document::parse(current.unwrap_or(b"{}"))
}

fn owned<'a>(
    document: &'a Document<'_>,
    provider: &str,
) -> Result<Option<&'a Value>, QoderNativeError> {
    let Some(providers) = document.root.value.get("providers") else {
        return Ok(None);
    };
    Ok(providers
        .as_object()
        .ok_or_else(|| qoder_error("native providers object"))?
        .get(provider))
}

pub(super) fn validate_configuration(
    current: Option<&[u8]>,
    provider_id: &str,
    endpoint: &str,
    models: &[QoderAdditionalModelV1],
    previous: Option<&Restore>,
) -> Result<(), QoderNativeError> {
    validate_declaration(provider_id, endpoint, models)?;
    let document = document(current)?;
    if let Some(previous) = previous {
        if previous.provider_id != provider_id || !previous.applied(current)? {
            return Err(qoder_error("owned provider changed"));
        }
        let retain = models
            .iter()
            .map(|model| format!("{provider_id}/{}", model.alias))
            .collect();
        default_not_removed(&document, provider_id, &retain)?;
    } else if owned(&document, provider_id)?.is_some() {
        return Err(qoder_error("provider already exists"));
    }
    Ok(())
}

fn default_not_removed(
    document: &Document<'_>,
    provider: &str,
    retain: &BTreeSet<String>,
) -> Result<(), QoderNativeError> {
    let Some(model) = document.root.value.get("model") else {
        return Ok(());
    };
    let model = model
        .as_object()
        .ok_or_else(|| qoder_error("native model object"))?;
    if let Some(default) = model.get("name") {
        let default = default
            .as_str()
            .ok_or_else(|| qoder_error("native default model"))?;
        // Every selector inside our owned provider disappears when it is removed, including a
        // stale alias no longer present in its declared models. Never choose a replacement.
        if default.starts_with(&format!("{provider}/")) && !retain.contains(default) {
            return Err(qoder_error("default in use"));
        }
    }
    Ok(())
}

pub(super) fn configure(
    current: Option<&[u8]>,
    provider_id: &str,
    endpoint: &str,
    models: &[QoderAdditionalModelV1],
    material: &AgentAccessGrantMaterial,
    previous: Option<&Restore>,
) -> Result<Edit, QoderNativeError> {
    validate_configuration(current, provider_id, endpoint, models, previous)?;
    let base = match previous {
        Some(previous) => previous.remove_owned(current, false)?,
        None => current.map(|bytes| Zeroizing::new(bytes.to_vec())),
    };
    let credential =
        std::str::from_utf8(material.expose()).map_err(|_| qoder_error("local grant encoding"))?;
    let models = models
        .iter()
        .map(|model| {
            qoder_provider::model(
                &model.alias,
                Some(model.context_window_tokens),
                model.max_output_tokens,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    // Omit provider.model and routing. Qoder retains the selected model for auxiliary purposes;
    // a multi-model provider must not redirect every purpose to its first entry.
    let provider = qoder_provider::provider(endpoint, credential, models);
    let document = document(base.as_deref().map(Vec::as_slice))?;
    let bytes = if let Some(providers) = document.object(&document.root, "providers")? {
        document.edit(&providers, provider_id, Some(&provider))?
    } else {
        document.edit(
            &document.root,
            "providers",
            Some(&serde_json::json!({(provider_id):provider})),
        )?
    };
    let restore = Restore {
        schema: "hiroute.qoder-native-restore/v1".into(),
        provider_id: provider_id.into(),
        provider_digest: digest(&provider)?,
        rendered_digest: CanonicalDigest::of_bytes(&bytes),
        original_exists: base.is_some(),
        original: base.map_or_else(Vec::new, |bytes| bytes.to_vec()),
    };
    Ok(Edit { bytes, restore })
}

impl Restore {
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, QoderNativeError> {
        let header = serde_json::to_vec(self).map_err(|_| qoder_error("restore encoding"))?;
        let size = header
            .len()
            .checked_add(self.original.len())
            .and_then(|size| size.checked_add(4))
            .filter(|size| *size <= LIMIT)
            .ok_or_else(|| qoder_error("restore bound"))?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(size));
        bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&self.original);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, QoderNativeError> {
        if bytes.len() < 5 || bytes.len() > LIMIT {
            return Err(qoder_error("restore bound"));
        }
        let length = u32::from_le_bytes(bytes[..4].try_into().expect("bounded header")) as usize;
        let end = 4usize
            .checked_add(length)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| qoder_error("restore bound"))?;
        let mut record: Self =
            serde_json::from_slice(&bytes[4..end]).map_err(|_| qoder_error("restore encoding"))?;
        record.original = bytes[end..].to_vec();
        if record.schema != "hiroute.qoder-native-restore/v1"
            || record
                .provider_id
                .strip_prefix("hiroute-main-")
                .is_none_or(|s| {
                    s.len() != 64
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
            || (!record.original_exists && !record.original.is_empty())
        {
            return Err(qoder_error("restore schema"));
        }
        CanonicalDigest::parse(record.provider_digest.as_str())
            .map_err(|_| qoder_error("restore digest"))?;
        CanonicalDigest::parse(record.rendered_digest.as_str())
            .map_err(|_| qoder_error("restore digest"))?;
        let original = document(record.original_exists.then_some(record.original.as_slice()))?;
        if owned(&original, &record.provider_id)?.is_some() {
            return Err(qoder_error("restore ownership"));
        }
        Ok(record)
    }

    pub fn applied(&self, current: Option<&[u8]>) -> Result<bool, QoderNativeError> {
        let document = document(current)?;
        Ok(owned(&document, &self.provider_id)?
            .map(digest)
            .transpose()?
            .as_ref()
            == Some(&self.provider_digest))
    }

    pub fn validate_restoration(&self, current: Option<&[u8]>) -> Result<(), QoderNativeError> {
        self.remove_owned(current, true).map(|_| ())
    }

    pub fn restore(
        &self,
        current: Option<&[u8]>,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, QoderNativeError> {
        self.remove_owned(current, true)
    }

    fn remove_owned(
        &self,
        current: Option<&[u8]>,
        check_default: bool,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, QoderNativeError> {
        let document = document(current)?;
        if check_default {
            default_not_removed(&document, &self.provider_id, &BTreeSet::new())?;
        }
        if current.is_none() && !self.original_exists {
            return Ok(None);
        }
        if !self.applied(current)? {
            return Err(qoder_error("owned provider changed"));
        }
        if current.is_some_and(|bytes| CanonicalDigest::of_bytes(bytes) == self.rendered_digest) {
            return Ok(self
                .original_exists
                .then(|| Zeroizing::new(self.original.clone())));
        }
        let providers = document
            .object(&document.root, "providers")?
            .ok_or_else(|| qoder_error("owned provider missing"))?;
        // A user may have added comments/foreign members since Apply. Preserve those exact bytes,
        // including an empty providers container, instead of replaying a whole-file backup.
        Ok(Some(document.edit(&providers, &self.provider_id, None)?))
    }
}

fn digest(value: &Value) -> Result<CanonicalDigest, QoderNativeError> {
    CanonicalDigest::of(value).map_err(|_| qoder_error("provider digest"))
}

#[cfg(test)]
#[path = "qoder_native_tests.rs"]
mod tests;
