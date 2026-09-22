use super::*;

impl ProductionControlRuntime {
    pub(crate) fn register_manual_protected_input(
        &self,
        candidate: hiroute_application_api::ComputeCandidateRefV2,
        secret: hiroute_domain::ProtectedSecret,
    ) -> Result<(), String> {
        if candidate
            .candidate_ref
            .starts_with("candidate/native/agent-token-")
        {
            candidate
                .validate_shape()
                .map_err(|_| "protected input registration is invalid".to_owned())?;
            if candidate.candidate_revision != 1
                || !hiroute_domain::valid_user_agent_token(secret.expose())
            {
                return Err("protected agent token is invalid".to_owned());
            }
            let mut inputs = self
                .adapter
                .agent_token_inputs
                .lock()
                .map_err(|_| "protected input registry is unavailable".to_owned())?;
            if inputs.len() >= 256 || inputs.contains_key(&candidate.candidate_ref) {
                return Err("protected input registration is invalid".to_owned());
            }
            inputs.insert(candidate.candidate_ref, secret);
            return Ok(());
        }
        self.adapter
            .model_connections
            .reserve_candidate_ref(&candidate)
            .map_err(|error| error.to_string())?;
        let mut inputs = self
            .adapter
            .manual_protected_inputs
            .lock()
            .map_err(|_| "protected input registry is unavailable".to_owned())?;
        if inputs.len() >= 256 || inputs.contains_key(&candidate.candidate_ref) {
            return Err("protected input registration is invalid".to_owned());
        }
        inputs.insert(candidate.candidate_ref, secret);
        Ok(())
    }

    pub(crate) fn release_manual_protected_input(&self, candidate_ref: &str) -> Result<(), String> {
        if candidate_ref.trim().is_empty() {
            return Err("protected input reference is invalid".to_owned());
        }
        self.adapter
            .manual_protected_inputs
            .lock()
            .map_err(|_| "protected input registry is unavailable".to_owned())?
            .remove(candidate_ref);
        self.adapter
            .agent_token_inputs
            .lock()
            .map_err(|_| "protected input registry is unavailable".to_owned())?
            .remove(candidate_ref);
        Ok(())
    }
}
