//! The durable publication contains choices and grants; executable aliases are projections.
use super::*;

pub const STORED_PUBLICATION_SCHEMA: &str = "hiroute.stored-publication/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPublicationV1 {
    pub schema: String,
    pub workspace_id: WorkspaceId,
    pub authority_id: String,
    pub authority_epoch: u64,
    pub publication_revision: GatewayPublicationRevision,
    pub alias_registry: AliasRegistryV1,
    pub plans: Vec<crate::StoredPlanV1>,
    pub plan_heads: Vec<crate::PlanHeadV1>,
    pub grants: Vec<GatewayExecutableGrantV2>,
}

impl StoredPublicationV1 {
    pub fn freeze(publication: &GatewayPublicationV1) -> Result<Self, PublicationError> {
        publication.validate()?;
        Ok(Self {
            schema: STORED_PUBLICATION_SCHEMA.into(),
            workspace_id: publication.workspace_id.clone(),
            authority_id: publication.authority_id.clone(),
            authority_epoch: publication.authority_epoch,
            publication_revision: publication.publication_revision,
            alias_registry: publication.alias_registry.clone(),
            plans: publication
                .plans
                .iter()
                .map(|p| crate::StoredPlanV1::freeze(p).map_err(|_| PublicationError::InvalidPlan))
                .collect::<Result<_, _>>()?,
            plan_heads: publication.plan_heads.clone(),
            grants: publication.grants.clone(),
        })
    }

    pub fn build(&self) -> Result<GatewayPublicationV1, PublicationError> {
        if self.schema != STORED_PUBLICATION_SCHEMA {
            return Err(PublicationError::UnsupportedSchema);
        }
        let plans = self
            .plans
            .iter()
            .map(|p| p.build().map_err(|_| PublicationError::InvalidPlan))
            .collect::<Result<_, _>>()?;
        // Do not silently rebind stored grants. A current writer already freezes the exact routes.
        let mut publication = GatewayPublicationV1::from_parts(
            self.workspace_id.clone(),
            self.authority_id.clone(),
            self.authority_epoch,
            self.publication_revision,
            DEFAULT_CATALOG_RENDERER_REVISION.into(),
            self.alias_registry.clone(),
            plans,
            self.grants.clone(),
        )?;
        if publication.grants != self.grants {
            return Err(PublicationError::InvalidGrant);
        }
        publication.plan_heads = self.plan_heads.clone();
        publication.aliases =
            materialize_aliases(&publication.plans, &publication.enabled_grants()?)?;
        publication.validate()?;
        if Self::freeze(&publication)? != *self {
            return Err(PublicationError::InvalidExecutableProjection);
        }
        Ok(publication)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_fixture_is_a_deterministic_projection_of_stable_business_facts() {
        let stored: StoredPublicationV1 = serde_json::from_str(include_str!(
            "../../../../e2e/product/fixtures/routing/stored-publication.v1.json"
        ))
        .unwrap();
        let publication: GatewayPublicationV1 = serde_json::from_str(include_str!(
            "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
        ))
        .unwrap();
        publication.validate_current_contract().unwrap();
        assert_eq!(StoredPublicationV1::freeze(&publication).unwrap(), stored);
        let rebuilt = stored.build().unwrap();
        assert_eq!(StoredPublicationV1::freeze(&rebuilt).unwrap(), stored);
        assert_eq!(
            rebuilt.canonical_bytes().unwrap(),
            publication.canonical_bytes().unwrap()
        );
        assert_eq!(stored.build().unwrap(), rebuilt);
        let mut corrupt = publication.plans[0].clone();
        std::sync::Arc::make_mut(&mut corrupt.body).agent_plan_revision += 1;
        assert!(corrupt.validate().is_err());
    }
}
