//! Native launch-reason observation for the resident host.
//!
//! A login-item launch is identified from the open-application Apple event itself
//! (`keyAELaunchedAsLogInItem`, lgit, in `AERegistry.h`) — never from the launchd parent PID.
//! The observer chains to whatever kAEOpenApplication handler was already installed, so
//! NSApplication's own handling of the launch event is preserved.
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, Once};

// Four-char codes from AEDataModel.h / AERegistry.h.
const K_CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const K_AE_OPEN_APPLICATION: u32 = u32::from_be_bytes(*b"oapp");
const KEY_AE_LAUNCHED_AS_LOGIN_ITEM: u32 = u32::from_be_bytes(*b"lgit");
const TYPE_BOOLEAN: u32 = u32::from_be_bytes(*b"bool");

const NO_ERR: i16 = 0;

/// Identifies our observer in AEGetEventHandler without comparing function pointers.
const OBSERVER_REFCON: isize = 0x4869_526F_7574;

type AppleEventRef = *const std::ffi::c_void;
type EventHandler = unsafe extern "C" fn(AppleEventRef, *mut std::ffi::c_void, isize) -> i16;
type LaunchDecision = Box<dyn FnOnce(bool) + Send>;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AEInstallEventHandler(
        event_class: u32,
        event_id: u32,
        handler: Option<EventHandler>,
        handler_refcon: isize,
        is_sys_handler: u8,
    ) -> i16;
    fn AEGetEventHandler(
        event_class: u32,
        event_id: u32,
        handler: *mut Option<EventHandler>,
        handler_refcon: *mut isize,
        is_sys_handler: u8,
    ) -> i16;
    fn AEGetAttributePtr(
        event: AppleEventRef,
        name: u32,
        desired_type: u32,
        type_code: *mut u32,
        data: *mut std::ffi::c_void,
        maximum_size: isize,
        actual_size: *mut isize,
    ) -> i16;
}

// RECORDED: 0 unknown, 1 normal foreground, 2 login-item launch.
static RECORDED: AtomicU8 = AtomicU8::new(0);
static DISPATCHED: AtomicBool = AtomicBool::new(false);
static PREVIOUS: Mutex<(Option<EventHandler>, isize)> = Mutex::new((None, 0));
static ON_LAUNCH: Mutex<Option<LaunchDecision>> = Mutex::new(None);
static OBSERVE_LIFECYCLE: Once = Once::new();

fn observe_appkit_registration() {
    OBSERVE_LIFECYCLE.call_once(|| {
        // NSApplication replaces the Carbon event handler during finishLaunching,
        // after App construction but before WillFinishLaunching. Register at the
        // documented lifecycle boundary, without replacing the framework delegate.
        let callback =
            block2::RcBlock::new(|_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                reinstate_launch_reason_observer();
            });
        let observer = unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter()
                .addObserverForName_object_queue_usingBlock(
                    Some(objc2_app_kit::NSApplicationWillFinishLaunchingNotification),
                    None,
                    None,
                    &callback,
                )
        };
        // A single process-lifetime subscription, just like the native handler.
        // The notification is synchronous; no queue or timing retry is involved.
        std::mem::forget(observer);
    });
}

unsafe extern "C" fn open_application_handler(
    event: AppleEventRef,
    reply: *mut std::ffi::c_void,
    _refcon: isize,
) -> i16 {
    let login_item = reads_launched_as_login_item(event);
    RECORDED.store(if login_item { 2 } else { 1 }, Ordering::SeqCst);
    // Preserve the previously installed handler first; the observation must not change how
    // the system dispatches its own launch event.
    // The chained AppKit handler may synchronously enter Tauri setup/Ready and
    // reinstall handlers. Never hold our lock across that reentrant native call.
    let previous = *PREVIOUS.lock().unwrap();
    let status = match previous {
        (Some(previous), refcon) => unsafe { previous(event, reply, refcon) },
        _ => NO_ERR,
    };
    if !DISPATCHED.swap(true, Ordering::SeqCst)
        && let Some(on_launch) = ON_LAUNCH.lock().unwrap().take()
    {
        on_launch(login_item);
    }
    status
}

