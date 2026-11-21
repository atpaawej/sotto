//! Speech to text, behind one seam.
//!
//! [`Transcriber`] takes a WAV and an API key and gives back text or a typed
//! failure. [`Groq`] is the only adapter in this build; the seam exists so a
//! second provider costs an adapter rather than a rewrite, and so every test
//! can run against a local server instead of the network.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use reqwest::blocking::multipart::{Form, Part};

use crate::audio::MarkedReader;

/// What the user asked us to send.
pub struct Request<'a> {
    pub wav: &'a [u8],
    pub api_key: &'a str,
    /// ISO-639-1 hint; providers auto-detect when this is `None`.
    pub language: Option<&'a str>,
    /// Set to the moment the request body is fully written, so the caller can
    /// separate upload time from provider time.
    pub uploaded: &'a Arc<Mutex<Option<Instant>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    #[error("no API key is stored for this provider; add one in Settings")]
    MissingKey,
    #[error("the provider rejected the API key")]
    Rejected,
    #[error("the provider is rate limiting this key; try again in a moment")]
    RateLimited,
    #[error("the provider failed with status {status}: {message}")]
    Unavailable { status: u16, message: String },
    #[error("the provider could not be reached: {source}")]
    Network {
        #[source]
        source: reqwest::Error,
    },
    #[error("the provider's reply was not readable: {detail}")]
    Malformed { detail: String },
    #[error("the request could not be built: {detail}")]
    Request { detail: String },
}

pub trait Transcriber {
    fn transcribe(&self, request: &Request<'_>) -> Result<Transcript, TranscribeError>;
}

/// Groq's OpenAI-compatible transcription endpoint.
#[derive(Debug, Clone)]
pub struct Groq {
    endpoint: String,
    model: String,
}

impl Groq {
    pub const DEFAULT_ENDPOINT: &'static str =
        "https://api.groq.com/openai/v1/audio/transcriptions";
    pub const DEFAULT_MODEL: &'static str = "whisper-large-v3-turbo";

    pub fn new() -> Self {
        Self {
            endpoint: Self::DEFAULT_ENDPOINT.to_owned(),
            model: Self::DEFAULT_MODEL.to_owned(),
        }
    }

