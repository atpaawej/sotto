//! Putting text into the app that had focus.
//!
//! The [`TextInjector`] seam is what the session delivers text through. The
//! clipboard adapter is the primary: write to the clipboard and tap Ctrl+V,
//! which every app accepts, and restore the user's clipboard after the paste
//! (ADR-0004, amended by ADR-0006). The ydotool adapter is the optional
//! fallback, documented as never required. The order they are tried in is
//! Config.text_injectors, so ordering is data, not code.

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

/// The clipboard adapter's name, as the bridge reports it and Settings shows it.
pub const CLIPBOARD: &str = "clipboard";

/// The ydotool adapter's name, as the bridge reports it and Settings shows it.
pub const YDOTOOL: &str = "ydotool";

/// Why the clipboard adapter is the active one.
pub const CLIPBOARD_REASON: &str =
    "Clipboard write plus Ctrl+V — works in every app, even ones that ignore typed input.";

/// Why the ydotool adapter is the active one.
pub const YDOTOOL_REASON: &str =
    "The ydotool daemon types straight into the focused window.";

/// The TextInjector seam ADR-0004 puts at the session: the adapters that put
/// text into the focused app, and why Settings shows the active one. The
/// session itself reaches the bridge through the job it already has
/// (`InjectPort`), so this is the wider seam the bridge orders by.
pub trait TextInjector {
    /// The name Settings shows for the adapter.
    fn name(&self) -> &'static str;
    /// The sentence Settings shows for why this adapter is the one that injects.
    fn reason(&self) -> String;
    /// False when it cannot run on this machine right now.
    fn available(&self) -> bool;
    /// Puts the text into the focused app.
    fn inject(&self, text: &str) -> Result<(), String>;
}

/// The optional fallback: ydotool types straight into the focused window. It
/// needs the daemon and it mangles non-ASCII text, so it is documented as
/// never required rather than required for the few apps where clipboard paste
/// is ignored.
pub struct YdotoolAdapter;

impl TextInjector for YdotoolAdapter {
    fn name(&self) -> &'static str {
        YDOTOOL
    }

    fn reason(&self) -> String {
        YDOTOOL_REASON.to_owned()
    }

    fn available(&self) -> bool {
        on_path(YDOTOOL)
    }

    fn inject(&self, text: &str) -> Result<(), String> {
        let status = std::process::Command::new(YDOTOOL)
            .arg("type")
            .arg(text)
            .status()
            .map_err(|error| format!("could not run {YDOTOOL}: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{YDOTOOL} exited with {status}"))
        }
    }
}

/// Whether `command` is an executable file on this process's PATH.
fn on_path(command: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).any(|dir| dir.join(command).is_file()))
        .unwrap_or(false)
}

/// A handle the session thread can call without knowing about the uinput worker.
#[derive(Clone)]
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

impl TextInjector for PortalInjector {
    fn name(&self) -> &'static str {
        CLIPBOARD
    }

    fn reason(&self) -> String {
        CLIPBOARD_REASON.to_owned()
    }

    fn available(&self) -> bool {
        // The clipboard path is the primary: a writable clipboard means the
        // paste can land whenever the virtual keyboard is granted.
        Clipboard::new().map(|_| ()).is_ok()
    }

    fn inject(&self, text: &str) -> Result<(), String> {
        self.send_paste(text)
    }
}

impl InjectPort for PortalInjector {
    fn inject(&mut self, text: &str) -> Result<(), String> {
        self.send_paste(text)
    }
}

