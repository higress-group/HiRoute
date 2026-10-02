use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hiroute_domain::{
    CanonicalDigest, GatewayPublicationRevision, PortError, PortErrorCode, PortResult,
    PreparePublicationOutcome, PublicationRecordV1, PublicationRepositoryPort, WorkspaceId,
};

use crate::compiler::test_fixtures::compiled_publication;

use super::*;

#[derive(Default)]
struct MemoryState {
    prepared: Option<PublicationRecordV1>,
    active: Option<PublicationRecordV1>,
    lkg: Option<PublicationRecordV1>,
}

#[derive(Default)]
struct MemoryRepository {
    state: Mutex<MemoryState>,
}

impl PublicationRepositoryPort for MemoryRepository {
    fn prepare_publication(
        &self,
        record: &PublicationRecordV1,
        expected_active_revision: Option<GatewayPublicationRevision>,
    ) -> PortResult<PreparePublicationOutcome> {
        let mut state = self.state.lock().unwrap();
        if state.prepared.as_ref() == Some(record) || state.active.as_ref() == Some(record) {
            return Ok(PreparePublicationOutcome::ExistingSame);
        }
        let active = state
            .active
            .as_ref()
            .map(|value| value.publication_revision);
        if active != expected_active_revision || state.prepared.is_some() {
            return Err(port(PortErrorCode::Conflict, "memory.prepare.cas"));
        }
        state.prepared = Some(record.clone());
        Ok(PreparePublicationOutcome::Created)
    }

