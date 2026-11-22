//! The Sotto desktop shell.
//!
//! Three layers meet here: the window and the pill render state, the session
//! thread owns the one dictation that may exist at a time, and the portal tasks
//! turn gestures and Ctrl+V into something the desktop accepts.

mod audio;
mod cleanup;
mod config;
mod devlog;
mod dictation;
mod failure;
mod history;
mod hotkey;
mod inject;
mod keyboard;
mod pill;
mod portal;
mod session;
mod startup;
mod transcribe;
mod update;

use std::{
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};

use config::{
    describe, normalize_dictionary, Cleanup, Config, ConfigStore, PillVisibility, ProviderId,
};
use dictation::{Captured, Dictation};
use failure::{Failure, FailureKind};
use history::{Entry, History};
use portal::SharedAccess;
use serde::Serialize;
use session::{CapturePort, Effect, Input, Ports, Session, View};
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
    /// Every Dictation and its Recording. The same instance the session
    /// writes to, so History and Retry see one truth.
    history: Arc<History>,
    /// History's Copy button, which shares the injector's clipboard.
    copy: inject::CopyHandle,
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
    loaded: Arc<Mutex<Result<Config, String>>>,
    app: AppHandle,
}

impl CapturePort for Microphone {
    fn start(&mut self) -> Result<(), Failure> {
        let config = current_config(&self.loaded).map_err(failure::settings)?;
        let app = self.app.clone();
        let on_level = Arc::new(move |level: f32| {
            let _ = app.emit("dictation://level", level);
        });
        self.dictation
            .start_with_levels(&config, Some(on_level))
            .map_err(failure::from_dictation)
    }

    fn stop(&mut self) -> Result<Captured, Failure> {
        self.dictation
            .stop_recording()
            .map_err(failure::from_dictation)
    }
}

fn current_config(loaded: &Arc<Mutex<Result<Config, String>>>) -> Result<Config, String> {
    let guard = loaded
        .lock()
        .map_err(|_| "the config state was left locked by a panicking thread".to_owned())?;
    guard.clone()
}

fn snapshot_for(store: &ConfigStore, config: Config) -> Result<ConfigSnapshot, String> {
    let metadata = std::fs::metadata(store.path())
        .map_err(|error| format!("could not read the config file's permissions: {error}"))?;

    Ok(ConfigSnapshot {
        path: store.path().display().to_string(),
        mode: unix_mode(&metadata),
        config,
    })
}

fn mutate_config(
    state: &State<'_, AppState>,
    update: impl FnOnce(&mut Config),
) -> Result<ConfigSnapshot, String> {
    let mut loaded = state
        .loaded
        .lock()
        .map_err(|_| "the config state was left locked by a panicking thread".to_owned())?;
    let mut config = loaded.clone()?;
    update(&mut config);
    state
        .store
        .save(&config)
        .map_err(|error| describe(&error))?;
    *loaded = Ok(config.clone());
    snapshot_for(&state.store, config)
}

#[tauri::command]
fn config_snapshot(state: State<'_, AppState>) -> Result<ConfigSnapshot, String> {
    let config = current_config(&state.loaded)?;
    snapshot_for(&state.store, config)
}

#[tauri::command]
fn update_cleanup(state: State<'_, AppState>, cleanup: Cleanup) -> Result<ConfigSnapshot, String> {
    mutate_config(&state, |config| {
        config.cleanup = cleanup;
    })
}

#[tauri::command]
fn update_pill_visibility(
    state: State<'_, AppState>,
    app: AppHandle,
    visibility: PillVisibility,
) -> Result<ConfigSnapshot, String> {
    let snapshot = mutate_config(&state, |config| {
        config.pill_visibility = visibility;
    })?;
    let phase = state
        .view
        .lock()
        .map(|view| view.phase)
        .unwrap_or(session::Phase::Idle);
    pill::sync_visibility(&app, visibility, phase);
    if visibility == PillVisibility::ActiveOnly && phase == session::Phase::Idle {
        pill::hide(&app);
    }
    if let Err(error) = app.emit("pill://visibility", visibility) {
        devlog::step("pill", format!("could not emit pill visibility: {error}"));
    }
    Ok(snapshot)
}

#[tauri::command]
fn pill_visibility(state: State<'_, AppState>) -> PillVisibility {
    current_config(&state.loaded)
        .map(|config| config.pill_visibility)
        .unwrap_or_default()
}

