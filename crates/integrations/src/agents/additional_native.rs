//! Additional main-Agent providers. No native catalog import or default/purpose takeover.
use std::collections::BTreeSet;

use hiroute_domain::{
    AdditionalAgentModelV1, AgentAccessGrantMaterial, AgentKindV1, CanonicalDigest,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use super::{QoderNativeError, qoder_error, qoder_provider};
#[path = "qoder_jsonc.rs"]
mod jsonc;
use jsonc::Document;
use jsonc::LIMIT;

pub(super) fn parse_native_jsonc(bytes: &[u8]) -> Result<Value, QoderNativeError> {
    Ok(Document::parse(bytes)?.root.value)
}

// This structure is serialized only into NativeAgentArtifactPort's encrypted restore envelope.
// It deliberately has no Debug implementation or public JSON conversion.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Restore {
    schema: String,
    provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_digest: Option<CanonicalDigest>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    providers: std::collections::BTreeMap<String, CanonicalDigest>,
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
    kind: AgentKindV1,
    provider_id: &str,
    endpoint: &str,
    models: &[AdditionalAgentModelV1],
) -> Result<(), QoderNativeError> {
    if kind == AgentKindV1::Qoder {
        qoder_provider::validate_model_route(provider_id, endpoint)?;
    } else if !matches!(kind, AgentKindV1::Pi | AgentKindV1::DeepseekHarness)
        || !endpoint.starts_with("http://")
        || !endpoint.ends_with("/v1")
    {
        return Err(qoder_error("additional endpoint"));
    }
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
        (if matches!(kind, AgentKindV1::Pi | AgentKindV1::DeepseekHarness) {
            model.validate_pi()
        } else {
            model.validate()
        })
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

fn native_root(kind: AgentKindV1, current: Option<&[u8]>) -> Result<Value, QoderNativeError> {
    if kind == AgentKindV1::DeepseekHarness {
        super::dsh_config::Patch::parse(current)?.model_root()
    } else {
        Ok(document(current)?.root.value)
    }
}
fn owned<'a>(root: &'a Value, provider: &str) -> Result<Option<&'a Value>, QoderNativeError> {
    let Some(providers) = root.get("providers") else {
        return Ok(None);
    };
    Ok(providers
        .as_object()
        .ok_or_else(|| qoder_error("native providers object"))?
        .get(provider))
}
fn edit_provider(
    kind: AgentKindV1,
    bytes: &[u8],
    id: &str,
    provider: Option<&Value>,
) -> Result<Zeroizing<Vec<u8>>, QoderNativeError> {
    if kind == AgentKindV1::DeepseekHarness {
        return super::dsh_config::Patch::parse(Some(bytes))?.edit_provider(id, provider);
    }
    let document = document(Some(bytes))?;
    if let Some(providers) = document.object(&document.root, "providers")? {
        document.edit(&providers, id, provider)
    } else {
        document.edit(
            &document.root,
            "providers",
            Some(&serde_json::json!({(id):provider})),
        )
    }
}

pub(super) fn validate_configuration(
    kind: AgentKindV1,
    current: Option<&[u8]>,
    provider_id: &str,
    endpoint: &str,
    models: &[AdditionalAgentModelV1],
    previous: Option<&Restore>,
) -> Result<(), QoderNativeError> {
    validate_declaration(kind, provider_id, endpoint, models)?;
    let document = native_root(kind, current)?;
    let desired: BTreeSet<_> = models
        .iter()
        .map(|m| hiroute_domain::additional_model_provider_for(provider_id, models, m))
        .collect();
    if let Some(previous) = previous {
        if previous.kind()? != kind
            || previous.provider_id != provider_id
            || !previous.applied(current)?
        {
            return Err(qoder_error("owned provider changed"));
        }
        let retain = models
            .iter()
            .map(|m| {
                format!(
                    "{}/{}",
                    hiroute_domain::additional_model_provider_for(provider_id, models, m),
                    m.alias
                )
            })
            .collect();
        for id in previous.owned_providers().keys() {
            default_not_removed(&document, id, &retain)?;
        }
    }
    for id in desired {
        if owned(&document, &id)?.is_some()
            && !previous.is_some_and(|p| p.owned_providers().contains_key(&id))
        {
            return Err(qoder_error("provider already exists"));
        }
    }
    Ok(())
}