    fn prepared_publication(
        &self,
        _workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>> {
        Ok(self.state.lock().unwrap().prepared.clone())
    }

    fn active_publication(
        &self,
        _workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>> {
        Ok(self.state.lock().unwrap().active.clone())
    }

    fn last_known_good_publication(
        &self,
        _workspace: &WorkspaceId,
    ) -> PortResult<Option<PublicationRecordV1>> {
        Ok(self.state.lock().unwrap().lkg.clone())
    }

    fn mark_publication_active(
        &self,
        _workspace: &WorkspaceId,
        publication_revision: GatewayPublicationRevision,
        digest: &CanonicalDigest,
    ) -> PortResult<()> {
        let mut state = self.state.lock().unwrap();
        if state.active.as_ref().is_some_and(|value| {
            value.publication_revision == publication_revision && &value.digest == digest
        }) {
            return Ok(());
        }
        let prepared = state
            .prepared
            .take()
            .filter(|value| {
                value.publication_revision == publication_revision && &value.digest == digest
            })
            .ok_or_else(|| port(PortErrorCode::Conflict, "memory.activate.identity"))?;
        state.lkg = state.active.take();
        state.active = Some(prepared);
        Ok(())
    }
}

struct FailOnceTarget {
    failed: AtomicBool,
    inner: Arc<AtomicPublicationTarget>,
}

impl FailOnceTarget {
    fn new(inner: Arc<AtomicPublicationTarget>) -> Self {
        Self {
            failed: AtomicBool::new(false),
            inner,
        }
    }
}

impl PublicationTargetPort for FailOnceTarget {
    fn activate_verified(
        &self,
        record: &PublicationRecordV1,
    ) -> Result<(), PublicationTargetError> {
        if !self.failed.swap(true, Ordering::AcqRel) {
            return Err(PublicationTargetError::Unavailable);
        }
        self.inner.activate_verified(record)
    }
}

#[test]
fn publication_prepared_target_failure_recovers_and_activates_once() {
    let workspace = WorkspaceId::default();
    let store = MemoryRepository::default();
    let target = Arc::new(AtomicPublicationTarget::default());
    let failing = FailOnceTarget::new(Arc::clone(&target));
    let publication = compiled_publication(11);
    let record = PublicationRecordV1::from_publication(workspace.clone(), &publication).unwrap();
    let activator = PublicationActivator::new(&store, &failing);
    assert!(matches!(
        activator.publish(&record, None),
        Err(PublicationActivationError::Target(
            PublicationTargetError::Unavailable
        ))
    ));
    assert!(target.pin_request().is_none());
    assert_eq!(
        activator.recover(&workspace).unwrap(),
        PublicationRecoveryOutcome::CompletedPrepared
    );
    assert_eq!(
        target.pin_request().unwrap().publication_revision,
        GatewayPublicationRevision::new(11).unwrap()
    );
}

#[test]
fn publication_request_arc_keeps_old_plan_revision_after_atomic_swap() {
    let workspace = WorkspaceId::default();
    let store = MemoryRepository::default();
    let target = AtomicPublicationTarget::default();
    let activator = PublicationActivator::new(&store, &target);
    let first = PublicationRecordV1::from_publication(workspace.clone(), &compiled_publication(11))
        .unwrap();
    activator.publish(&first, None).unwrap();
    let old_request = target.pin_request().unwrap();
    let old_plan_revisions = old_request
        .plans
        .iter()
        .map(|plan| plan.body.agent_plan_revision)
        .collect::<Vec<_>>();

    let second =
        PublicationRecordV1::from_publication(workspace, &compiled_publication(12)).unwrap();
    activator
        .publish(&second, Some(GatewayPublicationRevision::new(11).unwrap()))
        .unwrap();
    assert_eq!(
        old_request.publication_revision,
        GatewayPublicationRevision::new(11).unwrap()
    );
    assert_eq!(
        old_plan_revisions,
        old_request
            .plans
            .iter()
            .map(|plan| plan.body.agent_plan_revision)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        target.pin_request().unwrap().publication_revision,
        GatewayPublicationRevision::new(12).unwrap()
    );
}

#[test]
fn publication_retry_of_same_record_is_idempotent_after_activation() {
    let workspace = WorkspaceId::default();
    let store = MemoryRepository::default();
    let target = AtomicPublicationTarget::default();
    let activator = PublicationActivator::new(&store, &target);
    let record =
        PublicationRecordV1::from_publication(workspace, &compiled_publication(11)).unwrap();
    assert_eq!(
        activator.publish(&record, None).unwrap(),
        PublicationActivationOutcome::Activated
    );
    assert_eq!(
        activator.publish(&record, None).unwrap(),
        PublicationActivationOutcome::AlreadyPreparedActivated
    );
    assert_eq!(
        target.pin_request().unwrap().publication_revision,
        record.publication_revision
    );
}

#[test]
fn publication_tampering_is_rejected_before_target_activation() {
    let workspace = WorkspaceId::default();
    let publication = compiled_publication(11);
    let record = PublicationRecordV1::from_publication(workspace.clone(), &publication).unwrap();
    let original = serde_json::to_value(&publication).unwrap();
    let mut variants = Vec::new();

    let mut authority = original.clone();
    authority["authority_epoch"] = serde_json::json!(2);
    variants.push(authority);
    let mut renderer = original.clone();
    renderer["catalog_renderer_revision"] = serde_json::json!("renderer/v2");
    variants.push(renderer);
    let mut candidate = original.clone();
    candidate["aliases"][0]["candidates"][0]["endpoint"] =
        serde_json::json!("https://tampered.provider.invalid/v1/responses");
    variants.push(candidate);
    let mut credential = original.clone();
    credential["aliases"][0]["candidates"][0]["credential_ref"] =
        serde_json::json!("pool/tampered");
    variants.push(credential);
    let mut grant = original;
    grant["grants"][0]["generation"] = serde_json::json!(99);
    variants.push(grant);

    for value in variants {
        let mut tampered = record.clone();
        tampered.bytes = serde_json::to_vec(&value).unwrap().into();
        let store = MemoryRepository::default();
        let target = AtomicPublicationTarget::default();
        let activator = PublicationActivator::new(&store, &target);
        assert!(matches!(
            activator.publish(&tampered, None),
            Err(PublicationActivationError::VerificationFailed)
        ));
        assert!(target.pin_request().is_none());
    }
}

#[test]
fn publication_revision_and_grant_generation_invariants_are_independent() {
    let workspace = WorkspaceId::default();
    let store = MemoryRepository::default();
    let target = AtomicPublicationTarget::default();
    let activator = PublicationActivator::new(&store, &target);
    let active = compiled_publication(11);
    let first = PublicationRecordV1::from_publication(workspace.clone(), &active).unwrap();
    activator.publish(&first, None).unwrap();

    let mut same_revision = compiled_publication(11);
    same_revision.authority_epoch += 1;
    same_revision.validate().unwrap();
    let same_record =
        PublicationRecordV1::from_publication(workspace.clone(), &same_revision).unwrap();
    assert!(matches!(
        activator.publish(
            &same_record,
            Some(GatewayPublicationRevision::new(11).unwrap())
        ),
        Err(PublicationActivationError::InvalidTransition)
    ));

    let mut reused_generation = compiled_publication(12);
    reused_generation.grants[0].bearer_token_sha256 =
        CanonicalDigest::of_bytes(b"rotated-verifier-without-generation");
    reused_generation.validate().unwrap();
    assert_eq!(
        reused_generation
            .validate_transition_from(&active)
            .unwrap_err(),
        hiroute_domain::PublicationError::ImmutableGrantGenerationConflict
    );

    let mut changed_grants = crate::compiler::test_fixtures::publication_grants(&active.plans);
    changed_grants[0].generation += 1;
    // Rotating the grant's ingress protocol re-seals its route mapping over the Plans that
    // actually serve that protocol; the executable projection changes without new Plan bodies.
    changed_grants[0] = hiroute_domain::GatewayAccessGrantV1::new(
        changed_grants[0].grant_id.clone(),
        changed_grants[0].generation,
        changed_grants[0].bearer_token_sha256.clone(),
        hiroute_domain::AgentIngressProtocolV1::Messages,
        changed_grants[1].model_grant.clone(),
    )
    .unwrap();
    let changed_protocol = crate::compiler::compile_publication(
        workspace,
        active.authority_id.clone(),
        active.authority_epoch,
        GatewayPublicationRevision::new(12).unwrap(),
        active.catalog_renderer_revision.clone(),
        active.alias_registry.clone(),
        active.plans.clone(),
        changed_grants,
    )
    .unwrap();
    // Executable alias protocols are derived from the independently versioned grant. Rotating
    // that grant may change its projection without manufacturing a new immutable AgentPlan body.
    changed_protocol.validate_transition_from(&active).unwrap();
    assert_eq!(changed_protocol.plans, active.plans);
    assert_eq!(
        target.pin_request().unwrap().publication_revision,
        GatewayPublicationRevision::new(11).unwrap()
    );
}

#[test]
fn publication_rejects_reusing_an_immutable_plan_revision_for_new_bytes() {
    let workspace = WorkspaceId::default();
    let store = MemoryRepository::default();
    let target = AtomicPublicationTarget::default();
    let activator = PublicationActivator::new(&store, &target);
    let first = PublicationRecordV1::from_publication(workspace.clone(), &compiled_publication(11))
        .unwrap();
    activator.publish(&first, None).unwrap();

    // Revoking the last client grant removes the Gateway alias, not the immutable Product Plan.
    // A later client may reuse that Plan, but cannot redefine its content at the same revision.
    let mut disconnected = compiled_publication(12);
    disconnected = crate::compiler::compile_publication(
        workspace.clone(),
        disconnected.authority_id,
        disconnected.authority_epoch,
        disconnected.publication_revision,
        disconnected.catalog_renderer_revision,
        disconnected.alias_registry,
        disconnected.plans,
        Vec::new(),
    )
    .unwrap();
    assert!(disconnected.aliases.is_empty());
    let disconnected_record =
        PublicationRecordV1::from_publication(workspace.clone(), &disconnected).unwrap();
    activator
        .publish(
            &disconnected_record,
            Some(GatewayPublicationRevision::new(11).unwrap()),
        )
        .unwrap();

    let mut changed = compiled_publication(13);
    let index = changed
        .plans
        .iter()
        .position(|plan| plan.agent_plan_id().as_str() == "plan/smart")
        .unwrap();
    let mut body = changed.plans[index].body.as_ref().clone();
    body.identity.purpose =
        hiroute_domain::AgentPlanPurpose::parse("A changed purpose needs a new revision").unwrap();
    changed.plans[index] = hiroute_domain::CompiledAgentPlanV1::seal_current(body).unwrap();
    let changed_grants = crate::compiler::test_fixtures::publication_grants(&changed.plans);
    changed = crate::compiler::compile_publication(
        workspace.clone(),
        changed.authority_id,
        changed.authority_epoch,
        changed.publication_revision,
        changed.catalog_renderer_revision,
        changed.alias_registry,
        changed.plans,
        changed_grants,
    )
    .unwrap();
    let record = PublicationRecordV1::from_publication(workspace, &changed).unwrap();
    assert!(matches!(
        activator.publish(&record, Some(GatewayPublicationRevision::new(12).unwrap())),
        Err(PublicationActivationError::InvalidTransition)
    ));
    assert_eq!(
        target.pin_request().unwrap().publication_revision,
        GatewayPublicationRevision::new(12).unwrap()
    );
}

#[test]
fn publication_transition_requires_removed_aliases_to_be_tombstoned() {
    let active = compiled_publication(11);
    let mut next = compiled_publication(12);
    let removed_id = next.plans[0].agent_plan_id().clone();
    next.plans.remove(0);
    next.alias_registry.active.remove(&removed_id);
    let next_grants = crate::compiler::test_fixtures::publication_grants(&next.plans);
    let next = crate::compiler::compile_publication(
        WorkspaceId::default(),
        next.authority_id,
        next.authority_epoch,
        next.publication_revision,
        next.catalog_renderer_revision,
        next.alias_registry,
        next.plans,
        next_grants,
    )
    .unwrap();
    assert_eq!(
        next.validate_transition_from(&active).unwrap_err(),
        hiroute_domain::PublicationError::AliasLifecycleConflict
    );
}

#[test]
fn publication_corrupt_active_falls_back_to_last_known_good() {
    let workspace = WorkspaceId::default();
    let store = MemoryRepository::default();
    let first_target = AtomicPublicationTarget::default();
    let activator = PublicationActivator::new(&store, &first_target);
    let first = PublicationRecordV1::from_publication(workspace.clone(), &compiled_publication(11))
        .unwrap();
    activator.publish(&first, None).unwrap();
    let second =
        PublicationRecordV1::from_publication(workspace.clone(), &compiled_publication(12))
            .unwrap();
    activator
        .publish(&second, Some(GatewayPublicationRevision::new(11).unwrap()))
        .unwrap();
    std::sync::Arc::make_mut(&mut store.state.lock().unwrap().active.as_mut().unwrap().bytes)[0] ^=
        1;

    let recovered_target = AtomicPublicationTarget::default();
    let recovery = PublicationActivator::new(&store, &recovered_target);
    assert_eq!(
        recovery.recover(&workspace).unwrap(),
        PublicationRecoveryOutcome::RestoredLastKnownGood
    );
    assert_eq!(
        recovered_target.pin_request().unwrap().publication_revision,
        GatewayPublicationRevision::new(11).unwrap()
    );
}

fn port(code: PortErrorCode, context: &'static str) -> PortError {
    PortError::new(code, context)
}

#[test]
fn atomic_publication_readiness_distinguishes_installed_from_serving() {
    let target = AtomicPublicationTarget::default();
    assert!(target.observes_verified(None).unwrap());
    let publication = compiled_publication(1);
    let record =
        PublicationRecordV1::from_publication(publication.workspace_id.clone(), &publication)
            .unwrap();
    target.activate_verified(&record).unwrap();
    assert!(target.observes_verified(Some(&record)).unwrap());
    target.suspend_requests().unwrap();
    assert!(target.verify_installed(&record).unwrap());
    assert!(!target.observes_verified(Some(&record)).unwrap());
    assert!(target.pin_request().is_none());
    target.resume_requests().unwrap();
    assert!(target.observes_verified(Some(&record)).unwrap());
}
