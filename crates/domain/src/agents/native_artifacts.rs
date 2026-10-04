//! Native Agent rendering extensions to the existing Operation artifact port.
use crate::{ExternalEffectIntentV1, ExternalEffectPort, OperationId, OwnedEffectV1, PortResult};
use zeroize::Zeroizing;

/// Implemented by the existing protected artifact store, never by an untrusted frontend.
/// Operation admission and management authorization remain with the caller.
pub trait NativeAgentArtifactPort: ExternalEffectPort {
    /// Inspect the existing native file marker before rendering/staging. This lower-level
    /// artifact operation does not authorize a control-plane Operation or install a publication.
    fn observe_artifact(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<crate::EffectReconciliation>;
    /// A trusted adapter has verified semantic restoration against the protected field record.
    /// Acknowledge only this exact safe snapshot; never write or compensate the user's file.
    fn acknowledge_native_restoration(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        current: Option<&[u8]>,
    ) -> PortResult<OwnedEffectV1>;
    /// Stage a Skill effect while carrying only validated original directory ownership into its marker.
    fn stage_native_skill_target(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        bytes: Option<&[u8]>,
        previous: Option<&crate::SkillFileEffectRef>,
    ) -> PortResult<OwnedEffectV1>;
    /// After a successful removal, clean only empty directories created by this original effect.
    /// False means user content or changed directory identity was retained.
    fn cleanup_native_parents(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<bool>;
    fn read_native_target(&self, target: &str) -> PortResult<Option<Zeroizing<Vec<u8>>>>;
    /// Read a native file containing a local bearer only after verifying owner-only 0600
    /// permissions on the same opened file used for reading. Import/restore of ordinary user
    /// configuration remains on read_native_target; unsupported adapters must fail closed.
    fn read_private_native_target(&self, _target: &str) -> PortResult<Option<Zeroizing<Vec<u8>>>> {
        Err(crate::PortError::new(
            crate::PortErrorCode::PermissionDenied,
            "native.private_read.unsupported",
        ))
    }
    fn save_native_restore(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        bytes: &[u8],
    ) -> PortResult<()>;
    fn load_native_restore(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
    ) -> PortResult<Option<Zeroizing<Vec<u8>>>>;
    /// None stages an absent target. Activation/recovery uses the same existing artifact journal.
    fn stage_native_target(
        &self,
        operation: &OperationId,
        intent: &ExternalEffectIntentV1,
        bytes: Option<&[u8]>,
        sensitive: bool,
    ) -> PortResult<OwnedEffectV1>;
    /// Absolute filesystem path of a registered target. Only the trusted native adapter may use
    /// it to render a client-owned pointer; it is never a public locator.
    fn native_target_path(&self, target: &str) -> PortResult<std::path::PathBuf>;
}
