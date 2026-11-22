//! One dictation at a time, and every rule about when one may happen.
//!
//! The rules are the product: a press that is too short, a room that is silent,
//! a press while the last one is still being transcribed — none of these may
//! reach the provider or the focused app. Testing that through a real
//! microphone and a real portal is impossible, so all of them live here as a
//! state machine over [`Input`], and the microphone, provider, injector, pill
//! and history sit behind ports that tests replace with fakes.
//!
//! Timestamps arrive with each press and release, so a press that happened
//! while a transcription was running is judged by when it happened, not by when
//! the loop got to it.
//!
//! A Recording is saved before the provider is asked, so a failure leaves
//! something Retry can send again. The user never repeats the sentence.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    dictation::{Attempt, Captured},
    failure::{self, Failure, FailureKind},
    history::{HistoryPort, Injection, RetrySource, StoredRecording},
};

/// Shorter than this and the press is a tap, not an attempt to speak.
pub const MIN_HOLD: Duration = Duration::from_millis(400);

/// Two taps closer together than this lock dictation on.
pub const DOUBLE_TAP: Duration = Duration::from_millis(350);

/// How long a finished pill stays before it disappears.
pub const DONE_VISIBLE: Duration = Duration::from_millis(900);
/// A failure needs to be read, so it stays longer.
pub const FAILED_VISIBLE: Duration = Duration::from_secs(4);
/// "Too short" and "nothing heard" are advice, not errors.
pub const HINT_VISIBLE: Duration = Duration::from_millis(1_600);

/// Press and release come from the portal with the instant they happened.
#[derive(Debug, Clone)]
pub enum Input {
    Press {
        at: Instant,
    },
    Release {
        at: Instant,
    },
    /// The window's button, which has no press and release to give us.
    Toggle {
        at: Instant,
    },
    Cancel,
    /// The provider answered. `id` is the stored Recording this attempt belongs
    /// to; it is absent only when the Recording itself could not be saved.
    Transcript {
        id: Option<String>,
        result: Result<Attempt, Failure>,
    },
    /// Re-transcribe a Recording already on disk. `None` retries the last
    /// failed dictation, which is what the Pill's Retry button means.
    Retry {
        id: Option<String>,
    },
    /// A background capability failed: the shortcut, the keyboard bridge or
    /// the store. These have no Recording to retry.
    CapabilityFailed {
        failure: Failure,
    },
    /// A capability came back after the user fixed it, so a matching failure
    /// should stop being shown.
    CapabilityRestored {
        kind: FailureKind,
    },
}

/// Work the runner has to do outside the state machine: the transcription is
/// the one part that takes long enough to make a blocking call feel broken.
#[derive(Debug)]
pub enum Effect {
    Transcribe {
        id: Option<String>,
        captured: Captured,
    },
    /// Something failed before there was anything to transcribe.
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Listening,
    Working,
    Done,
    Failed,
    Hint,
}

impl Phase {
    /// Whether the pill should be on screen at all.
    #[cfg(test)]
    pub fn visible(self) -> bool {
        self != Phase::Idle
    }
}

/// What the window and the pill render.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub phase: Phase,
    pub locked: bool,
    /// Releases never arrived, so a press is a start and the next is a stop.
    pub tap_mode: bool,
    /// Advice, never an error: "Too short", "Nothing was heard", "Cancelled".
    pub message: Option<String>,
    pub attempt: Option<Attempt>,
    /// The one failure vocabulary. The short reason comes from
    /// [`Failure::reason`] when this is serialized.
    pub failure: Option<Failure>,
    /// The Recording Retry would send again. Absent when there is nothing on
    /// disk to retry.
    pub retry_id: Option<String>,
}

impl View {
    pub(crate) fn idle(tap_mode: bool) -> Self {
        Self {
            phase: Phase::Idle,
            locked: false,
            tap_mode,
            message: None,
            attempt: None,
            failure: None,
            retry_id: None,
        }
    }
}

pub trait CapturePort {
    fn start(&mut self) -> Result<(), Failure>;
    fn stop(&mut self) -> Result<Captured, Failure>;
}

pub trait InjectPort {
    fn inject(&mut self, text: &str) -> Result<(), String>;
}

pub trait SurfacePort {
    fn show(&mut self, view: &View, hide_after: Option<Duration>);
}

pub struct Ports {
    pub capture: Box<dyn CapturePort>,
    pub injector: Box<dyn InjectPort>,
    pub surface: Box<dyn SurfacePort>,
    pub history: Box<dyn HistoryPort>,
}

#[derive(Debug)]
enum State {
    Idle,
    Listening { since: Instant, locked: bool },
    Working { cancelled: bool },
}

