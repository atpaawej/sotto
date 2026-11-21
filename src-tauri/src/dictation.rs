//! One dictation, from microphone to text.
//!
//! [`Dictation`] owns the only recording in flight and turns a stop into an
//! [`Attempt`]: the transcript, the provider that produced it, and how long each
//! stage took. The stages are measured here rather than in the UI, because the
//! UI can only see the two ends.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::{
    audio::{self, AudioError, Capture, LevelCallback},
    config::{Config, ProviderId},
    transcribe::{Groq, Request, TranscribeError, Transcriber},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    /// Stopping the microphone and collecting what it captured.
    pub stop_ms: u64,
    /// Mixing down, resampling to 16 kHz and writing the WAV.
    pub encode_ms: u64,
    /// Sending the audio, up to the last byte of the request body.
    pub upload_ms: u64,
    /// The provider thinking: from the end of the upload to the reply.
    pub provider_ms: u64,
    /// Key release to text, measured end to end.
    pub total_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    pub transcript: String,
    pub provider: String,
    pub model: String,
    pub duration_ms: u64,
    pub timings: Timings,
}

/// What the microphone produced, encoded and ready to send.
#[derive(Debug, Clone)]
pub struct Captured {
    pub wav: Vec<u8>,
    pub duration_ms: u64,
    /// Stopping the microphone and collecting what it captured.
    pub stop_ms: u64,
    /// Mixing down, resampling and writing the WAV.
    pub encode_ms: u64,
    /// Nothing louder than the room; sending it would only cost time.
    pub silent: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DictationError {
    #[error("a dictation is already being recorded")]
    AlreadyRecording,
    #[error("no dictation is being recorded")]
    NotRecording,
    #[error(transparent)]
    Audio(#[from] AudioError),
    #[error(transparent)]
    Transcribe(#[from] TranscribeError),
    #[error("{0}")]
    Unsupported(&'static str),
    #[error("the dictation state was left locked by a panicking thread")]
    Locked,
}

#[derive(Default)]
pub struct Dictation {
    capture: Mutex<Option<Capture>>,
}

impl Dictation {
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the microphone after checking that the dictation could finish.
    ///
    /// Refusing before recording, rather than after, is the difference between
    /// "add a key" and "you just spoke for ten seconds and lost it".
    #[cfg(test)]
    pub fn start(&self, config: &Config) -> Result<(), DictationError> {
        self.start_with_levels(config, None)
    }

    pub fn start_with_levels(
        &self,
        config: &Config,
        on_level: Option<LevelCallback>,
    ) -> Result<(), DictationError> {
        match config.active_provider {
            ProviderId::Groq => {
                if config
                    .providers
                    .groq_key
                    .as_deref()
                    .is_none_or(|key| key.trim().is_empty())
                {
                    return Err(DictationError::Transcribe(TranscribeError::MissingKey));
                }
            }
            ProviderId::ElevenLabs => {
                return Err(DictationError::Unsupported(
                    "ElevenLabs is not wired up in this build; Sotto transcribes with Groq for now",
                ))
            }
        }

        let mut slot = self.capture.lock().map_err(|_| DictationError::Locked)?;
        if slot.is_some() {
            return Err(DictationError::AlreadyRecording);
        }
        *slot = Some(Capture::start_with_levels(on_level)?);
        Ok(())
    }

    /// Stops the microphone and encodes what it heard, without sending it.
    ///
    /// Splitting the stop from the upload is what lets the session decide not
    /// to spend a request on half a second of room tone.
    pub fn stop_recording(&self) -> Result<Captured, DictationError> {
        let capture = self
            .capture
            .lock()
            .map_err(|_| DictationError::Locked)?
            .take()
            .ok_or(DictationError::NotRecording)?;

        let total = Instant::now();

        let audio = capture.stop()?;
        let stop_ms = millis(total.elapsed());
        let duration_ms = millis(audio.duration());
        let silent = audio.is_silent();

        let encoding = Instant::now();
        let wav = audio::encode_wav(&audio)?;
        let encode_ms = millis(encoding.elapsed());

        Ok(Captured {
            wav,
            duration_ms,
            stop_ms,
            encode_ms,
            silent,
        })
    }

    /// Sends a finished recording to the configured provider.
    pub fn transcribe(config: &Config, captured: &Captured) -> Result<Attempt, DictationError> {
        let total = Instant::now();
        let Captured {
            wav,
            duration_ms,
            stop_ms,
            encode_ms,
            ..
        } = captured.clone();

        let (provider, transcriber, model): (_, Box<dyn Transcriber>, String) =
            match config.active_provider {
                ProviderId::Groq => {
                    let transcriber = Groq::new();
                    let model = transcriber.model().to_owned();
                    ("groq", Box::new(transcriber), model)
                }
                ProviderId::ElevenLabs => {
                    return Err(DictationError::Unsupported(
                        "ElevenLabs is not wired up in this build; Sotto transcribes with Groq for now",
                    ))
                }
            };

        let api_key = config
            .providers
            .groq_key
            .clone()
            .ok_or(TranscribeError::MissingKey)?;

        let prompt = if config.cleanup.dictionary {
            crate::transcribe::dictionary_prompt(&config.dictionary)
        } else {
            None
        };
        let uploaded = Arc::new(Mutex::new(None));
        let sent = Instant::now();
        let transcript = transcriber.transcribe(&Request {
            wav: &wav,
            api_key: &api_key,
            language: None,
            prompt: prompt.as_deref(),
            uploaded: &uploaded,
        })?;
        let round_trip = sent.elapsed();
        let marked = uploaded.lock().ok().and_then(|mark| *mark);
        let (upload_ms, provider_ms) = split_round_trip(round_trip, marked, sent);

        Ok(Attempt {
            transcript: transcript.text,
            provider: provider.to_owned(),
            model,
            duration_ms,
            timings: Timings {
                stop_ms,
                encode_ms,
                upload_ms,
                provider_ms,
                total_ms: stop_ms + encode_ms + millis(total.elapsed()),
            },
        })
    }

}

/// Splits a round trip into "sending" and "the provider thinking".
///
/// Without a mark from the body reader there is nothing to split on, so the
/// whole time is reported as provider time rather than invented.
fn split_round_trip(
    round_trip: Duration,
    uploaded: Option<Instant>,
    started_at: Instant,
) -> (u64, u64) {
    let Some(marked) = uploaded else {
        return (0, millis(round_trip));
    };
    let upload = marked.saturating_duration_since(started_at).min(round_trip);
    (millis(upload), millis(round_trip.saturating_sub(upload)))
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_round_trip_splits_into_upload_and_provider() {
        let start = Instant::now();
        let uploaded = start + Duration::from_millis(120);
        let round_trip = Duration::from_millis(500);

        let (upload_ms, provider_ms) = split_round_trip(round_trip, Some(uploaded), start);

        assert_eq!(upload_ms, 120);
        assert_eq!(provider_ms, 380);
    }

    #[test]
    fn an_unmarked_round_trip_is_not_split_by_guesswork() {
        let start = Instant::now();
        let (upload_ms, provider_ms) =
            split_round_trip(Duration::from_millis(300), None, start);

        assert_eq!(upload_ms, 0);
        assert_eq!(provider_ms, 300);
    }

    #[test]
    fn a_mark_after_the_reply_still_does_not_overshoot_the_total() {
        let start = Instant::now();
        let uploaded = start + Duration::from_millis(900);

        let (upload_ms, provider_ms) =
            split_round_trip(Duration::from_millis(400), Some(uploaded), start);

        assert_eq!(upload_ms, 400);
        assert_eq!(provider_ms, 0);
    }

    #[test]
    fn starting_without_a_key_refuses_before_opening_the_microphone() {
        let dictation = Dictation::new();
        let config = Config::default();

        let error = dictation.start(&config).expect_err("no key means no recording");

        assert!(matches!(
            error,
            DictationError::Transcribe(TranscribeError::MissingKey)
        ));
        assert!(matches!(
            dictation.stop_recording(),
            Err(DictationError::NotRecording)
        ));
    }

    #[test]
    fn elevenlabs_is_refused_with_a_reason_instead_of_failing_silently() {
        let dictation = Dictation::new();
        let config = Config {
            active_provider: ProviderId::ElevenLabs,
            ..Config::default()
        };

        let error = dictation.start(&config).expect_err("not implemented");

        assert!(matches!(error, DictationError::Unsupported(_)));
    }

    /// The acceptance measurement for ticket 04. Ignored so no test run ever
    /// opens a microphone or spends someone's provider quota by accident.
    ///
    /// `GROQ_API_KEY=… cargo test -- --ignored --nocapture ten_second`
    #[test]
    #[ignore = "records from the real microphone and calls Groq"]
    fn a_ten_second_dictation_timed_end_to_end() {
        let key = std::env::var("GROQ_API_KEY").expect("set GROQ_API_KEY to run this");
        let config = Config {
            providers: crate::config::Providers {
                groq_key: Some(key),
                ..Default::default()
            },
            ..Config::default()
        };

        let dictation = Dictation::new();
        let started = Instant::now();
        dictation.start(&config).expect("the microphone opens");
        std::thread::sleep(Duration::from_secs(10));
        let captured = dictation.stop_recording().expect("the recording stops");
        let attempt = Dictation::transcribe(&config, &captured).expect("the provider answers");

        println!("recorded {} ms of audio", attempt.duration_ms);
        println!("timings: {:?}", attempt.timings);
        println!("wall clock from start to text: {} ms", millis(started.elapsed()));
        println!("transcript: {:?}", attempt.transcript);
        if let Ok(json) = serde_json::to_string_pretty(&attempt.timings) {
            let _ = std::fs::write("/tmp/sotto-timing.json", json);
        }

        assert!(attempt.duration_ms >= 9_500, "ten seconds were recorded");
    }

    #[test]
    fn stopping_without_starting_is_an_error_not_an_empty_transcript() {
        let dictation = Dictation::new();
        let error = dictation
            .stop_recording()
            .expect_err("nothing to stop");
        assert!(matches!(error, DictationError::NotRecording));
    }
}
