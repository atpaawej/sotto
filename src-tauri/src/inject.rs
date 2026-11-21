//! Putting text into the app that had focus.
//!
//! The clipboard carries the text because keysym typing silently drops every
//! character the keymap cannot produce (ADR-0006). The Ctrl+V that pastes it is
//! three ASCII keysyms through a RemoteDesktop session, which the portal grants
//! with a dialog.
//!
//! The portal session lives in its own task. Injection requests cross a channel,
//! because the state machine runs on a plain thread and the portal speaks async.

use std::{
    io::Write as _,
    process::{Command, Stdio},
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};

use ashpd::desktop::{
    remote_desktop::{DeviceType, KeyState, RemoteDesktop},
    PersistMode, Session,
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    portal::{self, SharedAccess},
    session::InjectPort,
};

const CONTROL_L: i32 = 0xffe3;
const V: i32 = 0x76;

/// How long to wait for the clipboard to be served before pasting anyway.
const CLIPBOARD_WAIT: Duration = Duration::from_millis(500);

/// A handle the session thread can call without knowing about the portal.
pub struct PortalInjector {
    jobs: UnboundedSender<Job>,
}

struct Job {
    text: String,
    reply: SyncSender<Result<(), String>>,
}

impl InjectPort for PortalInjector {
    fn inject(&mut self, text: &str) -> Result<(), String> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.jobs
            .send(Job {
                text: text.to_owned(),
                reply,
            })
            .map_err(|_| "the keyboard bridge is not running".to_owned())?;

        answer
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "the keyboard bridge did not answer in time".to_owned())?
    }
}

/// Where the session thread's injector comes from, plus the retry handle the
/// window uses when the user dismissed the permission dialog.
pub struct Bridge {
    pub injector: PortalInjector,
    pub retry: UnboundedSender<()>,
}

pub fn bridge(access: SharedAccess) -> Bridge {
    let (jobs_tx, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (retry_tx, retry_rx) = tokio::sync::mpsc::unbounded_channel();

    let injector = PortalInjector { jobs: jobs_tx };

    tauri::async_runtime::spawn(serve(access, jobs_rx, retry_rx));

    Bridge {
        injector,
        retry: retry_tx,
    }
}

type Desk = RemoteDesktop<'static>;
type DeskSession = Session<'static, Desk>;

async fn serve(
    access: SharedAccess,
    mut jobs: UnboundedReceiver<Job>,
    mut retries: UnboundedReceiver<()>,
) {
    let mut desk: Option<(Desk, DeskSession)> = None;
    let mut problem: Option<String> = None;

    loop {
        if desk.is_none() {
            match open(access.clone()).await {
                Ok(pair) => {
                    desk = Some(pair);
                    problem = None;
                    portal::update(&access, |state| {
                        state.injection.ready = true;
                        state.injection.problem = None;
                    });
                }
                Err(reason) => {
                    eprintln!("inject: {reason}");
                    problem = Some(reason.clone());
                    portal::update(&access, |state| {
                        state.injection.ready = false;
                        state.injection.problem = Some(reason);
                    });
                }
            }
        }

        tokio::select! {
            Some(()) = retries.recv() => {
                desk = None;
                continue;
            }
            Some(job) = jobs.recv() => {
                let result = match &desk {
                    Some((proxy, session)) => deliver(proxy, session, &job.text).await,
                    None => Err(problem
                        .clone()
                        .unwrap_or_else(|| "keyboard access was not granted".to_owned())),
                };
                let _ = job.reply.send(result);
            }
            else => break,
        }
    }
}

/// Asks for a keyboard session. This is the dialog the user sees once per
/// launch: GNOME returns no restore token, so it cannot be remembered.
async fn open(access: SharedAccess) -> Result<(Desk, DeskSession), String> {
    let proxy = RemoteDesktop::new()
        .await
        .map_err(|error| format!("the remote desktop portal is unavailable: {error}"))?;
    let session = proxy
        .create_session()
        .await
        .map_err(|error| format!("the portal would not open a keyboard session: {error}"))?;

    proxy
        .select_devices(
            &session,
            DeviceType::Keyboard.into(),
            None,
            PersistMode::Application,
        )
        .await
        .map_err(|error| format!("could not select the keyboard: {error}"))?
        .response()
        .map_err(|error| format!("the keyboard selection was refused: {error}"))?;

    proxy
        .start(&session, None)
        .await
        .map_err(|error| format!("could not start the keyboard session: {error}"))?
        .response()
        .map_err(|error| {
            format!("keyboard access was refused ({error}); text cannot be typed until it is granted")
        })?;

    portal::update(&access, |state| {
        state.injection.ready = true;
        state.injection.problem = None;
    });
    Ok((proxy, session))
}

async fn deliver(proxy: &Desk, session: &DeskSession, text: &str) -> Result<(), String> {
    let previous = read_clipboard();
    set_clipboard(text)?;
    wait_for_clipboard();

    press(proxy, session, CONTROL_L, KeyState::Pressed).await?;
    press(proxy, session, V, KeyState::Pressed).await?;
    press(proxy, session, V, KeyState::Released).await?;
    press(proxy, session, CONTROL_L, KeyState::Released).await?;

    restore_clipboard(previous, text);
    Ok(())
}

async fn press(
    proxy: &Desk,
    session: &DeskSession,
    keysym: i32,
    state: KeyState,
) -> Result<(), String> {
    proxy
        .notify_keyboard_keysym(session, keysym, state)
        .await
        .map_err(|error| format!("the portal would not press the paste keys: {error}"))
}

/// Replaces the clipboard. `wl-copy` forks and serves the selection, so the
/// text survives after the command exits.
fn set_clipboard(text: &str) -> Result<(), String> {
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg("text/plain")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "wl-copy is missing; install wl-clipboard, then dictate again".to_owned()
            } else {
                format!("could not start wl-copy: {error}")
            }
        })?;

    child
        .stdin
        .take()
        .ok_or_else(|| "wl-copy did not accept any input".to_owned())?
        .write_all(text.as_bytes())
        .map_err(|error| format!("could not write to the clipboard: {error}"))?;

    let output = child
        .wait_with_output()
        .map_err(|error| format!("wl-copy did not finish: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "wl-copy failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// The clipboard is a service, not a variable: a paste before it is serving
/// reads the old contents.
fn wait_for_clipboard() {
    let started = Instant::now();
    while started.elapsed() < CLIPBOARD_WAIT {
        if Command::new("wl-paste")
            .arg("--list-types")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map(|output| {
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("text/plain")
            })
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn read_clipboard() -> Option<String> {
    let output = Command::new("wl-paste")
        .arg("--no-newline")
        .arg("--type")
        .arg("text/plain")
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Best effort, as ADR-0004 asks: put the user's clipboard back unless they
/// copied something else in the meantime.
fn restore_clipboard(previous: Option<String>, ours: &str) {
    let Some(previous) = previous else {
        return;
    };
    let ours = ours.to_owned();

    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1_200));
        if read_clipboard().as_deref() == Some(ours.as_str()) {
            let _ = set_clipboard(&previous);
        }
    });
}

