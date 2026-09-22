use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use hiroute_application_api::{ApplyRequestV1, PreviewRequestV1, PrincipalKind};
use hiroute_domain::{
    BeginOperationOutcome, CredentialPoolMutationV1, CredentialRefV1, ExternalEffectIntentV1,
    IdempotencyScopeV1, PortErrorCode, PortResult, ProtectedApplyCapability, ProtectedSecret,
    RevisionSetV1, RuntimeMutationV1, SecretMutationV1, VerifiedApplyAuthorizationV1,
    VerifiedSecretSubjectV1,
};
use serde_json::{Value, json};

use super::*;

mod admission;
mod settings_tail;

#[derive(Clone)]
struct Grant {
    principal: String,
    workspace: WorkspaceId,
    operation_kind: String,
    accepted: CanonicalDigest,
    revisions: CanonicalDigest,
    revoked: bool,
    expired: bool,
    consumed: Option<String>,
}

#[derive(Default)]
struct MemoryState {
    operations: HashMap<String, OperationV1>,
    idempotency: HashMap<String, String>,
    grants: HashMap<String, Grant>,
    effects: HashMap<String, (OwnedEffectV1, bool)>,
    compensated: HashSet<String>,
    tamper_after_activate: Option<String>,
    writer: Option<String>,
    durable_writes: usize,
    operation_reads: usize,
    secret_reads: usize,
    external_reads: usize,
    protected_reads: usize,
    staged_pool: Option<CredentialPoolMutationV1>,
    compute_pool: Option<hiroute_domain::CredentialPoolV1>,
    credential_reference_count: Option<u64>,
    input: Vec<u8>,
    activation_log: Vec<String>,
    fail_activation: Option<String>,
    publication_displaced: bool,
    publication_revision: u64,
    host_login_item_observed: bool,
    surface_checks: HashMap<(String, String), hiroute_domain::AgentSurfaceCheckRecordV1>,
}

#[derive(Default)]
struct MemoryPorts {
    state: RefCell<MemoryState>,
}

impl MemoryPorts {
    fn with_input(input: &[u8]) -> Self {
        let ports = Self::default();
        ports.state.borrow_mut().input = input.to_vec();
        ports
    }

    fn grant(&self, capability: &str, accepted: &CanonicalDigest, revisions: &RevisionSetV1) {
        self.grant_for(capability, accepted, revisions, "ApplySetup");
    }

    fn grant_for(
        &self,
        capability: &str,
        accepted: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation_kind: &str,
    ) {
        let digest = CanonicalDigest::of_bytes(capability.as_bytes());
        self.state.borrow_mut().grants.insert(
            digest.as_str().to_owned(),
            Grant {
                principal: "interactive-user".to_owned(),
                workspace: WorkspaceId::default(),
                operation_kind: operation_kind.to_owned(),
                accepted: accepted.clone(),
                revisions: CanonicalDigest::of(revisions).unwrap(),
                revoked: false,
                expired: false,
                consumed: None,
            },
        );
    }

    fn counters(&self) -> (usize, usize, usize, usize) {
        let state = self.state.borrow();
        (
            state.durable_writes,
            state.secret_reads,
            state.external_reads,
            state.protected_reads,
        )
    }

    fn idempotency_key(workspace: &WorkspaceId, scope: &IdempotencyScopeV1) -> String {
        format!(
            "{}\0{}\0{}\0{}",
            workspace, scope.principal, scope.operation_kind, scope.key
        )
    }

    fn effect_key(operation_id: &OperationId, effect_id: &str) -> String {
        format!("{operation_id}\0{effect_id}")
    }

    fn stage(&self, operation_id: &OperationId, effect: OwnedEffectV1) -> OwnedEffectV1 {
        let key = Self::effect_key(operation_id, &effect.effect_id);
        let mut state = self.state.borrow_mut();
        state.compensated.remove(&key);
        state.effects.insert(key, (effect.clone(), false));
        effect
    }

