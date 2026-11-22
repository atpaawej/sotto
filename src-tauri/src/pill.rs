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

use crate::{
    config::PillVisibility,
    session::{Phase, SurfacePort, View},
};

const PILL_WIDTH: i32 = 380;
const PILL_HEIGHT: f64 = 128.0;
/// Distance from the bottom of the screen, leaving room for a dock and
/// vertical expansion when the pill shows an error card.
const BOTTOM_MARGIN: i32 = 156;
const MARGIN: i32 = 64;

/// Creates the persistent pill window at bottom centre, click-through and
/// non-focusable. Must be called on the main thread during app setup.
pub fn create(app: &AppHandle, visibility: PillVisibility) -> Result<(), String> {
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
    .visible(false)
    .build()
    .map_err(|error| format!("the pill window could not be created: {error}"))?;

    #[cfg(target_os = "linux")]
    if let Ok(gtk_win) = window.gtk_window() {
        use gtk::prelude::GtkWindowExt;
        gtk_win.set_accept_focus(false);
        gtk_win.set_focus_on_map(false);
        gtk_win.set_skip_taskbar_hint(true);
        gtk_win.set_skip_pager_hint(true);
        gtk_win.set_type_hint(gtk::gdk::WindowTypeHint::Notification);
    }

    place(&window);
    let _ = window.set_always_on_top(true);
    let _ = window.set_visible_on_all_workspaces(true);
    // Keep cursor events enabled so hover tooltips, click-to-dictate, Cancel,
    // and error dismiss work; `accept_focus(false)` prevents stealing focus.
    let _ = window.set_ignore_cursor_events(false);

    if visibility == PillVisibility::Always {
        window
            .show()
            .map_err(|error| format!("the pill window could not be shown: {error}"))?;
        place(&window);
    }
    Ok(())
}

/// Shows or hides the pill window to match the user's `PillVisibility` setting
/// and the current dictation phase.
pub fn sync_visibility(app: &AppHandle, visibility: PillVisibility, phase: Phase) {
    let Some(window) = app.get_webview_window("pill") else {
        return;
    };
    match visibility {
        PillVisibility::Disabled => {
            let _ = window.hide();
        }
        PillVisibility::Always => {
            let _ = window.show();
            place(&window);
        }
        PillVisibility::ActiveOnly => {
            if phase != Phase::Idle {
                let _ = window.show();
                place(&window);
            }
        }
    }
}

/// Hides the pill window after an `ActiveOnly` fade-out completes.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("pill") {
        let _ = window.hide();
    }
}

/// Restricts pointer input on the pill window to the given logical CSS
/// rectangle so transparent padding around `.pill` passes clicks through.
pub fn set_input_region(app: &AppHandle, x: f64, y: f64, width: f64, height: f64) {
    #[cfg(target_os = "linux")]
    {
        let Some(window) = app.get_webview_window("pill") else {
            return;
        };
        let target = window.clone();
        if let Err(error) = window.run_on_main_thread(move || {
            use gtk::prelude::WidgetExt;
            let Ok(gtk_win) = target.gtk_window() else {
                return;
            };
            let w = width.ceil().max(0.0) as i32;
            let h = height.ceil().max(0.0) as i32;
            if w <= 0 || h <= 0 {
                let empty = gtk::cairo::Region::create();
                gtk_win.input_shape_combine_region(Some(&empty));
                return;
            }
            let rect = gtk::cairo::RectangleInt::new(
                x.floor().max(0.0) as i32,
                y.floor().max(0.0) as i32,
                w,
                h,
            );
            let region = gtk::cairo::Region::create_rectangle(&rect);
            gtk_win.input_shape_combine_region(Some(&region));
        }) {
            crate::devlog::step("pill", format!("could not update input region: {error}"));
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (app, x, y, width, height);
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
    fn show(&mut self, _view: &View, hide_after: Option<Duration>) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;

        if let Some(after) = hide_after {
            let app = self.app.clone();
            let generation_guard = Arc::clone(&self.generation);
            std::thread::spawn(move || {
                std::thread::sleep(after);
                if generation_guard.load(Ordering::SeqCst) != generation {
                    return;
                }
                let idle = View::idle(false);
                let _ = app.emit("dictation://state", &idle);
            });
        }
    }
}

/// Bottom centre of the monitor the pill is on. Only X11/Xwayland honours this;
/// on native Wayland the compositor decides.
fn place(window: &WebviewWindow) {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let origin = monitor.position();
    let screen = monitor.size();
    let scale = monitor.scale_factor();
    let window_width = (PILL_WIDTH as f64 * scale).round() as i32;
    let bottom_margin = (BOTTOM_MARGIN as f64 * scale).round() as i32;
    let margin = (MARGIN as f64 * scale).round() as i32;
    let x = origin.x + (screen.width as i32 - window_width) / 2;
    let y = origin.y + screen.height as i32 - bottom_margin;
    let _ = window.set_position(PhysicalPosition::new(
        x.max(origin.x + margin),
        y.max(origin.y + margin),
    ));
}

#[cfg(test)]
mod tests {
    use crate::session::Phase;

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