/// The last dictation that finished, so the window can keep showing it while
/// the next one runs.
struct Last {
    attempt: Option<Attempt>,
    message: Option<String>,
    failure: Option<Failure>,
    retry_id: Option<String>,
    /// What an idle session is showing: nothing, a result, advice, or a failure.
    face: Phase,
}

pub struct Session {
    ports: Ports,
    state: State,
    tap_mode: bool,
    last_tap_end: Option<Instant>,
    last: Last,
    /// A Recording that could not be saved. The dictation still runs, and the
    /// warning is attached to whatever the provider says.
    pending_storage_failure: Option<Failure>,
    /// While a transcription runs, presses from before it finished are late
    /// echoes of a busy app and must not start dictation.
    busy_until: Option<Instant>,
}

impl Session {
    pub fn new(ports: Ports) -> Self {
        Self {
            ports,
            state: State::Idle,
            tap_mode: false,
            last_tap_end: None,
            last: Last {
                attempt: None,
                message: None,
                failure: None,
                retry_id: None,
                face: Phase::Idle,
            },
            pending_storage_failure: None,
            busy_until: None,
        }
    }

    pub fn view(&self) -> View {
        let mut view = View::idle(self.tap_mode);
        view.attempt = self.last.attempt.clone();
        view.message = self.last.message.clone();
        view.failure = self.last.failure.clone();
        view.retry_id = self.last.retry_id.clone();

        match self.state {
            State::Idle => view.phase = self.last.face,
            State::Listening { locked, .. } => {
                view.phase = Phase::Listening;
                view.locked = locked;
                view.message = None;
                view.failure = None;
                view.retry_id = None;
            }
            State::Working { .. } => {
                view.phase = Phase::Working;
                view.message = None;
                view.failure = None;
                view.retry_id = None;
            }
        }
        view
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Press { at } => self.press(at),
            Input::Release { at } => self.release(at),
            Input::Toggle { at } => match self.state {
                State::Idle => self.press(at),
                State::Listening { .. } => self.release(at),
                State::Working { .. } => Vec::new(),
            },
            Input::Cancel => self.cancel(),
            Input::Transcript { id, result } => self.transcript(id, result),
            Input::Retry { id } => self.retry(id),
            Input::CapabilityFailed { failure } => self.capability_failed(failure),
            Input::CapabilityRestored { kind } => self.capability_restored(kind),
        }
    }

    fn press(&mut self, at: Instant) -> Vec<Effect> {
        if matches!(self.state, State::Working { .. }) {
            return Vec::new();
        }
        if let Some(busy) = self.busy_until {
            if at <= busy {
                return Vec::new();
            }
        }

        match self.state {
            State::Listening { locked: true, .. } => self.finish(at),
            State::Listening { .. } => {
                // A press while the last one is still recording means the
                // environment never sends releases: switch to taps by itself.
                self.tap_mode = true;
                self.finish(at)
            }
            State::Idle => {
                let locked = self
                    .last_tap_end
                    .is_some_and(|end| at.saturating_duration_since(end) <= DOUBLE_TAP);

                match self.ports.capture.start() {
                    Ok(()) => {
                        self.state = State::Listening { since: at, locked };
                        self.last_tap_end = None;
                        self.last.message = None;
                        self.last.failure = None;
                        self.last.retry_id = None;
                        self.last.face = Phase::Idle;
                        self.pending_storage_failure = None;
                        self.show(None);
                        Vec::new()
                    }
                    Err(failure) => {
                        self.state = State::Idle;
                        self.last.attempt = None;
                        self.fail(failure, None, Some(FAILED_VISIBLE))
                    }
                }
            }
            State::Working { .. } => Vec::new(),
        }
    }

    fn release(&mut self, at: Instant) -> Vec<Effect> {
        match self.state {
            State::Idle => Vec::new(),
            State::Working { .. } => Vec::new(),
            State::Listening { locked: true, .. } => Vec::new(),
            State::Listening { since, .. } => {
                if at.saturating_duration_since(since) < MIN_HOLD {
                    self.last_tap_end = Some(at);
                    self.state = State::Idle;
                    if let Err(failure) = self.ports.capture.stop() {
                        return self.fail(failure, None, Some(FAILED_VISIBLE));
                    }
                    return self.hint("Too short — hold a little longer");
                }
                self.finish(at)
            }
        }
    }

    /// Stops the microphone, saves the Recording, and asks the runner to
    /// transcribe it.
    fn finish(&mut self, _at: Instant) -> Vec<Effect> {
        let captured = match self.ports.capture.stop() {
            Ok(captured) => captured,
            Err(failure) => {
                self.state = State::Idle;
                return self.fail(failure, None, Some(FAILED_VISIBLE));
            }
        };

        self.state = State::Idle;
        self.last.face = Phase::Idle;

        if captured.duration_ms < MIN_HOLD.as_millis() as u64 {
            self.last_tap_end = None;
            return self.hint("Too short — hold a little longer");
        }
        if captured.silent {
            return self.hint("Nothing was heard");
        }

        // The Recording is saved before the provider is asked. If the save
        // fails, the words are still transcribed and inserted; the warning is
        // attached to the result rather than losing the sentence.
        let (id, storage_failure) = match self.ports.history.record(&captured) {
            Ok(StoredRecording { id }) => (Some(id), None),
            Err(failure) => {
                failure.log();
                (None, Some(failure))
            }
        };
        self.pending_storage_failure = storage_failure;

        self.state = State::Working { cancelled: false };
        self.show(None);
        vec![Effect::Transcribe { id, captured }]
    }

    fn cancel(&mut self) -> Vec<Effect> {
        match self.state {
            State::Idle => Vec::new(),
            State::Listening { .. } => {
                self.state = State::Idle;
                self.pending_storage_failure = None;
                if let Err(failure) = self.ports.capture.stop() {
                    return self.fail(failure, None, Some(FAILED_VISIBLE));
                }
                self.hint("Cancelled")
            }
            State::Working { .. } => {
                // The reply is in flight and cannot be recalled, so the session
                // goes idle at once and the reply is dropped when it lands.
                self.state = State::Idle;
                self.pending_storage_failure = None;
                self.hint("Cancelled")
            }
        }
    }

    fn transcript(&mut self, id: Option<String>, result: Result<Attempt, Failure>) -> Vec<Effect> {
        let cancelled = match self.state {
            State::Working { cancelled } => cancelled,
            // A reply that arrives after a cancel is stale by definition.
            _ => true,
        };
        self.busy_until = Some(Instant::now());
        self.state = State::Idle;

        if cancelled {
            self.pending_storage_failure = None;
            match result {
                Ok(attempt) => {
                    if let Some(id) = id {
                        if let Err(failure) = self.ports.history.complete(
                            &id,
                            &attempt,
                            Injection::NotAttempted,
                            None,
                        ) {
                            failure.log();
                        }
                    }
                }
                Err(failure) => {
                    failure.log();
                    if let Some(id) = id {
                        if let Err(error) = self.ports.history.fail(&id, &failure) {
                            error.log();
                        }
                    }
                }
            }
            return Vec::new();
        }

        match result {
            Ok(attempt) => {
                let text = attempt.cleaned.trim().to_owned();
                if text.is_empty() {
                    let history_error = match &id {
                        Some(id) => self
                            .ports
                            .history
                            .complete(id, &attempt, Injection::NotAttempted, None)
                            .err(),
                        None => None,
                    };
                    let warning = self.pending_storage_failure.take().or(history_error);
                    if let Some(warning) = warning {
                        self.last.attempt = Some(attempt);
                        return self.fail(warning, None, Some(FAILED_VISIBLE));
                    }
                    return self.hint("No words came back");
                }

                match self.ports.injector.inject(&text) {
                    Ok(()) => {
                        let history_error = match &id {
                            Some(id) => self
                                .ports
                                .history
                                .complete(id, &attempt, Injection::Inserted, None)
                                .err(),
                            None => None,
                        };
                        let warning = attempt
                            .cleanup_failure
                            .clone()
                            .or_else(|| self.pending_storage_failure.take())
                            .or(history_error);
                        self.last.attempt = Some(attempt);

                        if let Some(warning) = warning {
                            // The text already landed, so Retry would insert it
                            // a second time. The warning is worth showing, but
                            // it is not a retryable dictation.
                            return self.fail(warning, None, Some(FAILED_VISIBLE));
                        }

                        self.last.message = None;
                        self.last.failure = None;
                        self.last.retry_id = None;
                        self.last.face = Phase::Done;
                        self.show(Some(DONE_VISIBLE));
                        Vec::new()
                    }
                    Err(reason) => {
                        let failure = failure::injection(reason);
                        let history_error = match &id {
                            Some(id) => self
                                .ports
                                .history
                                .complete(id, &attempt, Injection::Blocked, Some(&failure))
                                .err(),
                            None => None,
                        };
                        let failure = match history_error {
                            Some(error) => merge_history_failure(failure, error),
                            None => failure,
                        };
                        self.last.attempt = Some(attempt);
                        let retry_id = id.clone();
                        self.fail(failure, retry_id, Some(FAILED_VISIBLE))
                    }
                }
            }
            Err(failure) => {
                let history_error = match &id {
                    Some(id) => self.ports.history.fail(id, &failure).err(),
                    None => None,
                };
                let storage_error = self.pending_storage_failure.take();
                let failure = match history_error {
                    Some(error) => merge_history_failure(failure, error),
                    None => failure,
                };
                let failure = match storage_error {
                    Some(error) => merge_history_failure(failure, error),
                    None => failure,
                };
                self.last.attempt = None;
                self.fail(failure, id, Some(FAILED_VISIBLE))
            }
        }
    }

    /// Sends a Recording already on disk to the provider again.
    fn retry(&mut self, id: Option<String>) -> Vec<Effect> {
        if !matches!(self.state, State::Idle) {
            return Vec::new();
        }

        let Some(id) = id.or_else(|| self.last.retry_id.clone()) else {
            return self.hint("Nothing to retry");
        };

        match self.ports.history.retry_source(&id) {
            Ok(RetrySource { captured }) => {
                self.state = State::Working { cancelled: false };
                self.pending_storage_failure = None;
                self.last.message = None;
                self.last.failure = None;
                self.last.retry_id = None;
                self.last.face = Phase::Idle;
                self.show(None);
                vec![Effect::Transcribe {
                    id: Some(id),
                    captured,
                }]
            }
            Err(failure) => {
                self.state = State::Idle;
                self.last.attempt = None;
                self.fail(failure, None, Some(FAILED_VISIBLE))
            }
        }
    }

    fn capability_failed(&mut self, failure: Failure) -> Vec<Effect> {
        failure.log();
        self.last.message = None;
        self.last.attempt = None;
        self.last.failure = Some(failure);
        self.last.retry_id = None;
        self.last.face = Phase::Failed;
        if matches!(self.state, State::Idle) {
            self.show(Some(FAILED_VISIBLE));
        }
        vec![Effect::Failed]
    }

    fn capability_restored(&mut self, kind: FailureKind) -> Vec<Effect> {
        if !matches!(self.state, State::Idle) {
            return Vec::new();
        }
        let matches = self.last.face == Phase::Failed
            && self
                .last
                .failure
                .as_ref()
                .is_some_and(|failure| failure.kind == kind);
        if matches {
            self.last.failure = None;
            self.last.retry_id = None;
            self.last.face = Phase::Idle;
            self.show(None);
        }
        Vec::new()
    }

    fn fail(
        &mut self,
        failure: Failure,
        retry_id: Option<String>,
        hide_after: Option<Duration>,
    ) -> Vec<Effect> {
        failure.log();
        self.last.message = None;
        self.last.failure = Some(failure);
        self.last.retry_id = retry_id;
        self.last.face = Phase::Failed;
        self.show(hide_after);
        vec![Effect::Failed]
    }

    fn hint(&mut self, text: &str) -> Vec<Effect> {
        self.last.message = Some(text.to_owned());
        self.last.failure = None;
        self.last.retry_id = None;
        self.last.face = Phase::Hint;
        self.show(Some(HINT_VISIBLE));
        vec![]
    }

    fn show(&mut self, hide_after: Option<Duration>) {
        let view = self.view();
        self.ports.surface.show(&view, hide_after);
    }
}

