//! The gesture: a global shortcut read directly from `/dev/input/event*`
//! via `evdev`, decoded into presses, releases, and Escape cancels for
//! [`crate::session`].

use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::PathBuf,
    sync::{mpsc::Sender, Arc, Mutex},
    thread,
    time::Instant,
};

use evdev::{Device, EventSummary, EventType, KeyCode};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    config::Config,
    failure::{self, FailureKind},
    keyboard,
    portal::{self, SharedAccess},
    session::{Input, Phase, SharedView},
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
        "Tab" | "KEY_TAB" => Ok(KeyCode::KEY_TAB),
        "Enter" | "KEY_ENTER" => Ok(KeyCode::KEY_ENTER),
        "Backspace" | "KEY_BACKSPACE" => Ok(KeyCode::KEY_BACKSPACE),
        "Backquote" | "KEY_GRAVE" => Ok(KeyCode::KEY_GRAVE),
        "Minus" | "KEY_MINUS" => Ok(KeyCode::KEY_MINUS),
        "Equal" | "KEY_EQUAL" => Ok(KeyCode::KEY_EQUAL),
        "BracketLeft" | "KEY_LEFTBRACE" => Ok(KeyCode::KEY_LEFTBRACE),
        "BracketRight" | "KEY_RIGHTBRACE" => Ok(KeyCode::KEY_RIGHTBRACE),
        "Backslash" | "KEY_BACKSLASH" => Ok(KeyCode::KEY_BACKSLASH),
        "Semicolon" | "KEY_SEMICOLON" => Ok(KeyCode::KEY_SEMICOLON),
        "Quote" | "KEY_APOSTROPHE" => Ok(KeyCode::KEY_APOSTROPHE),
        "Comma" | "KEY_COMMA" => Ok(KeyCode::KEY_COMMA),
        "Period" | "KEY_DOT" => Ok(KeyCode::KEY_DOT),
        "Slash" | "KEY_SLASH" => Ok(KeyCode::KEY_SLASH),
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
        "KeyA" | "A" | "a" => Ok(KeyCode::KEY_A),
        "KeyB" | "B" | "b" => Ok(KeyCode::KEY_B),
        "KeyC" | "C" | "c" => Ok(KeyCode::KEY_C),
        "KeyD" | "D" | "d" => Ok(KeyCode::KEY_D),
        "KeyE" | "E" | "e" => Ok(KeyCode::KEY_E),
        "KeyF" | "F" | "f" => Ok(KeyCode::KEY_F),
        "KeyG" | "G" | "g" => Ok(KeyCode::KEY_G),
        "KeyH" | "H" | "h" => Ok(KeyCode::KEY_H),
        "KeyI" | "I" | "i" => Ok(KeyCode::KEY_I),
        "KeyJ" | "J" | "j" => Ok(KeyCode::KEY_J),
        "KeyK" | "K" | "k" => Ok(KeyCode::KEY_K),
        "KeyL" | "L" | "l" => Ok(KeyCode::KEY_L),
        "KeyM" | "M" | "m" => Ok(KeyCode::KEY_M),
        "KeyN" | "N" | "n" => Ok(KeyCode::KEY_N),
        "KeyO" | "O" | "o" => Ok(KeyCode::KEY_O),
        "KeyP" | "P" | "p" => Ok(KeyCode::KEY_P),
        "KeyQ" | "Q" | "q" => Ok(KeyCode::KEY_Q),
        "KeyR" | "R" | "r" => Ok(KeyCode::KEY_R),
        "KeyS" | "S" | "s" => Ok(KeyCode::KEY_S),
        "KeyT" | "T" | "t" => Ok(KeyCode::KEY_T),
        "KeyU" | "U" | "u" => Ok(KeyCode::KEY_U),
        "KeyV" | "V" | "v" => Ok(KeyCode::KEY_V),
        "KeyW" | "W" | "w" => Ok(KeyCode::KEY_W),
        "KeyX" | "X" | "x" => Ok(KeyCode::KEY_X),
        "KeyY" | "Y" | "y" => Ok(KeyCode::KEY_Y),
        "KeyZ" | "Z" | "z" => Ok(KeyCode::KEY_Z),
        "Digit0" | "0" => Ok(KeyCode::KEY_0),
        "Digit1" | "1" => Ok(KeyCode::KEY_1),
        "Digit2" | "2" => Ok(KeyCode::KEY_2),
        "Digit3" | "3" => Ok(KeyCode::KEY_3),
        "Digit4" | "4" => Ok(KeyCode::KEY_4),
        "Digit5" | "5" => Ok(KeyCode::KEY_5),
        "Digit6" | "6" => Ok(KeyCode::KEY_6),
        "Digit7" | "7" => Ok(KeyCode::KEY_7),
        "Digit8" | "8" => Ok(KeyCode::KEY_8),
        "Digit9" | "9" => Ok(KeyCode::KEY_9),
        other => Err(format!("unsupported trigger key `{other}`")),
    }
}

/// Parses a 1–3 key shortcut spec such as `"ControlRight"` or `"ControlLeft+AltLeft+Space"`.
pub fn keys_from_spec(spec: &str) -> Result<Vec<KeyCode>, String> {
    let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    if parts.is_empty() || parts.iter().any(|part| part.is_empty()) {
        return Err(format!("invalid trigger shortcut `{spec}`"));
    }
    if parts.len() > 3 {
        return Err("a trigger shortcut can use at most 3 keys".to_owned());
    }
    let mut keys = Vec::with_capacity(parts.len());
    for part in parts {
        let code = key_from_name(part)?;
        if keys.contains(&code) {
            return Err(format!("duplicate key `{part}` in trigger shortcut"));
        }
        keys.push(code);
    }
    Ok(keys)
}

