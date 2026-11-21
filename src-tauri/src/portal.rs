//! The parts of the desktop that Sotto asks permission for.
//!
//! Two capabilities, both through xdg-desktop-portal: a global shortcut to
//! start and stop dictation, and a remote-desktop session that lets us press
//! Ctrl+V in the app that has focus. Both refuse to work without the app id
//! registration described in ADR-0007, so that happens first, once.

use std::sync::{Arc, Mutex};

use serde::Serialize;

/// Must match the Tauri identifier and the installed desktop entry, or the
/// portal will not associate this process with it (ADR-0007).
pub const APP_ID: &str = "io.github.atpaawej.Sotto";

/// Registers this process as `APP_ID` with the portal on the connection ashpd
/// uses for every later call.
pub async fn register() -> Result<(), String> {
    let app_id = ashpd::AppID::try_from(APP_ID)
        .map_err(|error| format!("{APP_ID} is not a valid portal application id: {error}"))?;

    match ashpd::register_host_app(app_id).await {
        Ok(()) => Ok(()),
        // A second registration on the same connection is success, not an error.
        Err(error) if is_already_registered(&error) => Ok(()),
        Err(error) => Err(format!("the portal refused the app id: {error}")),
    }
}

fn is_already_registered(error: &ashpd::Error) -> bool {
    error
        .to_string()
        .contains("already associated with an application ID")
}

/// What the window needs to explain two invisible capabilities.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Access {
    pub hotkey: Hotkey,
    pub injection: Injection,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotkey {
    pub bound: bool,
    /// What the system actually assigned, which on GNOME is the user's choice
    /// rather than the trigger we asked for.
    pub trigger: Option<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Injection {
    /// A remote-desktop session is open, so Ctrl+V can be pressed.
    pub ready: bool,
    pub problem: Option<String>,
}

pub type SharedAccess = Arc<Mutex<Access>>;

pub fn shared_access() -> SharedAccess {
    Arc::new(Mutex::new(Access::default()))
}

/// Runs `change` under the mutex, and does nothing if a thread panicked.
pub fn update(access: &SharedAccess, change: impl FnOnce(&mut Access)) {
    if let Ok(mut slot) = access.lock() {
        change(&mut slot);
    }
}

pub fn snapshot(access: &SharedAccess) -> Access {
    access.lock().map(|slot| slot.clone()).unwrap_or_default()
}
