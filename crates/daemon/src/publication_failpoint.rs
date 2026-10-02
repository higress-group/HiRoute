//! Process-exit injection for the real-binary recovery suite. Absent in normal builds.
#[cfg(feature = "integration-test-hooks")]
thread_local! {
    static OPERATION_INSTALL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn during_operation_install<T>(install: impl FnOnce() -> T) -> T {
    #[cfg(feature = "integration-test-hooks")]
    {
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                OPERATION_INSTALL.with(|armed| armed.set(self.0));
            }
        }
        let _reset = Reset(OPERATION_INSTALL.with(|armed| armed.replace(true)));
        install()
    }
    #[cfg(not(feature = "integration-test-hooks"))]
    install()
}

pub(crate) fn requested(boundary: &str) -> bool {
    #[cfg(feature = "integration-test-hooks")]
    {
        std::env::var("HIROUTE_TEST_PUBLICATION_CRASH_AT").as_deref() == Ok(boundary)
            && (boundary != "after_gateway_durable" || OPERATION_INSTALL.with(std::cell::Cell::get))
    }
    #[cfg(not(feature = "integration-test-hooks"))]
    {
        let _ = boundary;
        false
    }
}

pub(crate) fn crash(boundary: &str) {
    if requested(boundary) {
        std::process::exit(86);
    }
}