#[tauri::command]
fn hide_pill_window(state: State<'_, AppState>, app: AppHandle) {
    let visibility = current_config(&state.loaded)
        .map(|config| config.pill_visibility)
        .unwrap_or_default();
    let phase = state
        .view
        .lock()
        .map(|view| view.phase)
        .unwrap_or(session::Phase::Idle);
    if visibility == PillVisibility::Disabled
        || (visibility == PillVisibility::ActiveOnly && phase == session::Phase::Idle)
    {
        pill::hide(&app);
    }
}

#[tauri::command]
fn set_pill_input_region(app: AppHandle, x: f64, y: f64, width: f64, height: f64) {
    pill::set_input_region(&app, x, y, width, height);
}

#[tauri::command]
fn update_dictionary(
    state: State<'_, AppState>,
    dictionary: Vec<String>,
) -> Result<ConfigSnapshot, String> {
    mutate_config(&state, |config| {
        config.dictionary = normalize_dictionary(dictionary);
    })
}

#[tauri::command]
fn update_provider(state: State<'_, AppState>, provider: ProviderId) -> Result<ConfigSnapshot, String> {
    mutate_config(&state, |config| {
        config.active_provider = provider;
    })
}

#[tauri::command]
fn update_provider_key(
    state: State<'_, AppState>,
    provider: ProviderId,
    key: String,
) -> Result<ConfigSnapshot, String> {
    let trimmed = key.trim().to_owned();
    let stored = if trimmed.is_empty() { None } else { Some(trimmed) };
    mutate_config(&state, |config| {
        config.providers.set_key(provider, stored);
    })
}

/// A real request with the key under test. The failure keeps the one
/// vocabulary, so the wizard and Speech say what the Pill would say.
#[tauri::command]
fn test_provider(state: State<'_, AppState>, provider: ProviderId) -> Result<(), Failure> {
    let config = current_config(&state.loaded).map_err(failure::settings)?;
    let key = config
        .providers
        .key(provider)
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| {
            Failure::new(
                FailureKind::KeyRejected,
                format!("No key is stored for {}.", provider.as_str()),
            )
        })?
        .clone();

    let result = transcribe::key_test(transcribe::transcriber_for(provider).as_ref(), &key);
    match result {
        Ok(_) => Ok(()),
        Err(error) => {
            let failure = failure::from_transcribe(error);
            failure.log();
            Err(failure)
        }
    }
}

/// Saves the shortcut and makes the listener use it at once, without leaving
/// the previous key bound.
#[tauri::command]
fn update_trigger_key(
    state: State<'_, AppState>,
    key: String,
) -> Result<ConfigSnapshot, String> {
    let key = key.trim().to_owned();
    hotkey::keys_from_spec(&key)?;
    let snapshot = mutate_config(&state, |config| {
        config.trigger_key = key.clone();
    })?;
    portal::update(&state.access, |access| {
        access.hotkey.trigger = Some(key);
    });
    Ok(snapshot)
}

#[tauri::command]
fn dictation_toggle(state: State<'_, AppState>) -> Result<(), String> {
    send(&state, Input::Toggle { at: Instant::now() })
}

#[tauri::command]
fn dictation_cancel(state: State<'_, AppState>) -> Result<(), String> {
    send(&state, Input::Cancel)
}

/// Retries the last failed Dictation from its stored Recording. The user does
/// not speak again.
#[tauri::command]
fn dictation_retry(state: State<'_, AppState>) -> Result<(), String> {
    send(&state, Input::Retry { id: None })
}

#[tauri::command]
fn history_list(state: State<'_, AppState>, query: String) -> Result<Vec<Entry>, String> {
    state.history.list(&query).map_err(fail)
}

#[tauri::command]
fn history_insights(
    state: State<'_, AppState>,
    range: history::InsightsRange,
    tz_offset_minutes: i32,
) -> Result<history::InsightsSnapshot, String> {
    state
        .history
        .insights(range, tz_offset_minutes)
        .map_err(fail)
}

#[tauri::command]
fn history_retry(state: State<'_, AppState>, id: String) -> Result<(), String> {
    send(&state, Input::Retry { id: Some(id) })
}

/// Re-transcribes a stored Recording with the other provider, keeping the
/// first result in History beside the new one.
#[tauri::command]
fn history_retry_other(state: State<'_, AppState>, id: String) -> Result<(), String> {
    send(&state, Input::RetryOther { id: Some(id) })
}

