//! Capture active publication bytes under a read snapshot; authenticate them outside
//! the shared storage lock before using them for any status or authority decision.
use super::*;
use hiroute_domain::{GatewayPublicationRevision, PublicationRecordV1};

fn invalid() -> PortError {
    port(PortErrorCode::Corrupt, "publication.snapshot")
}

pub struct CapturedActivePublication {
    workspace: WorkspaceId,
    revision: u64,
    digest: String,
    bytes: Vec<u8>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ActivePublicationIdentity {
    workspace: WorkspaceId,
    revision: u64,
    digest: String,
}

impl CapturedActivePublication {
    pub fn identity(&self) -> ActivePublicationIdentity {
        ActivePublicationIdentity {
            workspace: self.workspace.clone(),
            revision: self.revision,
            digest: self.digest.clone(),
        }
    }
    pub fn verify(self) -> PortResult<PublicationRecordV1> {
        PublicationRecordV1::from_parts(
            self.workspace,
            GatewayPublicationRevision::new(self.revision).map_err(|_| invalid())?,
            CanonicalDigest::parse(self.digest).map_err(|_| invalid())?,
            self.bytes,
        )
        .map_err(|_| invalid())
    }
}

impl ControlStore {
    pub fn capture_active_publication(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<CapturedActivePublication>> {
        let db = self.connection.borrow();
        if db.is_autocommit() {
            return Err(port(
                PortErrorCode::Conflict,
                "publication.snapshot.transaction",
            ));
        }
        db.query_row(
            "SELECT publication_revision,digest,publication_bytes FROM gateway_publications WHERE workspace_id=?1 AND state='active'",
            [workspace.as_str()], |r| Ok(CapturedActivePublication { workspace: workspace.clone(), revision: r.get(0)?, digest: r.get(1)?, bytes: r.get(2)? }),
        ).optional().map_err(|_| invalid())
    }

    pub fn active_publication_identity(
        &self,
        workspace: &WorkspaceId,
    ) -> PortResult<Option<ActivePublicationIdentity>> {
        let db = self.connection.borrow();
        db.query_row(
            "SELECT publication_revision,digest FROM gateway_publications WHERE workspace_id=?1 AND state='active'",
            [workspace.as_str()], |r| Ok(ActivePublicationIdentity { workspace: workspace.clone(), revision: r.get(0)?, digest: r.get(1)? }),
        ).optional().map_err(|_| invalid())
    }
}
