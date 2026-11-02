//! An index of validated decision-only bindings; resolution performs no discovery or I/O.
use hiroute_domain::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct DecisionContextIndex {
    exact: BTreeMap<(u8, String, String), Vec<(usize, usize)>>,
    templates: Vec<(u8, String, String, String, usize, usize)>,
}

fn protocol_key(protocol: &str) -> Option<u8> {
    match protocol {
        "system-one" => Some(0),
        "openai-decisions" => Some(1),
        _ => None,
    }
}

fn wire_key(protocol: DecisionWireProtocolV2) -> u8 {
    match protocol {
        DecisionWireProtocolV2::SystemOne => 0,
        DecisionWireProtocolV2::OpenaiDecisions => 1,
    }
}

fn canonical_endpoint(value: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}

impl DecisionContextIndex {
    pub(crate) fn build(catalog: &ModelMetadataCatalogV1) -> Self {
        let mut result = Self::default();
        let interfaces: BTreeMap<_, _> = catalog
            .access_products
            .iter()
            .flat_map(|product| product.interfaces.iter())
            .map(|interface| (interface.interface_key.as_str(), interface))
            .collect();
        for (binding_index, binding) in catalog.endpoint_bindings.iter().enumerate() {
            for (constraint_index, constraint) in binding.decision_constraints.iter().enumerate() {
                let Some(interface) = interfaces.get(constraint.interface_key.as_str()) else {
                    continue;
                };
                let Some(protocol) = protocol_key(&interface.protocol) else {
                    continue;
                };
                let Some(path) = interface.request_path.as_deref() else {
                    continue;
                };
                if let Some(base) = &interface.base_url {
                    if let Some(endpoint) = canonical_endpoint(&format!("{base}{path}")) {
                        result
                            .exact
                            .entry((
                                protocol,
                                endpoint.to_string(),
                                binding.upstream_model_id.clone(),
                            ))
                            .or_default()
                            .push((binding_index, constraint_index));
                    }
                } else if let Some(base) = &interface.base_url_template {
                    // Only the documented, single DNS-label WorkspaceId template is supported.
                    let Some(suffix) = base.strip_prefix("https://{WorkspaceId}.") else {
                        continue;
                    };
                    if suffix.contains(['/', '{', '}']) {
                        continue;
                    }
                    result.templates.push((
                        protocol,
                        suffix.into(),
                        path.into(),
                        binding.upstream_model_id.clone(),
                        binding_index,
                        constraint_index,
                    ));
                }
            }
        }
        result
    }

    fn matching<'a>(
        &self,
        catalog: &'a ModelMetadataCatalogV1,
        endpoint: &str,
        protocol: DecisionWireProtocolV2,
        model: &str,
        header: bool,
    ) -> Result<
        Option<(
            &'a MetadataEndpointBindingV1,
            &'a MetadataDecisionConstraintV1,
        )>,
        DecisionContextResolutionError,
    > {
        let Some(url) = canonical_endpoint(endpoint) else {
            return Ok(None);
        };
        let key = wire_key(protocol);
        let mut matches = self
            .exact
            .get(&(key, url.to_string(), model.into()))
            .cloned()
            .unwrap_or_default();
        if url.port_or_known_default() == Some(443) {
            for (wire, suffix, path, upstream, binding, constraint) in &self.templates {
                if *wire != key || path != url.path() || upstream != model {
                    continue;
                }
                let Some(label) = url
                    .host_str()
                    .and_then(|host| host.strip_suffix(&format!(".{suffix}")))
                else {
                    continue;
                };
                if !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                {
                    matches.push((*binding, *constraint));
                }
            }
        }
        let mut applicable = matches.into_iter().filter_map(|(binding, constraint)| {
            let binding = &catalog.endpoint_bindings[binding];
            let constraint = &binding.decision_constraints[constraint];
            let applies = match constraint.condition {
                MetadataDecisionConditionV1::Always => true,
                MetadataDecisionConditionV1::TypesafeLongContext => header,
                MetadataDecisionConditionV1::TypesafeDefault => !header,
            };
            applies.then_some((binding, constraint))
        });
        let result = applicable.next();
        if applicable.next().is_some() {
            return Err(DecisionContextResolutionError::AmbiguousMetadata);
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum DecisionContextResolutionError {
    #[error("invalid decision context policy")]
    InvalidPolicy,
    #[error("ambiguous decision context metadata")]
    AmbiguousMetadata,
}

impl super::TrustedReleaseCatalog {
    pub fn resolve_decision_context(
        &self,
        endpoint: &str,
        protocol: DecisionWireProtocolV2,
        model: &str,
        policy: &DecisionContextPolicyV1,
    ) -> Result<ResolvedDecisionContextV1, DecisionContextResolutionError> {
        if !policy.validate() {
            return Err(DecisionContextResolutionError::InvalidPolicy);
        }
        let catalog = self.model_metadata();
        let header = policy.typesafe_long_context == TypeSafeLongContextV1::Automatic
            && official_typesafe_long_context_destination(endpoint, protocol);
        let index = self
            .decision_context_index
            .get_or_init(|| DecisionContextIndex::build(catalog));
        let matched = index.matching(catalog, endpoint, protocol, model, header)?;
        let (window, request_source) = if let Some(window) = policy.window_tokens {
            (window, DecisionContextSourceV1::User)
        } else if let Some((_, constraint)) = matched {
            (constraint.request_tokens, DecisionContextSourceV1::Metadata)
        } else {
            (
                DECISION_CONTEXT_FALLBACK_TOKENS,
                DecisionContextSourceV1::Fallback,
            )
        };
        let (question, question_source) = if let Some(question) = policy.per_question_tokens {
            (question, DecisionContextSourceV1::User)
        } else if let Some(question) = matched.and_then(|(_, c)| c.per_question_tokens) {
            (question.min(window), DecisionContextSourceV1::Metadata)
        } else {
            (window, request_source)
        };
        let metadata = matched.map(|(binding, constraint)| DecisionContextMetadataV1 {
            binding_key: binding.binding_key.clone(),
            interface_key: constraint.interface_key.clone(),
            constraint_key: constraint.constraint_key.clone(),
            basis: constraint.basis,
            evidence_refs: constraint.evidence_refs.clone(),
            note: constraint.note.clone(),
        });
        let revision = self
            .official_snapshot
            .as_ref()
            .map(|s| s.data_revision)
            .unwrap_or(1);
        let mut result = ResolvedDecisionContextV1 {
            request_window_tokens: window,
            per_question_window_tokens: question,
            request_source,
            per_question_source: question_source,
            catalog_revision: revision,
            catalog_digest: catalog.source_catalog_digest.clone(),
            metadata,
            typesafe_long_context_header: header,
            accounting_algorithm: DECISION_CONTEXT_ALGORITHM_V1.into(),
            reserve_policy: DECISION_CONTEXT_RESERVE_V1.into(),
            context_digest: CanonicalDigest::of_bytes(&[]),
        };
        result.context_digest = result
            .calculate_digest()
            .expect("context identity serialization");
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
