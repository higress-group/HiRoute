use super::authority::*;
use hiroute_application_api::*;
use hiroute_client_core::{Client, LocalEndpoint};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;
#[derive(Default)]
struct Host(Mutex<BTreeMap<String, String>>);
impl HostAuthority for Host {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn has_authority(&self) -> bool {
        true
    }
    fn register_operation(
        &self,
        _: &CanonicalDigest,
        _: &RevisionSetV1,
        _: &'static str,
    ) -> Result<Zeroizing<String>, String> {
        Ok(Zeroizing::new(crate::random_id()?))
    }
    fn register_input(
        &self,
        c: &ComputeCandidateRefV2,
        secret: Zeroizing<String>,
    ) -> Result<(), String> {
        assert!(
            self.0
                .lock()
                .unwrap()
                .insert(c.candidate_ref.clone(), secret.to_string())
                .is_none()
        );
        Ok(())
    }
    fn release_model_input(&self, c: &ComputeCandidateRefV2) -> Result<(), String> {
        self.0.lock().unwrap().remove(&c.candidate_ref);
        Ok(())
    }
}
#[test]
fn simultaneous_sessions_release_only_their_own_inputs_including_pending_prompts() {
    let host = Arc::new(Host::default());
    let client = || Client::new("test", LocalEndpoint::from_runtime_root("unused"));
    let first = crate::ProductSession::new(client(), host.clone());
    let second = crate::ProductSession::new(client(), host.clone());
    let a = first
        .authority
        .register_model_input(Zeroizing::new("first".into()))
        .unwrap();
    let b = second
        .authority
        .register_model_input(Zeroizing::new("second".into()))
        .unwrap();
    assert_ne!(a.candidate_ref, b.candidate_ref);
    // A reference from a different session cannot release the second session's input.
    first.authority.release_model_input(&b).unwrap();
    assert_eq!(host.0.lock().unwrap().len(), 2);
    drop(first);
    assert!(!host.0.lock().unwrap().contains_key(&a.candidate_ref));
    assert!(host.0.lock().unwrap().contains_key(&b.candidate_ref));
    drop(second);
    assert!(host.0.lock().unwrap().is_empty());
}

#[test]
fn checked_model_input_releases_after_revision_advances_without_releasing_another_session() {
    let host = Arc::new(Host::default());
    let client = || Client::new("test", LocalEndpoint::from_runtime_root("unused"));
    let first = crate::ProductSession::new(client(), host.clone());
    let second = crate::ProductSession::new(client(), host.clone());
    let mut checked = first
        .authority
        .register_model_input(Zeroizing::new("first".into()))
        .unwrap();
    let foreign = second
        .authority
        .register_model_input(Zeroizing::new("second".into()))
        .unwrap();
    checked.candidate_revision += 1;
    first.authority.release_model_input(&foreign).unwrap();
    assert_eq!(host.0.lock().unwrap().len(), 2);
    first.authority.release_model_input(&checked).unwrap();
    assert!(!host.0.lock().unwrap().contains_key(&checked.candidate_ref));
    assert!(host.0.lock().unwrap().contains_key(&foreign.candidate_ref));
    // Terminal operation polling and session shutdown may both request release.
    first.authority.release_model_input(&checked).unwrap();
    drop(first);
    assert!(host.0.lock().unwrap().contains_key(&foreign.candidate_ref));
    drop(second);
    assert!(host.0.lock().unwrap().is_empty());
}
