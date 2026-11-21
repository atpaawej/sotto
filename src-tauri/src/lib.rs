//! The Sotto desktop shell.
//!
//! The window reads settings, and the Speech section runs one dictation at a
//! time through [`dictation::Dictation`]. Hotkey and pill arrive in later
//! tickets; today a button is the only way to start one.

mod audio;
mod config;
mod dictation;
mod transcribe;

use std::sync::{Arc, Mutex};

use config::{describe, Config, ConfigStore};
use dictation::{Attempt, Dictation};
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, State, WindowEvent,
};

struct AppState {
    store: ConfigStore,
    /// The loaded config, or the reason it could not be loaded. It is kept in
    /// state rather than returned from setup so the window can still open and
    /// show the problem (ADR-0001).
    loaded: Arc<Mutex<Result<Config, String>>>,
    dictation: Arc<Dictation>,
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
async fn dictation_start(state: State<'_, AppState>) -> Result<(), String> {
    let dictation = Arc::clone(&state.dictation);
    let config = loaded_config(&state)?;

    // Opening the microphone and waiting for it to run blocks, so it never
    // happens on the thread that paints the window.
    tauri::async_runtime::spawn_blocking(move || dictation.start(&config))
        .await
        .map_err(|error| format!("the microphone task failed: {error}"))?
        .map_err(|error| describe(&error))
}

#[tauri::command]
async fn dictation_stop(state: State<'_, AppState>) -> Result<Attempt, String> {
    let dictation = Arc::clone(&state.dictation);
    let config = loaded_config(&state)?;

    tauri::async_runtime::spawn_blocking(move || dictation.stop(&config))
        .await
        .map_err(|error| format!("the dictation task failed: {error}"))?
        .map_err(|error| describe(&error))
}

#[tauri::command]
fn dictation_recording(state: State<'_, AppState>) -> bool {
    state.dictation.is_recording()
}

/// The config, or the reason it is unavailable, as a message fit for the window.
fn loaded_config(state: &State<'_, AppState>) -> Result<Config, String> {
    state
        .loaded
        .lock()
        .map_err(|_| "the config state was left locked by a panicking thread".to_owned())?
        .clone()
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
            dictation_start,
            dictation_stop,
            dictation_recording,
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

            app.manage(AppState {
                store,
                loaded: Arc::new(Mutex::new(loaded)),
                dictation: Arc::new(Dictation::new()),
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