#[derive(Default)]
struct ComboTracker {
    pressed: HashSet<KeyCode>,
    active: bool,
}

impl ComboTracker {
    fn handle(&mut self, code: KeyCode, value: i32, trigger: &[KeyCode]) -> Option<bool> {
        match value {
            1 => {
                self.pressed.insert(code);
                if !self.active && !trigger.is_empty() && trigger.iter().all(|k| self.pressed.contains(k)) {
                    self.active = true;
                    return Some(true);
                }
                None
            }
            0 => {
                self.pressed.remove(&code);
                if self.active && trigger.iter().any(|k| !self.pressed.contains(k)) {
                    self.active = false;
                    return Some(false);
                }
                None
            }
            _ => None, // Ignore key auto-repeat (2).
        }
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
    view: SharedView,
    mut retries: UnboundedReceiver<()>,
) {
    let (exited_tx, mut exited_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut bound = false;
    loop {
        if !bound {
            match bind_keyboards(
                &access,
                &inbox,
                Arc::clone(&loaded),
                Arc::clone(&view),
                exited_tx.clone(),
            ) {
                Ok(()) => {
                    bound = true;
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
                }
            }
        }
        // Readers already open keep reading the live trigger, so a retry must
        // not start a second set: one press would become two. A reader that
        // exits clears the flag, so "Ask again" can re-enumerate after a device
        // disappears.
        tokio::select! {
            Some(()) = retries.recv() => {}
            Some(()) = exited_rx.recv() => {
                bound = false;
            }
            else => return,
        }
    }
}

fn bind_keyboards(
    access: &SharedAccess,
    inbox: &Sender<Input>,
    loaded: Arc<Mutex<Result<Config, String>>>,
    view: SharedView,
    exited: UnboundedSender<()>,
) -> Result<(), String> {
    let configured_key = current_key(&loaded)?;
    let devices = find_keyboards()?;

    for (path, device) in devices {
        let tx = inbox.clone();
        let loaded = Arc::clone(&loaded);
        let view = Arc::clone(&view);
        let exited = exited.clone();
        thread::spawn(move || read_loop(path, device, loaded, view, tx, exited));
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
    view: SharedView,
    inbox: Sender<Input>,
    exited: UnboundedSender<()>,
) {
    let mut tracker = ComboTracker::default();
    loop {
        let events = match device.fetch_events() {
            Ok(events) => events,
            Err(error) => {
                eprintln!("hotkey: stopped reading {}: {error}", path.display());
                if exited.send(()).is_err() {
                    eprintln!("hotkey: the serve task stopped before the exit was reported");
                }
                return;
            }
        };

        for event in events {
            if let EventSummary::Key(_, code, value) = event.destructure() {
                if code == KeyCode::KEY_ESC && value == 1 {
                    let active = view
                        .lock()
                        .is_ok_and(|v| matches!(v.phase, Phase::Listening | Phase::Working));
                    if active && inbox.send(Input::Cancel).is_err() {
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
                let trigger = match keys_from_spec(&configured) {
                    Ok(keys) => keys,
                    Err(problem) => {
                        eprintln!("hotkey: {problem}");
                        continue;
                    }
                };
                if let Some(pressed) = tracker.handle(code, value, &trigger) {
                    let input = if pressed {
                        Input::Press { at: Instant::now() }
                    } else {
                        Input::Release { at: Instant::now() }
                    };
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
    fn parses_multi_key_combos_up_to_three_keys() {
        assert_eq!(
            keys_from_spec("ControlLeft+Space"),
            Ok(vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_SPACE])
        );
        assert_eq!(
            keys_from_spec("ControlLeft+AltLeft+KeyD"),
            Ok(vec![
                KeyCode::KEY_LEFTCTRL,
                KeyCode::KEY_LEFTALT,
                KeyCode::KEY_D
            ])
        );
        assert!(keys_from_spec("ControlLeft+AltLeft+ShiftLeft+Space").is_err());
        assert!(keys_from_spec("ControlLeft+ControlLeft").is_err());
        assert!(keys_from_spec("ControlLeft+").is_err());
    }

    #[test]
    fn combo_tracker_fires_press_when_all_held_and_release_when_any_lifted() {
        let mut tracker = ComboTracker::default();
        let trigger = vec![KeyCode::KEY_LEFTCTRL, KeyCode::KEY_SPACE];

        assert_eq!(tracker.handle(KeyCode::KEY_LEFTCTRL, 1, &trigger), None);
        assert_eq!(tracker.handle(KeyCode::KEY_SPACE, 1, &trigger), Some(true));
        // Auto-repeat does not re-trigger.
        assert_eq!(tracker.handle(KeyCode::KEY_SPACE, 2, &trigger), None);
        // Lifting either key releases the combo; lifting the second does not double-release.
        assert_eq!(tracker.handle(KeyCode::KEY_SPACE, 0, &trigger), Some(false));
        assert_eq!(tracker.handle(KeyCode::KEY_LEFTCTRL, 0, &trigger), None);
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
