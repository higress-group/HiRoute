//! Credential inspection emits only closed failure categories, never paths or input bytes.
use super::*;

impl ManagedCpaRuntime {
    pub(super) fn inspect_subscription_with_diagnostics(
        &self,
        interactive: bool,
    ) -> Result<BorrowedSubscriptionEvidence, CpaLifecycleError> {
        let _scope = self.operation_context().enter();
        let started = std::time::Instant::now();
        let stage = if self.is_managed_oauth() {
            CpaStageKind::ManagedCredentialRead
        } else {
            CpaStageKind::NativeCredentialRead
        };
        // These are source reads, independent of any CPA process generation.
        self.emit_stage(stage, CpaStageOutcome::Entered, 0, 0);
        let result = (|| {
            if let Some(source) = self.managed_oauth_source() {
                let evidence = source.inspect()?;
                self.inspect_managed_authentication(&evidence)?;
                return Ok(BorrowedSubscriptionEvidence::Managed(evidence));
            }
            if let Some(spec) = &self.spec.borrowed_claude_auth {
                return if interactive {
                    spec.inspect_for_check()
                } else {
                    spec.inspect()
                }
                .map(Into::into);
            }
            self.spec
                .borrowed_codex_auth
                .as_ref()
                .ok_or(CpaLifecycleError::InvalidSpec)?
                .inspect()
                .map(Into::into)
        })();
        let outcome = match &result {
            Ok(_) => CpaStageOutcome::Completed,
            Err(error) => CpaStageOutcome::Failed {
                code: credential_failure_code(error),
            },
        };
        self.emit_stage(
            stage,
            outcome,
            started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            0,
        );
        result
    }
}

fn credential_failure_code(error: &CpaLifecycleError) -> CpaFailureCode {
    match error {
        CpaLifecycleError::BorrowedCodexAuthMissing
        | CpaLifecycleError::BorrowedClaudeAuthMissing => CpaFailureCode::CredentialSourceMissing,
        CpaLifecycleError::BorrowedCodexAuthIo
        | CpaLifecycleError::BorrowedClaudeAuthUnavailable => CpaFailureCode::CredentialReadFailed,
        CpaLifecycleError::BorrowedCodexStoreUnsupported => {
            CpaFailureCode::CredentialStoreUnsupported
        }
        CpaLifecycleError::BorrowedCodexLoginUnsupported => {
            CpaFailureCode::CredentialLoginUnsupported
        }
        CpaLifecycleError::BorrowedCodexAccountMissing => CpaFailureCode::CredentialAccountMissing,
        CpaLifecycleError::InvalidBorrowedCodexAuth
        | CpaLifecycleError::InvalidBorrowedClaudeAuth
        | CpaLifecycleError::BorrowedCodexAuthUnavailable => CpaFailureCode::CredentialInvalid,
        CpaLifecycleError::ManagedOAuthCredentialsMissing
        | CpaLifecycleError::InvalidManagedOAuthCredentials
        | CpaLifecycleError::ManagedOAuthAuthenticationRequired => {
            CpaFailureCode::ManagedLoginRequired
        }
        _ => CpaFailureCode::Unknown,
    }
}
