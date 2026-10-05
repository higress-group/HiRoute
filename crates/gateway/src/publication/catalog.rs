use std::sync::Arc;

use bytes::Bytes;
use http::StatusCode;
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::PublishedGatewayPublication;
use crate::server::composition::RuntimePublicationFeed;
use crate::server::request_plan::IngressProtocol;

#[derive(Clone)]
pub struct GatewayCatalog {
    publications: Arc<dyn RuntimePublicationFeed>,
}

impl GatewayCatalog {
    pub fn new<F>(publications: Arc<F>) -> Self
    where
        F: RuntimePublicationFeed + 'static,
    {
        Self::from_feed(publications)
    }

    pub fn from_feed(publications: Arc<dyn RuntimePublicationFeed>) -> Self {
        Self { publications }
    }

    pub fn get(
        &self,
        authorization: Option<&str>,
        if_none_match: Option<&str>,
    ) -> Result<CatalogResponse, CatalogError> {
        let publication = self
            .publications
            .pin()
            .ok_or(CatalogError::PublicationUnavailable)?;
        render(&publication, authorization, if_none_match)
    }
}

fn render(
    publication: &PublishedGatewayPublication,
    authorization: Option<&str>,
    if_none_match: Option<&str>,
) -> Result<CatalogResponse, CatalogError> {
    let grant = publication
        .authenticate_bearer(authorization.ok_or(CatalogError::Unauthorized)?)
        .ok_or(CatalogError::Unauthorized)?;
    let mut models = Vec::new();
    for (name, route) in &grant.routes {
        let (purpose, revision) = match route {
            super::CompiledGrantRoute::Plan { alias } => {
                let Some(alias) = publication.aliases.get(alias) else {
                    continue;
                };
                if !alias
                    .execution
                    .protocols
                    .contains(&grant.protocol_for(name))
                {
                    continue;
                }
                (
                    Some(alias.purpose.as_ref()),
                    Some(alias.agent_plan_revision),
                )
            }
            super::CompiledGrantRoute::Fixed { .. } => (None, None),
        };
        models.push(CatalogModel {
            id: name,
            object: "model",
            created: 0,
            owned_by: "hiroute",
            purpose,
            agent_plan_revision: revision,
            protocols: vec![grant.protocol_for(name)],
        });
    }
    let document = CatalogDocument {
        object: "list",
        data: models,
    };
    let body = serde_json::to_vec(&document).map_err(CatalogError::Json)?;
    let etag_projection = CatalogEtagProjection {
        authority_id: publication.authority_id(),
        authority_epoch: publication.authority_epoch(),
        publication_revision: publication.publication_revision(),
        publication_digest: publication.payload_digest(),
        renderer_revision: publication.catalog_renderer_revision(),
        grant_id: &grant.grant_id,
        grant_generation: grant.generation,
        body_sha256: format!("sha256:{:x}", Sha256::digest(&body)),
    };
    let etag_bytes = serde_json::to_vec(&etag_projection).map_err(CatalogError::Json)?;
    let etag = format!("\"hiroute-{:x}\"", Sha256::digest(etag_bytes));
    if if_none_match == Some(etag.as_str()) {
        return Ok(CatalogResponse {
            status: StatusCode::NOT_MODIFIED,
            etag,
            body: Bytes::new(),
        });
    }
    Ok(CatalogResponse {
        status: StatusCode::OK,
        etag,
        body: Bytes::from(body),
    })
}

#[derive(Clone, Debug)]
pub struct CatalogResponse {
    pub status: StatusCode,
    pub etag: String,
    pub body: Bytes,
}

#[derive(Serialize)]
struct CatalogDocument<'a> {
    object: &'static str,
    data: Vec<CatalogModel<'a>>,
}

#[derive(Serialize)]
struct CatalogModel<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    owned_by: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    purpose: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_plan_revision: Option<u64>,
    protocols: Vec<IngressProtocol>,
}

#[derive(Serialize)]
struct CatalogEtagProjection<'a> {
    authority_id: &'a str,
    authority_epoch: u64,
    publication_revision: u64,
    publication_digest: &'a str,
    renderer_revision: &'a str,
    grant_id: &'a str,
    grant_generation: u64,
    body_sha256: String,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("gateway publication is unavailable")]
    PublicationUnavailable,
    #[error("gateway grant is missing or invalid")]
    Unauthorized,
    #[error("catalog JSON failed: {0}")]
    Json(serde_json::Error),
}