#[tauri::command]
fn history_copy(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let entry = state.history.entry(&id).map_err(fail)?;
    let text = if entry.cleaned.trim().is_empty() {
        entry.transcript
    } else {
        entry.cleaned
    };
    if text.trim().is_empty() {
        return Err("that dictation has no text to copy".to_owned());
    }
    state.copy.copy(&text).map_err(|reason| {
        let failure = failure::injection(reason);
        fail(failure)
    })
}

/// The WAV for one entry, so History can play it without exposing the disk.
#[tauri::command]
fn history_recording(
    state: State<'_, AppState>,
    id: String,
) -> Result<tauri::ipc::Response, String> {
    let wav = state.history.recording(&id).map_err(fail)?;
    Ok(tauri::ipc::Response::new(wav))
}

#[tauri::command]
fn history_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.history.delete(&id).map_err(fail)
}

#[tauri::command]
fn history_delete_all(state: State<'_, AppState>) -> Result<(), String> {
    state.history.delete_all().map_err(fail)
}

/// Saves the retention limit and applies it at once, so choosing a limit is
/// not a promise about some future launch.
#[tauri::command]
fn update_retention(
    state: State<'_, AppState>,
    days: Option<u32>,
) -> Result<ConfigSnapshot, String> {
    let snapshot = mutate_config(&state, |config| {
        config.retention_days = days;
    })?;
    state.history.enforce_retention(days).map_err(fail)?;
    Ok(snapshot)
}

fn fail(failure: Failure) -> String {
    failure.log();
    failure.detail
}

#[tauri::command]
fn dictation_state(state: State<'_, AppState>) -> session::View {
    state
        .view
        .lock()
        .map(|view| view.clone())
        .unwrap_or_else(|_| View {
            phase: session::Phase::Idle,
            locked: false,
            tap_mode: false,
            message: Some("the dictation state was left locked by a panicking thread".to_owned()),
            attempt: None,
            failure: None,
            retry_id: None,
            injected: false,
        })
}

#[tauri::command]
fn access_status(state: State<'_, AppState>) -> portal::Access {
    portal::snapshot(&state.access)
}

#[tauri::command]
fn start_at_login_enabled() -> bool {
    startup::is_start_at_login_enabled()
}

#[tauri::command]
fn set_start_at_login(enabled: bool) -> Result<(), String> {
    startup::set_start_at_login(enabled)
}

/// Asks GitHub for the newest release and reports whether it is newer than
/// the running version.
#[tauri::command]
async fn check_for_updates() -> Result<update::UpdateStatus, String> {
    tauri::async_runtime::spawn_blocking(|| update::check(env!("CARGO_PKG_VERSION")))
        .await
        .map_err(|error| format!("update check task failed: {error}"))?
}