    /// Points the adapter somewhere else. Tests use this; nothing else should.
    #[cfg(test)]
    pub fn at(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model: Self::DEFAULT_MODEL.to_owned(),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn form(&self, request: &Request<'_>) -> Result<Form, TranscribeError> {
        let part = Part::reader_with_length(
            MarkedReader::new(request.wav.to_vec(), Arc::clone(request.uploaded)),
            request.wav.len() as u64,
        )
        .file_name("speech.wav")
        .mime_str("audio/wav")
        .map_err(|error| TranscribeError::Request {
            detail: error.to_string(),
        })?;

        let mut form = Form::new()
            .text("model", self.model.clone())
            .text("response_format", "json")
            .text("temperature", "0")
            .part("file", part);

        if let Some(language) = request.language {
            form = form.text("language", language.to_owned());
        }

        Ok(form)
    }
}

impl Default for Groq {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(serde::Deserialize)]
struct GroqReply {
    text: String,
}

impl Transcriber for Groq {
    fn transcribe(&self, request: &Request<'_>) -> Result<Transcript, TranscribeError> {
        if request.api_key.trim().is_empty() {
            return Err(TranscribeError::MissingKey);
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| TranscribeError::Request {
                detail: error.to_string(),
            })?;

        let response = client
            .post(&self.endpoint)
            .bearer_auth(request.api_key)
            .multipart(self.form(request)?)
            .send()
            .map_err(|source| TranscribeError::Network { source })?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .map_err(|source| TranscribeError::Network { source })?;

        match status {
            200..=299 => {
                let reply: GroqReply =
                    serde_json::from_str(&body).map_err(|error| TranscribeError::Malformed {
                        detail: format!("{error}; body was {}", summarize(&body)),
                    })?;
                Ok(Transcript {
                    text: reply.text.trim().to_owned(),
                })
            }
            401 | 403 => Err(TranscribeError::Rejected),
            429 => Err(TranscribeError::RateLimited),
            other => Err(TranscribeError::Unavailable {
                status: other,
                message: summarize(&body),
            }),
        }
    }
}

fn summarize(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 300 {
        let cut: String = collapsed.chars().take(300).collect();
        format!("{cut}…")
    } else {
        collapsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct Captured {
        path: String,
        authorization: Option<String>,
        content_type: Option<String>,
        body: Vec<u8>,
    }

    /// Serves exactly one request, then answers with `status` and `body`.
    fn mock(
        status: u16,
        reply: &'static str,
    ) -> (String, mpsc::Receiver<Captured>, std::thread::JoinHandle<()>) {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock server");
        let port = server
            .server_addr()
            .to_ip()
            .expect("mock server listens on ip")
            .port();
        let (tx, rx) = mpsc::channel();

        let handle = std::thread::spawn(move || {
            let Ok(mut request) = server.recv() else {
                return;
            };

            let mut body = Vec::new();
            let _ = request.as_reader().read_to_end(&mut body);

            let headers: Vec<(String, String)> = request
                .headers()
                .iter()
                .map(|h| (h.field.as_str().as_str().to_lowercase(), h.value.as_str().to_owned()))
                .collect();
            let header = |name: &str| {
                headers
                    .iter()
                    .find(|(field, _)| field == name)
                    .map(|(_, value)| value.clone())
            };

            let _ = tx.send(Captured {
                path: request.url().to_owned(),
                authorization: header("authorization"),
                content_type: header("content-type"),
                body,
            });

            let response = tiny_http::Response::from_string(reply).with_status_code(status);
            let _ = request.respond(response);
        });

        (
            format!("http://127.0.0.1:{port}/openai/v1/audio/transcriptions"),
            rx,
            handle,
        )
    }

    fn ask(endpoint: &str, key: &str) -> Result<Transcript, TranscribeError> {
        let wav = b"RIFFmock".to_vec();
        let uploaded = Arc::new(Mutex::new(None));
        Groq::at(endpoint).transcribe(&Request {
            wav: &wav,
            api_key: key,
            language: Some("en"),
            uploaded: &uploaded,
        })
    }

    #[test]
    fn the_request_looks_like_groq_expects_and_the_text_comes_back() {
        let (endpoint, captured, handle) = mock(200, r#"{"text":"  hello there  "}"#);

        let transcript = ask(&endpoint, "gsk-test").expect("transcribe");
        let request = captured.recv().expect("the adapter made a request");
        handle.join().expect("mock thread");

        assert_eq!(transcript.text, "hello there");
        assert_eq!(request.path, "/openai/v1/audio/transcriptions");
        assert_eq!(request.authorization.as_deref(), Some("Bearer gsk-test"));
        assert!(request
            .content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("multipart/form-data")));

        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("name=\"model\""));
        assert!(body.contains(Groq::DEFAULT_MODEL));
        assert!(body.contains("name=\"file\"; filename=\"speech.wav\""));
        assert!(body.contains("name=\"language\""));
        assert!(body.contains("RIFFmock"), "the audio bytes are the part's body");
    }

    #[test]
    fn a_rejected_key_is_its_own_failure() {
        let (endpoint, _captured, _handle) = mock(401, r#"{"error":"bad key"}"#);
        assert!(matches!(
            ask(&endpoint, "gsk-wrong"),
            Err(TranscribeError::Rejected)
        ));
    }

    #[test]
    fn rate_limiting_is_its_own_failure() {
        let (endpoint, _captured, _handle) = mock(429, "slow down");
        assert!(matches!(
            ask(&endpoint, "gsk-test"),
            Err(TranscribeError::RateLimited)
        ));
    }

    #[test]
    fn a_server_error_keeps_the_status_and_the_body() {
        let (endpoint, _captured, _handle) = mock(503, "upstream unavailable");
        match ask(&endpoint, "gsk-test") {
            Err(TranscribeError::Unavailable { status, message }) => {
                assert_eq!(status, 503);
                assert!(message.contains("upstream unavailable"));
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn an_unreachable_provider_is_a_network_failure() {
        let result = ask("http://127.0.0.1:1/openai/v1/audio/transcriptions", "gsk-test");
        assert!(matches!(result, Err(TranscribeError::Network { .. })));
    }

    #[test]
    fn a_reply_that_is_not_json_is_malformed_not_empty_text() {
        let (endpoint, _captured, _handle) = mock(200, "<html>gateway</html>");
        assert!(matches!(
            ask(&endpoint, "gsk-test"),
            Err(TranscribeError::Malformed { .. })
        ));
    }

    #[test]
    fn a_missing_key_is_refused_before_any_request() {
        let uploaded = Arc::new(Mutex::new(None));
        let wav = b"RIFFmock".to_vec();
        let result = Groq::at("http://127.0.0.1:1/never").transcribe(&Request {
            wav: &wav,
            api_key: "   ",
            language: None,
            uploaded: &uploaded,
        });
        assert!(matches!(result, Err(TranscribeError::MissingKey)));
    }
}