    fn observe(&self, operation_id: &OperationId, effect_id: &str) -> EffectReconciliation {
        match self
            .state
            .borrow()
            .effects
            .get(&Self::effect_key(operation_id, effect_id))
            .cloned()
        {
            Some((effect, false)) => EffectReconciliation::Staged(effect),
            Some((effect, true)) => EffectReconciliation::Applied(effect),
            None => EffectReconciliation::Missing,
        }
    }

    fn activate(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        let operation = effect
            .compensation
            .get("operation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| PortError::new(PortErrorCode::InvalidData, "test.effect.operation"))?;
        let operation = OperationId::parse(operation)
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "test.effect.operation"))?;
        let key = Self::effect_key(&operation, &effect.effect_id);
        let mut state = self.state.borrow_mut();
        if state.fail_activation.as_deref() == Some(effect.effect_id.as_str()) {
            return Err(PortError::new(
                PortErrorCode::Unavailable,
                "test.activation.failed",
            ));
        }
        let record = state
            .effects
            .get_mut(&key)
            .ok_or_else(|| PortError::new(PortErrorCode::NotFound, "test.effect.missing"))?;
        record.1 = true;
        let activated = record.0.clone();
        state.activation_log.push(effect.effect_id.to_owned());
        if state.tamper_after_activate.as_deref() == Some(effect.effect_id.as_str()) {
            state.effects.remove(&key);
        }
        Ok(activated)
    }

    fn compensate(&self, effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        let operation = effect
            .compensation
            .get("operation_id")
            .and_then(Value::as_str)
            .ok_or_else(|| PortError::new(PortErrorCode::InvalidData, "test.effect.operation"))?;
        let operation = OperationId::parse(operation)
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "test.effect.operation"))?;
        let key = Self::effect_key(&operation, &effect.effect_id);
        let mut state = self.state.borrow_mut();
        if state.effects.remove(&key).is_some() {
            state.compensated.insert(key);
            Ok(CompensationOutcome::Compensated)
        } else if state.compensated.contains(&key) {
            Ok(CompensationOutcome::AlreadyCompensated)
        } else {
            Err(PortError::new(
                PortErrorCode::NotFound,
                "test.effect.missing",
            ))
        }
    }
}

impl ControlRepositoryPort for MemoryPorts {
    fn operation_is_current(&self, operation: &OperationV1) -> PortResult<bool> {
        Ok(self
            .state
            .borrow()
            .operations
            .get(operation.operation_id.as_str())
            .is_some_and(|stored| stored == operation))
    }

    fn current_revisions(&self, _workspace: &WorkspaceId) -> PortResult<RevisionSetV1> {
        Ok(RevisionSetV1 {
            target: 0,
            dependencies: BTreeMap::new(),
        })
    }

    fn operation_for_idempotency(
        &self,
        workspace: &WorkspaceId,
        scope: &IdempotencyScopeV1,
    ) -> PortResult<Option<OperationV1>> {
        let state = self.state.borrow();
        Ok(state
            .idempotency
            .get(&Self::idempotency_key(workspace, scope))
            .and_then(|id| state.operations.get(id))
            .cloned())
    }

    fn verify_apply_authorization(
        &self,
        capability: &ProtectedApplyCapability,
        workspace: &WorkspaceId,
        principal: &str,
        operation_kind: &str,
        accepted_digest: &CanonicalDigest,
        expected_revisions: &RevisionSetV1,
    ) -> PortResult<VerifiedApplyAuthorizationV1> {
        let digest = CanonicalDigest::of_bytes(capability.expose());
        let revisions = CanonicalDigest::of(expected_revisions).unwrap();
        let state = self.state.borrow();
        let grant = state
            .grants
            .get(digest.as_str())
            .ok_or_else(|| denied("test.capability.missing"))?;
        if grant.principal != principal
            || grant.workspace != *workspace
            || grant.operation_kind != operation_kind
            || grant.accepted != *accepted_digest
            || grant.revisions != revisions
            || grant.revoked
            || grant.expired
            || grant.consumed.is_some()
        {
            return Err(denied("test.capability.denied"));
        }
        VerifiedApplyAuthorizationV1::from_capability_verifier(
            digest,
            principal,
            workspace.clone(),
            operation_kind,
            accepted_digest.clone(),
            revisions,
            "apply:one-shot",
            2,
        )
        .map_err(|_| denied("test.capability.invalid"))
    }