fn reads_launched_as_login_item(event: AppleEventRef) -> bool {
    let mut value: u8 = 0;
    let mut type_code: u32 = 0;
    let mut actual_size: isize = 0;
    let status = unsafe {
        AEGetAttributePtr(
            event,
            KEY_AE_LAUNCHED_AS_LOGIN_ITEM,
            TYPE_BOOLEAN,
            &mut type_code,
            std::ptr::addr_of_mut!(value).cast(),
            1,
            &mut actual_size,
        )
    };
    // An unreadable attribute cannot prove a login-item launch; a foreground launch is the
    // only safe fallback because hiding a normal launch would break the product.
    status == NO_ERR && actual_size >= 1 && value != 0
}

fn observer_is_installed() -> bool {
    let mut handler: Option<EventHandler> = None;
    let mut refcon: isize = 0;
    let status = unsafe {
        AEGetEventHandler(
            K_CORE_EVENT_CLASS,
            K_AE_OPEN_APPLICATION,
            &mut handler,
            &mut refcon,
            0,
        )
    };
    status == NO_ERR && refcon == OBSERVER_REFCON && handler.is_some()
}

fn install_chained() -> bool {
    let mut previous: Option<EventHandler> = None;
    let mut refcon: isize = 0;
    let status = unsafe {
        AEGetEventHandler(
            K_CORE_EVENT_CLASS,
            K_AE_OPEN_APPLICATION,
            &mut previous,
            &mut refcon,
            0,
        )
    };
    let captured = if status == NO_ERR {
        (previous, refcon)
    } else {
        (None, 0)
    };
    let installed = unsafe {
        AEInstallEventHandler(
            K_CORE_EVENT_CLASS,
            K_AE_OPEN_APPLICATION,
            Some(open_application_handler),
            OBSERVER_REFCON,
            0,
        ) == NO_ERR
    };
    if installed {
        *PREVIOUS.lock().unwrap() = captured;
    }
    installed
}

/// Installs the chained launch-event observer. `on_launch` runs exactly once, on the main
/// thread, when the system dispatches the process's open-application event; its argument is
/// true only for a launch caused by the login item. Returns false when no observer could be
/// installed, in which case the caller falls back to foreground presentation.
pub fn observe_launch_reason(on_launch: impl FnOnce(bool) + Send + 'static) -> bool {
    *ON_LAUNCH.lock().unwrap() = Some(Box::new(on_launch));
    observe_appkit_registration();
    install_chained()
}

/// Reinstalls the observer when something replaced it after setup (NSApplication may
/// register its own kAEOpenApplication handler late). A no-op while ours is installed or
/// the launch event has already been observed.
pub fn reinstate_launch_reason_observer() {
    if DISPATCHED.load(Ordering::SeqCst) || observer_is_installed() {
        return;
    }
    install_chained();
}

