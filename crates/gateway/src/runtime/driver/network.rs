use crate::ports::ExecutionScope;
use base64::Engine;
use hiroute_domain::{CanonicalDigest, NativeNetworkDestinationV1, NativeNetworkSnapshotV1};
use hiroute_gateway_core::{
    core::execution_plan::{CaPolicy, TransportScheme, TransportTarget},
    runtime::attempt::{NetworkDialRoute, ProxyDialRoute},
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

/// Network policy remains independent of the publication-owned original target and Key grant.
pub(super) async fn resolve(
    snapshot: &NativeNetworkSnapshotV1,
    binding: &str,
    uri: &str,
    target: TransportTarget,
    scope: &ExecutionScope,
) -> Result<(Option<NetworkDialRoute>, TransportTarget), Arc<str>> {
    let unavailable = || Arc::from("network_policy_unavailable");
    let destination = NativeNetworkDestinationV1::from_url(uri).map_err(|_| unavailable())?;
    let (record, route) = snapshot
        .resolve_binding(binding, &destination)
        .map_err(|_| unavailable())?;
    materialize(record, route, target, scope).await
}

pub(crate) async fn resolve_source(
    snapshot: &NativeNetworkSnapshotV1,
    source: &str,
    expected_lineage: &CanonicalDigest,
    uri: &str,
    target: TransportTarget,
    scope: &ExecutionScope,
) -> Result<(Option<NetworkDialRoute>, TransportTarget), Arc<str>> {
    let unavailable = || Arc::from("network_policy_unavailable");
    let record = snapshot.source(source).map_err(|_| unavailable())?;
    if &record.lineage_digest != expected_lineage {
        return Err(unavailable());
    }
    let destination = NativeNetworkDestinationV1::from_url(uri).map_err(|_| unavailable())?;
    let route = snapshot
        .resolve_source(source, &destination)
        .map_err(|_| unavailable())?;
    materialize(record, route, target, scope).await
}

async fn materialize(
    record: &hiroute_domain::NativeSourceNetworkRecordV1,
    route: hiroute_domain::ResolvedNativeNetworkRouteV1,
    target: TransportTarget,
    scope: &ExecutionScope,
) -> Result<(Option<NetworkDialRoute>, TransportTarget), Arc<str>> {
    let unavailable = || Arc::from("network_policy_unavailable");
    let scope_id = CanonicalDigest::of(&(record.source_id.as_str(), &record.lineage_digest))
        .map_err(|_| unavailable())?;
    let identity = CanonicalDigest::of(&(
        &scope_id,
        record.network_revision,
        &record.policy,
        &route.environment_epoch,
    ))
    .map_err(|_| unavailable())?;
    let mut network = NetworkDialRoute {
        scope: digest_bytes(&scope_id),
        identity: digest_bytes(&identity),
        generation: record.network_revision,
        proxy: None,
    };
    let Some(proxy) = route.proxy else {
        return Ok((Some(network), target));
    };
    let addresses: Arc<[SocketAddr]> = if let Ok(ip) = proxy.host.parse::<IpAddr>() {
        Arc::from([SocketAddr::new(ip, proxy.port)])
    } else {
        scope
            .run(tokio::net::lookup_host((proxy.host.as_str(), proxy.port)))
            .await
            .map_err(|_| Arc::from("proxy_connect_cancelled"))?
            .map_err(|_| Arc::from("proxy_dns_failed"))?
            .take(32)
            .collect::<Vec<_>>()
            .into()
    };
    if addresses.is_empty() {
        return Err(Arc::from("proxy_dns_failed"));
    }
    let authority: Arc<str> = if proxy.host.contains(':') {
        format!("[{}]:{}", proxy.host, proxy.port).into()
    } else {
        format!("{}:{}", proxy.host, proxy.port).into()
    };
    let authorization = proxy
        .basic_auth_material()
        .map(|material| {
            let value = zeroize::Zeroizing::new(format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(material.expose())
            ));
            http::HeaderValue::from_str(&value).map_err(|_| unavailable())
        })
        .transpose()?;
    network.proxy = Some(
        ProxyDialRoute::new(
            if proxy.scheme == "https" {
                TransportScheme::Https
            } else {
                TransportScheme::Http
            },
            authority,
            Arc::clone(&addresses),
            CaPolicy::System,
            authorization,
        )
        .map_err(|_| unavailable())?,
    );
    let target = if target.requires_resolution() {
        target
            .with_resolved_addresses(addresses)
            .map_err(|_| unavailable())?
    } else {
        target
    };
    Ok((Some(network), target))
}

fn digest_bytes(digest: &CanonicalDigest) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(digest.as_str().as_bytes()).into()
}