    fn begin_operation(
        &self,
        operation: &OperationV1,
        authorization: &VerifiedApplyAuthorizationV1,
    ) -> PortResult<BeginOperationOutcome> {
        let mut state = self.state.borrow_mut();
        let key = Self::idempotency_key(&operation.workspace_id, &operation.idempotency);
        if let Some(id) = state.idempotency.get(&key)
            && let Some(existing) = state.operations.get(id)
        {
            return if existing.accepted_digest == operation.accepted_digest {
                Ok(BeginOperationOutcome::ExistingSame(Box::new(
                    existing.clone(),
                )))
            } else {
                Ok(BeginOperationOutcome::ExistingDifferent)
            };
        }
        if state.writer.is_some() || !authorization.matches_operation(operation) {
            return Err(PortError::new(PortErrorCode::Conflict, "test.writer.claim"));
        }
        let grant = state
            .grants
            .get_mut(authorization.capability_digest().as_str())
            .ok_or_else(|| denied("test.capability.missing"))?;
        if grant.consumed.is_some() || grant.revoked || grant.expired {
            return Err(denied("test.capability.consumed"));
        }
        grant.consumed = Some(operation.operation_id.to_string());
        state.writer = Some(operation.operation_id.to_string());
        state
            .idempotency
            .insert(key, operation.operation_id.to_string());
        state
            .operations
            .insert(operation.operation_id.to_string(), operation.clone());
        state.durable_writes += 1;
        Ok(BeginOperationOutcome::Created)
    }

    fn load_operation(&self, operation_id: &OperationId) -> PortResult<Option<OperationV1>> {
        self.state.borrow_mut().operation_reads += 1;
        Ok(self
            .state
            .borrow()
            .operations
            .get(operation_id.as_str())
            .cloned())
    }

    fn recoverable_operations(&self) -> PortResult<Vec<OperationV1>> {
        let state = self.state.borrow();
        let writer = state.writer.as_deref();
        Ok(state
            .operations
            .values()
            .filter(|operation| {
                !operation.state.is_terminal()
                    || (operation.state == OperationState::NeedsAttention
                        && writer == Some(operation.operation_id.as_str()))
            })
            .cloned()
            .collect())
    }

    fn writer_recovery_required(&self) -> PortResult<bool> {
        Ok(self.state.borrow().writer.is_some())
    }

    fn save_operation(&self, operation: &mut OperationV1) -> PortResult<()> {
        operation.generation += 1;
        let mut state = self.state.borrow_mut();
        state
            .operations
            .insert(operation.operation_id.to_string(), operation.clone());
        state.durable_writes += 1;
        Ok(())
    }

