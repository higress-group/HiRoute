//! Typed Task Scheduler COM boundary. Recovery may export an owned native Task's XML.
use std::io;
use windows::Win32::Foundation::{VARIANT_BOOL, VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::TaskScheduler::*;
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, Interface};

use super::background::{Definition, Manager, com_error};

struct Apartment;
impl Apartment {
    fn new() -> io::Result<Self> {
        // SAFETY: every public operation runs in its own apartment/thread and
        // balances successful initialization after all COM interfaces drop.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(com_error)?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn with_folder<T: Send + 'static>(
    definition: &Definition,
    operation: impl FnOnce(&ITaskFolder, &Definition) -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let definition = definition.clone();
    definition.validate()?;
    if definition.manager != Manager::Task {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    std::thread::spawn(move || {
        let _apartment = Apartment::new()?;
        // SAFETY: live interfaces stay on this initialized apartment, never
        // shared with Tokio threads. Empty credential variants select self.
        let folder = unsafe {
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(com_error)?;
            let empty = VARIANT::default();
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(com_error)?;
            service.GetFolder(&BSTR::from("\\")).map_err(com_error)?
        };
        operation(&folder, &definition)
    })
    .join()
    .map_err(|_| io::Error::other("Task Scheduler apartment panicked"))?
}

/// An existing registration can be updated only after matching the previous
/// owned definition. The installer retains that definition until commit.
pub fn register(definition: &Definition, previous: Option<&Definition>) -> io::Result<()> {
    definition.verify_image()?;
    let previous = previous.cloned();
    with_folder(definition, move |folder, definition| {
        // SAFETY: all out-pointers below point to initialized Rust values; all
        // BSTR/VARIANT buffers and interfaces outlive their synchronous calls.
        unsafe {
            let current = folder.GetTask(&BSTR::from(&definition.name));
            let mut autostart = false;
            let flags = match current {
                Ok(task) => {
                    if verify_task(&task, previous.as_ref().unwrap_or(definition)).is_err() {
                        verify_task(&task, definition)?;
                    }
                    let instances = task.GetInstances(0).map_err(com_error)?;
                    let running = instances.Count().map_err(com_error)?;
                    if running != 0 {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                    autostart = trigger_enabled(&task)?;
                    TASK_UPDATE.0 | TASK_IGNORE_REGISTRATION_TRIGGERS.0
                }
                Err(error) if missing(&error) => TASK_CREATE.0,
                Err(error) => return Err(com_error(error)),
            };
            // Obtain the connected service through a fresh in-apartment proxy.
            let service: ITaskService =
                CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER).map_err(com_error)?;
            let empty = VARIANT::default();
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(com_error)?;
            let task = service.NewTask(0).map_err(com_error)?;
            let principal = task.Principal().map_err(com_error)?;
            principal
                .SetUserId(&BSTR::from(&definition.owner_sid))
                .map_err(com_error)?;
            principal
                .SetLogonType(TASK_LOGON_INTERACTIVE_TOKEN)
                .map_err(com_error)?;
            principal
                .SetRunLevel(TASK_RUNLEVEL_LUA)
                .map_err(com_error)?;
            let settings = task.Settings().map_err(com_error)?;
            settings.SetEnabled(VARIANT_TRUE).map_err(com_error)?;
            settings
                .SetAllowDemandStart(VARIANT_TRUE)
                .map_err(com_error)?;
            settings
                .SetMultipleInstances(TASK_INSTANCES_IGNORE_NEW)
                .map_err(com_error)?;
            settings
                .SetExecutionTimeLimit(&BSTR::from("PT0S"))
                .map_err(com_error)?;
            settings
                .SetDisallowStartIfOnBatteries(VARIANT_FALSE)
                .map_err(com_error)?;
            settings
                .SetStopIfGoingOnBatteries(VARIANT_FALSE)
                .map_err(com_error)?;
            settings
                .SetRunOnlyIfIdle(VARIANT_FALSE)
                .map_err(com_error)?;
            settings
                .SetRunOnlyIfNetworkAvailable(VARIANT_FALSE)
                .map_err(com_error)?;
            settings
                .SetStartWhenAvailable(VARIANT_TRUE)
                .map_err(com_error)?;
            settings
                .SetRestartInterval(&BSTR::from("PT1M"))
                .map_err(com_error)?;
            settings.SetRestartCount(3).map_err(com_error)?;
            let trigger = task
                .Triggers()
                .map_err(com_error)?
                .Create(TASK_TRIGGER_LOGON)
                .map_err(com_error)?;
            let logon: ILogonTrigger = trigger.cast().map_err(com_error)?;
            logon
                .SetUserId(&BSTR::from(&definition.owner_sid))
                .map_err(com_error)?;
            // First install is demand-only. Updating preserves login policy.
            trigger.SetEnabled(boolean(autostart)).map_err(com_error)?;
            let action: IExecAction = task
                .Actions()
                .map_err(com_error)?
                .Create(TASK_ACTION_EXEC)
                .map_err(com_error)?
                .cast()
                .map_err(com_error)?;
            let path = definition
                .daemon
                .to_str()
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            let directory = definition
                .daemon
                .parent()
                .and_then(|p| p.to_str())
                .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
            action.SetPath(&BSTR::from(path)).map_err(com_error)?;
            action
                .SetArguments(&BSTR::from(definition.arguments()))
                .map_err(com_error)?;
            action
                .SetWorkingDirectory(&BSTR::from(directory))
                .map_err(com_error)?;
            let sid = VARIANT::from(definition.owner_sid.as_str());
            let registered = folder
                .RegisterTaskDefinition(
                    &BSTR::from(&definition.name),
                    &task,
                    flags,
                    &sid,
                    &empty,
                    TASK_LOGON_INTERACTIVE_TOKEN,
                    &empty,
                )
                .map_err(com_error)?;
            verify_task(&registered, definition)
        }
    })
}

pub fn start(definition: &Definition) -> io::Result<()> {
    definition.verify_image()?;
    with_folder(definition, |folder, definition| unsafe {
        let task = owned_task(folder, definition)?;
        task.Run(&VARIANT::default()).map_err(com_error)?;
        Ok(())
    })
}

/// Only used after graceful stop's deadline. Task Stop is not proof of child
/// cleanup; the actual daemon holds the kill-on-close Job for that guarantee.
pub fn force_stop(definition: &Definition) -> io::Result<()> {
    with_folder(definition, |folder, definition| unsafe {
        let task = owned_task(folder, definition)?;
        task.Stop(0).map_err(com_error)
    })
}

pub fn active(definition: &Definition) -> io::Result<bool> {
    with_folder(definition, |folder, definition| unsafe {
        let task = match folder.GetTask(&BSTR::from(&definition.name)) {
            Ok(task) => {
                verify_task(&task, definition)?;
                task
            }
            Err(error) if missing(&error) => return Ok(false),
            Err(error) => return Err(com_error(error)),
        };
        // Do not use IRunningTask::EnginePID: it identifies the scheduler host.
        Ok(task
            .GetInstances(0)
            .map_err(com_error)?
            .Count()
            .map_err(com_error)?
            > 0)
    })
}

pub fn autostart(definition: &Definition) -> io::Result<bool> {
    with_folder(definition, |folder, definition| unsafe {
        let registered = match folder.GetTask(&BSTR::from(&definition.name)) {
            Ok(task) => {
                verify_task(&task, definition)?;
                task
            }
            Err(error) if missing(&error) => return Ok(false),
            Err(error) => return Err(com_error(error)),
        };
        trigger_enabled(&registered)
    })
}

/// Preserve the scheduler's actual registration for manual whole-group recovery.
/// This reads only the exact owned Task; it never registers or generates a Task.
pub fn recovery_xml(definition: &Definition) -> io::Result<Vec<u8>> {
    definition.verify_image()?;
    with_folder(definition, |folder, definition| unsafe {
        let registered = owned_task(folder, definition)?;
        let xml = registered.Xml().map_err(com_error)?.to_string();
        if xml.is_empty() || xml.len() > 256 * 1024 {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        // Scheduler XML declares UTF-16. Preserve that encoding with a BOM so
        // Task Scheduler and schtasks can import the retained file directly.
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        Ok(bytes)
    })
}

pub fn set_autostart(definition: &Definition, enabled: bool) -> io::Result<()> {
    with_folder(definition, move |folder, definition| unsafe {
        let registered = owned_task(folder, definition)?;
        let task = registered.Definition().map_err(com_error)?;
        let trigger = task
            .Triggers()
            .map_err(com_error)?
            .get_Item(1)
            .map_err(com_error)?;
        trigger.SetEnabled(boolean(enabled)).map_err(com_error)?;
        let empty = VARIANT::default();
        folder
            .RegisterTaskDefinition(
                &BSTR::from(&definition.name),
                &task,
                TASK_UPDATE.0 | TASK_IGNORE_REGISTRATION_TRIGGERS.0,
                &VARIANT::from(definition.owner_sid.as_str()),
                &empty,
                TASK_LOGON_INTERACTIVE_TOKEN,
                &empty,
            )
            .map_err(com_error)?;
        // The task stays enabled, so demand start still works without logon autostart.
        Ok(())
    })
}

pub fn unregister(definition: &Definition) -> io::Result<()> {
    with_folder(definition, |folder, definition| unsafe {
        match folder.GetTask(&BSTR::from(&definition.name)) {
            Ok(task) => {
                verify_task(&task, definition)?;
                folder
                    .DeleteTask(&BSTR::from(&definition.name), 0)
                    .map_err(com_error)
            }
            Err(error) if missing(&error) => Ok(()),
            Err(error) => Err(com_error(error)),
        }
    })
}

unsafe fn owned_task(folder: &ITaskFolder, definition: &Definition) -> io::Result<IRegisteredTask> {
    // SAFETY: caller holds initialized in-apartment interfaces.
    let task = unsafe { folder.GetTask(&BSTR::from(&definition.name)) }.map_err(com_error)?;
    unsafe {
        verify_task(&task, definition)?;
    }
    Ok(task)
}

unsafe fn trigger_enabled(task: &IRegisteredTask) -> io::Result<bool> {
    let mut enabled = VARIANT_FALSE;
    // SAFETY: caller validated this single-trigger owned task, and output is live.
    unsafe {
        task.Definition()
            .map_err(com_error)?
            .Triggers()
            .map_err(com_error)?
            .get_Item(1)
            .map_err(com_error)?
            .Enabled(&mut enabled)
            .map_err(com_error)?;
    }
    Ok(enabled == VARIANT_TRUE)
}

unsafe fn verify_task(registered: &IRegisteredTask, expected: &Definition) -> io::Result<()> {
    expected.validate()?;
    // SAFETY: caller owns this apartment and all output values below.
    unsafe {
        let task = registered.Definition().map_err(com_error)?;
        let principal = task.Principal().map_err(com_error)?;
        let mut sid = BSTR::new();
        let mut logon_type = TASK_LOGON_NONE;
        let mut level = TASK_RUNLEVEL_HIGHEST;
        principal.UserId(&mut sid).map_err(com_error)?;
        principal.LogonType(&mut logon_type).map_err(com_error)?;
        principal.RunLevel(&mut level).map_err(com_error)?;
        let actions = task.Actions().map_err(com_error)?;
        let mut count = 0;
        actions.Count(&mut count).map_err(com_error)?;
        let owner_matches = same_owner(&sid.to_string(), &expected.owner_sid)?;
        let enabled = registered.Enabled().map_err(com_error)?;
        if !owner_matches
            || logon_type != TASK_LOGON_INTERACTIVE_TOKEN
            || level != TASK_RUNLEVEL_LUA
            || count != 1
            || enabled != VARIANT_TRUE
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "background task principal differs: owner_match={owner_matches}, logon={}, level={}, actions={count}, enabled={}",
                    logon_type.0, level.0, enabled.0
                ),
            ));
        }
        let action: IExecAction = actions
            .get_Item(1)
            .map_err(com_error)?
            .cast()
            .map_err(com_error)?;
        let mut path = BSTR::new();
        let mut arguments = BSTR::new();
        let mut working = BSTR::new();
        action.Path(&mut path).map_err(com_error)?;
        action.Arguments(&mut arguments).map_err(com_error)?;
        action.WorkingDirectory(&mut working).map_err(com_error)?;
        if !same_path(&path.to_string(), &expected.daemon, false)?
            || arguments != expected.arguments().as_str()
            || !same_path(
                &working.to_string(),
                expected.daemon.parent().ok_or_else(foreign)?,
                true,
            )?
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "background task action image, arguments or working directory differs",
            ));
        }
        let triggers = task.Triggers().map_err(com_error)?;
        triggers.Count(&mut count).map_err(com_error)?;
        if count != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("background task trigger count differs: {count}"),
            ));
        }
        let logon: ILogonTrigger = triggers
            .get_Item(1)
            .map_err(com_error)?
            .cast()
            .map_err(com_error)?;
        let mut trigger_sid = BSTR::new();
        logon.UserId(&mut trigger_sid).map_err(com_error)?;
        if !same_owner(&trigger_sid.to_string(), &expected.owner_sid)? {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "background task logon trigger owner differs",
            ));
        }
        let settings = task.Settings().map_err(com_error)?;
        let mut limit = BSTR::new();
        let mut policy = TASK_INSTANCES_PARALLEL;
        let mut demand = VARIANT_FALSE;
        settings.ExecutionTimeLimit(&mut limit).map_err(com_error)?;
        settings.MultipleInstances(&mut policy).map_err(com_error)?;
        settings.AllowDemandStart(&mut demand).map_err(com_error)?;
        if limit != "PT0S" || policy != TASK_INSTANCES_IGNORE_NEW || demand != VARIANT_TRUE {
            let limit = limit.to_string();
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "background task settings differ: limit={:?}, instances={}, demand={}",
                    limit.chars().take(128).collect::<String>(),
                    policy.0,
                    demand.0
                ),
            ));
        }
        Ok(())
    }
}

