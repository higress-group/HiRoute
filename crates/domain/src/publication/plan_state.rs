//! Product V3 keeps configured grants intact while projecting only enabled ordinary aliases.
use super::*;
use crate::{PlanHeadV1, PlanLifecycleV1};

pub const GATEWAY_PUBLICATION_SCHEMA_V3: &str = "hiroute.gateway-publication/v3";

impl GatewayPublicationV1 {
    pub fn next_with_plan_content(
        &self,
        revision: GatewayPublicationRevision,
        aliases: AliasRegistryV1,
        plan: CompiledAgentPlanV1,
        heads: Vec<PlanHeadV1>,
    ) -> Result<Self, PublicationError> {
        self.validate()?;
        let mut next = self.clone();
        next.schema = GATEWAY_PUBLICATION_SCHEMA_V3.into();
        next.compiler_revision = crate::AGENT_PLAN_COMPILER_REVISION_V3.into();
        next.publication_revision = revision;
        next.alias_registry = aliases;
        next.plans
            .retain(|p| p.agent_plan_id() != plan.agent_plan_id());
        next.plans.push(
            plan.into_current()
                .map_err(|_| PublicationError::InvalidPlan)?,
        );
        next.plans = next
            .plans
            .into_iter()
            .map(|plan| {
                plan.into_current()
                    .map_err(|_| PublicationError::InvalidPlan)
            })
            .collect::<Result<Vec<_>, _>>()?;
        next.plans
            .sort_by(|a, b| a.agent_plan_id().cmp(b.agent_plan_id()));
        next.plan_heads = heads;
        next.plan_heads
            .sort_by(|a, b| a.reference.plan_id.cmp(&b.reference.plan_id));
        next.aliases = materialize_aliases(&next.plans, &next.enabled_grants()?)?;
        if self.plans.is_empty()
            && self.grants.is_empty()
            && self.publication_revision.get() == 1
            && revision.get() == 1
        {
            // An unpersisted initial bootstrap is a constructor seed, not an active revision.
            next.validate()?;
        } else {
            next.validate_transition_from(self)?;
        }
        Ok(next)
    }

    pub fn next_with_plan_lifecycle(
        &self,
        revision: GatewayPublicationRevision,
        head: PlanHeadV1,
    ) -> Result<Self, PublicationError> {
        self.validate()?;
        let previous = self
            .plan_heads
            .iter()
            .find(|h| h.reference.plan_id == head.reference.plan_id)
            .ok_or(PublicationError::InvalidPlanSet)?;
        if previous.reference != head.reference
            || previous.model_alias != head.model_alias
            || previous.head_revision.checked_add(1) != Some(head.head_revision)
            || previous.status == PlanLifecycleV1::Deleted
            || previous.status == head.status
        {
            return Err(PublicationError::InvalidTransition);
        }
        let mut next = self.clone();
        next.schema = GATEWAY_PUBLICATION_SCHEMA_V3.into();
        next.compiler_revision = crate::AGENT_PLAN_COMPILER_REVISION_V3.into();
        next.plans = next
            .plans
            .into_iter()
            .map(|plan| {
                plan.into_current()
                    .map_err(|_| PublicationError::InvalidPlan)
            })
            .collect::<Result<Vec<_>, _>>()?;
        next.publication_revision = revision;
        if head.status == PlanLifecycleV1::Deleted {
            // Referencing configured grants must be explicitly removed by their owner first.
            if next
                .grants
                .iter()
                .any(|g| g.permits_plan(&head.model_alias))
            {
                return Err(PublicationError::InvalidGrant);
            }
            next.alias_registry
                .retire(&head.reference.plan_id)
                .map_err(|_| PublicationError::InvalidAliasRegistry)?;
            next.plans
                .retain(|p| p.agent_plan_id() != &head.reference.plan_id);
        }
        let slot = next
            .plan_heads
            .iter_mut()
            .find(|h| h.reference.plan_id == head.reference.plan_id)
            .ok_or(PublicationError::InvalidPlanSet)?;
        *slot = head;
        next.aliases = materialize_aliases(&next.plans, &next.enabled_grants()?)?;
        next.validate_transition_from(self)?;
        Ok(next)
    }