impl PortalInjector {
    /// Sends the paste job and waits for the bridge to answer.
    fn send_paste(&self, text: &str) -> Result<(), String> {
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

pub fn bridge(
    access: SharedAccess,
    inbox: std::sync::mpsc::Sender<Input>,
    order: Vec<String>,
) -> Bridge {
    let (jobs_tx, jobs_rx) = tokio::sync::mpsc::unbounded_channel();
    let (retry_tx, retry_rx) = tokio::sync::mpsc::unbounded_channel();

    let injector = PortalInjector {
        jobs: jobs_tx.clone(),
    };
    let copy = CopyHandle { jobs: jobs_tx };

    tauri::async_runtime::spawn(serve(
        access,
        inbox,
        jobs_rx,
        retry_rx,
        order,
        injector.clone(),
    ));

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
    order: Vec<String>,
    injector: PortalInjector,
) {
    let mut device: Option<VirtualDevice> = None;
    let mut problem: Option<String> = None;
    // Keep the Clipboard instance alive so Wayland continues serving the selection.
    let mut clipboard: Option<Clipboard> = None;
    let mut reported = false;
    let ydotool = YdotoolAdapter;

    loop {
        if device.is_none() {
            match keyboard::open() {
                Ok(dev) => {
                    device = Some(dev);
                    problem = None;
                    let active = choose(&order, true, &injector, &ydotool);
                    portal::update(&access, |state| {
                        state.injection.ready = true;
                        state.injection.problem = None;
                        state.injection.active = active.clone();
                        state.injection.reason = reason_for(&active, &injector, &ydotool);
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
                        let delivered =
                            deliver(&mut device, &mut clipboard, &order, &ydotool, &problem, &text);
                        if let Ok(active) = &delivered {
                            portal::update(&access, |state| {
                                state.injection.active = active.clone();
                                state.injection.reason =
                                    reason_for(active, &injector, &ydotool);
                            });
                        }
                        let result = delivered.map(|_| ());
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

/// The first available adapter in the order, so Settings can name the one
/// that will deliver.
fn choose(
    order: &[String],
    clipboard_ready: bool,
    clipboard: &PortalInjector,
    ydotool: &YdotoolAdapter,
) -> String {
    for name in order {
        match name.as_str() {
            CLIPBOARD if clipboard_ready => return clipboard.name().to_owned(),
            YDOTOOL if ydotool.available() => return ydotool.name().to_owned(),
            _ => {}
        }
    }
    String::new()
}

/// The reason Settings shows for the named active adapter.
fn reason_for(active: &str, clipboard: &PortalInjector, ydotool: &YdotoolAdapter) -> String {
    match active {
        CLIPBOARD => clipboard.reason(),
        YDOTOOL => ydotool.reason(),
        _ => String::new(),
    }
}

/// Tries each injector in the configured order; the first success returns its
/// name, so the bridge can tell Settings which adapter delivered.
fn deliver(
    device: &mut Option<VirtualDevice>,
    clipboard: &mut Option<Clipboard>,
    order: &[String],
    ydotool: &YdotoolAdapter,
    problem: &Option<String>,
    text: &str,
) -> Result<String, String> {
    let mut tried = Vec::new();
    for name in order {
        match name.as_str() {
            CLIPBOARD => match device.as_mut() {
                Some(device) => match paste(device, clipboard, text) {
                    Ok(()) => return Ok(CLIPBOARD.to_owned()),
                    Err(error) => tried.push(error),
                },
                None => tried.push(
                    problem
                        .clone()
                        .unwrap_or_else(|| "the virtual keyboard is not available".to_owned()),
                ),
            },
            YDOTOOL => {
                if !ydotool.available() {
                    tried.push("ydotool is not on PATH".to_owned());
                    continue;
                }
                match ydotool.inject(text) {
                    Ok(()) => {
                        crate::devlog::step(
                            "inject",
                            format!("pasted {} bytes via ydotool", text.len()),
                        );
                        return Ok(YDOTOOL.to_owned());
                    }
                    Err(error) => tried.push(error),
                }
            }
            other => tried.push(format!("unknown injector: {other}")),
        }
    }
    Err(tried.join("; "))
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
    let previous = ensure_clipboard(clipboard)?.get_text().ok();
    let delivered = copy(clipboard, text).and_then(|()| {
        thread::sleep(CLIPBOARD_SETTLE);
        crate::devlog::step(
            "inject",
            format!("pasting {} bytes via clipboard+uinput (Ctrl+V)", text.len()),
        );
        keyboard::tap(device, &keyboard::PASTE)
    });
    // The user's clipboard is saved and restored on a best-effort basis: the
    // dictation may have landed, and even when it did not, the clipboard is
    // not theirs to leave clobbered.
    if let Some(previous) = previous {
        if let Some(clip) = clipboard.as_mut() {
            if let Err(error) = clip.set_text(previous) {
                eprintln!("inject: could not restore the previous clipboard: {error}");
            }
        }
    }
    delivered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clipboard_adapter_answers_when_the_bridge_is_not_running() {
        let (jobs, closed) = tokio::sync::mpsc::unbounded_channel();
        drop(closed);
        let injector = PortalInjector { jobs };

        assert_eq!(injector.name(), CLIPBOARD);
        assert_eq!(injector.reason(), CLIPBOARD_REASON);
        let error = TextInjector::inject(&injector, "hello")
            .expect_err("the bridge is not running, so the caller must get an error");
        assert_eq!(error, "the keyboard bridge is not running");
    }

    #[test]
    fn the_ydotool_adapter_is_optional() {
        let adapter = YdotoolAdapter;

        assert_eq!(adapter.name(), YDOTOOL);
        assert_eq!(adapter.reason(), YDOTOOL_REASON);
        // A name that cannot be on PATH, so the check is provably real.
        assert!(!on_path("sotto-no-such-command"));
        if !adapter.available() {
            assert!(
                adapter.inject("hello").is_err(),
                "an unavailable ydotool must not claim success"
            );
        }
    }

    #[test]
    fn the_active_adapter_is_named_with_its_reason() {
        let (jobs, closed) = tokio::sync::mpsc::unbounded_channel();
        drop(closed);
        let clipboard = PortalInjector { jobs };
        let ydotool = YdotoolAdapter;

        assert_eq!(reason_for(CLIPBOARD, &clipboard, &ydotool), CLIPBOARD_REASON);
        assert_eq!(reason_for(YDOTOOL, &clipboard, &ydotool), YDOTOOL_REASON);
        assert_eq!(reason_for("", &clipboard, &ydotool), "");
        assert_eq!(clipboard.name(), CLIPBOARD);
    }
}
