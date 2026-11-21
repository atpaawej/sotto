//! Pressing the paste chord through a virtual keyboard.
//!
//! The kernel delivers a uinput device like any other keyboard, so the
//! compositor needs no involvement and no dialog: the price is one-time write
//! access to `/dev/uinput` (ADR-0011). The alternative was a RemoteDesktop
//! session, which GNOME re-prompts for on every launch.

use std::{thread, time::Duration};

use evdev::{
    uinput::VirtualDevice,
    AttributeSet, EventType, InputEvent, KeyCode,
};

/// How long the keys stay down. A press and its release in the same batch is
/// not a keystroke to most toolkits; they need to arrive separately.
const HOLD: Duration = Duration::from_millis(20);

/// Ctrl+V, which every GUI toolkit reads as paste.
pub const PASTE: [KeyCode; 2] = [KeyCode::KEY_LEFTCTRL, KeyCode::KEY_V];

/// The one-time grant, shown in the window while `/dev/uinput` is not writable.
pub const SETUP: &str = "\
sudo usermod -aG input $USER
echo 'KERNEL==\"uinput\", GROUP=\"input\", MODE=\"0660\"' | sudo tee /etc/udev/rules.d/99-sotto-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger";
/// Why the grant above needs a new login.
pub const SETUP_FOOTNOTE: &str = "Then log out and back in: group membership only applies to a new session.";

/// Creates the virtual keyboard. Fails with something the user can act on when
/// the device is missing or not writable.
pub fn open() -> Result<VirtualDevice, String> {
    let mut keys = AttributeSet::new();
    for key in PASTE {
        keys.insert(key);
    }

    VirtualDevice::builder()
        .map_err(|error| describe(error, "create"))?
        .name("Sotto virtual keyboard")
        .with_keys(&keys)
        .map_err(|error| describe(error, "declare"))?
        .build()
        .map_err(|error| describe(error, "register"))
}

fn describe(error: std::io::Error, step: &str) -> String {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => {
            "/dev/uinput is not writable by this user, so Sotto cannot press Ctrl+V".to_owned()
        }
        std::io::ErrorKind::NotFound => {
            "this system has no /dev/uinput, so Sotto cannot press Ctrl+V; try `sudo modprobe uinput`"
                .to_owned()
        }
        _ => format!("could not {step} the virtual keyboard: {error}"),
    }
}

/// Presses the chord, waits for the target app to see it, releases it again.
pub fn tap(device: &mut VirtualDevice, chord: &[KeyCode]) -> Result<(), String> {
    device
        .emit(&events(chord, 1))
        .map_err(|error| format!("the virtual keyboard would not press Ctrl+V: {error}"))?;
    thread::sleep(HOLD);
    device
        .emit(&events(chord, 0))
        .map_err(|error| format!("the virtual keyboard would not release Ctrl+V: {error}"))
}

/// The events for one edge of the chord. Releases undo the presses in the
/// opposite order, so a slow target app never sees the modifier go up first.
pub fn events(chord: &[KeyCode], value: i32) -> Vec<InputEvent> {
    let keys: Vec<KeyCode> = if value == 0 {
        chord.iter().rev().copied().collect()
    } else {
        chord.to_vec()
    };

    keys.into_iter()
        .map(|key| InputEvent::new(EventType::KEY.0, key.code(), value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releases_undo_the_presses_in_reverse() {
        let down: Vec<u16> = events(&PASTE, 1).iter().map(|event| event.code()).collect();
        let up: Vec<u16> = events(&PASTE, 0).iter().map(|event| event.code()).collect();

        let pressed: Vec<u16> = PASTE.iter().map(|key| key.code()).collect();
        let mut released = pressed.clone();
        released.reverse();

        assert_eq!(down, pressed);
        assert_eq!(up, released);
        assert!(events(&PASTE, 1).iter().all(|event| event.value() == 1));
        assert!(events(&PASTE, 0).iter().all(|event| event.value() == 0));
        assert!(events(&PASTE, 1)
            .iter()
            .all(|event| event.event_type() == EventType::KEY));
    }
}