    pub(super) fn validate_plan_heads(&self) -> Result<(), PublicationError> {
        if self.schema != GATEWAY_PUBLICATION_SCHEMA_V3 {
            return if self.plan_heads.is_empty() {
                Ok(())
            } else {
                Err(PublicationError::UnsupportedSchema)
            };
        }
        if self.plan_heads.is_empty() {
            return Ok(());
        }
        let mut ids = BTreeSet::new();
        let mut previous = None;
        for head in &self.plan_heads {
            head.validate().map_err(|_| PublicationError::InvalidPlan)?;
            if head.reference.workspace_id != self.workspace_id
                || !ids.insert(&head.reference.plan_id)
                || previous.is_some_and(|id| id >= &head.reference.plan_id)
            {
                return Err(PublicationError::InvalidPlanSet);
            }
            previous = Some(&head.reference.plan_id);
            let plan = self
                .plans
                .iter()
                .find(|p| p.agent_plan_id() == &head.reference.plan_id);
            if head.status == PlanLifecycleV1::Deleted {
                if plan.is_some()
                    || !self
                        .alias_registry
                        .retired_plan_ids
                        .contains(&head.reference.plan_id)
                    || !self.alias_registry.tombstones.contains(&head.model_alias)
                {
                    return Err(PublicationError::InvalidPlanSet);
                }
            } else if !plan.is_some_and(|p| {
                p.body.agent_plan_revision == head.reference.content_revision
                    && p.model_alias() == &head.model_alias
            }) {
                return Err(PublicationError::InvalidPlanSet);
            }
        }
        if self.plans.iter().any(|p| !ids.contains(p.agent_plan_id())) {
            return Err(PublicationError::InvalidPlanSet);
        }
        Ok(())
    }

    pub(super) fn enabled_grants(&self) -> Result<Vec<GatewayExecutableGrantV2>, PublicationError> {
        if self.schema != GATEWAY_PUBLICATION_SCHEMA_V3 || self.plan_heads.is_empty() {
            return Ok(self.grants.clone());
        }
        let allowed = self
            .plan_heads
            .iter()
            .filter(|h| h.status == PlanLifecycleV1::Enabled)
            .map(|h| &h.model_alias)
            .collect::<BTreeSet<_>>();
        let mut enabled = Vec::new();
        for grant in &self.grants {
            let mut routes = grant.model_grant.routes.clone();
            routes.retain(|_, route| match route {
                crate::AgentModelRouteV2::Plan { alias, .. } => allowed.contains(alias),
                crate::AgentModelRouteV2::Fixed { .. } => true,
            });
            if !routes.is_empty() {
                let mut projected = grant.clone();
                let mut protocols = grant.model_grant.route_protocols.clone();
                protocols.retain(|name, _| routes.contains_key(name));
                projected.model_grant = crate::AgentModelGrantV2::seal_routes(
                    grant.model_grant.protocol,
                    routes,
                    protocols,
                )
                .map_err(|_| PublicationError::InvalidGrant)?;
                enabled.push(projected);
            }
        }
        Ok(enabled)
    }

    pub(super) fn preserve_heads(mut self, source: &Self) -> Result<Self, PublicationError> {
        if source.schema == GATEWAY_PUBLICATION_SCHEMA_V3 {
            self.schema = GATEWAY_PUBLICATION_SCHEMA_V3.into();
            self.plan_heads = source.plan_heads.clone();
            self.aliases = materialize_aliases(&self.plans, &self.enabled_grants()?)?;
            self.validate()?;
        }
        Ok(self)
    }

