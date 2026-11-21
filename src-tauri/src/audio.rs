//! Microphone capture, downmixed and resampled for a speech provider.
//!
//! Two pieces, both behind small interfaces: [`Capture`], which owns the device
//! and a thread, and [`encode_wav`], a pure function over samples. Everything
//! that can go wrong with a microphone is an [`AudioError`]; nothing here logs
//! and pretends it worked.

use std::{
    io::{Cursor, Read},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample as _,
};

/// Whisper-family providers want 16 kHz mono; sending more is wasted upload.
pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// A finished recording, already mixed down to one channel, at the device rate.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Audio {
    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate as f64)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no microphone is available: {0}")]
    NoDevice(String),
    #[error("the microphone could not be opened: {0}")]
    Open(String),
    #[error("the microphone stopped while recording: {0}")]
    Capture(String),
    #[error("nothing was recorded; check that the microphone is not muted")]
    Empty,
    #[error("the recording could not be encoded: {0}")]
    Encode(String),
    #[error("the recording thread ended without a result")]
    Stopped,
}

/// A running microphone capture.
///
/// The device and its stream live on their own thread, because a cpal stream is
/// not safe to move between threads on every platform. Dropping this handle
/// without calling [`Capture::stop`] ends the capture and discards it.
pub struct Capture {
    stop: Sender<()>,
    join: thread::JoinHandle<Result<Audio, AudioError>>,
}

impl Capture {
    /// Opens the default input device and starts recording.
    ///
    /// Returns only once the stream is actually running, so a caller that gets
    /// `Ok` knows sound is being captured, and a caller that gets `Err` has a
    /// reason to show.
    pub fn start() -> Result<Self, AudioError> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let join = thread::spawn(move || capture_until_stopped(&stop_rx, &ready_tx));

        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop: stop_tx,
                join,
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => Err(AudioError::Open(
                "the microphone did not start within five seconds".to_owned(),
            )),
        }
    }

    /// Stops the stream and returns everything captured.
    pub fn stop(self) -> Result<Audio, AudioError> {
        // The thread may already be gone if the device failed mid-recording;
        // in that case the join below carries the real reason.
        let _ = self.stop.send(());
        self.join.join().map_err(|_| AudioError::Stopped)?
    }
}

fn capture_until_stopped(
    stop: &Receiver<()>,
    ready: &Sender<Result<(), AudioError>>,
) -> Result<Audio, AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| AudioError::NoDevice("the sound server reports no input".to_owned()))?;

    let supported = device
        .default_input_config()
        .map_err(|error| AudioError::Open(error.to_string()))?;

    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();

    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let failure: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let stream = build_stream(&device, &config, channels, &samples, &failure, format)?;

    stream
        .play()
        .map_err(|error| AudioError::Open(format!("could not start the microphone: {error}")))?;

    if ready.send(Ok(())).is_err() {
        return Err(AudioError::Stopped);
    }

    // Block until the caller stops us. A disconnected channel means the handle
    // was dropped, which is also a stop.
    let _ = stop.recv();
    drop(stream);

    if let Some(message) = failure.lock().ok().and_then(|slot| slot.clone()) {
        return Err(AudioError::Capture(message));
    }

    let samples = Arc::try_unwrap(samples)
        .map_err(|_| AudioError::Capture("the recording buffer was still in use".to_owned()))?
        .into_inner()
        .map_err(|_| AudioError::Capture("the recording buffer was poisoned".to_owned()))?;

    if samples.is_empty() {
        return Err(AudioError::Empty);
    }

    Ok(Audio {
        samples,
        sample_rate,
    })
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    samples: &Arc<Mutex<Vec<f32>>>,
    failure: &Arc<Mutex<Option<String>>>,
    format: cpal::SampleFormat,
) -> Result<cpal::Stream, AudioError> {
    let sink = Arc::clone(samples);
    let errors = Arc::clone(failure);

    let on_error = move |error: cpal::StreamError| {
        if let Ok(mut slot) = errors.lock() {
            *slot = Some(error.to_string());
        }
    };

    let unopened = |error: cpal::BuildStreamError| AudioError::Open(error.to_string());

    match format {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _| mix_into(&sink, data, channels),
                on_error,
                None,
            )
            .map_err(unopened),
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _| mix_into(&sink, data, channels),
                on_error,
                None,
            )
            .map_err(unopened),
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                config,
                move |data: &[u16], _| mix_into(&sink, data, channels),
                on_error,
                None,
            )
            .map_err(unopened),
        other => Err(AudioError::Open(format!(
            "the microphone uses an unsupported sample format: {other:?}"
        ))),
    }
}

