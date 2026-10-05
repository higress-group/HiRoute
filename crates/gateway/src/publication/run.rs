//! Ephemeral, exact-plan publication handles for one delegated Worker run.
//!
//! A Worker run must never mutate or masquerade as the aggregate publication.  This module
//! compiles one already-sealed, single-alias snapshot into an isolated core publication and
//! keeps that core alive for the request pins issued against the run.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hiroute_domain::CanonicalDigest;
use hiroute_gateway_core::core::publication::{PrepareOutcome, PublicationInstaller};
use thiserror::Error;

use super::compiler::{CompiledGrant, compile};
use super::{GatewayPublicationSnapshotV3, PublishedGatewayPublication};
use crate::server::request_plan::IngressProtocol;

const RUN_CORE_INSTALL_DEADLINE: Duration = Duration::from_secs(5);

/// A Gateway-owned immutable exact-plan closure for one Worker run.
///
/// Construction is deliberately stricter than an ordinary aggregate publication: there is one
/// alias, one grant, one ingress protocol, and one high-entropy run-token verifier.  The daemon
/// may hold this opaque handle, but cannot inspect or bind its compiler-owned internals.
pub struct RunPublicationHandle {
    publication: Arc<PublishedGatewayPublication>,
    // Active core bindings are request-owned Arcs today.  Retaining the installer makes that
    // lifetime an explicit invariant rather than depending on an implementation detail.
    _core_installer: Arc<PublicationInstaller>,
    model_alias: Arc<str>,
    protocol: IngressProtocol,
}

impl RunPublicationHandle {
    pub fn compile_exact(
        snapshot: GatewayPublicationSnapshotV3,
        model_alias: &str,
        protocol: IngressProtocol,
        token_fingerprint: &CanonicalDigest,
    ) -> Result<Arc<Self>, RunPublicationError> {
        snapshot
            .validate()
            .map_err(|_| RunPublicationError::Invalid)?;
        let [alias] = snapshot.aliases.as_slice() else {
            return Err(RunPublicationError::Invalid);
        };
        let [grant] = snapshot.grants.as_slice() else {
            return Err(RunPublicationError::Invalid);
        };
        if alias.served_model_id != model_alias
            || alias.protocols.len() != 1
            || alias.protocols[0] != protocol
            || grant.protocol != protocol
            || !grant.route_protocols.is_empty()
            || grant.routes.len() != 1
            || !matches!(grant.routes.get(model_alias), Some(super::schema::ModelRouteV2::Plan { alias, .. }) if alias == model_alias)
            || grant.bearer_token_sha256 != token_fingerprint.as_str()
        {
            return Err(RunPublicationError::Invalid);
        }

        let compiled = compile(&snapshot).map_err(|_| RunPublicationError::Unavailable)?;
        let core_installer = Arc::new(PublicationInstaller::new());
        let prepared = match core_installer
            .prepare_uncancelled(
                compiled.envelope,
                Instant::now() + RUN_CORE_INSTALL_DEADLINE,
            )
            .map_err(|_| RunPublicationError::Unavailable)?
        {
            PrepareOutcome::Prepared(prepared) => prepared,
            PrepareOutcome::Duplicate(_) => return Err(RunPublicationError::Unavailable),
        };
        let core = core_installer
            .publish_uncancelled(prepared, Instant::now() + RUN_CORE_INSTALL_DEADLINE)
            .map_err(|_| RunPublicationError::Unavailable)?;
        Ok(Arc::new(Self {
            publication: Arc::new(PublishedGatewayPublication::from_ephemeral(
                Arc::new(snapshot),
                compiled.aliases,
                compiled.grants,
                core,
            )),
            _core_installer: core_installer,
            model_alias: Arc::from(model_alias),
            protocol,
        }))
    }

    pub(crate) fn authenticate(&self, authorization: &str) -> Option<CompiledGrant> {
        self.publication.authenticate_bearer(authorization)
    }

    pub(crate) fn publication(&self) -> Arc<PublishedGatewayPublication> {
        Arc::clone(&self.publication)
    }

    pub(crate) fn model_alias(&self) -> &str {
        &self.model_alias
    }

    pub(crate) const fn protocol(&self) -> IngressProtocol {
        self.protocol
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RunPublicationError {
    #[error("delegated run publication is invalid")]
    Invalid,
    #[error("delegated run publication cannot be compiled")]
    Unavailable,
}