    pub(super) fn validate_head_transition(&self, active: &Self) -> Result<(), PublicationError> {
        if active.schema == GATEWAY_PUBLICATION_SCHEMA_V3
            && self.schema != GATEWAY_PUBLICATION_SCHEMA_V3
        {
            return Err(PublicationError::InvalidTransition);
        }
        for old in &active.plan_heads {
            let new = self
                .plan_heads
                .iter()
                .find(|h| h.reference.plan_id == old.reference.plan_id)
                .ok_or(PublicationError::InvalidTransition)?;
            if new.head_revision < old.head_revision
                || (new.head_revision == old.head_revision && new != old)
                || (old.status == PlanLifecycleV1::Deleted && new != old)
            {
                return Err(PublicationError::InvalidTransition);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_publication_with_heads() -> GatewayPublicationV1 {
        let legacy: GatewayPublicationV1 = serde_json::from_str(include_str!(
            "../../../../e2e/product/fixtures/routing/current-publication.v3.json"
        ))
        .unwrap();
        let heads = legacy
            .plans
            .iter()
            .cloned()
            .map(|plan| {
                let version = crate::PlanVersionV1::from_unversioned_compiled_recovery(
                    legacy.workspace_id.clone(),
                    plan,
                )
                .unwrap();
                PlanHeadV1 {
                    head_revision: version.reference.content_revision,
                    model_alias: version.compiled.model_alias().clone(),
                    reference: version.reference,
                    status: PlanLifecycleV1::Enabled,
                }
            })
            .collect();
        let mut current = legacy.into_current().unwrap();
        current.plan_heads = heads;
        current.validate_current_contract().unwrap();
        current
    }
    fn publication() -> GatewayPublicationV1 {
        let current = current_publication_with_heads();
        current
            .next_with_plan_content(
                GatewayPublicationRevision::new(12).unwrap(),
                current.alias_registry.clone(),
                current.plans[0].clone(),
                current.plan_heads.clone(),
            )
            .unwrap()
    }
    #[test]
    fn lifecycle_write_keeps_every_compiled_plan_current() {
        let current = current_publication_with_heads();
        let mut head = current.plan_heads[0].clone();
        head.head_revision += 1;
        head.status = PlanLifecycleV1::Disabled;
        let next = current
            .next_with_plan_lifecycle(GatewayPublicationRevision::new(12).unwrap(), head)
            .unwrap();
        next.validate_current_contract().unwrap();
        assert!(
            next.plans
                .iter()
                .all(|plan| plan.body.schema == crate::AGENT_PLAN_COMPILED_SCHEMA_V3)
        );
    }
    #[test]
    fn disable_projects_no_new_calls_without_revoking_configured_grants() {
        let mut current = publication();
        let grants = current.grants.clone();
        let saved = current.plans.clone();
        for original in current.plan_heads.clone() {
            let mut head = original;
            head.head_revision += 1;
            head.status = PlanLifecycleV1::Disabled;
            current = current
                .next_with_plan_lifecycle(
                    GatewayPublicationRevision::new(current.publication_revision.get() + 1)
                        .unwrap(),
                    head,
                )
                .unwrap();
        }
        assert_eq!(current.plans, saved);
        assert_eq!(current.grants, grants);
        assert!(current.aliases.is_empty());
        let projection = current.gateway_snapshot().unwrap();
        assert_eq!(projection.admission, GatewayAdmissionStateV1::NoNewCalls);
        assert!(projection.aliases.is_empty() && projection.grants.is_empty());
        assert_eq!(
            GatewayPublicationV1::decode(&current.canonical_bytes().unwrap()).unwrap(),
            current
        );
        let mut head = current.plan_heads[0].clone();
        let reference = head.reference.clone();
        head.status = PlanLifecycleV1::Enabled;
        head.head_revision += 1;
        let restored = current
            .next_with_plan_lifecycle(
                GatewayPublicationRevision::new(current.publication_revision.get() + 1).unwrap(),
                head,
            )
            .unwrap();
        assert_eq!(restored.plan_heads[0].reference, reference);
        assert!(!restored.aliases.is_empty());
        assert_eq!(restored.grants, grants);
    }
    #[test]
    fn referenced_delete_and_state_downgrade_fail_closed() {
        let current = publication();
        let mut head = current.plan_heads[0].clone();
        head.head_revision += 1;
        head.status = PlanLifecycleV1::Deleted;
        assert_eq!(
            current.next_with_plan_lifecycle(GatewayPublicationRevision::new(13).unwrap(), head),
            Err(PublicationError::InvalidGrant)
        );
        let mut downgraded = current.clone();
        downgraded.schema = LEGACY_GATEWAY_PUBLICATION_SCHEMA_V2.into();
        assert!(downgraded.validate().is_err());
        let mut inconsistent = current;
        inconsistent.plan_heads[0].reference.content_revision += 1;
        assert!(inconsistent.validate().is_err());
    }
}