fn default_not_removed(
    document: &Value,
    provider: &str,
    retain: &BTreeSet<String>,
) -> Result<(), QoderNativeError> {
    let Some(model) = document.get("model") else {
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
    kind: AgentKindV1,
    current: Option<&[u8]>,
    provider_id: &str,
    endpoint: &str,
    models: &[AdditionalAgentModelV1],
    material: &AgentAccessGrantMaterial,
    previous: Option<&Restore>,
) -> Result<Edit, QoderNativeError> {
    validate_configuration(kind, current, provider_id, endpoint, models, previous)?;
    let base = match previous {
        Some(previous) => previous.remove_owned(current, false)?,
        None => current.map(|bytes| Zeroizing::new(bytes.to_vec())),
    };
    let credential =
        std::str::from_utf8(material.expose()).map_err(|_| qoder_error("local grant encoding"))?;
    let mut groups = std::collections::BTreeMap::<String, Vec<AdditionalAgentModelV1>>::new();
    for model in models {
        groups
            .entry(hiroute_domain::additional_model_provider_for(
                provider_id,
                models,
                model,
            ))
            .or_default()
            .push(model.clone());
    }
    let mut bytes = base.as_deref().map_or_else(
        || {
            Zeroizing::new(if kind == AgentKindV1::DeepseekHarness {
                b"[]\n".to_vec()
            } else {
                b"{}".to_vec()
            })
        },
        |b| Zeroizing::new(b.to_vec()),
    );
    let mut digests = std::collections::BTreeMap::new();
    for (id, models) in groups {
        let provider = render_provider(kind, endpoint, credential, &models)?;
        digests.insert(id.clone(), digest(&provider)?);
        bytes = edit_provider(kind, &bytes, &id, Some(&provider))?;
    }
    let restore = Restore {
        schema: if kind == AgentKindV1::DeepseekHarness {
            "hiroute.dsh-native-restore/v2"
        } else if kind == AgentKindV1::Pi {
            "hiroute.pi-native-restore/v2"
        } else {
            "hiroute.qoder-native-restore/v2"
        }
        .into(),
        provider_id: provider_id.into(),
        provider_digest: None,
        providers: digests,
        rendered_digest: CanonicalDigest::of_bytes(&bytes),
        original_exists: base.is_some(),
        original: base.map_or_else(Vec::new, |bytes| bytes.to_vec()),
    };
    Ok(Edit { bytes, restore })
}

fn render_provider(
    kind: AgentKindV1,
    endpoint: &str,
    credential: &str,
    models: &[AdditionalAgentModelV1],
) -> Result<Value, QoderNativeError> {
    let protocol = models
        .first()
        .ok_or_else(|| qoder_error("additional models"))?
        .protocol;
    Ok(if kind == AgentKindV1::Qoder {
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
        qoder_provider::provider(endpoint, credential, models, protocol)
    } else {
        let models = models
            .iter()
            .map(|model| {
                serde_json::json!({
                    "id":model.alias,"name":model.alias,"reasoning":false,"input":["text"],
                    "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},
                    "contextWindow":model.context_window_tokens,"maxTokens":model.max_output_tokens,
                })
            })
            .collect::<Vec<_>>();
        // Pi's saved auth can replace its Bearer. The scoped custom header remains authoritative.
        let (api, base_url) = super::pi_native_provider_api(protocol, endpoint)
            .map_err(|_| qoder_error("Pi native endpoint"))?;
        if kind == AgentKindV1::DeepseekHarness {
            // A unique custom route has no ambient catalog auth. Both public wire implementations
            // accept explicit Authorization; X-HiRoute-Token binds the independent local grant.
            serde_json::json!({"api":api,"baseURL":base_url,
                "headers":{"Authorization":format!("Bearer {credential}"),"X-HiRoute-Token":credential},"models":models})
        } else {
            serde_json::json!({"api":api,"baseUrl":base_url,"apiKey":credential,
            "headers":{"X-HiRoute-Token":credential},"models":models})
        }
    })
}

impl Restore {
    pub fn kind(&self) -> Result<AgentKindV1, QoderNativeError> {
        match self.schema.as_str() {
            "hiroute.qoder-native-restore/v1" | "hiroute.qoder-native-restore/v2" => {
                Ok(AgentKindV1::Qoder)
            }
            "hiroute.pi-native-restore/v1" | "hiroute.pi-native-restore/v2" => Ok(AgentKindV1::Pi),
            "hiroute.dsh-native-restore/v2" => Ok(AgentKindV1::DeepseekHarness),
            _ => Err(qoder_error("restore schema")),
        }
    }

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
        if record.kind().is_err()
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
        let legacy = record.schema.ends_with("/v1");
        if legacy != record.provider_digest.is_some()
            || (legacy && !record.providers.is_empty())
            || (!legacy && (record.providers.is_empty() || record.providers.len() > 256))
        {
            return Err(qoder_error("restore schema"));
        }
        CanonicalDigest::parse(record.rendered_digest.as_str())
            .map_err(|_| qoder_error("restore digest"))?;
        let original = native_root(
            record.kind()?,
            record.original_exists.then_some(record.original.as_slice()),
        )?;
        for (id, digest) in record.owned_providers() {
            if !legacy
                && id
                    .strip_prefix(&format!("{}-", record.provider_id))
                    .is_none_or(|suffix| {
                        suffix.len() != 32
                            || !suffix
                                .bytes()
                                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                    })
            {
                return Err(qoder_error("restore ownership"));
            }
            CanonicalDigest::parse(digest.as_str()).map_err(|_| qoder_error("restore digest"))?;
            if owned(&original, &id)?.is_some() {
                return Err(qoder_error("restore ownership"));
            }
        }
        Ok(record)
    }

    fn owned_providers(&self) -> std::collections::BTreeMap<String, CanonicalDigest> {
        if let Some(digest) = &self.provider_digest {
            [(self.provider_id.clone(), digest.clone())].into()
        } else {
            self.providers.clone()
        }
    }

    pub fn applied(&self, current: Option<&[u8]>) -> Result<bool, QoderNativeError> {
        let document = native_root(self.kind()?, current)?;
        for (id, expected) in self.owned_providers() {
            if owned(&document, &id)?.map(digest).transpose()?.as_ref() != Some(&expected) {
                return Ok(false);
            }
        }
        Ok(true)
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
        let document = native_root(self.kind()?, current)?;
        if check_default {
            for id in self.owned_providers().keys() {
                default_not_removed(&document, id, &BTreeSet::new())?;
            }
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
        // Remove only members whose exact ownership was verified; preserve foreign JSONC bytes.
        let mut bytes = Zeroizing::new(
            current
                .ok_or_else(|| qoder_error("owned provider missing"))?
                .to_vec(),
        );
        for id in self.owned_providers().keys() {
            bytes = edit_provider(self.kind()?, &bytes, id, None)?;
        }
        Ok(Some(bytes))
    }
}

fn digest(value: &Value) -> Result<CanonicalDigest, QoderNativeError> {
    CanonicalDigest::of(value).map_err(|_| qoder_error("provider digest"))
}

#[cfg(test)]
#[path = "qoder_native_tests.rs"]
mod tests;