// Scheduler may expose a resolved account name after registration. Authorize
// that value only by resolving its SID, never by display name or casing alone.
fn same_owner(actual: &str, expected: &str) -> io::Result<bool> {
    if actual == expected {
        return Ok(true);
    }
    if actual.is_empty() || actual.len() > 512 || actual.contains('\0') {
        return Ok(false);
    }
    use windows_sys::Win32::Security::{GetLengthSid, LookupAccountNameW, SID_NAME_USE};
    let name: Vec<u16> = actual.encode_utf16().chain(Some(0)).collect();
    // A SID contains native DWORD fields; retain an aligned allocation even on
    // targets that do not permit the unaligned integer reads tolerated by x64.
    let mut sid_words = [0_usize; 68_usize.div_ceil(std::mem::size_of::<usize>())];
    let sid = unsafe { std::slice::from_raw_parts_mut(sid_words.as_mut_ptr().cast::<u8>(), 68) };
    let mut sid_length = sid.len() as u32;
    let mut domain = [0_u16; 1024];
    let mut domain_length = domain.len() as u32;
    let mut usage: SID_NAME_USE = 0;
    if unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_length,
            domain.as_mut_ptr(),
            &mut domain_length,
            &mut usage,
        )
    } == 0
    {
        return Ok(false);
    }
    // cbSid is an input capacity; success does not promise to replace it with
    // the actual SID length. The returned native SID header determines that.
    if sid_length > sid.len() as u32 || sid[0] != 1 || sid[1] > 15 {
        return Ok(false);
    }
    let length = unsafe { GetLengthSid(sid.as_ptr().cast_mut().cast()) } as usize;
    if !(8..=sid.len()).contains(&length) {
        return Ok(false);
    }
    Ok(super::background::sid_string(&sid[..length])? == expected)
}

