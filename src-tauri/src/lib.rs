//! The Sotto desktop shell.
//!
//! Three layers meet here: the window and the pill render state, the session
//! thread owns the one dictation that may exist at a time, and the portal tasks
//! turn gestures and Ctrl+V into something the desktop accepts.

mod audio;
mod config;
mod dictation;
mod hotkey;
mod inject;
mod keyboard;
mod pill;
mod portal;
mod session;
mod transcribe;

use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};

use config::{describe, Config, ConfigStore};
use dictation::{Captured, Dictation};
use portal::SharedAccess;
use serde::Serialize;
use session::{CapturePort, Effect, Input, Ports, Session};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

struct AppState {
    store: ConfigStore,
    /// The loaded config, or the reason it could not be loaded. It is kept in
    /// state rather than returned from setup so the window can still open and
    /// show the problem (ADR-0001).
    loaded: Arc<Mutex<Result<Config, String>>>,
    /// Every press, release and cancel goes through here, so the window's
    /// button and the global shortcut obey the same rules.
    inbox: mpsc::Sender<Input>,
    view: session::SharedView,
    access: SharedAccess,
    /// Asking again for `/dev/input` and `/dev/uinput` access after setup.
    retry_inject: tokio::sync::mpsc::UnboundedSender<()>,
    retry_hotkey: tokio::sync::mpsc::UnboundedSender<()>,
}

#[derive(Serialize)]
struct ConfigSnapshot {
    path: String,
    /// Unix mode such as `600`, so the window can say the file is private.
    mode: String,
    config: Config,
}

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    license: &'static str,
    repository: &'static str,
}

/// The microphone, as the session sees it.
struct Microphone {
    dictation: Dictation,
    config: Config,
}

impl CapturePort for Microphone {
    fn start(&mut self) -> Result<(), String> {
        self.dictation
            .start(&self.config)
            .map_err(|error| describe(&error))
    }

    fn stop(&mut self) -> Result<Captured, String> {
        self.dictation
            .stop_recording()
            .map_err(|error| describe(&error))
    }
}

#[tauri::command]
fn config_snapshot(state: State<'_, AppState>) -> Result<ConfigSnapshot, String> {
    let loaded = state
        .loaded
        .lock()
        .map_err(|_| "the config state was left locked by a panicking thread".to_owned())?;

    let config = loaded.clone()?;

    let metadata = std::fs::metadata(state.store.path())
        .map_err(|error| format!("could not read the config file's permissions: {error}"))?;

    Ok(ConfigSnapshot {
        path: state.store.path().display().to_string(),
        mode: unix_mode(&metadata),
        config,
    })
}

#[tauri::command]
fn dictation_toggle(state: State<'_, AppState>) -> Result<(), String> {
    send(&state, Input::Toggle { at: Instant::now() })
}

#[tauri::command]
fn dictation_cancel(state: State<'_, AppState>) -> Result<(), String> {
    send(&state, Input::Cancel)
}

#[tauri::command]
fn dictation_state(state: State<'_, AppState>) -> session::View {
    state
        .view
        .lock()
        .map(|view| view.clone())
        .unwrap_or_else(|_| session::View {
            phase: session::Phase::Idle,
            locked: false,
            tap_mode: false,
            message: Some("the dictation state was left locked by a panicking thread".to_owned()),
            attempt: None,
        })
}

#[tauri::command]
fn access_status(state: State<'_, AppState>) -> portal::Access {
    portal::snapshot(&state.access)
}

#[tauri::command]
fn retry_keyboard_access(state: State<'_, AppState>) -> Result<(), String> {
    state
        .retry_inject
        .send(())
        .map_err(|_| "the keyboard bridge is not running".to_owned())?;
    state
        .retry_hotkey
        .send(())
        .map_err(|_| "the hotkey listener is not running".to_owned())
}

fn send(state: &State<'_, AppState>, input: Input) -> Result<(), String> {
    state
        .inbox
        .send(input)
        .map_err(|_| "the dictation thread is not running".to_owned())
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        license: "MIT",
        repository: "https://github.com/atpaawej/typeless",
    }
}