/// Downloads the newest `.deb`, verifies its checksum and installs it with
/// pkexec (or sudo fallback). The running app keeps going; the user restarts it
/// to use the new version.
#[tauri::command]
async fn install_update() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(update::install_latest)
        .await
        .map_err(|error| format!("update install task failed: {error}"))?
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
fn open_main_window(app: AppHandle) {
    show_main(&app);
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

/// Surfaces a failure via desktop notification when the Pill is disabled (ADR-0005).
fn notify_failure(reason: &str) {
    // best-effort: desktop notifications may be unavailable in headless or minimal sessions.
    if let Err(error) = std::process::Command::new("notify-send")
        .args(["Sotto — Dictation failed", reason])
        .spawn()
    {
        devlog::step("notify", format!("could not send desktop notification: {error}"));
    }
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
            update_cleanup,
            update_pill_visibility,
            pill_visibility,
            hide_pill_window,
            set_pill_input_region,
            update_dictionary,
            update_provider,
            update_provider_key,
            update_trigger_key,
            test_provider,
            app_info,
            dictation_toggle,
            dictation_cancel,
            dictation_retry,
            dictation_state,
            history_list,
            history_insights,
            history_retry,
            history_retry_other,
            history_copy,
            history_recording,
            history_delete,
            history_delete_all,
            update_retention,
            access_status,
            retry_keyboard_access,
            start_at_login_enabled,
            set_start_at_login,
            check_for_updates,
            install_update,
            open_main_window,
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

            pill::create(app.handle(), config.pill_visibility).map_err(std::io::Error::other)?;

            let loaded_state = Arc::new(Mutex::new(loaded));
            let history = Arc::new(History::open_default());
            if let Err(failure) = history.enforce_retention(config.retention_days) {
                // Retention runs before there is a window to show it in. The
                // cause chain goes to the log; History still lists, so the
                // user can delete by hand.
                failure.log();
            }

            let (inbox, inbox_rx) = mpsc::channel::<Input>();
            let bridge = inject::bridge(
                Arc::clone(&access),
                inbox.clone(),
                config.text_injectors.clone(),
            );
            let retry_inject = bridge.retry.clone();
            let copy = bridge.copy.clone();
            let view = session::shared_view();

            start_session(
                app.handle().clone(),
                inbox_rx,
                inbox.clone(),
                view.clone(),
                Arc::clone(&loaded_state),
                Arc::clone(&history),
                bridge,
            );

            let (retry_hotkey, retry_hotkey_rx) = tokio::sync::mpsc::unbounded_channel();
            tauri::async_runtime::spawn(hotkey::serve(
                Arc::clone(&access),
                inbox.clone(),
                Arc::clone(&loaded_state),
                Arc::clone(&view),
                retry_hotkey_rx,
            ));

            app.manage(AppState {
                store,
                loaded: loaded_state,
                inbox,
                view,
                access,
                history,
                copy,
                retry_inject,
                retry_hotkey,
            });

            let open = MenuItem::with_id(app, "open", "Open Sotto", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            let app_icon = tauri::include_image!("icons/icon.png");
            if let Some(main_window) = app.get_webview_window("main") {
                if let Err(error) = main_window.set_icon(app_icon) {
                    devlog::step("window", format!("could not set main window icon: {error}"));
                }
            }

            let tray_icon = tauri::include_image!("icons/tray.png");

            TrayIconBuilder::with_id("sotto")
                .icon(tray_icon)
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
    loaded: Arc<Mutex<Result<Config, String>>>,
    history: Arc<History>,
    bridge: inject::Bridge,
) {
    let session_view = Arc::clone(&view);
    let shared_cleaner = Arc::new(match cleanup::GroqCleaner::new() {
        Ok(cleaner) => Some(cleaner),
        Err(error) => {
            eprintln!("cleanup client init failed: {}", describe(&error));
            None
        }
    });

    thread::spawn(move || {
        let ports = Ports {
            capture: Box::new(Microphone {
                dictation: Dictation::new(),
                loaded: Arc::clone(&loaded),
                app: app.clone(),
            }),
            injector: Box::new(bridge.injector),
            surface: Box::new(pill::PillSurface::new(app.clone())),
            history: Box::new(history),
        };
        let mut session = Session::new(ports);
        let _ = app.emit("dictation://state", session.view());

        // A second sender for the transcription threads, so a result can arrive
        // while the loop is doing something else.
        while let Ok(input) = inbox.recv() {
            let work = session.handle(input);
            let current = session.view();
            session::publish(&session_view, current.clone());
            if let Ok(config) = current_config(&loaded) {
                pill::sync_visibility(&app, config.pill_visibility, current.phase);
                if config.pill_visibility == PillVisibility::Disabled
                    && current.phase == session::Phase::Failed
                {
                    if let Some(failure) = &current.failure {
                        notify_failure(failure.reason());
                    }
                }
            }
            let _ = app.emit("dictation://state", &current);

            for effect in work {
                match effect {
                    Effect::Transcribe {
                        id,
                        captured,
                        provider,
                    } => {
                        let loaded = Arc::clone(&loaded);
                        let cleaner = Arc::clone(&shared_cleaner);
                        let reply = effects.clone();
                        thread::spawn(move || {
                            let (result, used_provider) = match current_config(&loaded) {
                                Ok(config) => {
                                    let used = provider.unwrap_or(config.active_provider);
                                    (
                                        Dictation::transcribe_with(
                                            &config,
                                            &captured,
                                            used,
                                            cleaner
                                                .as_ref()
                                                .as_ref()
                                                .map(|c| c as &dyn cleanup::Cleaner),
                                        )
                                        .map_err(failure::from_dictation),
                                        Some(used),
                                    )
                                }
                                Err(problem) => (Err(failure::settings(problem)), None),
                            };
                            if let Err(error) = reply.send(Input::Transcript {
                                id,
                                result: Box::new(result),
                                provider: used_provider,
                            }) {
                                eprintln!(
                                    "session: the dictation thread stopped before the reply: {error}"
                                );
                            }
                        });
                    }
                    // The failure has already been logged and shown by the
                    // state machine; there is no work to hand off.
                    Effect::Failed => {}
                }
            }
        }
    });
}
