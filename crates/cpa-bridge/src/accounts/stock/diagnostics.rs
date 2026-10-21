use std::time::{Duration, Instant};

use hiroute_diagnostics::context::DiagnosticContext;
use hiroute_diagnostics::event::{
    CpaControl, CpaControlPhase, CpaPinChecks, CpaStageOutcome, DiagnosticEvent,
};

use crate::accounts::AccountDiscoveryError;

pub(super) struct DiscoveryTrace<'a> {
    context: &'a DiagnosticContext,
    started: Instant,
    budget_ms: u64,
    refresh_models: bool,
    pub account_ordinal: Option<u32>,
    pub polls: u32,
    pub checks: CpaPinChecks,
    model_discovery: super::model_discovery::Observations,
}

impl<'a> DiscoveryTrace<'a> {
    pub fn new(context: &'a DiagnosticContext, timeout: Duration, refresh_models: bool) -> Self {
        Self {
            context,
            started: Instant::now(),
            budget_ms: millis(timeout),
            refresh_models,
            account_ordinal: None,
            polls: 0,
            checks: CpaPinChecks::default(),
            model_discovery: Default::default(),
        }
    }

    pub fn model_discovery(&mut self, response: &serde_json::Value) {
        for observation in self.model_discovery.read(response, self.account_ordinal) {
            self.context
                .emit(DiagnosticEvent::CpaModelDiscovery(observation));
        }
    }

    pub fn run<T>(
        &mut self,
        phase: CpaControlPhase,
        operation: impl FnOnce(&mut Self) -> Result<T, AccountDiscoveryError>,
    ) -> Result<T, AccountDiscoveryError> {
        self.polls = 0;
        let started = Instant::now();
        self.emit(phase, CpaStageOutcome::Entered, Duration::ZERO);
        let result = operation(self);
        let outcome = match &result {
            Ok(_) => CpaStageOutcome::Completed,
            Err(error) => CpaStageOutcome::Failed {
                code: error.diagnostic_code(),
            },
        };
        self.emit(phase, outcome, started.elapsed());
        result
    }

    fn emit(&self, phase: CpaControlPhase, outcome: CpaStageOutcome, elapsed: Duration) {
        self.context.emit(DiagnosticEvent::CpaControl(CpaControl {
            phase,
            outcome,
            elapsed_ms: millis(elapsed),
            total_elapsed_ms: millis(self.started.elapsed()),
            budget_ms: self.budget_ms,
            refresh_models: self.refresh_models,
            account_ordinal: self.account_ordinal,
            polls: self.polls,
            checks: self.checks.clone(),
        }));
    }
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}
