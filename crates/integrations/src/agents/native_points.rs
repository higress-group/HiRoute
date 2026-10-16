//! Private native binding provenance. Public scans expose opaque references, never these fields.
use hiroute_domain::{CanonicalDigest, UpstreamProtocol};
use serde::Serialize;

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NativeApiPointBindingV1 {
    pub native_scope: CanonicalDigest,
    pub provider_definition: String,
    pub endpoint: String,
    pub authentication: hiroute_domain::GatewayAuthenticationSemanticsV1,
    pub endpoint_selector: String,
    pub protocol_selector: String,
    pub model_override_scope: Option<String>,
    /// Selected native selector and precedence branch; contains no credential value/digest.
    pub credential_binding: String,
    pub configured_default_model_id: Option<String>,
}

#[derive(Clone)]
pub struct NativeApiConnectionPointV2 {
    pub key_digest: CanonicalDigest,
    pub snapshot_digest: CanonicalDigest,
    pub sources: Vec<super::NativeApiSource>,
}
impl NativeApiConnectionPointV2 {
    pub fn revision(&self) -> u64 {
        u64::from_str_radix(&self.snapshot_digest.as_str()[7..23], 16)
            .expect("canonical digest")
            .max(1)
    }
    pub fn protocol(&self) -> Option<UpstreamProtocol> {
        self.sources.first().and_then(|s| s.protocol)
    }
}

/// Group only leaf-proven bindings. Model handles and resolved secret bytes are not identities.
pub fn native_api_connection_points(
    sources: Vec<super::NativeApiSource>,
) -> Vec<NativeApiConnectionPointV2> {
    let mut points = std::collections::BTreeMap::<String, Vec<super::NativeApiSource>>::new();
    for source in sources {
        let Some(binding) = &source.point_binding else {
            continue;
        };
        let key = CanonicalDigest::of(&(
            "native-api-connection-point/v2",
            &binding.native_scope,
            &binding.provider_definition,
            &binding.endpoint,
            source.protocol,
            &binding.authentication,
            &binding.model_override_scope,
            &binding.credential_binding,
            source.supported_auth,
        ))
        .expect("bounded native binding");
        points
            .entry(key.as_str().to_owned())
            .or_default()
            .push(source);
    }
    points
        .into_iter()
        .map(|(key, mut sources)| {
            sources.sort_by(|a, b| a.model_id.cmp(&b.model_id));
            let snapshot_digest = CanonicalDigest::of(&(
                "native-api-connection-point-snapshot/v2",
                &key,
                sources
                    .iter()
                    .map(|s| (s, &s.point_binding))
                    .collect::<Vec<_>>(),
            ))
            .expect("bounded native source facts");
            NativeApiConnectionPointV2 {
                key_digest: CanonicalDigest::parse(key).expect("canonical key"),
                snapshot_digest,
                sources,
            }
        })
        .collect()
}
