//! Deterministic admission pressure for production-dispatch regression tests.
use crate::LocalObservationStore;

impl LocalObservationStore {
    /// Hold the same four permits used by production reads for this closure.
    /// No query result, error mapping or retry response is substituted.
    pub fn with_all_query_permits_for_test<T>(&self, read: impl FnOnce() -> T) -> T {
        let _permits: Vec<_> = (0..4)
            .map(|_| self.query_permit().expect("test starts with idle readers"))
            .collect();
        read()
    }
}