/// The observed launch reason: `Some(true)` for a login-item launch, `Some(false)` for a
/// normal foreground launch, `None` until the open-application event was dispatched.
pub fn launch_reason_recorded() -> Option<bool> {
    match RECORDED.load(Ordering::SeqCst) {
        2 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // A synthetic open-application event exercises the attribute parsing without a GUI.
    // AEDesc is 16 bytes on LP64 (descriptorType plus a data handle).
    type DescStorage = [u64; 2];

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AECreateDesc(
            type_code: u32,
            data: *const std::ffi::c_void,
            size: isize,
            desc: *mut std::ffi::c_void,
        ) -> i16;
        fn AECreateAppleEvent(
            event_class: u32,
            event_id: u32,
            address: *const std::ffi::c_void,
            return_id: i16,
            transaction_id: i32,
            event: *mut std::ffi::c_void,
        ) -> i16;
        fn AEPutAttributePtr(
            event: *mut std::ffi::c_void,
            name: u32,
            type_code: u32,
            data: *const std::ffi::c_void,
            size: isize,
        ) -> i16;
        fn AEDisposeDesc(event: *mut std::ffi::c_void) -> i16;
    }

    fn synthetic_event(login_item: Option<bool>) -> DescStorage {
        // An empty address desc is enough: nothing dispatches this event.
        let mut address: DescStorage = [0; 2];
        let address_type: u32 = 0;
        unsafe {
            assert_eq!(
                AECreateDesc(
                    address_type,
                    std::ptr::null(),
                    0,
                    std::ptr::addr_of_mut!(address).cast(),
                ),
                NO_ERR
            );
        }
        let mut event: DescStorage = [0; 2];
        let status = unsafe {
            AECreateAppleEvent(
                K_CORE_EVENT_CLASS,
                K_AE_OPEN_APPLICATION,
                std::ptr::addr_of!(address).cast(),
                -1,
                0,
                std::ptr::addr_of_mut!(event).cast(),
            )
        };
        unsafe {
            AEDisposeDesc(std::ptr::addr_of_mut!(address).cast());
        }
        assert_eq!(status, NO_ERR, "the synthetic launch event must build");
        if let Some(login_item) = login_item {
            let value: u8 = u8::from(login_item);
            let status = unsafe {
                AEPutAttributePtr(
                    std::ptr::addr_of_mut!(event).cast(),
                    KEY_AE_LAUNCHED_AS_LOGIN_ITEM,
                    TYPE_BOOLEAN,
                    std::ptr::addr_of!(value).cast(),
                    1,
                )
            };
            assert_eq!(status, NO_ERR);
        }
        event
    }

    #[test]
    fn the_launch_attribute_decides_the_login_item_reason() {
        for (login_item, expected) in [(Some(true), true), (Some(false), false), (None, false)] {
            let mut event = synthetic_event(login_item);
            assert_eq!(
                reads_launched_as_login_item(std::ptr::addr_of!(event).cast()),
                expected
            );
            unsafe { AEDisposeDesc(std::ptr::addr_of_mut!(event).cast()) };
        }
    }

    #[test]
    fn the_observer_installs_and_reports_no_reason_before_dispatch() {
        static CHAIN_ENTERED: AtomicBool = AtomicBool::new(false);
        static CHAIN_UNLOCKED: AtomicBool = AtomicBool::new(false);
        static REASON_RECORDED_BEFORE_CHAIN: AtomicBool = AtomicBool::new(false);
        unsafe extern "C" fn previous(_: AppleEventRef, _: *mut std::ffi::c_void, _: isize) -> i16 {
            CHAIN_ENTERED.store(true, Ordering::SeqCst);
            CHAIN_UNLOCKED.store(PREVIOUS.try_lock().is_ok(), Ordering::SeqCst);
            REASON_RECORDED_BEFORE_CHAIN
                .store(launch_reason_recorded() == Some(true), Ordering::SeqCst);
            -1708
        }
        let callback_ran = Arc::new(AtomicBool::new(false));
        let callback = callback_ran.clone();
        let installed = observe_launch_reason(move |login_item| {
            assert!(login_item);
            assert!(CHAIN_ENTERED.load(Ordering::SeqCst));
            callback.store(true, Ordering::SeqCst);
        });
        assert!(installed, "AEInstallEventHandler must succeed in-process");
        assert_eq!(launch_reason_recorded(), None);
        assert!(observer_is_installed());
        // A guarded reinstall never stacks the observer over itself.
        reinstate_launch_reason_observer();
        assert!(observer_is_installed());
        let native_previous = *PREVIOUS.lock().unwrap();
        *PREVIOUS.lock().unwrap() = (Some(previous), 0);
        let mut event = synthetic_event(Some(true));
        let status = unsafe {
            open_application_handler(std::ptr::addr_of!(event).cast(), std::ptr::null_mut(), 0)
        };
        unsafe { AEDisposeDesc(std::ptr::addr_of_mut!(event).cast()) };
        *PREVIOUS.lock().unwrap() = native_previous;
        assert_eq!(status, -1708, "preserve the native handler result");
        assert!(REASON_RECORDED_BEFORE_CHAIN.load(Ordering::SeqCst));
        assert!(CHAIN_UNLOCKED.load(Ordering::SeqCst));
        assert!(callback_ran.load(Ordering::SeqCst));
    }
}
