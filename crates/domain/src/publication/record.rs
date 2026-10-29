use std::sync::{Arc, OnceLock};

use super::*;

/// The proof belongs to these exact immutable bytes, never to a workspace or an epoch.
#[derive(schemars::JsonSchema, Clone, Debug, Serialize)]
pub struct PublicationRecordV1 {
    pub workspace_id: WorkspaceId,
    pub publication_revision: GatewayPublicationRevision,
    pub digest: CanonicalDigest,
    #[serde(serialize_with = "crate::operation::shared_input::serialize")]
    pub bytes: Arc<Vec<u8>>,
    #[serde(skip)]
    verified: Arc<VerifiedRecord>,
}

#[derive(Debug)]
struct VerifiedRecord {
    bytes: Arc<Vec<u8>>,
    digest: CanonicalDigest,
    publication: GatewayPublicationV1,
    gateway: OnceLock<Result<Arc<GatewayPublicationSnapshotProjectionV3>, PublicationError>>,
}

impl PartialEq for PublicationRecordV1 {
    fn eq(&self, other: &Self) -> bool {
        self.workspace_id == other.workspace_id
            && self.publication_revision == other.publication_revision
            && self.digest == other.digest
            && self.bytes == other.bytes
    }
}
impl Eq for PublicationRecordV1 {}

impl<'de> Deserialize<'de> for PublicationRecordV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            workspace_id: WorkspaceId,
            publication_revision: GatewayPublicationRevision,
            digest: CanonicalDigest,
            bytes: Vec<u8>,
        }
        let value = Encoded::deserialize(deserializer)?;
        Self::from_parts(
            value.workspace_id,
            value.publication_revision,
            value.digest,
            value.bytes,
        )
        .map_err(serde::de::Error::custom)
    }
}

impl PublicationRecordV1 {
    pub fn from_publication(
        workspace_id: WorkspaceId,
        publication: &GatewayPublicationV1,
    ) -> Result<Self, PublicationError> {
        if workspace_id != publication.workspace_id {
            return Err(PublicationError::InvalidWorkspace);
        }
        let publication = super::StoredPublicationV1::freeze(publication)?.build()?;
        let bytes = publication.canonical_bytes()?;
        Ok(Self::from_verified(bytes, publication))
    }

    /// Storage and untrusted inputs always enter through complete authentication.
    pub fn from_parts(
        workspace_id: WorkspaceId,
        revision: GatewayPublicationRevision,
        digest: CanonicalDigest,
        bytes: Vec<u8>,
    ) -> Result<Self, PublicationError> {
        let publication = authenticate(&workspace_id, revision, &digest, &bytes)?;
        Ok(Self::from_verified(bytes, publication))
    }

    fn from_verified(bytes: Vec<u8>, publication: GatewayPublicationV1) -> Self {
        let bytes = Arc::new(bytes);
        let digest = CanonicalDigest::of_bytes(&bytes);
        Self {
            workspace_id: publication.workspace_id.clone(),
            publication_revision: publication.publication_revision,
            digest: digest.clone(),
            bytes: bytes.clone(),
            verified: Arc::new(VerifiedRecord {
                bytes,
                digest,
                publication,
                gateway: OnceLock::new(),
            }),
        }
    }

    pub fn verify(&self) -> Result<GatewayPublicationV1, PublicationError> {
        if self.matches_proof() {
            return Ok(self.verified.publication.clone());
        }
        authenticate(
            &self.workspace_id,
            self.publication_revision,
            &self.digest,
            &self.bytes,
        )
    }

    fn matches_proof(&self) -> bool {
        Arc::ptr_eq(&self.bytes, &self.verified.bytes)
            && self.digest == self.verified.digest
            && self.workspace_id == self.verified.publication.workspace_id
            && self.publication_revision == self.verified.publication.publication_revision
    }

    /// Reuse a pure projection of these authenticated immutable bytes. This says
    /// nothing about current installation, suspension, grants or recovery state.
    /// Detached/mutated record fields never inherit the original projection.
    pub fn gateway_snapshot(
        &self,
    ) -> Result<Arc<GatewayPublicationSnapshotProjectionV3>, PublicationError> {
        if self.matches_proof() {
            return self
                .verified
                .gateway
                .get_or_init(|| self.verified.publication.gateway_snapshot().map(Arc::new))
                .clone();
        }
        self.verify()?.gateway_snapshot().map(Arc::new)
    }

    pub fn verify_current(&self) -> Result<GatewayPublicationV1, PublicationError> {
        let publication = self.verify()?;
        publication.check_current_schema()?;
        Ok(publication)
    }
}

fn authenticate(
    workspace_id: &WorkspaceId,
    revision: GatewayPublicationRevision,
    digest: &CanonicalDigest,
    bytes: &[u8],
) -> Result<GatewayPublicationV1, PublicationError> {
    if CanonicalDigest::of_bytes(bytes) != *digest {
        return Err(PublicationError::DigestMismatch);
    }
    let publication = GatewayPublicationV1::decode_persisted(bytes)?;
    if publication.workspace_id != *workspace_id {
        return Err(PublicationError::InvalidWorkspace);
    }
    if publication.publication_revision != revision {
        return Err(PublicationError::InvalidRevision);
    }
    Ok(publication)
}
