//! Shared readiness state for the two Linux input capabilities:
//! reading the global trigger key from `/dev/input/event*` and pressing
//! `Ctrl+V` through `/dev/uinput`.

use std::sync::{Arc, Mutex};

use serde::Serialize;

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
    pub trigger: Option<String>,
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Injection {
    /// A virtual keyboard is open on `/dev/uinput`, so Ctrl+V can be pressed.
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
