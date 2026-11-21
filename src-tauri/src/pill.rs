//! The small window that says what is happening.
//!
//! Wayland does not let an ordinary window place itself or stay above others,
//! so the app runs as an X11 client under Xwayland on GNOME, where Mutter
//! honours both (ADR-0005). The pill stays persistently mapped at the bottom
//! centre and never takes focus, so `Ctrl+V` always lands in the target app.

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
const PILL_HEIGHT: f64 = 64.0;
/// Distance from the bottom of the screen, leaving room for a dock.
const BOTTOM_MARGIN: i32 = 132;
const MARGIN: i32 = 64;

/// Creates the persistent pill window at bottom centre, click-through and
/// non-focusable. Must be called on the main thread during app setup.
pub fn create(app: &AppHandle) -> Result<(), String> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        "pill",
        tauri::WebviewUrl::App("index.html?window=pill".into()),
    )
    .title("Sotto")
    .inner_size(PILL_WIDTH as f64, PILL_HEIGHT)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .focused(false)
    .focusable(false)
    .visible(true)
    .build()
    .map_err(|error| format!("the pill window could not be created: {error}"))?;

    place(&window);
    let _ = window.set_always_on_top(true);
    let _ = window.set_visible_on_all_workspaces(true);
    let _ = window.set_ignore_cursor_events(true);
    Ok(())
}

pub struct PillSurface {
    app: AppHandle,
    /// Which reset timer is still the current one. A newer state cancels the
    /// older timer's right to return the pill to idle.
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
        let interactive = view.phase == Phase::Working;
        let app = self.app.clone();

        // Mutating GTK window properties must happen on the main thread.
        let _ = self.app.run_on_main_thread(move || {
            if let Some(window) = app.get_webview_window("pill") {
                let _ = window.set_ignore_cursor_events(!interactive);
            }
        });

        if let Some(after) = hide_after {
            let app = self.app.clone();
            let generation_guard = Arc::clone(&self.generation);
            std::thread::spawn(move || {
                std::thread::sleep(after);
                if generation_guard.load(Ordering::SeqCst) != generation {
                    return;
                }
                let idle = View {
                    phase: Phase::Idle,
                    locked: false,
                    tap_mode: false,
                    message: None,
                    attempt: None,
                };
                let _ = app.emit("dictation://state", &idle);
                let main_app = app.clone();
                let _ = app.run_on_main_thread(move || {
                    if let Some(window) = main_app.get_webview_window("pill") {
                        let _ = window.set_ignore_cursor_events(true);
                    }
                });
            });
        }
    }
}

/// Bottom centre of the monitor the pill is on. Only X11/Xwayland honours this;
/// on native Wayland the compositor decides.
fn place(window: &WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let origin = monitor.position();
    let screen = monitor.size();
    let window_width = window
        .outer_size()
        .map(|size| size.width as i32)
        .unwrap_or(PILL_WIDTH);
    let x = origin.x + (screen.width as i32 - window_width) / 2;
    let y = origin.y + screen.height as i32 - BOTTOM_MARGIN;
    let _ = window.set_position(PhysicalPosition::new(
        x.max(origin.x + MARGIN),
        y.max(origin.y + MARGIN),
    ));
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