    fn apply_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
        _expected_revision: u64,
        _desired: &Value,
    ) -> PortResult<OwnedEffectV1> {
        Ok(self.stage(
            operation_id,
            fake_effect(
                operation_id,
                &format!("control:{workspace}"),
                workspace.as_str(),
                OwnedEffectKind::Control,
            ),
        ))
    }

    fn observe_control(
        &self,
        operation_id: &OperationId,
        workspace: &WorkspaceId,
    ) -> PortResult<EffectReconciliation> {
        Ok(self.observe(operation_id, &format!("control:{workspace}")))
    }

    fn activate_control(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.activate(effect)
    }

    fn compensate_control(&self, _effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        self.compensate(_effect)
    }

    fn finish_operation(&self, operation: &mut OperationV1) -> PortResult<u64> {
        self.save_operation(operation)?;
        if operation.state != OperationState::NeedsAttention {
            self.state.borrow_mut().writer = None;
        }
        Ok(0)
    }

    fn save_operation_tail(&self, operation: &mut OperationV1) -> PortResult<()> {
        if operation.state.is_terminal() {
            return Err(PortError::new(
                PortErrorCode::InvalidData,
                "test.tail.terminal",
            ));
        }
        self.save_operation(operation)?;
        let mut state = self.state.borrow_mut();
        match &state.writer {
            Some(holder) if holder == operation.operation_id.as_str() => state.writer = None,
            Some(_) => return Err(PortError::new(PortErrorCode::Conflict, "test.tail.claim")),
            None => {}
        }
        Ok(())
    }

    fn reclaim_operation_writer(&self, operation_id: &OperationId) -> PortResult<()> {
        let mut state = self.state.borrow_mut();
        match &state.writer {
            Some(holder) if holder == operation_id.as_str() => {}
            Some(_) => {
                return Err(PortError::new(
                    PortErrorCode::Conflict,
                    "test.tail.reclaim.busy",
                ));
            }
            None => state.writer = Some(operation_id.as_str().to_string()),
        }
        Ok(())
    }

    fn save_agent_surface_check(
        &self,
        _workspace: &WorkspaceId,
        record: &hiroute_domain::AgentSurfaceCheckRecordV1,
    ) -> PortResult<bool> {
        record
            .validate()
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "test.surface-check"))?;
        let surface = surface_key(&record.surface);
        self.state
            .borrow_mut()
            .surface_checks
            .insert((record.context_id.clone(), surface), record.clone());
        Ok(true)
    }

    fn agent_surface_checks(
        &self,
        _workspace: &WorkspaceId,
        context_id: &str,
    ) -> PortResult<Vec<hiroute_domain::AgentSurfaceCheckRecordV1>> {
        Ok(self
            .state
            .borrow()
            .surface_checks
            .iter()
            .filter(|((record_context, _), _)| record_context == context_id)
            .map(|(_, record)| record.clone())
            .collect())
    }
}

