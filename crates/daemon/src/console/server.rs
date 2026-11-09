use super::{
    assets::Assets,
    auth::{AuthError, LaunchTarget},
    contexts::Contexts,
    control::{ConsoleControl, ConsoleEndpoint},
    transport::Connections,
};
use hiroute_diagnostics::context::DiagnosticHandle;
use hiroute_host_runtime::{
    ConsoleListenerDesiredV1, ConsoleListenerMode, ConsoleListenerStore, StandaloneLayout,
};
use hiroute_product_session::HostAuthority;
use std::{
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(super) struct Server {
    pub control: Arc<ConsoleControl>,
    pub client: hiroute_client_core::Client,
    pub authority: Arc<dyn HostAuthority>,
    pub diagnostics: DiagnosticHandle,
    pub assets: Option<Assets>,
    pub contexts: Contexts,
    pub layout: StandaloneLayout,
    pub diagnostic_runtime: Option<Arc<hiroute_diagnostics::runtime::DiagnosticRuntime>>,
    pub stop: CancellationToken,
    pub connections: Connections,
    runtime: tokio::runtime::Handle,
    running: AtomicBool,
    endpoints: Mutex<Endpoints>,
    desired: ConsoleListenerDesiredV1,
    store: ConsoleListenerStore,
}
#[derive(Default)]
struct Endpoints {
    primary: Option<SocketAddr>,
    loopback: Option<SocketAddr>,
}
pub struct ConsoleService {
    pub(super) server: Arc<Server>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl ConsoleService {
    pub fn start(
        role: &crate::RoleAllHandle,
        layout: StandaloneLayout,
        assets: PathBuf,
        diagnostics: Option<Arc<hiroute_diagnostics::runtime::DiagnosticRuntime>>,
    ) -> Result<Self, AuthError> {
        let control = Arc::new(ConsoleControl::open(&layout.data_root)?);
        role.set_console_control(control.clone())
            .map_err(|_| AuthError::Unavailable)?;
        let store = ConsoleListenerStore::new(layout.gateway_config_root());
        let desired = store.read().map_err(|_| AuthError::Unavailable)?.desired;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| AuthError::Unavailable)?;
        let stop = CancellationToken::new();
        let server = Arc::new(Server {
            control: control.clone(),
            client: role.console_client(),
            authority: role.console_authority(),
            diagnostics: diagnostics
                .as_ref()
                .map(|d| d.handle().clone())
                .unwrap_or_else(DiagnosticHandle::noop),
            assets: Assets::load(&assets).ok(),
            contexts: Contexts::default(),
            layout,
            diagnostic_runtime: diagnostics,
            stop: stop.clone(),
            connections: Connections::default(),
            runtime: runtime.handle().clone(),
            running: AtomicBool::new(true),
            endpoints: Mutex::new(Endpoints::default()),
            desired,
            store,
        });
        control.set_endpoint(Arc::new(LiveEndpoint(Arc::downgrade(&server))))?;
        let state = server.clone();
        let worker=std::thread::Builder::new().name("hiroute-console".into()).spawn(move||{
            runtime.block_on(async {
                let mut tick=tokio::time::interval(Duration::from_secs(30));
                loop {tokio::select!{_ = stop.cancelled()=>break,_ = tick.tick()=>{for context in state.contexts.retire(&state.control.auth,false){tokio::spawn(async move{context.close().await;});}}}}
                let mut closing=tokio::task::JoinSet::new();for context in state.contexts.retire(&state.control.auth,true){closing.spawn(async move{context.close().await;});}
                let _=tokio::time::timeout(Duration::from_secs(35),async{while closing.join_next().await.is_some(){}}).await;
            });
            runtime.shutdown_timeout(Duration::from_secs(2));
        }).map_err(|_|AuthError::Unavailable)?;
        let service = Self {
            server,
            worker: Some(worker),
        };
        let server = &service.server;
        match server.desired.reserve() {
            Ok(Some(listener)) => {
                let address = listener.local_addr().map_err(|_| AuthError::Unavailable)?;
                server.listen(listener)?;
                let loopback = if server.desired.mode == ConsoleListenerMode::Loopback {
                    Some(address)
                } else {
                    // Establish the local recovery/shell path at startup as well.
                    // Failure here leaves the primary network console usable.
                    ConsoleListenerDesiredV1::loopback()
                        .reserve()
                        .ok()
                        .flatten()
                        .and_then(|listener| {
                            let local = listener.local_addr().ok()?;
                            server.listen(listener).ok()?;
                            Some(local)
                        })
                };
                *server
                    .endpoints
                    .lock()
                    .map_err(|_| AuthError::Unavailable)? = Endpoints {
                    primary: Some(address),
                    loopback,
                };
                let _ = server.store.mark_applied(Some(address));
            }
            Ok(None) => {
                let _ = server.store.mark_applied(None);
            }
            Err(_) => {
                let _ = server.store.mark_failed();
            }
        }
        Ok(service)
    }
    pub fn shutdown(&mut self) {
        self.server.running.store(false, Ordering::Release);
        self.server.stop.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for ConsoleService {
    fn drop(&mut self) {
        self.shutdown();
    }
}
struct LiveEndpoint(std::sync::Weak<Server>);
impl ConsoleEndpoint for LiveEndpoint {
    fn launch_target(&self, loopback_only: bool) -> Result<LaunchTarget, AuthError> {
        self.0
            .upgrade()
            .ok_or(AuthError::Unavailable)?
            .launch_target(loopback_only)
    }

    fn qualifies_native_assets(&self, origin: &str, assets_sha256: &str) -> bool {
        self.0.upgrade().is_some_and(|server| {
            server
                .launch_target(true)
                .is_ok_and(|target| target.origin == origin)
                && server
                    .assets
                    .as_ref()
                    .is_some_and(|assets| assets.fingerprint() == assets_sha256)
        })
    }
}
impl Server {
    fn launch_target(&self, loopback_only: bool) -> Result<LaunchTarget, AuthError> {
        if !self.running.load(Ordering::Acquire) {
            return Err(AuthError::Unavailable);
        }
        let bound = self.endpoints.lock().map_err(|_| AuthError::Unavailable)?;
        let address = if loopback_only {
            bound.loopback
        } else {
            bound
                .primary
                .map(|address| self.desired.advertised_socket(address))
        }
        .ok_or(AuthError::ListenerUnavailable)?;
        if address.ip().is_unspecified() {
            return Err(AuthError::ListenerUnavailable);
        }
        Ok(LaunchTarget::new(address, address.ip().is_loopback()))
    }
    /// Advertisement is scoped to the established primary port, not a companion.
    pub(super) fn advertised_on(&self, local: SocketAddr) -> Option<SocketAddr> {
        let bound = self.endpoints.lock().ok()?;
        let primary = bound.primary?;
        (self.desired.mode == ConsoleListenerMode::Network
            && primary.port() == local.port()
            && (primary.ip().is_unspecified() || primary.ip() == local.ip()))
        .then(|| self.desired.advertised_socket(primary))
    }
}
impl Server {
    fn listen(self: &Arc<Self>, listener: TcpListener) -> Result<(), AuthError> {
        listener
            .set_nonblocking(true)
            .map_err(|_| AuthError::Unavailable)?;
        // The OnceLock contains this same Arc. A listener owns only service state, never role-all.
        let entered = self.runtime.enter();
        let listener =
            tokio::net::TcpListener::from_std(listener).map_err(|_| AuthError::Unavailable)?;
        drop(entered);
        let endpoint = self.clone();
        self.runtime.spawn(async move{
            loop{
                let connection=tokio::select!{_ = endpoint.stop.cancelled()=>break,result=listener.accept()=>result};
                let Ok((stream,peer))=connection else{break;};
                let Ok(permit)=endpoint.connections.acquire(peer.ip())else{continue;};
                let Ok(local)=stream.local_addr()else{continue;};let state=endpoint.clone();
                tokio::spawn(async move{
                    let _permit=permit;
                    let service=hyper::service::service_fn(move|request|super::http::handle(state.clone(),request,peer,local));
                    let _=super::transport::serve(stream,service).await;
                });
            }
        });
        Ok(())
    }
}
