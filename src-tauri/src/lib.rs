//! The Sotto desktop shell.
//!
//! Today the window is a shell: it shows where settings live and whether they
//! loaded. The dictation itself lands in later tickets behind the same
//! `DictationSession` seam.

mod config;

use std::sync::Mutex;

use config::{describe, Config, ConfigStore};
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
    loaded: Mutex<Result<Config, String>>,
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
        .invoke_handler(tauri::generate_handler![config_snapshot, app_info])
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
                loaded: Mutex::new(loaded),
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
