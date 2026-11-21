//! Putting text into the app that had focus.
//!
//! `arboard` (with `wayland-data-control`) writes the clipboard in-process so
//! any Unicode text survives, and [`crate::keyboard`] presses `Ctrl+V` through
//! `/dev/uinput`.

use std::{
    sync::mpsc::SyncSender,
    thread,
    time::Duration,
};

use arboard::Clipboard;
use evdev::uinput::VirtualDevice;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    keyboard,
    portal::{self, SharedAccess},
    session::InjectPort,
};

/// Brief settle time after setting the clipboard before pressing Ctrl+V.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(50);

/// A handle the session thread can call without knowing about the uinput worker.
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
/// window uses after the user grants `/dev/uinput` access.
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

async fn serve(
    access: SharedAccess,
    mut jobs: UnboundedReceiver<Job>,
    mut retries: UnboundedReceiver<()>,
) {
    let mut device: Option<VirtualDevice> = None;
    let mut problem: Option<String> = None;
    // Keep the Clipboard instance alive so Wayland continues serving the selection.
    let mut clipboard: Option<Clipboard> = None;

    loop {
        if device.is_none() {
            match keyboard::open() {
                Ok(dev) => {
                    device = Some(dev);
                    problem = None;
                    portal::update(&access, |state| {
                        state.injection.ready = true;
                        state.injection.problem = None;
                    });
                }
                Err(reason) => {
                    let full = format!(
                        "{reason}\n\n{}\n{}",
                        keyboard::SETUP,
                        keyboard::SETUP_FOOTNOTE
                    );
                    eprintln!("inject: {reason}");
                    problem = Some(full.clone());
                    portal::update(&access, |state| {
                        state.injection.ready = false;
                        state.injection.problem = Some(full);
                    });
                }
            }
        }

        tokio::select! {
            Some(()) = retries.recv() => {
                device = None;
                continue;
            }
            Some(job) = jobs.recv() => {
                let result = match device.as_mut() {
                    Some(dev) => deliver(dev, &mut clipboard, &job.text),
                    None => Err(problem
                        .clone()
                        .unwrap_or_else(|| "virtual keyboard access was not granted".to_owned())),
                };
                if let Err(ref error) = result {
                    eprintln!("inject: delivery failed: {error}");
                }
                let _ = job.reply.send(result);
            }
            else => break,
        }
    }
}

fn deliver(
    device: &mut VirtualDevice,
    clipboard: &mut Option<Clipboard>,
    text: &str,
) -> Result<(), String> {
    if clipboard.is_none() {
        *clipboard = Some(
            Clipboard::new().map_err(|error| format!("could not open clipboard: {error}"))?,
        );
    }

    if let Some(cb) = clipboard.as_mut() {
        cb.set_text(text.to_owned())
            .map_err(|error| format!("could not write to the clipboard: {error}"))?;
    }

    thread::sleep(CLIPBOARD_SETTLE);
    eprintln!("inject: pasting {} bytes via uinput", text.len());
    keyboard::tap(device, &keyboard::PASTE)
}