fn surface_key(surface: &hiroute_domain::AgentModelSurfaceV2) -> String {
    serde_json::to_value(surface)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl SecretStorePort for MemoryPorts {
    fn generation(&self, _credential: &CredentialRefV1) -> PortResult<u64> {
        self.state.borrow_mut().secret_reads += 1;
        Ok(0)
    }

    fn fingerprint(&self, secret: &ProtectedSecret) -> PortResult<CanonicalDigest> {
        Ok(CanonicalDigest::of_bytes(secret.expose()))
    }

    fn resolve_secret(
        &self,
        _subject: &VerifiedSecretSubjectV1,
        _credential: &CredentialRefV1,
        _purpose: &str,
        _destination: &str,
        _expected_generation: u64,
    ) -> PortResult<ProtectedSecret> {
        Err(PortError::new(
            PortErrorCode::NotFound,
            "test.secret.resolve",
        ))
    }

    fn apply_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
        _input: Option<&ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1> {
        Ok(self.stage(
            operation_id,
            fake_effect(
                operation_id,
                &format!("secret:{}", mutation.credential().credential_id()),
                mutation.credential().credential_id(),
                OwnedEffectKind::Secret,
            ),
        ))
    }

    fn observe_secret(
        &self,
        operation_id: &OperationId,
        mutation: &SecretMutationV1,
    ) -> PortResult<EffectReconciliation> {
        Ok(self.observe(
            operation_id,
            &format!("secret:{}", mutation.credential().credential_id()),
        ))
    }

    fn activate_secret(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.activate(effect)
    }

    fn compensate_secret(&self, _effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        self.compensate(_effect)
    }

    fn apply_agent_access_grant(
        &self,
        operation_id: &OperationId,
        mutation: &hiroute_domain::AgentAccessGrantMutationV1,
        _input: Option<&hiroute_domain::ProtectedSecret>,
    ) -> PortResult<OwnedEffectV1> {
        Ok(self.stage(
            operation_id,
            fake_effect(
                operation_id,
                &format!("agent-access-grant:{}", mutation.connection_id()),
                mutation.connection_id(),
                OwnedEffectKind::Secret,
            ),
        ))
    }

    fn observe_agent_access_grant(
        &self,
        operation_id: &OperationId,
        mutation: &hiroute_domain::AgentAccessGrantMutationV1,
    ) -> PortResult<EffectReconciliation> {
        Ok(self.observe(
            operation_id,
            &format!("agent-access-grant:{}", mutation.connection_id()),
        ))
    }

    fn activate_agent_access_grant(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.activate(effect)
    }

    fn compensate_agent_access_grant(
        &self,
        _effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        self.compensate(_effect)
    }
}

impl RuntimeStatePort for MemoryPorts {
    fn generation(&self, _key: &str) -> PortResult<u64> {
        Ok(0)
    }

    fn apply_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<OwnedEffectV1> {
        Ok(self.stage(
            operation_id,
            fake_effect(
                operation_id,
                &format!("runtime:{}", mutation.key()),
                mutation.key(),
                OwnedEffectKind::RuntimeState,
            ),
        ))
    }

    fn observe_runtime(
        &self,
        operation_id: &OperationId,
        mutation: &RuntimeMutationV1,
    ) -> PortResult<EffectReconciliation> {
        Ok(self.observe(operation_id, &format!("runtime:{}", mutation.key())))
    }

    fn activate_runtime(&self, effect: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        self.activate(effect)
    }

    fn compensate_runtime(&self, _effect: &OwnedEffectV1) -> PortResult<CompensationOutcome> {
        self.compensate(_effect)
    }
}

impl ExternalEffectPort for MemoryPorts {
    fn current_external_fingerprint(&self, _target: &str) -> PortResult<Option<CanonicalDigest>> {
        self.state.borrow_mut().external_reads += 1;
        Ok(None)
    }

    fn apply_external(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        let operation_id = &operation.operation_id;
        Ok(self.stage(
            operation_id,
            fake_effect(
                operation_id,
                intent.effect_id(),
                intent.target(),
                intent.kind(),
            ),
        ))
    }

    fn observe_external(
        &self,
        operation: &OperationV1,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation> {
        let operation_id = &operation.operation_id;
        if intent.kind() == OwnedEffectKind::LoginItem
            && self.state.borrow().host_login_item_observed
        {
            // The Desktop host applied this before admission. Daemon rollback cannot remove
            // it; the host acts on the definitive rolled_back response instead.
            return Ok(EffectReconciliation::Applied(fake_effect(
                operation_id,
                intent.effect_id(),
                intent.target(),
                OwnedEffectKind::LoginItem,
            )));
        }
        if intent.kind() == OwnedEffectKind::Publication
            && self.state.borrow().publication_displaced
        {
            let key = Self::effect_key(operation_id, intent.effect_id());
            return Ok(match self.state.borrow().effects.get(&key) {
                Some((effect, _)) => EffectReconciliation::OwnershipLost(effect.clone()),
                None => EffectReconciliation::Missing,
            });
        }
        Ok(self.observe(operation_id, intent.effect_id()))
    }

    fn activate_external(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        self.activate(effect)
    }

    fn prepare_publication_activation(
        &self,
        _operation: &OperationV1,
        effect: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        let mut effect = effect.clone();
        let revision = self.state.borrow().publication_revision;
        if let Some(compensation) =
            std::sync::Arc::make_mut(&mut effect.compensation).as_object_mut()
        {
            compensation.insert("publication_revision".to_owned(), json!(revision));
        }
        // The production adapter durably installs the checkpoint and later activation reports
        // the same publication identity, so the staged record carries the revision forward.
        if let Some(operation) = effect
            .compensation
            .get("operation_id")
            .and_then(Value::as_str)
            .and_then(|value| OperationId::parse(value).ok())
        {
            let key = Self::effect_key(&operation, &effect.effect_id);
            if let Some(record) = self.state.borrow_mut().effects.get_mut(&key) {
                record.0 = effect.clone();
            }
        }
        Ok(effect)
    }

    fn compensate_external(
        &self,
        _operation: &OperationV1,
        _effect: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        self.compensate(_effect)
    }
}

impl ProtectedInputPort for MemoryPorts {
    fn read_secret(&self, _input_slot: &str) -> PortResult<ProtectedSecret> {
        let mut state = self.state.borrow_mut();
        state.protected_reads += 1;
        ProtectedSecret::new(state.input.clone())
            .map_err(|_| PortError::new(PortErrorCode::InvalidData, "test.input"))
    }

    fn validate_discovery_evidence(
        &self,
        _input_slot: &str,
        _expected_evidence: &CanonicalDigest,
    ) -> PortResult<()> {
        Err(PortError::new(
            PortErrorCode::Unavailable,
            "test.discovery_evidence",
        ))
    }
}

fn denied(context: &'static str) -> PortError {
    PortError::new(PortErrorCode::PermissionDenied, context)
}

fn fake_effect(
    operation_id: &OperationId,
    effect_id: &str,
    target: &str,
    kind: OwnedEffectKind,
) -> OwnedEffectV1 {
    OwnedEffectV1 {
        effect_id: effect_id.to_owned(),
        kind,
        target: target.to_owned(),
        before_fingerprint: None,
        after_fingerprint: Some(CanonicalDigest::of_bytes(effect_id.as_bytes())),
        compensation: json!({"operation_id": operation_id.as_str()}).into(),
    }
}

fn change(with_secret: bool) -> hiroute_domain::ChangeSpecV1 {
    hiroute_domain::ChangeSpecV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        command_id: "setup.apply".to_owned(),
        resource_id: Some("personal/default".to_owned()),
        desired_state: if with_secret {
            json!({
                "connection_option_id": "source-a",
                "secret": {"input_slot": "primary", "expected_generation": 0},
                "runtime_expected_generation": 0
            })
        } else {
            json!({
                "connection_option_id": "source-a",
                "runtime_expected_generation": 0
            })
        },
    }
}

fn open<'a>(
    ports: &'a MemoryPorts,
    runtime: &'a TransactionRuntime,
) -> TransactionCoordinator<'a, MemoryPorts, MemoryPorts, MemoryPorts, MemoryPorts, MemoryPorts> {
    let coordinator = TransactionCoordinator::new(ports, ports, ports, ports, ports, runtime);
    coordinator.reconcile_startup_and_open().unwrap();
    coordinator
}

