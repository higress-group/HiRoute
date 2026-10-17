use hiroute_application_api::*;
use hiroute_product_session::HostAuthority;
use zeroize::Zeroizing;
pub(crate) struct ConsoleAuthority(pub(crate) crate::control::runtime::RuntimeAuthority);
impl HostAuthority for ConsoleAuthority {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn register_operation(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        let secret = Zeroizing::new(hiroute_product_session::random_id()?);
        let expires = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "CLOCK_UNAVAILABLE")?
            .as_secs()
            .checked_add(60)
            .ok_or("CLOCK_UNAVAILABLE")?;
        let registration =
            hiroute_local_storage::ApplyCapabilityRegistrationV1::from_protected_launcher(
                secret.to_string(),
                "desktop",
                hiroute_domain::WorkspaceId::default(),
                operation,
                digest.clone(),
                revisions.clone(),
                i64::try_from(expires).map_err(|_| "CLOCK_UNAVAILABLE")?,
            )
            .map_err(|_| "CAPABILITY_INVALID")?;
        self.0.register_apply_capability(registration)?;
        Ok(secret)
    }
    fn register_input(
        &self,
        candidate: &ComputeCandidateRefV2,
        secret: Zeroizing<String>,
    ) -> Result<(), String> {
        self.0.register_manual_protected_input(
            candidate.clone(),
            hiroute_domain::ProtectedSecret::new(secret.as_bytes().to_vec())
                .map_err(|_| "PROTECTED_INPUT_INVALID")?,
        )
    }
    fn release_model_input(&self, candidate: &ComputeCandidateRefV2) -> Result<(), String> {
        self.0
            .release_manual_protected_input(&candidate.candidate_ref)
    }
}
