//! Real AppKit registration runs on a fresh process's main thread. No windows,
//! login registration, daemon, credentials or user data are created by this test.
#[cfg(target_os = "macos")]
fn main() {
    if let Some(case) =
        std::env::args().find_map(|arg| arg.strip_prefix("--case=").map(str::to_owned))
    {
        macos::check(case == "login");
        return;
    }
    println!("running 2 tests");
    for case in ["foreground", "login"] {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .arg(format!("--case={case}"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{case}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        println!("test appkit_launch_{case} ... ok");
    }
    println!("test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out");
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("running 2 tests");
    println!("test appkit_launch_foreground ... ignored, requires macOS");
    println!("test appkit_launch_login ... ignored, requires macOS");
    println!("test result: ok. 0 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out");
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    use std::ffi::c_void;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    type Handler = unsafe extern "C" fn(*const c_void, *mut c_void, isize) -> i16;
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AEGetEventHandler(
            class: u32,
            id: u32,
            handler: *mut Option<Handler>,
            refcon: *mut isize,
            system: u8,
        ) -> i16;
        fn AECreateDesc(kind: u32, data: *const c_void, length: isize, desc: *mut c_void) -> i16;
        fn AECreateAppleEvent(
            class: u32,
            id: u32,
            address: *const c_void,
            return_id: i16,
            transaction: i32,
            desc: *mut c_void,
        ) -> i16;
        fn AEPutAttributePtr(
            event: *mut c_void,
            key: u32,
            kind: u32,
            data: *const c_void,
            length: isize,
        ) -> i16;
        fn AEDisposeDesc(desc: *mut c_void) -> i16;
    }
    pub fn check(login: bool) {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        // Prohibited activation keeps this isolated lifecycle test off the Dock.
        unsafe {
            let _: bool = objc2::msg_send![&*app, setActivationPolicy: 2isize];
        }
        let called = Arc::new(AtomicBool::new(false));
        let callback = called.clone();
        assert!(hiroute_desktop_host_effects::observe_launch_reason(
            move |actual| {
                assert_eq!(actual, login);
                callback.store(true, Ordering::SeqCst);
            }
        ));
        app.finishLaunching();
        // AppKit has now registered its own handlers. Exercise the handler that
        // is actually registered, rather than invoking our implementation directly.
        let mut handler = None;
        let mut refcon = 0;
        let class = u32::from_be_bytes(*b"aevt");
        let id = u32::from_be_bytes(*b"oapp");
        let mut address = [0u64; 2];
        let mut event = [0u64; 2];
        let mut reply = [0u64; 2];
        unsafe {
            assert_eq!(
                AEGetEventHandler(class, id, &mut handler, &mut refcon, 0),
                0
            );
            assert_eq!(
                AECreateDesc(0, std::ptr::null(), 0, address.as_mut_ptr().cast()),
                0
            );
            assert_eq!(
                AECreateAppleEvent(
                    class,
                    id,
                    address.as_ptr().cast(),
                    -1,
                    0,
                    event.as_mut_ptr().cast()
                ),
                0
            );
            assert_eq!(
                AECreateDesc(0, std::ptr::null(), 0, reply.as_mut_ptr().cast()),
                0
            );
            let value = u8::from(login);
            assert_eq!(
                AEPutAttributePtr(
                    event.as_mut_ptr().cast(),
                    u32::from_be_bytes(*b"lgit"),
                    u32::from_be_bytes(*b"bool"),
                    std::ptr::addr_of!(value).cast(),
                    1
                ),
                0
            );
            handler.unwrap()(event.as_ptr().cast(), reply.as_mut_ptr().cast(), refcon);
            AEDisposeDesc(reply.as_mut_ptr().cast());
            AEDisposeDesc(event.as_mut_ptr().cast());
            AEDisposeDesc(address.as_mut_ptr().cast());
        }
        assert!(
            called.load(Ordering::SeqCst),
            "AppKit replaced the launch observer"
        );
        assert_eq!(
            hiroute_desktop_host_effects::launch_reason_recorded(),
            Some(login)
        );
    }
}