fn request(
    coordinator: &TransactionCoordinator<
        '_,
        MemoryPorts,
        MemoryPorts,
        MemoryPorts,
        MemoryPorts,
        MemoryPorts,
    >,
    capability: Option<&str>,
    with_secret: bool,
) -> ApplyRequestV1 {
    let preview = coordinator
        .preview(
            &WorkspaceId::default(),
            PreviewRequestV1::new(change(with_secret)),
        )
        .unwrap();
    ApplyRequestV1 {
        schema_version: hiroute_domain::CHANGE_SPEC_SCHEMA_V1,
        spec: preview.normalized_spec,
        accept_digest: preview.change_digest,
        expected_revisions: preview.expected_revisions,
        idempotency_key: "idem-a".to_owned(),
        apply_capability: capability.map(str::to_owned),
    }
}

fn principal() -> VerifiedPrincipal {
    VerifiedPrincipal::from_protected_launcher(PrincipalKind::InteractiveUser).unwrap()
}

#[test]
fn disabled_variants_and_nested_urls_fail_before_secret_or_effect_ports() {
    let cases = [
        json!({"budgeted_paid": {}}),
        json!({"type": "PaidBudgetSpec"}),
        json!({"type": "BudgetQuote"}),
        json!({"type": "BudgetLease"}),
        json!({"nested": {"control_url": "https://control.invalid"}}),
        json!({"nested": {"desired": {"target": "//external.invalid"}}}),
        json!({"type": "ExternalSource"}),
    ];
    for disabled in cases {
        let ports = MemoryPorts::with_input(b"sentinel");
        let runtime = TransactionRuntime::default();
        let coordinator = open(&ports, &runtime);
        let before = ports.counters();
        let mut spec = change(true);
        spec.desired_state["disabled"] = disabled;
        assert!(matches!(
            coordinator.preview(&WorkspaceId::default(), PreviewRequestV1::new(spec)),
            Err(TransactionError::Preparation(
                ChangePreparationError::FeatureNotEnabled
            ))
        ));
        assert_eq!(ports.counters(), before);
    }
}

