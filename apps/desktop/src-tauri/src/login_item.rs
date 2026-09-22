//! Cleanup of login items created by older Agent settings, host-side only.
//!
//! New settings never register a login item. A final restore removes only an item with
//! journaled ownership evidence; a pre-existing user login item is never touched.
use hiroute_application_api::AgentLoginItemDeclarationV2;

#[cfg(target_os = "macos")]
use hiroute_application_api::AgentLoginItemStatusV2;

const SERVICE_UNAVAILABLE: &str = "SERVICE_UNAVAILABLE";

/// Unregisters the login item this feature owns when the last managed connection is restored.
/// The backend only requires this with journal proof of ownership, so a pre-existing user
/// item never reaches this path; an item the user already removed manually is already absent.
#[cfg(target_os = "macos")]
pub(crate) fn remove_resident_login_item() -> Result<AgentLoginItemDeclarationV2, &'static str> {
    use hiroute_application_api::AgentLoginItemStatusV2;
    use hiroute_desktop_host_effects as host;

    fn wire(status: host::LoginItemStatus) -> Option<AgentLoginItemStatusV2> {
        match status {
            host::LoginItemStatus::NotRegistered => Some(AgentLoginItemStatusV2::NotRegistered),
            host::LoginItemStatus::Enabled => Some(AgentLoginItemStatusV2::Enabled),
            host::LoginItemStatus::RequiresApproval => {
                Some(AgentLoginItemStatusV2::RequiresApproval)
            }
            host::LoginItemStatus::NotFound => Some(AgentLoginItemStatusV2::NotFound),
            host::LoginItemStatus::Unknown => None,
        }
    }

    let before = wire(host::main_app_login_item_status().map_err(|_| SERVICE_UNAVAILABLE)?)
        .ok_or(SERVICE_UNAVAILABLE)?;
    match before {
        AgentLoginItemStatusV2::Enabled | AgentLoginItemStatusV2::RequiresApproval => {
            host::unregister_main_app_login_item().map_err(|_| SERVICE_UNAVAILABLE)?;
            let after = wire(host::main_app_login_item_status().map_err(|_| SERVICE_UNAVAILABLE)?)
                .ok_or(SERVICE_UNAVAILABLE)?;
            if after == AgentLoginItemStatusV2::Enabled
                || after == AgentLoginItemStatusV2::RequiresApproval
            {
                return Err(SERVICE_UNAVAILABLE);
            }
            Ok(AgentLoginItemDeclarationV2 {
                before,
                after,
                created: false,
            })
        }
        AgentLoginItemStatusV2::NotRegistered | AgentLoginItemStatusV2::NotFound => {
            Ok(AgentLoginItemDeclarationV2 {
                before,
                after: before,
                created: false,
            })
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn remove_resident_login_item() -> Result<AgentLoginItemDeclarationV2, &'static str> {
    Err(SERVICE_UNAVAILABLE)
}

/// Re-registers the owned login item when a removal's restore definitively failed: the
/// rolled-back connection still needs the resident service at the next login.
#[cfg(target_os = "macos")]
pub(crate) fn compensate_resident_login_item_removal() {
    let _ = hiroute_desktop_host_effects::register_main_app_login_item();
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn compensate_resident_login_item_removal() {}
