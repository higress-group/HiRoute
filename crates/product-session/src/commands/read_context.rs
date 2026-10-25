//! Read-only transport work runs outside the mutable Session. Publication still observes
//! the current Session, authority and service generation, including same-Session mutations.
use super::*;
use crate::authority::{HostAuthority, SessionAuthority};
use hiroute_client_core::Client;
use hiroute_diagnostics::event::{ControlOperation, ProductReadEnd};
use std::sync::Arc;

#[cfg(test)]
mod tests;

async fn measured_read<T>(
    state: &CommandState<'_>,
    operation: ControlOperation,
    read: impl std::future::Future<Output = Result<T, DesktopFailure>>,
) -> Result<T, DesktopFailure> {
    let started = std::time::Instant::now();
    let result = read.await;
    state
        .3
        .try_emit(DiagnosticEvent::ProductReadEnd(ProductReadEnd {
            operation,
            error: result.as_ref().err().map(DesktopFailure::event_code),
            elapsed_ms: started.elapsed().as_millis() as u64,
        }));
    result
}

pub(super) struct ReadContext {
    pub client: Client,
    pub authority: Arc<SessionAuthority>,
    epoch: u64,
    native: Option<crate::NativeConnectionStatus>,
}

impl ReadContext {
    pub async fn capture(state: &CommandState<'_>, resume: bool) -> Result<Self, DesktopFailure> {
        let epoch = state.2.load(Ordering::SeqCst);
        let mut guard = confirmation_session(state, epoch).await?;
        let session = guard.as_mut().ok_or_else(|| {
            state
                .1
                .get()
                .cloned()
                .unwrap_or("RESIDENT_UNAVAILABLE".into())
        })?;
        if resume {
            session.resume_observing();
        }
        Ok(Self {
            client: session.client.clone(),
            authority: session.authority.clone(),
            epoch,
            native: session.authority.native_connection(),
        })
    }

    pub async fn status(&self) -> Result<ClientServiceStatusV1, DesktopFailure> {
        crate::session::query(
            &self.client,
            "GetClientServiceStatus",
            &ClientEmptyRequestV1 {},
        )
        .await
    }

    pub async fn finish<'a>(
        &self,
        state: &'a CommandState<'_>,
        before: &ClientServiceStatusV1,
    ) -> Result<tokio::sync::MutexGuard<'a, Option<Session>>, DesktopFailure> {
        let guard = confirmation_session(state, self.epoch).await?;
        let session = guard.as_ref().ok_or("RESIDENT_UNAVAILABLE")?;
        if !Arc::ptr_eq(&session.authority, &self.authority) {
            return Err("RESIDENT_UNAVAILABLE".into());
        }
        // Only the final bounded service query holds the lock. No catalog/body scan or
        // native discovery does. This is the same publication fence as Agent discovery.
        let after: ClientServiceStatusV1 = crate::session::query(
            &session.client,
            "GetClientServiceStatus",
            &ClientEmptyRequestV1 {},
        )
        .await?;
        if state.2.load(Ordering::SeqCst) != self.epoch {
            return Err("CONFIRMATION_STALE".into());
        }
        if before.revisions != after.revisions
            || before.active_publication != after.active_publication
            || before.recovery_ready != after.recovery_ready
            || before.mutation_available != after.mutation_available
            || self.native != session.authority.native_connection()
        {
            return Err("READ_SNAPSHOT_CHANGED".into());
        }
        Ok(guard)
    }
}

pub async fn desktop_snapshot(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    measured_read(
        state,
        ControlOperation::DesktopSnapshot,
        desktop_snapshot_inner(window, state),
    )
    .await
}

async fn desktop_snapshot_inner(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
) -> Result<DesktopSnapshot, DesktopFailure> {
    main_window(window)?;
    let read = ReadContext::capture(state, true).await?;
    let service = read.status().await?;
    let catalog = crate::session::read_plan_catalog(&read.client).await;
    let guard = read.finish(state, &service).await?;
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    Ok(guard
        .as_ref()
        .ok_or("RESIDENT_UNAVAILABLE")?
        .snapshot_from_read(service, catalog))
}

pub async fn observation_read(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: ObservationReadRequestV2,
) -> Result<Value, DesktopFailure> {
    measured_read(
        state,
        ControlOperation::ObservationRead,
        observation_read_inner(window, state, request),
    )
    .await
}

async fn observation_read_inner(
    window: &ClientContext<'_>,
    state: &CommandState<'_>,
    request: ObservationReadRequestV2,
) -> Result<Value, DesktopFailure> {
    main_window(window)?;
    let read = ReadContext::capture(state, false).await?;
    let before = read.status().await?;
    let capability = read
        .authority
        .register_observation(&request, &before.revisions)?;
    let envelope: MachineEnvelopeV2<Value> = read
        .client
        .observation_read(
            &crate::random_id()?,
            request,
            ProtectedClientGrantV2 {
                principal_kind: PrincipalKind::Desktop,
                capability: capability.to_string(),
            },
        )
        .await?;
    if envelope.error.is_some() {
        return Err(DesktopFailure::backend(envelope));
    }
    let _guard = read.finish(state, &before).await?;
    if !window.is_active() {
        return Err("CLIENT_CONTEXT_INVALID".into());
    }
    envelope.data.ok_or_else(|| "RESPONSE_DATA_MISSING".into())
}
