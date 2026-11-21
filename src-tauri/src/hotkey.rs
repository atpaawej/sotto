//! The gesture: a global shortcut read directly from `/dev/input/event*`
//! via `evdev`, decoded into presses, releases, and Escape cancels for
//! [`crate::session`].

use std::{
    fs,
    io::ErrorKind,
    path::PathBuf,
    sync::mpsc::Sender,
    thread,
    time::Instant,
};

use evdev::{Device, EventSummary, EventType, KeyCode};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    keyboard,
    portal::{self, SharedAccess},
    session::Input,
};

/// Maps a configured key name (GTK/DOM style or evdev style) to an `evdev::KeyCode`.
pub fn key_from_name(name: &str) -> Result<KeyCode, String> {
    match name.trim() {
        "ControlRight" | "KEY_RIGHTCTRL" => Ok(KeyCode::KEY_RIGHTCTRL),
        "ControlLeft" | "KEY_LEFTCTRL" => Ok(KeyCode::KEY_LEFTCTRL),
        "AltRight" | "KEY_RIGHTALT" => Ok(KeyCode::KEY_RIGHTALT),
        "AltLeft" | "KEY_LEFTALT" => Ok(KeyCode::KEY_LEFTALT),
        "ShiftRight" | "KEY_RIGHTSHIFT" => Ok(KeyCode::KEY_RIGHTSHIFT),
        "ShiftLeft" | "KEY_LEFTSHIFT" => Ok(KeyCode::KEY_LEFTSHIFT),
        "SuperRight" | "MetaRight" | "KEY_RIGHTMETA" => Ok(KeyCode::KEY_RIGHTMETA),
        "SuperLeft" | "MetaLeft" | "KEY_LEFTMETA" => Ok(KeyCode::KEY_LEFTMETA),
        "CapsLock" | "KEY_CAPSLOCK" => Ok(KeyCode::KEY_CAPSLOCK),
        "Space" | "KEY_SPACE" => Ok(KeyCode::KEY_SPACE),
        "F1" | "KEY_F1" => Ok(KeyCode::KEY_F1),
        "F2" | "KEY_F2" => Ok(KeyCode::KEY_F2),
        "F3" | "KEY_F3" => Ok(KeyCode::KEY_F3),
        "F4" | "KEY_F4" => Ok(KeyCode::KEY_F4),
        "F5" | "KEY_F5" => Ok(KeyCode::KEY_F5),
        "F6" | "KEY_F6" => Ok(KeyCode::KEY_F6),
        "F7" | "KEY_F7" => Ok(KeyCode::KEY_F7),
        "F8" | "KEY_F8" => Ok(KeyCode::KEY_F8),
        "F9" | "KEY_F9" => Ok(KeyCode::KEY_F9),
        "F10" | "KEY_F10" => Ok(KeyCode::KEY_F10),
        "F11" | "KEY_F11" => Ok(KeyCode::KEY_F11),
        "F12" | "KEY_F12" => Ok(KeyCode::KEY_F12),
        other => Err(format!("unsupported trigger key `{other}`")),
    }
}

/// Opens all readable keyboard devices and listens for `configured_key` and `Escape`.
/// Re-probes whenever `retries` receives a signal.
pub async fn serve(
    access: SharedAccess,
    inbox: Sender<Input>,
    configured_key: String,
    mut retries: UnboundedReceiver<()>,
) {
    loop {
        match bind_keyboards(&access, &inbox, &configured_key) {
            Ok(()) => {
                // Keep the task alive so a retry can re-enumerate if needed.
                if retries.recv().await.is_none() {
                    return;
                }
            }
            Err(problem) => {
                eprintln!("hotkey: {problem}");
                portal::update(&access, |state| {
                    state.hotkey.bound = false;
                    state.hotkey.problem = Some(problem);
                });
                if retries.recv().await.is_none() {
                    return;
                }
            }
        }
    }
}

fn bind_keyboards(
    access: &SharedAccess,
    inbox: &Sender<Input>,
    configured_key: &str,
) -> Result<(), String> {
    let trigger = key_from_name(configured_key)?;
    let devices = find_keyboards(trigger)?;

    for (path, device) in devices {
        let tx = inbox.clone();
        thread::spawn(move || read_loop(path, device, trigger, tx));
    }

    eprintln!("hotkey: listening on evdev for {configured_key}");
    portal::update(access, |state| {
        state.hotkey.bound = true;
        state.hotkey.trigger = Some(configured_key.to_owned());
        state.hotkey.problem = None;
    });
    Ok(())
}

fn find_keyboards(trigger: KeyCode) -> Result<Vec<(PathBuf, Device)>, String> {
    let entries = fs::read_dir("/dev/input")
        .map_err(|error| format!("could not read /dev/input: {error}"))?;

    let mut matched = Vec::new();
    let mut permission_denied = false;

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("event") {
            continue;
        }

        match Device::open(&path) {
            Ok(device) => {
                let is_keyboard = device.supported_events().contains(EventType::KEY)
                    && device
                        .supported_keys()
                        .is_some_and(|keys| keys.contains(trigger) || keys.contains(KeyCode::KEY_ESC));
                if is_keyboard {
                    matched.push((path, device));
                }
            }
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                permission_denied = true;
            }
            Err(_) => {}
        }
    }

    if matched.is_empty() {
        if permission_denied {
            return Err(format!(
                "/dev/input/event* is not readable by this user, so Sotto cannot listen for your hotkey.\n\n{}\n{}",
                keyboard::SETUP,
                keyboard::SETUP_FOOTNOTE
            ));
        }
        return Err("no keyboard device was found under /dev/input".to_owned());
    }

    Ok(matched)
}

fn read_loop(path: PathBuf, mut device: Device, trigger: KeyCode, inbox: Sender<Input>) {
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events,
            Err(error) => {
                eprintln!("hotkey: stopped reading {}: {error}", path.display());
                return;
            }
        };

        for event in events {
            if let EventSummary::Key(_, code, value) = event.destructure() {
                if code == trigger {
                    let msg = match value {
                        1 => Some(Input::Press { at: Instant::now() }),
                        0 => Some(Input::Release { at: Instant::now() }),
                        _ => None, // Ignore key auto-repeat (2).
                    };
                    if let Some(input) = msg {
                        if inbox.send(input).is_err() {
                            return;
                        }
                    }
                } else if code == KeyCode::KEY_ESC && value == 1 {
                    if inbox.send(Input::Cancel).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_trigger_keys() {
        assert_eq!(key_from_name("ControlRight"), Ok(KeyCode::KEY_RIGHTCTRL));
        assert_eq!(key_from_name("F9"), Ok(KeyCode::KEY_F9));
        assert_eq!(key_from_name("AltRight"), Ok(KeyCode::KEY_RIGHTALT));
        assert_eq!(key_from_name("Space"), Ok(KeyCode::KEY_SPACE));
    }

    #[test]
    fn rejects_unknown_trigger_keys() {
        assert!(key_from_name("NotAKey").is_err());
    }
}
