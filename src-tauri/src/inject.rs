//! Putting text into the app that had focus.
//!
//! `arboard` (with `wayland-data-control`) writes the clipboard in-process so
//! any Unicode text survives, and [`crate::keyboard`] presses `Ctrl+V` through
//! `/dev/uinput`.
//!
//! The same clipboard is used by History's Copy button, so a copied transcript
//! is owned by the same long-lived instance that owns pasted text.

use std::{sync::mpsc::SyncSender, thread, time::Duration};

use arboard::Clipboard;
use evdev::uinput::VirtualDevice;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{
    failure, keyboard,
    portal::{self, SharedAccess},
    session::{InjectPort, Input},
};

/// Brief settle time after setting the clipboard before pressing Ctrl+V.
const CLIPBOARD_SETTLE: Duration = Duration::from_millis(50);

/// A handle the session thread can call without knowing about the uinput worker.
pub struct PortalInjector {
    jobs: UnboundedSender<Job>,
}

enum Job {
    Paste {
        text: String,
        reply: SyncSender<Result<(), String>>,
    },
    Copy {
        text: String,
        reply: SyncSender<Result<(), String>>,
    },
}

impl InjectPort for PortalInjector {
    fn inject(&mut self, text: &str) -> Result<(), String> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.jobs
            .send(Job::Paste {
                text: text.to_owned(),
                reply,
            })
            .map_err(|_| "the keyboard bridge is not running".to_owned())?;

        answer
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "the keyboard bridge did not answer in time".to_owned())?
    }
}

/// History's Copy button. It uses the bridge's clipboard rather than a second
/// `arboard::Clipboard`, because on Wayland only the owning process keeps the
/// selection alive.
#[derive(Clone)]
pub struct CopyHandle {
    jobs: UnboundedSender<Job>,
}

impl CopyHandle {
    pub fn copy(&self, text: &str) -> Result<(), String> {
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        self.jobs
            .send(Job::Copy {
                text: text.to_owned(),
                reply,
            })
            .map_err(|_| "the keyboard bridge is not running".to_owned())?;

        answer
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "the keyboard bridge did not answer in time".to_owned())?
    }
}

/// Where the session thread's injector comes from, plus the handles the window
/// uses after the user grants `/dev/uinput` access.
pub struct Bridge {
    pub injector: PortalInjector,
    pub copy: CopyHandle,
    pub retry: UnboundedSender<()>,
}

pub fn bridge(access: SharedAccess, inbox: std::sync::mpsc::Sender<Input>) -> Bridge {
    let (jobs_tx, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (retry_tx, retry_rx) = tokio::sync::mpsc::unbounded_channel();

    let injector = PortalInjector {
        jobs: jobs_tx.clone(),
    };
    let copy = CopyHandle { jobs: jobs_tx };

    tauri::async_runtime::spawn(serve(access, inbox, jobs_rx, retry_rx));

    Bridge {
        injector,
        copy,
        retry: retry_tx,
    }
}

async fn serve(
    access: SharedAccess,
    inbox: std::sync::mpsc::Sender<Input>,
    mut jobs: UnboundedReceiver<Job>,
    mut retries: UnboundedReceiver<()>,
) {
    let mut device: Option<VirtualDevice> = None;
    let mut problem: Option<String> = None;
    // Keep the Clipboard instance alive so Wayland continues serving the selection.
    let mut clipboard: Option<Clipboard> = None;
    let mut reported = false;

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
                    if reported {
                        reported = false;
                        send_capability(
                            &inbox,
                            Input::CapabilityRestored {
                                kind: failure::FailureKind::InjectionBlocked,
                            },
                        );
                    }
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
                        state.injection.problem = Some(full.clone());
                    });
                    if !reported {
                        reported = true;
                        send_capability(
                            &inbox,
                            Input::CapabilityFailed {
                                failure: failure::injection(full),
                            },
                        );
                    }
                }
            }
        }

        tokio::select! {
            Some(()) = retries.recv() => {
                device = None;
                continue;
            }
            Some(job) = jobs.recv() => {
                match job {
                    Job::Paste { text, reply } => {
                        let result = match device.as_mut() {
                            Some(dev) => paste(dev, &mut clipboard, &text),
                            None => Err(problem
                                .clone()
                                .unwrap_or_else(|| "virtual keyboard access was not granted".to_owned())),
                        };
                        if let Err(ref error) = result {
                            eprintln!("inject: delivery failed: {error}");
                        }
                        if reply.send(result).is_err() {
                            eprintln!("inject: the caller stopped waiting for the paste");
                        }
                    }
                    Job::Copy { text, reply } => {
                        let result = copy(&mut clipboard, &text);
                        if let Err(ref error) = result {
                            eprintln!("history: copy failed: {error}");
                        }
                        if reply.send(result).is_err() {
                            eprintln!("history: the caller stopped waiting for the copy");
                        }
                    }
                }
            }
            else => break,
        }
    }
}

fn send_capability(inbox: &std::sync::mpsc::Sender<Input>, input: Input) {
    if let Err(error) = inbox.send(input) {
        eprintln!("inject: the session stopped accepting capability changes: {error}");
    }
}

fn ensure_clipboard(clipboard: &mut Option<Clipboard>) -> Result<&mut Clipboard, String> {
    if clipboard.is_none() {
        *clipboard =
            Some(Clipboard::new().map_err(|error| format!("could not open clipboard: {error}"))?);
    }
    clipboard
        .as_mut()
        .ok_or_else(|| "the clipboard could not be opened".to_owned())
}

fn copy(clipboard: &mut Option<Clipboard>, text: &str) -> Result<(), String> {
    ensure_clipboard(clipboard)?
        .set_text(text.to_owned())
        .map_err(|error| format!("could not write to the clipboard: {error}"))
}

fn paste(
    device: &mut VirtualDevice,
    clipboard: &mut Option<Clipboard>,
    text: &str,
) -> Result<(), String> {
    copy(clipboard, text)?;
    thread::sleep(CLIPBOARD_SETTLE);
    eprintln!("inject: pasting {} bytes via uinput", text.len());
    keyboard::tap(device, &keyboard::PASTE)
}
