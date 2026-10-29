//! Exact decision destinations are independent of generative model protocols.
use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use super::{ComputeManagementErrorV2, validate_identifier, validate_text};
use crate::{CanonicalDigest, DecisionWireProtocolV2};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTargetAuthenticationV1 {
    Bearer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionTargetConsumerV1 {
    pub service_id: String,
    pub revision: u64,
    pub model: String,
    pub consumer_binding_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComputeDecisionTargetV4 {
    pub endpoint: String,
    pub wire_protocol: DecisionWireProtocolV2,
    pub authentication: DecisionTargetAuthenticationV1,
    pub destination_id: String,
    pub consumers: Vec<DecisionTargetConsumerV1>,
}

/// Match the Gateway's decision endpoint normalization without authorizing another origin.
pub fn canonical_decision_endpoint(endpoint: &str) -> Result<String, ComputeManagementErrorV2> {
    if endpoint.len() > 2048 || endpoint.contains('#') || endpoint.chars().any(char::is_whitespace)
    {
        return Err(ComputeManagementErrorV2::InvalidTarget);
    }
    let uri: http::Uri = endpoint
        .parse()
        .map_err(|_| ComputeManagementErrorV2::InvalidTarget)?;
    let scheme = match uri.scheme_str() {
        Some("http") => "http",
        Some("https") => "https",
        _ => return Err(ComputeManagementErrorV2::InvalidTarget),
    };
    let authority = uri
        .authority()
        .ok_or(ComputeManagementErrorV2::InvalidTarget)?;
    let raw = authority.as_str();
    if raw.contains('@') {
        return Err(ComputeManagementErrorV2::InvalidTarget);
    }
    let host = authority.host();
    if host.is_empty() {
        return Err(ComputeManagementErrorV2::InvalidTarget);
    }
    let suffix = raw
        .strip_prefix(host)
        .ok_or(ComputeManagementErrorV2::InvalidTarget)?;
    let explicit_port = authority.port_u16();
    if (!suffix.is_empty() && (suffix != format!(":{}", explicit_port.unwrap_or(0))))
        || explicit_port == Some(0)
    {
        return Err(ComputeManagementErrorV2::InvalidTarget);
    }
    let bare_host = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host)
        .to_ascii_lowercase();
    let rendered_host = if bare_host.contains(':') {
        format!("[{bare_host}]")
    } else {
        bare_host.clone()
    };
    let port = explicit_port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    let authority = if explicit_port.is_some() || bare_host.parse::<IpAddr>().is_ok() {
        format!("{rendered_host}:{port}")
    } else {
        rendered_host
    };
    let path = uri
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    if !path.starts_with('/') || path.contains('#') {
        return Err(ComputeManagementErrorV2::InvalidTarget);
    }
    Ok(format!("{scheme}://{authority}{path}"))
}

impl ComputeDecisionTargetV4 {
    pub fn new(
        endpoint: &str,
        wire_protocol: DecisionWireProtocolV2,
    ) -> Result<Self, ComputeManagementErrorV2> {
        let endpoint = canonical_decision_endpoint(endpoint)?;
        let authentication = DecisionTargetAuthenticationV1::Bearer;
        let digest = CanonicalDigest::of(&(&endpoint, wire_protocol, authentication))
            .map_err(|_| ComputeManagementErrorV2::InvalidDigest)?;
        Ok(Self {
            endpoint,
            wire_protocol,
            authentication,
            destination_id: format!("decision-target/{}", &digest.as_str()[7..]),
            consumers: Vec::new(),
        })
    }

    pub fn consumer(
        &self,
        source_id: &str,
        lineage: &CanonicalDigest,
        service_id: &str,
        revision: u64,
        model: &str,
    ) -> Result<DecisionTargetConsumerV1, ComputeManagementErrorV2> {
        validate_identifier(source_id)?;
        validate_identifier(service_id)?;
        validate_text(model)?;
        if revision == 0 || model.len() > 256 {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }
        let digest = CanonicalDigest::of(&(
            source_id,
            lineage,
            service_id,
            revision,
            model,
            &self.destination_id,
        ))
        .map_err(|_| ComputeManagementErrorV2::InvalidDigest)?;
        Ok(DecisionTargetConsumerV1 {
            service_id: service_id.into(),
            revision,
            model: model.into(),
            consumer_binding_id: format!("decision-consumer/{}", &digest.as_str()[7..]),
        })
    }

    pub(super) fn validate_for(
        &self,
        source_id: &str,
        lineage: &CanonicalDigest,
    ) -> Result<(), ComputeManagementErrorV2> {
        let expected = Self::new(&self.endpoint, self.wire_protocol)?;
        if self.endpoint != expected.endpoint
            || self.authentication != expected.authentication
            || self.destination_id != expected.destination_id
            || self.consumers.is_empty()
        {
            return Err(ComputeManagementErrorV2::InvalidTarget);
        }
        for consumer in &self.consumers {
            if self.consumer(
                source_id,
                lineage,
                &consumer.service_id,
                consumer.revision,
                &consumer.model,
            )? != *consumer
            {
                return Err(ComputeManagementErrorV2::InvalidTarget);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_is_exact_and_rejects_unsafe_or_ambiguous_authorities() {
        assert_eq!(
            canonical_decision_endpoint("https://EXAMPLE.test/v1/decisions?mode=x").unwrap(),
            "https://example.test/v1/decisions?mode=x"
        );
        assert_eq!(
            canonical_decision_endpoint("http://127.0.0.1/v1/decisions").unwrap(),
            "http://127.0.0.1:80/v1/decisions"
        );
        assert_eq!(
            canonical_decision_endpoint("http://[::1]:9999/v1/decisions").unwrap(),
            "http://[::1]:9999/v1/decisions"
        );
        for invalid in [
            "ftp://example.test/v1/decisions",
            "https://user@example.test/v1/decisions",
            "https://example.test:bad/v1/decisions",
            "https://example.test:0/v1/decisions",
            "https://example.test/v1/decisions#fragment",
        ] {
            assert!(canonical_decision_endpoint(invalid).is_err(), "{invalid}");
        }
    }
}
