use super::*;

pub(super) fn identity_contract(
    input: &ProfileInput<'_>,
) -> Result<AcpNativeIdentityContract, DelegationErrorV1> {
    if matches!(
        input.harness,
        WorkerHarnessV1::QoderCli | WorkerHarnessV1::Pi
    ) && !input.native_context.is_borrowed()
    {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    // Codex's read-only ACP mode can select workspaceWrite internally. Qoder's restricted
    // modes have no proven permission contract; neither client may silently upgrade them.
    if matches!(
        input.harness,
        WorkerHarnessV1::CodexCli | WorkerHarnessV1::QoderCli | WorkerHarnessV1::Pi
    ) && input.permission_policy != WorkerPermissionPolicyV1::ApproveAll
    {
        return Err(DelegationErrorV1::CapabilityUnavailable);
    }
    Ok(match input.harness {
        WorkerHarnessV1::CodexCli => AcpNativeIdentityContract::CodexThreadV1,
        WorkerHarnessV1::ClaudeCode => AcpNativeIdentityContract::ClaudeSessionV1,
        WorkerHarnessV1::QoderCli => AcpNativeIdentityContract::QoderSessionV1,
        WorkerHarnessV1::Pi => AcpNativeIdentityContract::ExplicitResponseMetadata,
    })
}
