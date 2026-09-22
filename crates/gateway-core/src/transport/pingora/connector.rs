use super::*;

#[derive(Clone)]
pub struct PingoraConnectorAdapter {
    registry: Arc<PingoraConnectorRegistry>,
}

impl Default for PingoraConnectorAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PingoraConnectorAdapter {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(PingoraConnectorRegistry::default()),
        }
    }

    pub fn attempt_transport(&self) -> PingoraClientSession {
        PingoraClientSession::new(Arc::clone(&self.registry), [0; 32])
    }

    pub async fn connect(
        &self,
        target: &TransportTarget,
        address: SocketAddr,
    ) -> Result<PingoraClientSession, AttemptError> {
        if target.requires_resolution() {
            return Err(AttemptError::InvalidTarget(
                "transport authority requires exact-target resolution".into(),
            ));
        }
        let mut transport = self.attempt_transport();
        AttemptTransport::connect(&mut transport, target, address).await?;
        Ok(transport)
    }

    pub async fn release(&self, mut session: PingoraClientSession) -> Result<(), AttemptError> {
        session.finish_accepted(true).await
    }
}

#[derive(Default)]
pub(super) struct PingoraConnectorRegistry {
    slots: Mutex<HashMap<TransportReuseClassId, ConnectorSlot>>,
}

struct ConnectorSlot {
    fingerprint: ConnectionEpochFingerprint,
    pool_compatibility_fingerprint: [u8; 32],
    connection_config_fingerprint: [u8; 32],
    connector: Arc<Connector>,
}

fn pool_compatibility_fingerprint(target: &TransportTarget) -> [u8; 32] {
    // A hostname is resolved for every request. DNS answers and their order
    // can change without a publication epoch change; Pingora's peer key still
    // includes the selected socket address.
    target.derive_pool_compatibility_fingerprint()
}

impl PingoraConnectorRegistry {
    pub(super) fn connector_for(
        &self,
        target: &TransportTarget,
        connection_config_fingerprint: [u8; 32],
    ) -> Result<Arc<Connector>, AttemptError> {
        let requested = target.connection_epoch_fingerprint();
        let pool_compatibility_fingerprint = pool_compatibility_fingerprint(target);
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(current) = slots.get(&requested.reuse_class) else {
            let connector = Arc::new(Connector::new(None));
            slots.insert(
                requested.reuse_class,
                ConnectorSlot {
                    fingerprint: requested,
                    pool_compatibility_fingerprint,
                    connection_config_fingerprint,
                    connector: Arc::clone(&connector),
                },
            );
            return Ok(connector);
        };
        if requested.pool_epoch < current.fingerprint.pool_epoch {
            // An already-bound old publication may reach connect after a newer
            // request rotated the class. Give it an isolated, non-pooled
            // generation without rolling the live slot backward.
            return Ok(Arc::new(Connector::new(None)));
        }
        if requested.pool_epoch == current.fingerprint.pool_epoch {
            if pool_compatibility_fingerprint != current.pool_compatibility_fingerprint {
                return Err(AttemptError::Transport(
                    "connection fingerprint changed without a pool epoch bump".into(),
                ));
            }
            if current.connection_config_fingerprint == connection_config_fingerprint {
                return Ok(Arc::clone(&current.connector));
            }
        }

        // Replacing the only registry Arc explicitly drains the retired
        // generation. Active sessions retain their Arc until bounded finish or
        // reset, after which the old Pingora pools are dropped as one unit.
        let connector = Arc::new(Connector::new(None));
        slots.insert(
            requested.reuse_class,
            ConnectorSlot {
                fingerprint: requested,
                pool_compatibility_fingerprint,
                connection_config_fingerprint,
                connector: Arc::clone(&connector),
            },
        );
        Ok(connector)
    }
}

impl AttemptTransportFactory for PingoraConnectorAdapter {
    type Transport = PingoraClientSession;

    fn create_transport(&self, connection_configs: &ConfigScopeSnapshot) -> Self::Transport {
        PingoraClientSession::new(
            Arc::clone(&self.registry),
            connection_configs.stable_fingerprint(),
        )
    }
}

pub(super) fn build_peer(
    target: &TransportTarget,
    address: SocketAddr,
    connection_config_fingerprint: [u8; 32],
) -> Result<HttpPeer, AttemptError> {
    if target.requires_resolution() {
        return Err(AttemptError::InvalidTarget(
            "transport authority requires exact-target resolution".into(),
        ));
    }
    let tls = target.scheme == TransportScheme::Https;
    let mut peer = HttpPeer::new(
        address,
        tls,
        target.sni.as_deref().unwrap_or_default().to_owned(),
    );
    peer.group_key = target.connection_reuse_key()
        ^ connection_config_fingerprint
            .chunks_exact(8)
            .enumerate()
            .fold(0x9e37_79b9_7f4a_7c15_u64, |key, (index, chunk)| {
                let word = u64::from_le_bytes(chunk.try_into().expect("eight-byte digest chunk"));
                key.rotate_left(17) ^ word.rotate_left((index as u32).saturating_mul(7))
            });
    peer.options.connection_timeout = Some(target.connect_timeout);
    peer.options.total_connection_timeout = Some(target.connect_timeout);
    peer.options.max_h2_streams = target.h2_max_concurrent_streams;
    peer.options.h2_stream_window_size = Some(target.h2_stream_window_bytes);
    peer.options.h2_connection_window_size = Some(target.h2_connection_window_bytes);
    peer.options.alpn = match target.alpn.as_ref() {
        [only] if only.as_ref() == "h2" => ALPN::H2,
        [only] if only.as_ref() == "http/1.1" => ALPN::H1,
        protocols
            if protocols.iter().any(|value| value.as_ref() == "h2")
                && protocols.iter().any(|value| value.as_ref() == "http/1.1") =>
        {
            ALPN::H2H1
        }
        _ => return Err(AttemptError::Transport("unsupported HTTP ALPN set".into())),
    };
    if let CaPolicy::Pem(pem) = &target.ca {
        let certificates = rustls_pemfile::certs(&mut Cursor::new(pem.as_ref()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AttemptError::Transport(error.to_string().into()))?;
        if certificates.is_empty() {
            return Err(AttemptError::Transport("custom CA bundle is empty".into()));
        }
        let wrapped: Vec<WrappedX509> = certificates
            .into_iter()
            .map(|certificate| WrappedX509::new(certificate.as_ref().to_vec(), parse_x509))
            .collect();
        peer.options.ca = Some(Arc::from(wrapped.into_boxed_slice()));
    }
    Ok(peer)
}
