//! The gesture: a global shortcut read directly from `/dev/input/event*`
//! via `evdev`, decoded into presses, releases, and Escape cancels for
//! [`crate::session`].

use std::{
    fs,
    io::ErrorKind,
    path::PathBuf,
    sync::{mpsc::Sender, Arc, Mutex},
    thread,
    time::Instant,
};

use evdev::{Device, EventSummary, EventType, KeyCode};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::{
    config::Config,
    failure::{self, FailureKind},
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

/// Opens all readable keyboard devices and listens for the configured trigger
/// and Escape. The trigger is read from the live config on every key event, so
/// changing it in Settings takes effect without reopening devices or leaving
/// the old key bound. Re-probes whenever `retries` receives a signal.
pub async fn serve(
    access: SharedAccess,
    inbox: Sender<Input>,
    loaded: Arc<Mutex<Result<Config, String>>>,
    mut retries: UnboundedReceiver<()>,
) {
    loop {
        match bind_keyboards(&access, &inbox, Arc::clone(&loaded)) {
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
                    state.hotkey.problem = Some(problem.clone());
                });
                if inbox
                    .send(Input::CapabilityFailed {
                        failure: failure::shortcut(problem),
                    })
                    .is_err()
                {
                    eprintln!("hotkey: the session stopped accepting failures");
                }
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
    loaded: Arc<Mutex<Result<Config, String>>>,
) -> Result<(), String> {
    let configured_key = current_key(&loaded)?;
    let devices = find_keyboards()?;

    for (path, device) in devices {
        let tx = inbox.clone();
        let loaded = Arc::clone(&loaded);
        thread::spawn(move || read_loop(path, device, loaded, tx));
    }

    eprintln!("hotkey: listening on evdev for {configured_key}");
    portal::update(access, |state| {
        state.hotkey.bound = true;
        state.hotkey.trigger = Some(configured_key);
        state.hotkey.problem = None;
    });
    if inbox
        .send(Input::CapabilityRestored {
            kind: FailureKind::ShortcutRefused,
        })
        .is_err()
    {
        eprintln!("hotkey: the session stopped accepting capability changes");
    }
    Ok(())
}

fn find_keyboards() -> Result<Vec<(PathBuf, Device)>, String> {
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
                // Escape is the one key every reader needs; the trigger can
                // change while these threads are running.
                let is_keyboard = device.supported_events().contains(EventType::KEY)
                    && device
                        .supported_keys()
                        .is_some_and(|keys| keys.contains(KeyCode::KEY_ESC));
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

fn current_key(loaded: &Arc<Mutex<Result<Config, String>>>) -> Result<String, String> {
    loaded
        .lock()
        .map_err(|_| "the config state was left locked by a panicking thread".to_owned())?
        .clone()
        .map(|config| config.trigger_key)
}

fn read_loop(
    path: PathBuf,
    mut device: Device,
    loaded: Arc<Mutex<Result<Config, String>>>,
    inbox: Sender<Input>,
) {
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
                if code == KeyCode::KEY_ESC && value == 1 {
                    if inbox.send(Input::Cancel).is_err() {
                        return;
                    }
                    continue;
                }

                let configured = match current_key(&loaded) {
                    Ok(name) => name,
                    Err(problem) => {
                        eprintln!("hotkey: no trigger while settings are unreadable: {problem}");
                        continue;
                    }
                };
                let is_trigger = match key_from_name(&configured) {
                    Ok(trigger) => trigger == code,
                    Err(problem) => {
                        eprintln!("hotkey: {problem}");
                        false
                    }
                };
                if !is_trigger {
                    continue;
                }

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

    #[test]
    fn changing_the_trigger_takes_effect_without_rebinding() {
        let loaded = Arc::new(Mutex::new(Ok(Config::default())));
        assert_eq!(current_key(&loaded).expect("current key"), "ControlRight");

        *loaded.lock().expect("lock") = Ok(Config {
            trigger_key: "F9".to_owned(),
            ..Config::default()
        });

        let changed = current_key(&loaded).expect("changed key");
        assert_eq!(changed, "F9");
        assert_eq!(key_from_name(&changed), Ok(KeyCode::KEY_F9));
    }
}