#[test]
fn generic_effect_fields_are_rejected_by_the_registered_typed_planner() {
    let ports = MemoryPorts::default();
    let runtime = TransactionRuntime::default();
    let coordinator = open(&ports, &runtime);
    let mut spec = change(false);
    spec.desired_state["external"] = json!([]);
    assert!(matches!(
        coordinator.preview(&WorkspaceId::default(), PreviewRequestV1::new(spec)),
        Err(TransactionError::Preparation(
            ChangePreparationError::InvalidDesiredState(_)
        ))
    ));
}

#[test]
fn missing_wrong_expired_revoked_and_wrong_digest_capabilities_are_denied() {
    enum Case {
        Missing,
        Wrong,
        Expired,
        Revoked,
        WrongDigest,
    }
    for case in [
        Case::Missing,
        Case::Wrong,
        Case::Expired,
        Case::Revoked,
        Case::WrongDigest,
    ] {
        let ports = MemoryPorts::default();
        let runtime = TransactionRuntime::default();
        let coordinator = open(&ports, &runtime);
        let capability = match case {
            Case::Missing => None,
            Case::Wrong => Some("wrong"),
            _ => Some("capability-a"),
        };
        let request = request(&coordinator, capability, false);
        if !matches!(case, Case::Missing) {
            let accepted = if matches!(case, Case::WrongDigest) {
                CanonicalDigest::of_bytes(b"wrong-digest")
            } else {
                request.accept_digest.clone()
            };
            ports.grant("capability-a", &accepted, &request.expected_revisions);
            let mut state = ports.state.borrow_mut();
            let grant = state
                .grants
                .get_mut(CanonicalDigest::of_bytes(b"capability-a").as_str())
                .unwrap();
            grant.expired = matches!(case, Case::Expired);
            grant.revoked = matches!(case, Case::Revoked);
        }
        let before = ports.counters();
        assert!(matches!(
            coordinator.accept(&WorkspaceId::default(), &principal(), request),
            Err(TransactionError::CapabilityDenied)
        ));
        assert_eq!(ports.state.borrow().durable_writes, before.0);
    }
}

#[test]
fn one_shot_capability_is_consumed_atomically_but_idempotent_replay_wins() {
    let ports = MemoryPorts::default();
    let runtime = TransactionRuntime::default();
    let coordinator = open(&ports, &runtime);
    let request = request(&coordinator, Some("capability-a"), false);
    ports.grant(
        "capability-a",
        &request.accept_digest,
        &request.expected_revisions,
    );
    let accepted = coordinator
        .accept(&WorkspaceId::default(), &principal(), request.clone())
        .unwrap();
    assert!(!accepted.existing);
    let replay = coordinator
        .accept(&WorkspaceId::default(), &principal(), request.clone())
        .unwrap();
    assert!(replay.existing);
    let mut reused = request;
    reused.idempotency_key = "idem-b".to_owned();
    assert!(matches!(
        coordinator.accept(&WorkspaceId::default(), &principal(), reused),
        Err(TransactionError::CapabilityDenied)
    ));
    assert_eq!(ports.state.borrow().operations.len(), 1);
}

