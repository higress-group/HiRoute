//! Test-only file race at the real coordinator's final activation boundary.
use super::*;
use hiroute_application::control::ApplicationMutationPort;
use hiroute_application::{
    PreparedTransactionV1, TransactionCoordinator, TransactionError, VerifiedPrincipal,
};
use hiroute_domain::{
    CompensationOutcome, EffectReconciliation, ExternalEffectIntentV1, ExternalEffectPort,
    OperationV1, OwnedEffectV1, PortError, PortErrorCode, PortResult, PriceGenerationRefV1,
};

pub(super) struct FileRace {
    pub adapter: Arc<LocalControlAdapter>,
    pub path: std::path::PathBuf,
    pub fail_after_file: bool,
}
impl ApplicationMutationPort for FileRace {
    fn preview_change(
        &self,
        r: api::PreviewRequestV1,
    ) -> Result<api::PreviewResultV1, TransactionError> {
        self.adapter.preview_change(r)
    }
    fn apply_change(
        &self,
        p: api::PrincipalKind,
        r: api::ApplyRequestV1,
    ) -> Result<OperationV1, TransactionError> {
        self.adapter.apply_change(p, r)
    }
    fn apply_local_change(&self, r: api::ApplyRequestV1) -> Result<OperationV1, TransactionError> {
        self.adapter.apply_local_change(r)
    }
    fn apply_prepared_change(
        &self,
        p: api::PrincipalKind,
        r: PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        self.adapter.apply_prepared_change(p, r)
    }
    fn apply_local_prepared_change(
        &self,
        r: PreparedTransactionV1,
    ) -> Result<OperationV1, TransactionError> {
        let a = self.adapter.as_ref();
        let coordinator = TransactionCoordinator::new(a, a, a, self, a, &a.admission);
        let accepted = coordinator.accept_prepared(
            &WorkspaceId::default(),
            &VerifiedPrincipal::for_local_control(),
            r,
        )?;
        coordinator.run_accepted(accepted)
    }
}
impl ExternalEffectPort for FileRace {
    fn install_source_price_snapshot(&self, op: &OperationV1) -> PortResult<PriceGenerationRefV1> {
        self.adapter.install_source_price_snapshot(op)
    }
    fn validate_external_admission(&self, i: &ExternalEffectIntentV1) -> PortResult<()> {
        self.adapter.validate_external_admission(i)
    }
    fn begin_publication_activation(&self, op: &OperationV1) -> PortResult<()> {
        self.adapter.begin_publication_activation(op)
    }
    fn prepare_publication_activation(
        &self,
        op: &OperationV1,
        e: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        if self.fail_after_file {
            let restored = fs::read_to_string(&self.path).unwrap();
            assert!(!restored.contains("X-HiRoute-Token"));
            return Err(PortError::new(
                PortErrorCode::Unavailable,
                "test.profile.service_after_restore",
            ));
        }
        self.adapter.prepare_publication_activation(op, e)
    }
    fn prepare_publication_rollback(
        &self,
        op: &OperationV1,
        e: &OwnedEffectV1,
    ) -> PortResult<OwnedEffectV1> {
        self.adapter.prepare_publication_rollback(op, e)
    }
    fn finish_publication_activation(&self, op: &OperationV1) -> PortResult<()> {
        self.adapter.finish_publication_activation(op)
    }
    fn prepare_agent_artifact_activation(
        &self,
        op: &OperationV1,
        i: &ExternalEffectIntentV1,
    ) -> PortResult<()> {
        self.adapter.prepare_agent_artifact_activation(op, i)
    }
    fn current_external_fingerprint(&self, t: &str) -> PortResult<Option<CanonicalDigest>> {
        self.adapter.current_external_fingerprint(t)
    }
    fn apply_external(
        &self,
        op: &OperationV1,
        i: &ExternalEffectIntentV1,
    ) -> PortResult<OwnedEffectV1> {
        self.adapter.apply_external(op, i)
    }
    fn observe_external(
        &self,
        op: &OperationV1,
        i: &ExternalEffectIntentV1,
    ) -> PortResult<EffectReconciliation> {
        self.adapter.observe_external(op, i)
    }
    fn activate_external(&self, op: &OperationV1, e: &OwnedEffectV1) -> PortResult<OwnedEffectV1> {
        if !self.fail_after_file
            && op.plan.external().iter().any(|i| {
                i.target() == e.target
                    && (crate::control::runtime::native_model::is_settings_codex_model(i)
                        || crate::control::runtime::native_qoder_model::is_settings_qoder_model(i))
            })
        {
            let original = fs::read_to_string(&self.path).unwrap();
            fs::write(&self.path, format!("# user's concurrent edit\n{original}")).unwrap();
            return self.adapter.activate_external(op, e);
        }
        self.adapter.activate_external(op, e)
    }
    fn compensate_external(
        &self,
        op: &OperationV1,
        e: &OwnedEffectV1,
    ) -> PortResult<CompensationOutcome> {
        self.adapter.compensate_external(op, e)
    }
}