#[cfg(unix)]
fn unix_mode(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:o}", metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &std::fs::Metadata) -> String {
    "not tracked on this platform".to_owned()
}

fn show_main(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        eprintln!("the main window is missing, so the tray cannot open it");
        return;
    };

    if let Err(error) = window.unminimize() {
        eprintln!("could not unminimize the main window: {error}");
    }
    if let Err(error) = window.show() {
        eprintln!("could not show the main window: {error}");
    }
    if let Err(error) = window.set_focus() {
        eprintln!("could not focus the main window: {error}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            config_snapshot,
            app_info,
            dictation_toggle,
            dictation_cancel,
            dictation_state,
            access_status,
            retry_keyboard_access,
        ])
        .setup(|app| {
            let path = ConfigStore::default_path()
                .map_err(|error| std::io::Error::other(describe(&error)))?;
            let store = ConfigStore::at(path);

            let loaded = store.load_or_init().map_err(|error| {
                let message = describe(&error);
                eprintln!("config: {message}");
                message
            });

            // A broken config still opens the window, and the session refuses
            // to record with a reason rather than guessing at defaults.
            let config = loaded.clone().unwrap_or_default();

            let access = portal::shared_access();

            pill::create(app.handle()).map_err(std::io::Error::other)?;

            let bridge = inject::bridge(Arc::clone(&access));
            let retry_inject = bridge.retry.clone();
            let view = session::shared_view();
            let (inbox, inbox_rx) = mpsc::channel::<Input>();

            start_session(
                app.handle().clone(),
                inbox_rx,
                inbox.clone(),
                view.clone(),
                config.clone(),
                bridge,
            );

            let (retry_hotkey, retry_hotkey_rx) = tokio::sync::mpsc::unbounded_channel();
            tauri::async_runtime::spawn(hotkey::serve(
                Arc::clone(&access),
                inbox.clone(),
                config.trigger_key.clone(),
                retry_hotkey_rx,
            ));

            app.manage(AppState {
                store,
                loaded: Arc::new(Mutex::new(loaded)),
                inbox,
                view,
                access,
                retry_inject,
                retry_hotkey,
            });

            let open = MenuItem::with_id(app, "open", "Open Sotto", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            let icon = app
                .default_window_icon()
                .cloned()
                .ok_or_else(|| std::io::Error::other("the app has no window icon to reuse for the tray"))?;

            TrayIconBuilder::with_id("sotto")
                .icon(icon)
                .tooltip("Sotto")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main(app),
                    "quit" => app.exit(0),
                    other => eprintln!("unhandled tray menu item: {other}"),
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // The app lives in the tray; closing the window only hides it.
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("could not hide the window on close: {error}");
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start Sotto");
}

/// Owns the state machine on its own thread, because stopping the microphone
/// and waiting for the provider are both blocking.
fn start_session(
    app: AppHandle,
    inbox: mpsc::Receiver<Input>,
    effects: mpsc::Sender<Input>,
    view: session::SharedView,
    config: Config,
    bridge: inject::Bridge,
) {
    let speech = Arc::new(config.clone());
    let session_view = Arc::clone(&view);

    thread::spawn(move || {
        let ports = Ports {
            capture: Box::new(Microphone {
                dictation: Dictation::new(),
                config,
            }),
            injector: Box::new(bridge.injector),
            surface: Box::new(pill::PillSurface::new(app.clone())),
        };
        let mut session = Session::new(ports);
        let _ = app.emit("dictation://state", session.view());

        // A second sender for the transcription threads, so a result can arrive
        // while the loop is doing something else.
        while let Ok(input) = inbox.recv() {
            let work = session.handle(input);
            let current = session.view();
            session::publish(&session_view, current.clone());
            let _ = app.emit("dictation://state", &current);

            for effect in work {
                if let Effect::Transcribe { captured } = effect {
                    let config = Arc::clone(&speech);
                    let reply = effects.clone();
                    thread::spawn(move || {
                        let result = Dictation::transcribe(&config, &captured)
                            .map_err(|error| describe(&error));
                        let _ = reply.send(Input::Transcript { result });
                    });
                }
            }
        }
    });
}
