use super::*;

macro_rules! numeric_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

numeric_id!(PlanRevision);
numeric_id!(ConfigRevision);
numeric_id!(ConfigCellId);
numeric_id!(ConfigGeneration);
numeric_id!(AtomicityGroupId);
numeric_id!(ListenerEpoch);
numeric_id!(PoolEpoch);
numeric_id!(TransportReuseClassId);

pub const DEFAULT_OVERALL_REQUEST_TIMEOUT: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AdapterId(Arc<str>);

impl AdapterId {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, PlanError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(PlanError::EmptyAdapterId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable, non-secret credential selector. Secret bytes never enter a
/// compiled plan or a decision grant.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct CredentialRef(Arc<str>);

impl CredentialRef {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, PlanError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(PlanError::EmptyCredentialRef);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct AuthorityId(Arc<str>);

impl AuthorityId {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, PlanError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(PlanError::EmptyAuthority);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StableTargetKey(Arc<str>);

impl StableTargetKey {
    pub fn new(value: impl Into<Arc<str>>) -> Result<Self, PlanError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(PlanError::EmptyStableTargetKey);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque execution binding that is only meaningful within one plan revision.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTargetBindingId {
    plan_revision: PlanRevision,
    local_id: u32,
}

impl ResolvedTargetBindingId {
    pub fn new(plan_revision: PlanRevision, local_id: u32) -> Self {
        Self {
            plan_revision,
            local_id,
        }
    }

    pub fn plan_revision(self) -> PlanRevision {
        self.plan_revision
    }

    pub fn local_id(self) -> u32 {
        self.local_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TransportScheme {
    Http,
    Https,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CaPolicy {
    System,
    Pem(Arc<[u8]>),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ConnectionEpochFingerprint {
    pub reuse_class: TransportReuseClassId,
    pub pool_epoch: PoolEpoch,
    pub digest: [u8; 32],
}

const fn default_transport_read_buffer_bytes() -> usize {
    64 * 1024
}

const fn default_h2_stream_window_bytes() -> u32 {
    64 * 1024
}

const fn default_h2_connection_window_bytes() -> u32 {
    256 * 1024
}

const fn default_h2_max_concurrent_streams() -> usize {
    16
}

const RESOLUTION_REQUIRED_AUTHORITY_PREFIX: &str = "hiroute+unresolved://";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransportTarget {
    pub scheme: TransportScheme,
    pub authority: Arc<str>,
    pub addresses: Arc<[SocketAddr]>,
    pub sni: Option<Arc<str>>,
    pub ca: CaPolicy,
    pub alpn: Arc<[Arc<str>]>,
    pub connect_timeout: Duration,
    /// Maximum opaque H1/TCP codec body buffer charged before a read.
    #[serde(default = "default_transport_read_buffer_bytes")]
    pub transport_read_buffer_bytes: usize,
    /// Pingora H2 receive-flow settings. They are reuse compatibility inputs
    /// and their per-stream share is reserved in the hierarchical body budget.
    #[serde(default = "default_h2_stream_window_bytes")]
    pub h2_stream_window_bytes: u32,
    #[serde(default = "default_h2_connection_window_bytes")]
    pub h2_connection_window_bytes: u32,
    #[serde(default = "default_h2_max_concurrent_streams")]
    pub h2_max_concurrent_streams: usize,
    pub reuse_class: TransportReuseClassId,
    pub pool_epoch: PoolEpoch,
    /// Compiler-sealed identity of every property that may affect connection
    /// compatibility. Publication validation rejects stale or forged values.
    pub connection_fingerprint: [u8; 32],
}

impl TransportTarget {
    /// Marks a parsed hostname as a non-executable authority while preserving
    /// its exact host:port text for the later request-authorized resolver.
    pub fn mark_resolution_required(authority: impl AsRef<str>) -> Arc<str> {
        format!(
            "{RESOLUTION_REQUIRED_AUTHORITY_PREFIX}{}",
            authority.as_ref()
        )
        .into()
    }

    pub fn requires_resolution(&self) -> bool {
        self.authority
            .starts_with(RESOLUTION_REQUIRED_AUTHORITY_PREFIX)
    }

    pub fn unresolved_authority(&self) -> Option<&str> {
        self.authority
            .strip_prefix(RESOLUTION_REQUIRED_AUTHORITY_PREFIX)
    }

    /// Returns true only for the executable socket shape used by a supervised
    /// local connector. This deliberately excludes hostnames, TLS, multiple
    /// addresses, and non-loopback aliases.
    pub fn is_numeric_loopback_http(&self) -> bool {
        if self.scheme != TransportScheme::Http
            || self.sni.is_some()
            || self.ca != CaPolicy::System
            || self.addresses.len() != 1
        {
            return false;
        }
        let Ok(authority) = self.authority.parse::<SocketAddr>() else {
            return false;
        };
        authority.ip().is_loopback()
            && authority.port() != 0
            && self.addresses.first() == Some(&authority)
    }

    /// PROCESS-22006 handoff: create a new executable exact-target value only
    /// after the authorized candidate has been resolved. The publication-
    /// owned unresolved value remains immutable and non-executable.
    pub fn with_resolved_addresses(
        mut self,
        addresses: Arc<[SocketAddr]>,
    ) -> Result<Self, PlanError> {
        if addresses.is_empty() {
            return Err(PlanError::NoTransportAddress);
        }
        if let Some(authority) = self.unresolved_authority().map(ToOwned::to_owned) {
            self.authority = authority.into();
        }
        self.addresses = addresses;
        self.connection_fingerprint = self.derive_connection_fingerprint();
        self.validate()?;
        Ok(self)
    }

    pub fn transport_codec_reservation_bytes(&self) -> usize {
        if self.alpn.iter().any(|protocol| protocol.as_ref() == "h2") {
            let connection_share =
                (self.h2_connection_window_bytes as usize).div_ceil(self.h2_max_concurrent_streams);
            (self.h2_stream_window_bytes as usize).saturating_add(connection_share)
        } else {
            self.transport_read_buffer_bytes
        }
    }
    pub fn with_derived_connection_fingerprint(mut self) -> Self {
        self.connection_fingerprint = self.derive_connection_fingerprint();
        self
    }

    pub fn connection_epoch_fingerprint(&self) -> ConnectionEpochFingerprint {
        ConnectionEpochFingerprint {
            reuse_class: self.reuse_class,
            pool_epoch: self.pool_epoch,
            digest: self.connection_fingerprint,
        }
    }

    /// Stable Pingora extension key for connection-compatible target settings.
    /// Pingora also hashes the selected socket address, so a changed DNS answer
    /// list cannot redirect an existing connection to a newly selected IP.
    pub fn connection_reuse_key(&self) -> u64 {
        self.derive_pool_compatibility_fingerprint()
            .chunks_exact(8)
            .enumerate()
            .fold(0x517c_c1b7_2722_0a95_u64, |key, (index, chunk)| {
                let word = u64::from_le_bytes(chunk.try_into().expect("eight-byte digest chunk"));
                key.rotate_left(13) ^ word.rotate_left((index as u32).saturating_mul(11))
            })
    }

    pub(crate) fn derive_pool_compatibility_fingerprint(&self) -> [u8; 32] {
        self.derive_connection_fingerprint_for_addresses(&[])
    }

    pub fn derive_connection_fingerprint(&self) -> [u8; 32] {
        self.derive_connection_fingerprint_for_addresses(self.addresses.as_ref())
    }

    fn derive_connection_fingerprint_for_addresses(&self, addresses: &[SocketAddr]) -> [u8; 32] {
        fn update_field(hasher: &mut Sha256, bytes: &[u8]) {
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }

        let mut hasher = Sha256::new();
        hasher.update(b"hiroute-connection-fingerprint-v2");
        hasher.update([match self.scheme {
            TransportScheme::Http => 0,
            TransportScheme::Https => 1,
        }]);
        update_field(&mut hasher, self.authority.as_bytes());
        hasher.update((addresses.len() as u64).to_le_bytes());
        for address in addresses {
            update_field(&mut hasher, address.to_string().as_bytes());
        }
        match &self.sni {
            Some(sni) => {
                hasher.update([1]);
                update_field(&mut hasher, sni.as_bytes());
            }
            None => hasher.update([0]),
        }
        match &self.ca {
            CaPolicy::System => hasher.update([0]),
            CaPolicy::Pem(pem) => {
                hasher.update([1]);
                update_field(&mut hasher, pem);
            }
        }
        hasher.update((self.alpn.len() as u64).to_le_bytes());
        for protocol in self.alpn.iter() {
            update_field(&mut hasher, protocol.as_bytes());
        }
        hasher.update((self.transport_read_buffer_bytes as u64).to_le_bytes());
        hasher.update(self.h2_stream_window_bytes.to_le_bytes());
        hasher.update(self.h2_connection_window_bytes.to_le_bytes());
        hasher.update((self.h2_max_concurrent_streams as u64).to_le_bytes());
        hasher.update(self.reuse_class.0.to_le_bytes());
        hasher.update(self.pool_epoch.0.to_le_bytes());
        hasher.finalize().into()
    }

    pub fn validate(&self) -> Result<(), PlanError> {
        if self.authority.trim().is_empty()
            || self
                .unresolved_authority()
                .is_some_and(|authority| authority.trim().is_empty())
        {
            return Err(PlanError::EmptyTransportAuthority);
        }
        if self.requires_resolution() {
            if self.addresses.is_empty() {
                // Exact DNS and connect are deliberately deferred.
            } else {
                return Err(PlanError::TransportAddressStateMismatch);
            }
        } else if self.addresses.is_empty() {
            return Err(PlanError::NoTransportAddress);
        }
        if self.connect_timeout.is_zero() {
            return Err(PlanError::ZeroTransportTimeout);
        }
        if self.transport_read_buffer_bytes == 0
            || self.h2_stream_window_bytes == 0
            || self.h2_connection_window_bytes < self.h2_stream_window_bytes
            || self.h2_max_concurrent_streams == 0
        {
            return Err(PlanError::InvalidTransportFlowControl);
        }
        if self.scheme == TransportScheme::Https
            && self.sni.as_deref().is_none_or(|sni| sni.trim().is_empty())
        {
            return Err(PlanError::MissingTlsSni);
        }
        if self.scheme == TransportScheme::Http && self.sni.is_some() {
            return Err(PlanError::TlsOptionsOnPlainHttp);
        }
        if self.scheme == TransportScheme::Http && matches!(self.ca, CaPolicy::Pem(_)) {
            return Err(PlanError::TlsOptionsOnPlainHttp);
        }
        if matches!(&self.ca, CaPolicy::Pem(pem) if pem.is_empty()) {
            return Err(PlanError::EmptyCaBundle);
        }
        if self.alpn.is_empty() {
            return Err(PlanError::EmptyAlpnSet);
        }
        let mut seen_alpn = std::collections::HashSet::new();
        for protocol in self.alpn.iter() {
            if !matches!(protocol.as_ref(), "h2" | "http/1.1") {
                return Err(PlanError::UnsupportedAlpn(protocol.clone()));
            }
            if !seen_alpn.insert(protocol.as_ref()) {
                return Err(PlanError::DuplicateAlpn(protocol.clone()));
            }
        }
        if self.reuse_class.0 == 0 || self.pool_epoch.0 == 0 {
            return Err(PlanError::InvalidTransportReuseMetadata);
        }
        if self.connection_fingerprint == [0; 32]
            || self.connection_fingerprint != self.derive_connection_fingerprint()
        {
            return Err(PlanError::InvalidConnectionFingerprint);
        }
        Ok(())
    }
}