/// Averages interleaved frames to mono. The callback must never block or panic.
fn mix_into<T>(sink: &Arc<Mutex<Vec<f32>>>, data: &[T], channels: usize)
where
    T: cpal::Sample,
    f32: cpal::FromSample<T>,
{
    let Ok(mut samples) = sink.lock() else {
        return;
    };
    if channels == 0 {
        return;
    }
    for frame in data.chunks(channels) {
        let sum: f32 = frame.iter().map(|value| f32::from_sample(*value)).sum();
        samples.push(sum / channels as f32);
    }
}

/// Encodes mono samples as a 16 kHz, 16-bit PCM WAV, the form every speech
/// provider accepts and the smallest one that loses nothing a provider needs.
pub fn encode_wav(audio: &Audio) -> Result<Vec<u8>, AudioError> {
    let samples = resample(&audio.samples, audio.sample_rate, TARGET_SAMPLE_RATE);
    if samples.is_empty() {
        return Err(AudioError::Empty);
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut out = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut out, spec)
        .map_err(|error| AudioError::Encode(error.to_string()))?;

    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        writer
            .write_sample(value)
            .map_err(|error| AudioError::Encode(error.to_string()))?;
    }

    writer
        .finalize()
        .map_err(|error| AudioError::Encode(error.to_string()))?;

    Ok(out.into_inner())
}

/// Linear resampling to the target rate. Speech is band-limited enough that the
/// provider's own model does not care, and this keeps the dependency list short.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == 0 || to == 0 || from == to {
        return samples.to_vec();
    }

    let ratio = from as f64 / to as f64;
    let length = (samples.len() as f64 / ratio).floor() as usize;

    (0..length)
        .map(|index| {
            let position = index as f64 * ratio;
            let lower = position.floor() as usize;
            let upper = (lower + 1).min(samples.len() - 1);
            let t = (position - lower as f64) as f32;
            samples[lower] * (1.0 - t) + samples[upper] * t
        })
        .collect()
}

/// Reads a body while noting the moment it is exhausted, so a caller can tell
/// upload time from provider time without guessing.
pub struct MarkedReader {
    bytes: Cursor<Vec<u8>>,
    uploaded: Arc<Mutex<Option<Instant>>>,
}

impl MarkedReader {
    pub fn new(bytes: Vec<u8>, uploaded: Arc<Mutex<Option<Instant>>>) -> Self {
        Self {
            bytes: Cursor::new(bytes),
            uploaded,
        }
    }
}

impl Read for MarkedReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.bytes.read(buf)?;
        if read == 0 {
            if let Ok(mut mark) = self.uploaded.lock() {
                if mark.is_none() {
                    *mark = Some(Instant::now());
                }
            }
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_is_16khz_mono_16_bit() {
        let audio = Audio {
            samples: vec![0.0; 16_000],
            sample_rate: 16_000,
        };

        let wav = encode_wav(&audio).expect("encode");

        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        let spec = hound::WavReader::new(Cursor::new(&wav))
            .expect("parse")
            .spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        assert_eq!(spec.bits_per_sample, 16);
    }

    #[test]
    fn resampling_48k_to_16k_keeps_one_third_of_the_samples() {
        let samples = vec![0.5; 48_000];
        let resampled = resample(&samples, 48_000, 16_000);
        assert_eq!(resampled.len(), 16_000);
        assert!(resampled.iter().all(|value| (*value - 0.5).abs() < 1e-6));
    }

    #[test]
    fn resampling_to_the_same_rate_changes_nothing() {
        let samples = vec![0.1, -0.2, 0.3];
        assert_eq!(resample(&samples, 44_100, 44_100), samples);
    }

    #[test]
    fn silence_still_encodes_but_nothing_does_not() {
        let silent = Audio {
            samples: vec![0.0; 800],
            sample_rate: 16_000,
        };
        assert!(encode_wav(&silent).is_ok());

        let nothing = Audio {
            samples: Vec::new(),
            sample_rate: 16_000,
        };
        assert!(matches!(encode_wav(&nothing), Err(AudioError::Empty)));
    }

    #[test]
    fn the_marked_reader_records_the_moment_upload_finishes() {
        let mark = Arc::new(Mutex::new(None));
        let mut reader = MarkedReader::new(vec![1, 2, 3], Arc::clone(&mark));

        let mut out = Vec::new();
        reader.read_to_end(&mut out).expect("read");

        assert_eq!(out, vec![1, 2, 3]);
        assert!(mark.lock().expect("mark").is_some());
    }
}
