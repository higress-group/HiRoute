//! The Desktop adapter keeps the inherited descriptor frames and native lifecycle intact.
use super::*;
use hiroute_product_session::{HostAuthority, ProductSession};
use std::sync::{Mutex, MutexGuard};

pub struct NativeAuthority(pub Mutex<Resident>);

impl HostAuthority for NativeAuthority {
    fn as_any(&self) -> &dyn std::any::Any {
        &self.0
    }
    fn has_authority(&self) -> bool {
        self.0
            .lock()
            .is_ok_and(|mut resident| resident.has_authority())
    }
    fn register_operation(
        &self,
        digest: &CanonicalDigest,
        revisions: &RevisionSetV1,
        operation: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        self.0
            .lock()
            .map_err(|_| "TRUSTED_AUTHORITY_UNAVAILABLE")?
            .register_operation(digest, revisions, operation)
    }
    fn register_input(
        &self,
        candidate: &ComputeCandidateRefV2,
        secret: Zeroizing<String>,
    ) -> Result<(), String> {
        candidate
            .validate_shape()
            .map_err(|_| "PROTECTED_INPUT_INVALID")?;
        let mut resident = self.0.lock().map_err(|_| "TRUSTED_AUTHORITY_UNAVAILABLE")?;
        if secret.is_empty() || secret.len() > 32 * 1024 || !resident.has_authority() {
            return Err("PROTECTED_INPUT_INVALID".into());
        }
        if let Err(error) = resident.register_model_input_once(candidate, &secret) {
            if let Some(owned) = &mut resident.owned {
                owned.authority_healthy = false;
            }
            return Err(error);
        }
        resident
            .owned
            .as_mut()
            .ok_or("TRUSTED_AUTHORITY_UNAVAILABLE")?
            .manual_input_candidates
            .insert(candidate.candidate_ref.clone());
        Ok(())
    }
    fn release_model_input(&self, candidate: &ComputeCandidateRefV2) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "TRUSTED_AUTHORITY_UNAVAILABLE")?
            .release_model_input(candidate)
    }
    fn remove_resident_login_item(
        &self,
    ) -> Result<hiroute_application_api::AgentLoginItemDeclarationV2, String> {
        crate::login_item::remove_resident_login_item().map_err(str::to_owned)
    }
    fn compensate_resident_login_item_removal(&self) {
        crate::login_item::compensate_resident_login_item_removal();
    }
}

pub fn resident(session: &ProductSession) -> Result<MutexGuard<'_, Resident>, String> {
    session
        .authority
        .as_any()
        .downcast_ref::<Mutex<Resident>>()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .lock()
        .map_err(|_| "RESIDENT_UNAVAILABLE".to_owned())
}