/// Keeps the user-facing failure as the reason and appends the history write
/// that also failed to the cause chain, rather than replacing one with the
/// other.
fn merge_history_failure(primary: Failure, history: Failure) -> Failure {
    let cause = format!("{}; history: {}", primary.cause_chain(), history.detail);
    Failure::with_cause(primary.kind, primary.detail, cause)
}

/// Shared by the runner and the window: the pill and the window both render it.
pub type SharedView = Arc<std::sync::Mutex<View>>;

pub fn shared_view() -> SharedView {
    Arc::new(std::sync::Mutex::new(View::idle(false)))
}

pub fn publish(view: &SharedView, next: View) {
    if let Ok(mut slot) = view.lock() {
        *slot = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::Timings;
    use std::{cell::RefCell, rc::Rc};

    const ID: &str = "dictation-1";

    #[derive(Default)]
    struct Fake {
        started: usize,
        stopped: usize,
        injected: Vec<String>,
        pills: Vec<(Phase, Option<Duration>)>,
        /// What the next `start` returns.
        start_error: Option<Failure>,
        /// What the next `stop` returns.
        stop_error: Option<Failure>,
        /// What the next `inject` returns.
        inject_error: Option<String>,
        /// What the next `record` returns.
        record_error: Option<Failure>,
        captured: Option<Captured>,
        /// What Retry reads from history.
        retry: Option<Captured>,
        stored: Vec<Captured>,
        completed: Vec<(String, Injection, Option<Failure>)>,
        failed: Vec<(String, Failure)>,
    }

    #[derive(Clone, Default)]
    struct Shared(Rc<RefCell<Fake>>);

    impl Shared {
        fn new() -> Self {
            Self(Rc::new(RefCell::new(Fake::default())))
        }

        fn with_capture(duration_ms: u64, silent: bool) -> Self {
            let shared = Self::new();
            shared.0.borrow_mut().captured = Some(Captured {
                wav: vec![0u8; 4],
                duration_ms,
                stop_ms: 1,
                encode_ms: 2,
                silent,
            });
            shared
        }

        fn take(&self) -> std::cell::RefMut<'_, Fake> {
            self.0.borrow_mut()
        }
    }

    struct SharedCapture(Shared);
    impl CapturePort for SharedCapture {
        fn start(&mut self) -> Result<(), Failure> {
            let mut fake = self.0.take();
            fake.started += 1;
            match fake.start_error.clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn stop(&mut self) -> Result<Captured, Failure> {
            let mut fake = self.0.take();
            fake.stopped += 1;
            if let Some(error) = fake.stop_error.clone() {
                return Err(error);
            }
            match fake.captured.clone() {
                Some(captured) => Ok(captured),
                None => Err(Failure::new(
                    FailureKind::MicDenied,
                    "no fake recording was set",
                )),
            }
        }
    }

    struct SharedInjector(Shared);
    impl InjectPort for SharedInjector {
        fn inject(&mut self, text: &str) -> Result<(), String> {
            let mut fake = self.0.take();
            match fake.inject_error.clone() {
                Some(error) => Err(error),
                None => {
                    fake.injected.push(text.to_owned());
                    Ok(())
                }
            }
        }
    }

    struct SharedSurface(Shared);
    impl SurfacePort for SharedSurface {
        fn show(&mut self, view: &View, hide_after: Option<Duration>) {
            self.0.take().pills.push((view.phase, hide_after));
        }
    }

    struct SharedHistory(Shared);
    impl HistoryPort for SharedHistory {
        fn record(&mut self, captured: &Captured) -> Result<StoredRecording, Failure> {
            let mut fake = self.0.take();
            if let Some(error) = fake.record_error.clone() {
                return Err(error);
            }
            fake.stored.push(captured.clone());
            fake.retry = Some(captured.clone());
            Ok(StoredRecording { id: ID.to_owned() })
        }

        fn complete(
            &mut self,
            id: &str,
            _attempt: &Attempt,
            injection: Injection,
            failure: Option<&Failure>,
        ) -> Result<(), Failure> {
            self.0
                .take()
                .completed
                .push((id.to_owned(), injection, failure.cloned()));
            Ok(())
        }

        fn fail(&mut self, id: &str, failure: &Failure) -> Result<(), Failure> {
            self.0.take().failed.push((id.to_owned(), failure.clone()));
            Ok(())
        }

        fn retry_source(&mut self, _id: &str) -> Result<RetrySource, Failure> {
            match self.0.take().retry.clone() {
                Some(captured) => Ok(RetrySource { captured }),
                None => Err(Failure::new(
                    FailureKind::StorageFailed,
                    "no recording to retry",
                )),
            }
        }
    }

    fn session(capture: &Shared) -> Session {
        Session::new(Ports {
            capture: Box::new(SharedCapture(capture.clone())),
            injector: Box::new(SharedInjector(capture.clone())),
            surface: Box::new(SharedSurface(capture.clone())),
            history: Box::new(SharedHistory(capture.clone())),
        })
    }

    fn attempt(text: &str) -> Attempt {
        Attempt {
            transcript: text.to_owned(),
            cleaned: text.to_owned(),
            cleanup_failure: None,
            cleanup_model: None,
            provider: "fake".to_owned(),
            model: "fake".to_owned(),
            duration_ms: 2_000,
            timings: Timings {
                stop_ms: 1,
                encode_ms: 2,
                upload_ms: 3,
                provider_ms: 4,
                cleanup_ms: 0,
                total_ms: 10,
            },
        }
    }

    fn transcribed(text: &str) -> Input {
        Input::Transcript {
            id: Some(ID.to_owned()),
            result: Ok(attempt(text)),
        }
    }

    fn press_and_release(session: &mut Session) {
        let start = Instant::now();
        session.handle(Input::Press { at: start });
        session.handle(Input::Release {
            at: start + Duration::from_millis(2_000),
        });
    }

    #[test]
    fn a_press_and_a_real_hold_saves_the_recording_and_injects_the_text() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        press_and_release(&mut session);
        assert_eq!(fake.take().stored.len(), 1, "the recording is saved first");

        session.handle(transcribed("Hello there."));

        assert_eq!(fake.take().injected, vec!["Hello there.".to_owned()]);
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Done);
        assert_eq!(
            fake.take().completed,
            vec![(ID.to_owned(), Injection::Inserted, None)]
        );
    }

    #[test]
    fn cleanup_failure_injects_raw_transcript_and_surfaces_error() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        press_and_release(&mut session);

        let mut att = attempt("um raw whisper text");
        att.cleanup_failure = Some(Failure::new(FailureKind::RateLimited, "rate limited"));
        let effects = session.handle(Input::Transcript {
            id: Some(ID.to_owned()),
            result: Ok(att),
        });

        let snapshot = fake.take();
        assert_eq!(snapshot.injected, vec!["um raw whisper text".to_owned()]);
        assert_eq!(snapshot.pills.last().expect("a pill").0, Phase::Failed);
        assert!(matches!(effects.as_slice(), [Effect::Failed]));
        assert_eq!(
            session.view().failure.as_ref().map(|failure| failure.kind),
            Some(FailureKind::RateLimited)
        );
        assert!(
            session.view().retry_id.is_none(),
            "the raw text already landed, so retry would duplicate it"
        );
    }

    #[test]
    fn a_press_under_four_hundred_milliseconds_never_reaches_the_provider() {
        let fake = Shared::with_capture(300, false);
        let mut session = session(&fake);
        let start = Instant::now();

        session.handle(Input::Press { at: start });
        let effects = session.handle(Input::Release {
            at: start + Duration::from_millis(300),
        });

        assert!(effects.is_empty(), "nothing to transcribe");
        assert_eq!(fake.take().stopped, 1, "the microphone is still closed");
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Hint);
        assert!(fake.take().injected.is_empty());
        assert!(fake.take().stored.is_empty(), "nothing worth keeping");
    }

    #[test]
    fn a_held_but_silent_recording_never_reaches_the_provider() {
        let fake = Shared::with_capture(3_000, true);
        let mut session = session(&fake);
        let start = Instant::now();

        session.handle(Input::Press { at: start });
        let effects = session.handle(Input::Release {
            at: start + Duration::from_millis(3_000),
        });

        assert!(effects.is_empty());
        assert!(fake.take().injected.is_empty());
        assert!(fake.take().stored.is_empty());
        assert_eq!(
            fake.take().pills.last().expect("a pill").1,
            Some(HINT_VISIBLE)
        );
    }

    #[test]
    fn a_press_during_work_is_ignored_rather_than_queued() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);
        // Ten seconds ago, so work done "now" really is later than the press.
        let start = Instant::now() - Duration::from_secs(10);

        session.handle(Input::Press { at: start });
        session.handle(Input::Release {
            at: start + Duration::from_millis(2_000),
        });

        // A press that lands while the provider is still thinking.
        session.handle(Input::Press {
            at: start + Duration::from_millis(2_100),
        });
        assert_eq!(fake.take().started, 1, "no second recording was started");
        assert!(matches!(session.state, State::Working { .. }));

        session.handle(transcribed("First."));

        // And one that was queued behind the reply keeps its old timestamp.
        session.handle(Input::Press {
            at: start + Duration::from_millis(2_500),
        });
        assert_eq!(
            fake.take().started,
            1,
            "a stale press is still a press from while busy"
        );
    }

    #[test]
    fn a_press_after_the_reply_has_landed_starts_a_new_dictation() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);
        let start = Instant::now() - Duration::from_secs(5);

        session.handle(Input::Press { at: start });
        session.handle(Input::Release {
            at: start + Duration::from_millis(2_000),
        });
        session.handle(transcribed("First."));

        session.handle(Input::Press {
            at: Instant::now() + Duration::from_secs(1),
        });

        assert_eq!(fake.take().started, 2);
    }

    #[test]
    fn a_double_tap_locks_dictation_on_until_the_next_press() {
        let fake = Shared::with_capture(2_500, false);
        let mut session = session(&fake);
        let start = Instant::now();

        // First tap: too short, discarded, and remembered.
        session.handle(Input::Press { at: start });
        session.handle(Input::Release {
            at: start + Duration::from_millis(120),
        });

        // Second tap: locks on.
        session.handle(Input::Press {
            at: start + Duration::from_millis(300),
        });
        assert!(session.view().locked, "the second tap locks dictation on");

        // Releasing the locked press does not end the dictation.
        session.handle(Input::Release {
            at: start + Duration::from_millis(420),
        });
        assert_eq!(session.view().phase, Phase::Listening);

        // The next press ends it and sends what was said in between.
        let effects = session.handle(Input::Press {
            at: start + Duration::from_millis(3_000),
        });
        assert!(matches!(effects[0], Effect::Transcribe { .. }));
    }

    #[test]
    fn without_release_events_the_session_switches_to_taps_by_itself() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);
        let start = Instant::now();

        session.handle(Input::Press { at: start });
        assert!(!session.view().tap_mode);

        // The user let go, but no release ever arrived; their next press is the
        // only evidence we get, and it must both stop and switch the mode.
        let effects = session.handle(Input::Press {
            at: start + Duration::from_millis(1_500),
        });

        assert!(session.view().tap_mode);
        assert!(matches!(effects[0], Effect::Transcribe { .. }));
    }

    #[test]
    fn cancelling_while_working_discards_the_reply_instead_of_injecting_it() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        press_and_release(&mut session);

        session.handle(Input::Cancel);
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Hint);

        // The provider answers anyway; it must go nowhere.
        session.handle(transcribed("Never mind."));

        assert!(fake.take().injected.is_empty());
        assert_eq!(
            fake.take().completed,
            vec![(ID.to_owned(), Injection::NotAttempted, None)],
            "the words are still kept in history"
        );
    }

    #[test]
    fn cancelling_while_listening_closes_the_microphone_without_sending() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        session.handle(Input::Press { at: Instant::now() });
        session.handle(Input::Cancel);

        assert_eq!(fake.take().stopped, 1);
        assert!(fake.take().injected.is_empty());
        assert_eq!(session.view().phase, Phase::Hint);
        assert_eq!(session.view().message.as_deref(), Some("Cancelled"));
    }

    #[test]
    fn an_empty_transcript_is_not_injected_but_is_still_in_history() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        press_and_release(&mut session);
        session.handle(transcribed("   "));

        assert!(fake.take().injected.is_empty());
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Hint);
        assert_eq!(
            fake.take().completed,
            vec![(ID.to_owned(), Injection::NotAttempted, None)]
        );
    }

    #[test]
    fn a_provider_failure_is_shown_with_its_reason_and_offers_retry() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        press_and_release(&mut session);
        let failure = Failure::new(FailureKind::Network, "the provider could not be reached");
        session.handle(Input::Transcript {
            id: Some(ID.to_owned()),
            result: Err(failure),
        });

        assert!(fake.take().injected.is_empty());
        assert_eq!(
            session.view().failure.as_ref().map(|failure| failure.kind),
            Some(FailureKind::Network)
        );
        assert_eq!(session.view().retry_id.as_deref(), Some(ID));
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Failed);
        assert_eq!(
            fake.take().pills.last().expect("a pill").1,
            Some(FAILED_VISIBLE)
        );
        assert_eq!(fake.take().failed.len(), 1);
    }

    #[test]
    fn retry_sends_the_stored_recording_again_and_inserts_the_text() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        press_and_release(&mut session);
        session.handle(Input::Transcript {
            id: Some(ID.to_owned()),
            result: Err(Failure::new(FailureKind::Network, "offline")),
        });

        let effects = session.handle(Input::Retry { id: None });

        let transcribe = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::Transcribe { id, captured } => Some((id, captured)),
                Effect::Failed => None,
            })
            .expect("retry transcribes the stored recording");
        assert_eq!(transcribe.0.as_deref(), Some(ID));
        assert_eq!(transcribe.1.wav, vec![0u8; 4]);

        session.handle(transcribed("Back online."));

        assert_eq!(fake.take().injected, vec!["Back online.".to_owned()]);
        assert_eq!(session.view().phase, Phase::Done);
        assert!(session.view().failure.is_none());
    }

    /// The acceptance path from ticket 07, with the real History rather than a
    /// fake: the network drops, the Recording stays on disk, and Retry sends
    /// those exact bytes once the network is back.
    #[test]
    fn a_network_failure_then_retry_inserts_the_stored_recording() {
        use crate::history::{History, MemoryStore, RecordingStore};

        let dir = tempfile::tempdir().expect("temp dir");
        let history = Arc::new(History::with_store(
            Arc::new(MemoryStore::new()),
            RecordingStore::new(dir.path().join("recordings")),
        ));
        let fake = Shared::with_capture(2_000, false);
        let mut session = Session::new(Ports {
            capture: Box::new(SharedCapture(fake.clone())),
            injector: Box::new(SharedInjector(fake.clone())),
            surface: Box::new(SharedSurface(fake.clone())),
            history: Box::new(Arc::clone(&history)),
        });

        press_and_release(&mut session);
        let id = history.list("").expect("the recording is in history")[0]
            .id
            .clone();

        session.handle(Input::Transcript {
            id: Some(id.clone()),
            result: Err(Failure::new(
                FailureKind::Network,
                "the provider could not be reached",
            )),
        });
        assert_eq!(
            history
                .entry(&id)
                .expect("stored row")
                .failure
                .map(|f| f.kind),
            Some(FailureKind::Network)
        );
        assert_eq!(session.view().retry_id.as_deref(), Some(id.as_str()));

        // The network comes back and the user presses Retry, not the key.
        let effects = session.handle(Input::Retry { id: None });
        assert!(matches!(
            effects.as_slice(),
            [Effect::Transcribe { id: Some(retried), captured }]
                if retried == &id && captured.wav == vec![0u8; 4]
        ));

        session.handle(Input::Transcript {
            id: Some(id.clone()),
            result: Ok(attempt("Back online.")),
        });

        assert_eq!(fake.take().injected, vec!["Back online.".to_owned()]);
        let entry = history.entry(&id).expect("stored row");
        assert_eq!(entry.injection, Injection::Inserted);
        assert!(entry.failure.is_none());
        assert_eq!(entry.cleaned, "Back online.");
    }

    #[test]
    fn retry_without_a_recording_is_advice_not_a_silent_no_op() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        session.handle(Input::Retry { id: None });

        assert_eq!(session.view().phase, Phase::Hint);
        assert_eq!(session.view().message.as_deref(), Some("Nothing to retry"));
    }

    #[test]
    fn a_blocked_injection_is_a_surfaced_failure_with_a_retry() {
        let fake = Shared::with_capture(2_000, false);
        fake.take().inject_error = Some("the keyboard bridge is not running".to_owned());
        let mut session = session(&fake);

        press_and_release(&mut session);
        session.handle(transcribed("Hello there."));

        assert_eq!(
            session.view().failure.as_ref().map(|failure| failure.kind),
            Some(FailureKind::InjectionBlocked)
        );
        assert_eq!(session.view().retry_id.as_deref(), Some(ID));
        assert_eq!(
            fake.take().completed,
            vec![(
                ID.to_owned(),
                Injection::Blocked,
                Some(Failure::new(
                    FailureKind::InjectionBlocked,
                    "the keyboard bridge is not running"
                ))
            )]
        );
    }

    #[test]
    fn a_recording_that_cannot_be_saved_still_gets_its_text_inserted() {
        let fake = Shared::with_capture(2_000, false);
        fake.take().record_error =
            Some(Failure::new(FailureKind::StorageFailed, "the disk is full"));
        let mut session = session(&fake);

        press_and_release(&mut session);
        let effects = session.handle(Input::Transcript {
            id: None,
            result: Ok(attempt("Keep these words.")),
        });

        assert_eq!(fake.take().injected, vec!["Keep these words.".to_owned()]);
        assert!(matches!(effects.as_slice(), [Effect::Failed]));
        assert_eq!(
            session.view().failure.as_ref().map(|failure| failure.kind),
            Some(FailureKind::StorageFailed)
        );
        assert!(
            session.view().retry_id.is_none(),
            "there is nothing to retry"
        );
    }

    #[test]
    fn a_microphone_that_will_not_open_is_a_failure_not_a_silent_no_op() {
        let fake = Shared::new();
        fake.take().start_error = Some(Failure::new(
            FailureKind::MicDenied,
            "no microphone is available",
        ));
        let mut session = session(&fake);

        let effects = session.handle(Input::Press { at: Instant::now() });

        assert!(matches!(effects[0], Effect::Failed));
        assert_eq!(session.view().phase, Phase::Failed);
        assert_eq!(
            session.view().failure.as_ref().map(|failure| failure.kind),
            Some(FailureKind::MicDenied)
        );
        assert!(session.view().retry_id.is_none());
        assert_eq!(
            fake.take().pills.last().expect("a pill").1,
            Some(FAILED_VISIBLE)
        );
    }

    #[test]
    fn a_capability_failure_is_shown_once_and_cleared_when_it_is_fixed() {
        let fake = Shared::new();
        let mut session = session(&fake);

        session.handle(Input::CapabilityFailed {
            failure: failure::shortcut("/dev/input/event* is not readable"),
        });
        assert_eq!(
            session.view().failure.as_ref().map(|failure| failure.kind),
            Some(FailureKind::ShortcutRefused)
        );

        session.handle(Input::CapabilityRestored {
            kind: FailureKind::InjectionBlocked,
        });
        assert!(
            session.view().failure.is_some(),
            "a different capability is not cleared"
        );

        session.handle(Input::CapabilityRestored {
            kind: FailureKind::ShortcutRefused,
        });
        assert!(session.view().failure.is_none());
        assert_eq!(session.view().phase, Phase::Idle);
    }

    #[test]
    fn the_window_button_runs_a_dictation_without_a_key_release() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);
        let start = Instant::now() - Duration::from_secs(5);

        session.handle(Input::Toggle { at: start });
        assert_eq!(session.view().phase, Phase::Listening);

        let effects = session.handle(Input::Toggle { at: Instant::now() });

        assert!(matches!(effects[0], Effect::Transcribe { .. }));
    }

    #[test]
    fn a_tap_mode_press_starts_and_the_next_press_stops() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);
        let start = Instant::now();

        // Learn tap mode the way the app does: a press with no release.
        session.handle(Input::Press { at: start });
        session.handle(Input::Press {
            at: start + Duration::from_millis(1_500),
        });
        session.handle(transcribed("One."));

        session.handle(Input::Press {
            at: start + Duration::from_secs(3),
        });
        assert_eq!(session.view().phase, Phase::Listening);

        let effects = session.handle(Input::Press {
            at: start + Duration::from_secs(5),
        });

        assert!(matches!(effects[0], Effect::Transcribe { .. }));
        assert!(session.view().tap_mode);
    }
}
