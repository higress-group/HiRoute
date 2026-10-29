use super::auth::{AuthError, ConsoleAuth};
use hiroute_client_core::Client;
use hiroute_diagnostics::context::DiagnosticHandle;
use hiroute_product_session::{
    HostAuthority, ProductSession,
    commands::{ClientContext, CommandState, WorkerDependencyConfirmationState},
    web_confirmation::ConfirmationBroker,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
pub(super) struct Context {
    pub session_id: String,
    id: String,
    session: Arc<tokio::sync::Mutex<Option<ProductSession>>>,
    epoch: AtomicU64,
    active: AtomicBool,
    error: OnceLock<String>,
    pub confirmations: ConfirmationBroker,
    dependencies: WorkerDependencyConfirmationState,
    diagnostics: DiagnosticHandle,
}
impl Context {
    fn new(
        session_id: String,
        id: String,
        client: Client,
        authority: Arc<dyn HostAuthority>,
        diagnostics: DiagnosticHandle,
    ) -> Self {
        Self {
            session_id,
            id,
            session: Arc::new(tokio::sync::Mutex::new(Some(ProductSession::new(
                client, authority,
            )))),
            epoch: AtomicU64::new(0),
            active: AtomicBool::new(true),
            error: OnceLock::new(),
            confirmations: ConfirmationBroker::default(),
            dependencies: WorkerDependencyConfirmationState::default(),
            diagnostics,
        }
    }
    pub async fn command(
        &self,
        name: &str,
        args: serde_json::Value,
        auth: &Mutex<ConsoleAuth>,
    ) -> Result<serde_json::Value, hiroute_product_session::failure::DesktopFailure> {
        let active = || {
            self.active.load(Ordering::Acquire)
                && auth
                    .lock()
                    .is_ok_and(|mut a| a.session_active(&self.session_id))
        };
        if !active() {
            return Err("CONSOLE_AUTH_REQUIRED".into());
        }
        let window = ClientContext {
            id: &self.id,
            confirmations: &self.confirmations,
            active: &active,
            visible: &|| true,
            notify: None,
        };
        let state = CommandState(&self.session, &self.error, &self.epoch, &self.diagnostics);
        hiroute_product_session::commands::dispatch(name, args, &window, &state, &self.dependencies)
            .await
    }
    fn invalidate(&self) {
        self.active.store(false, Ordering::Release);
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.confirmations.cancel_context(&self.id);
        self.dependencies.cancel_window(&self.id);
    }
    pub async fn close(&self) {
        self.invalidate();
        let mut session = self.session.lock().await;
        if let Some(s) = session.as_mut() {
            s.invalidate_confirmation();
            s.stop_observing();
            let _ = s.close_subscription_check().await;
        }
        *session = None;
    }
}
struct Entry {
    context: Arc<Context>,
    last: Instant,
}
#[derive(Default)]
pub(super) struct Contexts {
    entries: Mutex<BTreeMap<(String, String), Entry>>,
}
impl Contexts {
    pub fn acquire(
        &self,
        session: &str,
        context: &str,
        client: Client,
        authority: Arc<dyn HostAuthority>,
        diagnostics: DiagnosticHandle,
    ) -> Result<Arc<Context>, AuthError> {
        let mut entries = self.entries.lock().map_err(|_| AuthError::Unavailable)?;
        let key = (session.to_owned(), context.to_owned());
        if let Some(entry) = entries.get_mut(&key) {
            entry.last = Instant::now();
            return Ok(entry.context.clone());
        }
        if entries.len() >= 32 || entries.keys().filter(|(s, _)| s == session).count() >= 8 {
            return Err(AuthError::Capacity);
        }
        let value = Arc::new(Context::new(
            session.into(),
            context.into(),
            client,
            authority,
            diagnostics,
        ));
        entries.insert(
            key,
            Entry {
                context: value.clone(),
                last: Instant::now(),
            },
        );
        Ok(value)
    }
    pub fn retire(&self, auth: &Mutex<ConsoleAuth>, all: bool) -> Vec<Arc<Context>> {
        let Ok(mut entries) = self.entries.lock() else {
            return vec![];
        };
        let keys: Vec<_> = entries
            .iter()
            .filter(|(_, e)| {
                all || e.last.elapsed() >= Duration::from_secs(30 * 60)
                    || !auth
                        .lock()
                        .is_ok_and(|mut a| a.session_active(&e.context.session_id))
            })
            .map(|(k, _)| k.clone())
            .collect();
        keys.into_iter()
            .filter_map(|key| entries.remove(&key))
            .map(|entry| {
                entry.context.invalidate();
                entry.context
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use hiroute_application_api::*;
    use zeroize::Zeroizing;
    #[derive(Default)]
    struct Authority(Mutex<std::collections::BTreeSet<String>>);
    impl HostAuthority for Authority {
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
            Err("unused".into())
        }
        fn register_input(
            &self,
            c: &ComputeCandidateRefV2,
            _: Zeroizing<String>,
        ) -> Result<(), String> {
            self.0.lock().unwrap().insert(c.candidate_ref.clone());
            Ok(())
        }
        fn release_model_input(&self, c: &ComputeCandidateRefV2) -> Result<(), String> {
            self.0.lock().unwrap().remove(&c.candidate_ref);
            Ok(())
        }
    }
    #[tokio::test]
    async fn idle_eviction_expires_confirmations_and_releases_only_owned_inputs() {
        let root = tempfile::tempdir().unwrap();
        let mut auth = ConsoleAuth::open(root.path()).unwrap();
        let target = super::super::auth::LaunchTarget::new("127.0.0.1:5838".parse().unwrap(), true);
        let code = auth.create_launch(target.clone()).unwrap();
        let identity = auth
            .exchange_launch(&code, "127.0.0.1".parse().unwrap(), &target.origin)
            .unwrap()
            .session;
        let auth = Mutex::new(auth);
        let authority = Arc::new(Authority::default());
        let contexts = Contexts::default();
        let client = || {
            Client::new(
                "test",
                hiroute_client_core::LocalEndpoint::from_runtime_root("unused"),
            )
        };
        let old = contexts
            .acquire(
                &identity.id,
                "1234567890abcdef",
                client(),
                authority.clone(),
                DiagnosticHandle::noop(),
            )
            .unwrap();
        let other = contexts
            .acquire(
                &identity.id,
                "abcdef1234567890",
                client(),
                authority.clone(),
                DiagnosticHandle::noop(),
            )
            .unwrap();
        old.session
            .lock()
            .await
            .as_ref()
            .unwrap()
            .authority
            .register_model_input(Zeroizing::new("old".into()))
            .unwrap();
        other
            .session
            .lock()
            .await
            .as_ref()
            .unwrap()
            .authority
            .register_model_input(Zeroizing::new("other".into()))
            .unwrap();
        let (_, answer) = old
            .confirmations
            .begin(
                &old.id,
                hiroute_product_session::web_confirmation::WebConfirmationPrompt {
                    title: "Confirm".into(),
                    message: "One exact action".into(),
                    confirm_label: "Apply".into(),
                    cancel_label: "Cancel".into(),
                },
            )
            .unwrap();
        contexts
            .entries
            .lock()
            .unwrap()
            .get_mut(&(identity.id.clone(), old.id.clone()))
            .unwrap()
            .last = Instant::now() - Duration::from_secs(1801);
        let expired = contexts.retire(&auth, false);
        assert_eq!(expired.len(), 1);
        assert!(!answer.await.unwrap());
        expired[0].close().await;
        assert_eq!(authority.0.lock().unwrap().len(), 1);
        for context in contexts.retire(&auth, true) {
            context.close().await;
        }
        assert!(authority.0.lock().unwrap().is_empty());
    }
}
