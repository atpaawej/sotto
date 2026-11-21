//! One dictation at a time, and every rule about when one may happen.
//!
//! The rules are the product: a press that is too short, a room that is silent,
//! a press while the last one is still being transcribed — none of these may
//! reach the provider or the focused app. Testing that through a real
//! microphone and a real portal is impossible, so all of them live here as a
//! state machine over [`Input`], and the microphone, provider, injector and
//! pill sit behind ports that tests replace with fakes.
//!
//! Timestamps arrive with each press and release, so a press that happened
//! while a transcription was running is judged by when it happened, not by when
//! the loop got to it.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::dictation::{Attempt, Captured};

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
    Press { at: Instant },
    Release { at: Instant },
    /// The window's button, which has no press and release to give us.
    Toggle { at: Instant },
    Cancel,
    Transcript {
        result: Result<Attempt, String>,
    },
}

/// Work the runner has to do outside the state machine: the transcription is
/// the one part that takes long enough to make a blocking call feel broken.
#[derive(Debug)]
pub enum Effect {
    Transcribe {
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
    pub message: Option<String>,
    pub attempt: Option<Attempt>,
}

impl View {
    fn idle(tap_mode: bool) -> Self {
        Self {
            phase: Phase::Idle,
            locked: false,
            tap_mode,
            message: None,
            attempt: None,
        }
    }
}

pub trait CapturePort {
    fn start(&mut self) -> Result<(), String>;
    fn stop(&mut self) -> Result<Captured, String>;
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
    /// What an idle session is showing: nothing, a result, or advice.
    face: Phase,
}

pub struct Session {
    ports: Ports,
    state: State,
    tap_mode: bool,
    last_tap_end: Option<Instant>,
    last: Last,
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
                face: Phase::Idle,
            },
            busy_until: None,
        }
    }

    pub fn view(&self) -> View {
        let mut view = View::idle(self.tap_mode);
        view.attempt = self.last.attempt.clone();
        view.message = self.last.message.clone();

        match self.state {
            State::Idle => view.phase = self.last.face,
            State::Listening { locked, .. } => {
                view.phase = Phase::Listening;
                view.locked = locked;
                view.message = None;
            }
            State::Working { .. } => {
                view.phase = Phase::Working;
                view.message = None;
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
            Input::Transcript { result } => self.transcript(result),
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
                        self.state = State::Listening {
                            since: at,
                            locked,
                        };
                        self.last_tap_end = None;
                        self.last.message = None;
                        self.last.face = Phase::Idle;
                        self.show(None);
                        Vec::new()
                    }
                    Err(reason) => {
                        self.last.attempt = None;
                        self.last.message = Some(reason.clone());
                        self.last.face = Phase::Failed;
                        self.show(Some(FAILED_VISIBLE));
                        vec![Effect::Failed]
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
                    if let Err(reason) = self.ports.capture.stop() {
                        self.last.message = Some(reason.clone());
                        self.last.face = Phase::Failed;
                        self.show(Some(FAILED_VISIBLE));
                        return vec![Effect::Failed];
                    }
                    return self.hint("Too short — hold a little longer");
                }
                self.finish(at)
            }
        }
    }

    /// Stops the microphone and either sends the recording or explains why not.
    fn finish(&mut self, _at: Instant) -> Vec<Effect> {
        let captured = match self.ports.capture.stop() {
            Ok(captured) => captured,
            Err(reason) => {
                self.state = State::Idle;
                self.last.message = Some(reason.clone());
                self.last.face = Phase::Failed;
                self.show(Some(FAILED_VISIBLE));
                return vec![Effect::Failed];
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

        self.state = State::Working { cancelled: false };
        self.show(None);
        vec![Effect::Transcribe { captured }]
    }

    fn cancel(&mut self) -> Vec<Effect> {
        match self.state {
            State::Idle => Vec::new(),
            State::Listening { .. } => {
                self.state = State::Idle;
                if let Err(reason) = self.ports.capture.stop() {
                    self.last.message = Some(reason.clone());
                    self.last.face = Phase::Failed;
                    self.show(Some(FAILED_VISIBLE));
                    return vec![Effect::Failed];
                }
                self.hint("Cancelled")
            }
            State::Working { .. } => {
                // The reply is in flight and cannot be recalled, so the session
                // goes idle at once and the reply is dropped when it lands.
                self.state = State::Idle;
                self.hint("Cancelled")
            }
        }
    }

    fn transcript(&mut self, result: Result<Attempt, String>) -> Vec<Effect> {
        let cancelled = match self.state {
            State::Working { cancelled } => cancelled,
            // A reply that arrives after a cancel is stale by definition.
            _ => true,
        };
        self.busy_until = Some(Instant::now());
        self.state = State::Idle;

        if cancelled {
            return Vec::new();
        }

        match result {
            Ok(attempt) => {
                let text = attempt.transcript.trim().to_owned();
                if text.is_empty() {
                    return self.hint("No words came back");
                }

                match self.ports.injector.inject(&text) {
                    Ok(()) => {
                        self.last.message = None;
                        self.last.face = Phase::Done;
                        self.last.attempt = Some(attempt);
                        self.show(Some(DONE_VISIBLE));
                        Vec::new()
                    }
                    Err(reason) => {
                        self.last.message = Some(reason.clone());
                        self.last.face = Phase::Failed;
                        self.show(Some(FAILED_VISIBLE));
                        vec![Effect::Failed]
                    }
                }
            }
            Err(reason) => {
                self.last.message = Some(reason.clone());
                self.last.face = Phase::Failed;
                self.show(Some(FAILED_VISIBLE));
                vec![Effect::Failed]
            }
        }
    }

    fn hint(&mut self, text: &str) -> Vec<Effect> {
        self.last.message = Some(text.to_owned());
        self.last.face = Phase::Hint;
        self.show(Some(HINT_VISIBLE));
        vec![]
    }

    fn show(&mut self, hide_after: Option<Duration>) {
        let view = self.view();
        self.ports.surface.show(&view, hide_after);
    }
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

    #[derive(Default)]
    struct Fake {
        started: usize,
        stopped: usize,
        injected: Vec<String>,
        pills: Vec<(Phase, Option<Duration>)>,
        /// What the next `start` returns.
        start_error: Option<String>,
        /// What the next `stop` returns.
        stop_error: Option<String>,
        captured: Option<Captured>,
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
        fn start(&mut self) -> Result<(), String> {
            let mut fake = self.0.take();
            fake.started += 1;
            match fake.start_error.clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        }

        fn stop(&mut self) -> Result<Captured, String> {
            let mut fake = self.0.take();
            fake.stopped += 1;
            if let Some(error) = fake.stop_error.clone() {
                return Err(error);
            }
            match fake.captured.clone() {
                Some(captured) => Ok(captured),
                None => Err("no fake recording was set".to_owned()),
            }
        }
    }

    struct SharedInjector(Shared);
    impl InjectPort for SharedInjector {
        fn inject(&mut self, text: &str) -> Result<(), String> {
            self.0.take().injected.push(text.to_owned());
            Ok(())
        }
    }

    struct SharedSurface(Shared);
    impl SurfacePort for SharedSurface {
        fn show(&mut self, view: &View, hide_after: Option<Duration>) {
            self.0.take().pills.push((view.phase, hide_after));
        }
    }

    fn session(capture: &Shared) -> Session {
        Session::new(Ports {
            capture: Box::new(SharedCapture(capture.clone())),
            injector: Box::new(SharedInjector(capture.clone())),
            surface: Box::new(SharedSurface(capture.clone())),
        })
    }

    fn attempt(text: &str) -> Attempt {
        Attempt {
            transcript: text.to_owned(),
            provider: "fake".to_owned(),
            model: "fake".to_owned(),
            duration_ms: 2_000,
            timings: Timings {
                stop_ms: 1,
                encode_ms: 2,
                upload_ms: 3,
                provider_ms: 4,
                total_ms: 10,
            },
        }
    }

    fn transcribed(text: &str) -> Input {
        Input::Transcript {
            result: Ok(attempt(text)),
        }
    }

    #[test]
    fn a_press_and_a_real_hold_sends_the_recording_and_injects_the_text() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);

        session.handle(Input::Press { at: Instant::now() });
        assert_eq!(fake.take().started, 1);

        let effects = session.handle(Input::Release {
            at: Instant::now() + Duration::from_millis(2_000),
        });
        assert!(matches!(effects[0], Effect::Transcribe { .. }));

        session.handle(transcribed("Hello there."));

        assert_eq!(fake.take().injected, vec!["Hello there.".to_owned()]);
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Done);
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
        session.handle(Input::Press { at: start + Duration::from_millis(2_100) });
        assert_eq!(fake.take().started, 1, "no second recording was started");
        assert!(matches!(session.state, State::Working { .. }));

        session.handle(transcribed("First."));

        // And one that was queued behind the reply keeps its old timestamp.
        session.handle(Input::Press { at: start + Duration::from_millis(2_500) });
        assert_eq!(fake.take().started, 1, "a stale press is still a press from while busy");
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
        let start = Instant::now();

        session.handle(Input::Press { at: start });
        session.handle(Input::Release {
            at: start + Duration::from_millis(2_000),
        });

        session.handle(Input::Cancel);
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Hint);

        // The provider answers anyway; it must go nowhere.
        session.handle(transcribed("Never mind."));

        assert!(fake.take().injected.is_empty());
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
    fn an_empty_transcript_is_not_injected() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);
        let start = Instant::now();

        session.handle(Input::Press { at: start });
        session.handle(Input::Release {
            at: start + Duration::from_millis(2_000),
        });
        session.handle(transcribed("   "));

        assert!(fake.take().injected.is_empty());
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Hint);
    }

    #[test]
    fn a_provider_failure_is_shown_with_its_reason_and_injects_nothing() {
        let fake = Shared::with_capture(2_000, false);
        let mut session = session(&fake);
        let start = Instant::now();

        session.handle(Input::Press { at: start });
        session.handle(Input::Release {
            at: start + Duration::from_millis(2_000),
        });
        session.handle(Input::Transcript {
            result: Err("Groq rejected the key".to_owned()),
        });

        assert!(fake.take().injected.is_empty());
        assert_eq!(session.view().message.as_deref(), Some("Groq rejected the key"));
        assert_eq!(fake.take().pills.last().expect("a pill").0, Phase::Failed);
        assert_eq!(
            fake.take().pills.last().expect("a pill").1,
            Some(FAILED_VISIBLE)
        );
    }

    #[test]
    fn a_microphone_that_will_not_open_is_a_failure_not_a_silent_no_op() {
        let fake = Shared::new();
        fake.take().start_error = Some("no microphone is available".to_owned());
        let mut session = session(&fake);

        let effects = session.handle(Input::Press { at: Instant::now() });

        assert!(matches!(effects[0], Effect::Failed));
        assert_eq!(session.view().phase, Phase::Failed);
        assert_eq!(
            session.view().message.as_deref(),
            Some("no microphone is available")
        );
        assert_eq!(
            fake.take().pills.last().expect("a pill").1,
            Some(FAILED_VISIBLE)
        );
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
