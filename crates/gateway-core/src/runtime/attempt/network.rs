//! Admitted runtime dial route; never serialized into a publication or credential destination.
use super::*;
use crate::core::execution_plan::{CaPolicy, TransportScheme};

#[derive(Clone, Debug)]
pub struct NetworkDialRoute {
    pub scope: [u8; 32],
    pub identity: [u8; 32],
    pub generation: u64,
    pub proxy: Option<ProxyDialRoute>,
}

#[derive(Clone)]
pub struct ProxyDialRoute {
    pub scheme: TransportScheme,
    pub authority: Arc<str>,
    pub addresses: Arc<[SocketAddr]>,
    pub ca: CaPolicy,
    authorization: Option<HeaderValue>,
}

impl std::fmt::Debug for ProxyDialRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProxyDialRoute([REDACTED])")
    }
}

impl ProxyDialRoute {
    pub fn new(
        scheme: TransportScheme,
        authority: Arc<str>,
        addresses: Arc<[SocketAddr]>,
        ca: CaPolicy,
        mut authorization: Option<HeaderValue>,
    ) -> Result<Self, AttemptError> {
        let parsed = authority
            .parse::<http::uri::Authority>()
            .map_err(|_| AttemptError::InvalidTarget("proxy authority is invalid".into()))?;
        if authority.contains('@')
            || parsed.host().is_empty()
            || addresses.is_empty()
            || addresses.iter().any(|a| a.port() == 0)
        {
            return Err(AttemptError::InvalidTarget(
                "proxy dial target is invalid".into(),
            ));
        }
        if let Some(value) = &mut authorization {
            value.set_sensitive(true);
        }
        Ok(Self {
            scheme,
            authority,
            addresses,
            ca,
            authorization,
        })
    }

    pub fn authorization(&self) -> Option<&HeaderValue> {
        self.authorization.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProxyFailurePhase {
    Connect,
    Tls,
    Authentication,
    Negotiation,
    UpstreamTls,
}