// Exact persisted spelling remains usable for uninstalling a damaged bundle.
// A casing change can be accepted only after matching held object identities;
// case-sensitive Windows directories must not turn casing into a new image.
fn same_path(actual: &str, expected: &std::path::Path, directory: bool) -> io::Result<bool> {
    let expected_text = expected.to_str().ok_or_else(foreign)?;
    let actual_text = actual.replace('/', "\\");
    let expected_text = expected_text.replace('/', "\\");
    if actual_text == expected_text {
        return Ok(true);
    }
    if !actual_text.eq_ignore_ascii_case(&expected_text) {
        return Ok(false);
    }
    let actual = super::security::open_identity(std::path::Path::new(actual), directory)?;
    let expected = super::security::open_identity(expected, directory)?;
    Ok(super::security::file_identity(&actual)? == super::security::file_identity(&expected)?)
}
fn boolean(value: bool) -> VARIANT_BOOL {
    if value { VARIANT_TRUE } else { VARIANT_FALSE }
}
fn missing(error: &windows::core::Error) -> bool {
    error.code().0 as u32 == 0x80070002
}
fn foreign() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "background task differs from owned definition",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_preserves_demand_start_and_logon_policy_across_reopen() {
        let temporary = tempfile::tempdir().unwrap();
        let image = temporary.path().join("hirouted.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &image).unwrap();
        let definition = Definition::new(Manager::Task, image).unwrap();
        assert!(
            same_owner(
                &super::super::service::account_name().unwrap(),
                &definition.owner_sid
            )
            .unwrap()
        );
        struct Remove(Definition);
        impl Drop for Remove {
            fn drop(&mut self) {
                let _ = unregister(&self.0);
            }
        }
        // A foreign/active registration with this user's name is never adopted.
        register(&definition, None).unwrap();
        let _remove = Remove(definition.clone());
        assert!(!autostart(&definition).unwrap());
        assert!(!active(&definition).unwrap());
        let xml = recovery_xml(&definition).unwrap();
        assert!(xml.starts_with(&[0xff, 0xfe]));
        assert!(
            String::from_utf16(
                &xml[2..]
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>()
            )
            .unwrap()
            .contains("<Task")
        );
        set_autostart(&definition, true).unwrap();
        assert!(autostart(&definition).unwrap());
        register(&definition, Some(&definition)).unwrap();
        assert!(autostart(&definition).unwrap());
        set_autostart(&definition, false).unwrap();
        assert!(!autostart(&definition).unwrap());
        with_folder(&definition, |folder, definition| unsafe {
            let registered = owned_task(folder, definition)?;
            assert_eq!(registered.Enabled().map_err(com_error)?, VARIANT_TRUE);
            Ok(())
        })
        .unwrap();
        let mut foreign = definition.clone();
        foreign.daemon = temporary.path().join("unowned/hirouted.exe");
        assert!(active(&foreign).is_err());
        assert!(recovery_xml(&foreign).is_err());
        unregister(&definition).unwrap();
        assert!(!active(&definition).unwrap());
        assert!(!autostart(&definition).unwrap());
    }
}
