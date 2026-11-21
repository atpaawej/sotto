//! The small window that says what is happening.
//!
//! Wayland does not let an ordinary window place itself or stay above others,
//! so the whole app runs as an X11 client under Xwayland on GNOME, where Mutter
//! honours both (ADR-0005). The pill is a normal Tauri window with no
//! decorations that is moved to the bottom centre each time it appears.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

use crate::session::{Phase, SurfacePort, View};

const PILL_WIDTH: i32 = 360;
/// Distance from the bottom of the screen, leaving room for a dock.
const BOTTOM_MARGIN: i32 = 132;
const MARGIN: i32 = 64;

/// Creates the pill window hidden. It is shown and hidden by the session, never
/// by the user.
pub fn create(app: &AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "pill",
        tauri::WebviewUrl::App("index.html?window=pill".into()),
    )
    .title("Sotto")
    .inner_size(PILL_WIDTH as f64, 64.0)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .focused(false)
    .visible(false)
    .build()
    .map_err(|error| format!("the pill window could not be created: {error}"))?;

    let _ = window.set_always_on_top(true);
    Ok(())
}

pub struct PillSurface {
    app: AppHandle,
    /// Which hide timer is still the current one. A newer pill cancels the
    /// older timer's right to hide it.
    generation: Arc<AtomicU64>,
}

impl PillSurface {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            generation: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl SurfacePort for PillSurface {
    fn show(&mut self, view: &View, hide_after: Option<Duration>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        let Some(window) = self.app.get_webview_window("pill") else {
            return;
        };

        if !view.phase.visible() {
            let _ = window.hide();
            return;
        }

        if let Err(error) = window.show() {
            eprintln!("pill: could not show the window: {error}");
            return;
        }
        place(&window);

        // Only the working pill has something to click; at every other moment
        // the window must pass clicks through so it cannot steal focus from
        // the app the text is going into. This must come after `show`: tao
        // panics on a window that GTK has not realised yet.
        let _ = window.set_ignore_cursor_events(view.phase != Phase::Working);
        // The pill is a surface, not a control: clicks must reach the app
        // underneath while listening, or dictation would type into the pill.
        let _ = self.app.emit_to("pill", "pill://state", view);

        if let Some(after) = hide_after {
            let app = self.app.clone();
            let generation_guard = Arc::clone(&self.generation);
            std::thread::spawn(move || {
                std::thread::sleep(after);
                if generation_guard.load(Ordering::SeqCst) != generation {
                    return;
                }
                if let Some(window) = app.get_webview_window("pill") {
                    let _ = window.hide();
                }
            });
        }
    }
}

/// Bottom centre of the monitor the pill is on. Only X11 honours this; on
/// Wayland the compositor decides and the pill lands wherever.
fn place(window: &WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let screen = monitor.size();
    let window_width = window
        .outer_size()
        .map(|size| size.width as i32)
        .unwrap_or(PILL_WIDTH);
    let x = (screen.width as i32 - window_width) / 2;
    let y = screen.height as i32 - BOTTOM_MARGIN;
    let _ = window.set_position(PhysicalPosition::new(x.max(MARGIN), y.max(MARGIN)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_is_the_only_phase_that_hides_the_pill() {
        assert!(!Phase::Idle.visible());
        assert!(Phase::Listening.visible());
        assert!(Phase::Working.visible());
        assert!(Phase::Done.visible());
        assert!(Phase::Failed.visible());
        assert!(Phase::Hint.visible(), "advice is worth a pill");
    }
}