#[test]
fn writes_stay_closed_until_startup_reconciliation_and_plaintext_never_serializes() {
    let sentinel = b"p25002-random-secret-sentinel-7d2f";
    let ports = MemoryPorts::with_input(sentinel);
    let runtime = TransactionRuntime::default();
    let coordinator = TransactionCoordinator::new(&ports, &ports, &ports, &ports, &ports, &runtime);
    let closed_request = request(&coordinator, Some("capability-a"), true);
    assert!(matches!(
        coordinator.accept(&WorkspaceId::default(), &principal(), closed_request),
        Err(TransactionError::RecoveryRequired)
    ));
    coordinator.reconcile_startup_and_open().unwrap();
    let request = request(&coordinator, Some("capability-a"), true);
    ports.grant(
        "capability-a",
        &request.accept_digest,
        &request.expected_revisions,
    );
    let accepted = coordinator
        .accept(&WorkspaceId::default(), &principal(), request)
        .unwrap();
    let encoded = serde_json::to_vec(&accepted.operation).unwrap();
    assert!(
        !encoded
            .windows(sentinel.len())
            .any(|window| window == sentinel)
    );
    assert!(
        !accepted
            .operation
            .request_digest
            .as_str()
            .contains("sentinel")
    );
    assert!(
        !accepted
            .operation
            .accepted_digest
            .as_str()
            .contains("sentinel")
    );
}

#[test]
fn restart_reconciles_the_durable_claim_before_reopening_writes() {
    let ports = MemoryPorts::default();
    let first_runtime = TransactionRuntime::default();
    let first = open(&ports, &first_runtime);
    let request = request(&first, Some("capability-a"), false);
    ports.grant(
        "capability-a",
        &request.accept_digest,
        &request.expected_revisions,
    );
    let accepted = first
        .accept(&WorkspaceId::default(), &principal(), request)
        .unwrap();
    assert!(ports.state.borrow().writer.is_some());

    let restarted_runtime = TransactionRuntime::default();
    let restarted =
        TransactionCoordinator::new(&ports, &ports, &ports, &ports, &ports, &restarted_runtime);
    let recovered = restarted.reconcile_startup_and_open().unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].operation_id, accepted.operation.operation_id);
    assert_eq!(recovered[0].state, OperationState::Succeeded);
    assert!(ports.state.borrow().writer.is_none());
    assert!(restarted_runtime.writes_open.load(Ordering::Acquire));
}

#[test]
fn terminal_reconciliation_turns_missing_owned_effect_into_attention() {
    let ports = MemoryPorts::default();
    let runtime = TransactionRuntime::default();
    let coordinator = open(&ports, &runtime);
    let request = request(&coordinator, Some("capability-a"), false);
    ports.grant(
        "capability-a",
        &request.accept_digest,
        &request.expected_revisions,
    );
    let accepted = coordinator
        .accept(&WorkspaceId::default(), &principal(), request)
        .unwrap();
    ports.state.borrow_mut().tamper_after_activate = Some("runtime:active/setup".to_owned());

    let recovered = coordinator.run(&accepted.operation.operation_id).unwrap();
    assert_eq!(recovered.state, OperationState::NeedsAttention);
    assert!(ports.state.borrow().writer.is_some());

    let restarted_runtime = TransactionRuntime::default();
    let restarted =
        TransactionCoordinator::new(&ports, &ports, &ports, &ports, &ports, &restarted_runtime);
    assert!(matches!(
        restarted.reconcile_startup_and_open(),
        Err(TransactionError::RecoveryRequired)
    ));
    assert!(ports.state.borrow().writer.is_some());
}

#[path = "classifier_secret_tests.rs"]
mod classifier_secret_tests;
#[path = "compute_tests.rs"]
mod compute_tests;
