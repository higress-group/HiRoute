//! Exact provider queries preserve their errors without inspecting healthy siblings.
use super::request_io::{SourceKind, active_fixture};
use super::*;
use crate::{CpaRegisteredSourcePort, ManagedCpaRuntimeSet};

#[test]
fn requested_provider_failure_remains_an_error_while_the_sibling_stays_available() {
    let claude = active_fixture(SourceKind::NativeClaude);
    let codex = active_fixture(SourceKind::NativeCodex);
    let runtimes = ManagedCpaRuntimeSet::new(vec![
        Arc::clone(&claude.runtime),
        Arc::clone(&codex.runtime),
    ])
    .unwrap();
    assert_eq!(
        runtimes
            .discover_registered_sources_for(CpaAccountKind::Claude)
            .unwrap()
            .len(),
        1
    );
    private_atomic_write(&claude.source_path, b"{").unwrap();
    let codex_before_failure = codex.control_calls();
    assert!(
        runtimes
            .discover_registered_sources_for(CpaAccountKind::Claude)
            .is_err(),
        "the failed requested provider became an empty successful query"
    );
    assert_eq!(codex.control_calls(), codex_before_failure);

    let failed_provider_calls = claude.control_calls();
    let healthy = runtimes
        .discover_registered_sources_for(CpaAccountKind::Codex)
        .unwrap();
    assert_eq!(healthy.len(), 1);
    assert_eq!(healthy[0].source.connector_id, "connector.cpa.codex");
    assert!(!healthy[0].inventory.is_empty());
    assert_eq!(
        claude.control_calls(),
        failed_provider_calls,
        "a healthy exact query performed I/O for the failed sibling"
    );

    // The availability list remains best effort; this distinct contract is still
    // useful while exact Check/Save queries preserve the requested failure.
    let aggregate = runtimes.discover_registered_sources().unwrap();
    assert_eq!(aggregate, healthy);
    runtimes.shutdown().unwrap();
}

#[test]
fn missing_requested_runtime_fails_without_touching_the_available_provider() {
    let codex = active_fixture(SourceKind::NativeCodex);
    let runtimes = ManagedCpaRuntimeSet::new(vec![Arc::clone(&codex.runtime)]).unwrap();
    let calls = codex.control_calls();
    assert!(matches!(
        runtimes.discover_registered_sources_for(CpaAccountKind::Claude),
        Err(CpaLifecycleError::NotStarted)
    ));
    assert_eq!(codex.control_calls(), calls);
    assert_eq!(
        runtimes
            .discover_registered_sources_for(CpaAccountKind::Codex)
            .unwrap()
            .len(),
        1
    );
    runtimes.shutdown().unwrap();
}
