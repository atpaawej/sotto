//! The gesture: a global shortcut bound through the portal, decoded into
//! presses and releases for [`crate::session`].
//!
//! On GNOME the portal shows a dialog and the user picks the keys; whatever we
//! ask for is a suggestion. The window shows what was actually bound, because
//! telling someone to hold a key that does nothing is worse than saying nothing.

use std::{sync::mpsc::Sender, time::Instant};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;


use crate::{
    portal::{self, SharedAccess},
    session::Input,
};


pub const SHORTCUT_ID: &str = "dictate";

/// What we ask the portal for. GNOME asks the user to press keys instead, so
/// this only decides what the dialog starts with.
const FALLBACK_TRIGGER: &str = "CTRL+ALT+SPACE";

/// The configured key is a GTK key name (`ControlRight`); the portal wants a
/// trigger (`CTRL+ALT+SPACE`). Anything that does not look like one falls back,
/// because a rejected preference can make the whole bind fail.
pub fn preferred_trigger(configured: &str) -> String {
    if configured.contains('+') {
        configured.to_uppercase()
    } else {
        FALLBACK_TRIGGER.to_owned()
    }
}


/// Binds the shortcut and forwards presses until the session ends.
///
/// Never returns normally; a failure is recorded in `access` so the window can
/// show it, and the task ends.
pub async fn serve(access: SharedAccess, inbox: Sender<Input>, preferred: String) {
    if let Err(problem) = listen(access.clone(), inbox, preferred).await {
        eprintln!("hotkey: {problem}");
        portal::update(&access, |state| {
            state.hotkey.bound = false;
            state.hotkey.problem = Some(problem);
        });
    }
}

async fn listen(
    access: SharedAccess,
    inbox: Sender<Input>,
    preferred: String,
) -> Result<(), String> {
    let shortcuts = GlobalShortcuts::new()
        .await
        .map_err(|error| format!("the global shortcuts portal is unavailable: {error}"))?;

    let session = shortcuts
        .create_session()
        .await
        .map_err(|error| format!("the portal would not open a shortcut session: {error}"))?;

    // Subscribe before binding: the first press can arrive the moment the
    // dialog closes.
    let mut activated = shortcuts
        .receive_activated()
        .await
        .map_err(|error| format!("could not listen for the shortcut: {error}"))?;
    let mut deactivated = shortcuts
        .receive_deactivated()
        .await
        .map_err(|error| format!("could not listen for shortcut releases: {error}"))?;

    let shortcut = NewShortcut::new(SHORTCUT_ID, "Hold to dictate")
        .preferred_trigger(Some(preferred.as_str()));

    let request = shortcuts
        .bind_shortcuts(&session, &[shortcut], None)
        .await
        .map_err(|error| format!("the shortcut could not be bound: {error}"))?;

    let bound = request
        .response()
        .map_err(|error| format!("the shortcut dialog was dismissed: {error}"))?;

    let trigger = bound
        .shortcuts()
        .iter()
        .find(|shortcut| shortcut.id() == SHORTCUT_ID)
        .map(|shortcut| shortcut.trigger_description().to_owned())
        .filter(|description| !description.is_empty());

    eprintln!(
        "hotkey: bound {} as {}",
        SHORTCUT_ID,
        trigger.as_deref().unwrap_or("(the system did not say)")
    );

    portal::update(&access, |state| {
        state.hotkey.bound = true;
        state.hotkey.trigger = trigger;
        state.hotkey.problem = None;
    });

    let reason = loop {
        tokio::select! {
            next = activated.next() => {
                if next.is_none() {
                    break "the shortcut session ended; reopen Sotto to bind it again".to_owned();
                }
                if inbox.send(Input::Press { at: Instant::now() }).is_err() {
                    return Ok(());
                }
            }
            next = deactivated.next() => {
                if next.is_none() {
                    break "the shortcut session ended; reopen Sotto to bind it again".to_owned();
                }
                if inbox.send(Input::Release { at: Instant::now() }).is_err() {
                    return Ok(());
                }
            }
        }
    };

    Err(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gtk_key_name_falls_back_to_a_portal_trigger() {
        assert_eq!(preferred_trigger("ControlRight"), "CTRL+ALT+SPACE");
    }

    #[test]
    fn a_configured_trigger_is_kept_and_uppercased() {
        assert_eq!(preferred_trigger("ctrl+alt+d"), "CTRL+ALT+D");
    }
}
