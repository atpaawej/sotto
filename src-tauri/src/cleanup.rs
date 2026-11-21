//! Post-transcription LLM cleanup, behind one seam.
//!
//! [`Cleaner`] takes a raw Whisper transcript and the user's Cleanup settings
//! and returns cleaned text or a typed failure. [`GroqCleaner`] reuses a single
//! HTTP client so the second call avoids repeating a TLS handshake.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::CleanupLevel;

pub struct CleanRequest<'a> {
    pub transcript: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    pub level: CleanupLevel,
    pub fillers: bool,
    pub spoken_punctuation: bool,
    pub whitespace: bool,
    pub dictionary: &'a [String],
}

#[derive(Debug, thiserror::Error)]
pub enum CleanupError {
    #[error("no API key is stored for Groq cleanup; add one in Settings")]
    MissingKey,
    #[error("Groq rejected the API key during cleanup")]
    Rejected,
    #[error("Groq is rate limiting cleanup requests; try again in a moment")]
    RateLimited,
    #[error("Groq cleanup failed with status {status}: {message}")]
    Unavailable { status: u16, message: String },
    #[error("Groq cleanup could not be reached: {source}")]
    Network {
        #[source]
        source: reqwest::Error,
    },
    #[error("Groq cleanup reply was not readable: {detail}")]
    Malformed { detail: String },
    #[error("Groq cleanup returned an empty reply")]
    EmptyReply,
    #[error("the cleanup request could not be built: {detail}")]
    Request { detail: String },
}

pub trait Cleaner {
    fn clean(&self, request: &CleanRequest<'_>) -> Result<String, CleanupError>;
}

/// Groq's OpenAI-compatible chat completions endpoint for fast transcript cleanup.
#[derive(Debug, Clone)]
pub struct GroqCleaner {
    endpoint: String,
    client: reqwest::blocking::Client,
}

impl GroqCleaner {
    pub const DEFAULT_ENDPOINT: &'static str =
        "https://api.groq.com/openai/v1/chat/completions";

    pub fn new() -> Result<Self, CleanupError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| CleanupError::Request {
                detail: error.to_string(),
            })?;
        Ok(Self {
            endpoint: Self::DEFAULT_ENDPOINT.to_owned(),
            client,
        })
    }

    #[cfg(test)]
    pub fn at(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("test HTTP client builds"),
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [Message<'a>; 2],
    temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatReply {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ReplyMessage,
}

#[derive(Deserialize)]
struct ReplyMessage {
    content: Option<String>,
}

impl Cleaner for GroqCleaner {
    fn clean(&self, request: &CleanRequest<'_>) -> Result<String, CleanupError> {
        if request.api_key.trim().is_empty() {
            return Err(CleanupError::MissingKey);
        }

        let system_prompt = build_system_prompt(request);
        let reasoning_effort = if request.model.to_ascii_lowercase().contains("qwen3") {
            Some("none")
        } else {
            None
        };

        let payload = ChatRequest {
            model: request.model,
            messages: [
                Message {
                    role: "system",
                    content: &system_prompt,
                },
                Message {
                    role: "user",
                    content: request.transcript,
                },
            ],
            temperature: 0.0,
            reasoning_effort,
        };

        let body = serde_json::to_string(&payload).map_err(|error| CleanupError::Request {
            detail: error.to_string(),
        })?;

        let response = self
            .client
            .post(&self.endpoint)
            .bearer_auth(request.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .map_err(|source| CleanupError::Network { source })?;

        let status = response.status().as_u16();
        let body = response
            .text()
            .map_err(|source| CleanupError::Network { source })?;

        match status {
            200..=299 => {
                let reply: ChatReply =
                    serde_json::from_str(&body).map_err(|error| CleanupError::Malformed {
                        detail: format!("{error}; body was {}", summarize(&body)),
                    })?;
                let text = reply
                    .choices
                    .into_iter()
                    .next()
                    .and_then(|choice| choice.message.content)
                    .map(|content| content.trim().to_owned())
                    .unwrap_or_default();

                if text.is_empty() {
                    return Err(CleanupError::EmptyReply);
                }

                Ok(text)
            }
            401 | 403 => Err(CleanupError::Rejected),
            429 => Err(CleanupError::RateLimited),
            other => Err(CleanupError::Unavailable {
                status: other,
                message: summarize(&body),
            }),
        }
    }
}

pub fn build_system_prompt(request: &CleanRequest<'_>) -> String {
    let mut parts = vec![
        "You are a speech-to-text post-processor. Output ONLY the cleaned text with no preamble, explanation, or surrounding quotes.".to_owned(),
    ];

    match request.level {
        CleanupLevel::None => {}
        CleanupLevel::Light => parts.push(
            "Preserve the speaker's exact wording and meaning. Fix capitalization and punctuation. Resolve mid-sentence self-corrections by keeping only the final intended words.".to_owned(),
        ),
        CleanupLevel::Medium => parts.push(
            "Fix capitalization and punctuation, resolve mid-sentence self-corrections, and lightly polish phrasing for clarity and conciseness while preserving the speaker's meaning.".to_owned(),
        ),
    }

    if request.fillers {
        parts.push(
            "Remove filler words and hesitation noises (um, uh, erm, hmm, ah, mm) and accidental immediate word repeats.".to_owned(),
        );
    }

    if request.spoken_punctuation {
        parts.push(
            "Convert spoken punctuation commands (comma, full stop, period, question mark, exclamation mark, colon, semicolon, new line, new paragraph, dash) into their punctuation or formatting.".to_owned(),
        );
    }

    if request.whitespace {
        parts.push(
            "Normalize whitespace with no doubled spaces and no space before punctuation.".to_owned(),
        );
    }

    let terms: Vec<&str> = request
        .dictionary
        .iter()
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
        .collect();
    if !terms.is_empty() {
        parts.push(format!(
            "Apply these exact spellings/terms whenever they appear or are phonetically misheard: {}.",
            terms.join(", ")
        ));
    }

    parts.join(" ")
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
        authorization: Option<String>,
        body: serde_json::Value,
    }

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

            let mut raw = Vec::new();
            let _ = request.as_reader().read_to_end(&mut raw);
            let body: serde_json::Value =
                serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null);
            let authorization = request
                .headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("authorization"))
                .map(|h| h.value.as_str().to_owned());

            let _ = tx.send(Captured { authorization, body });

            let response = tiny_http::Response::from_string(reply).with_status_code(status);
            let _ = request.respond(response);
        });

        (
            format!("http://127.0.0.1:{port}/openai/v1/chat/completions"),
            rx,
            handle,
        )
    }

    fn sample_request<'a>(dictionary: &'a [String]) -> CleanRequest<'a> {
        CleanRequest {
            transcript: "um we are building soto",
            api_key: "gsk_test",
            model: "qwen/qwen3.8-27b",
            level: CleanupLevel::Light,
            fillers: true,
            spoken_punctuation: true,
            whitespace: true,
            dictionary,
        }
    }

    #[test]
    fn groq_cleaner_sends_qwen3_reasoning_none_and_dictionary_in_prompt() {
        let (endpoint, rx, handle) = mock(
            200,
            r#"{"choices":[{"message":{"content":"We are building Sotto."}}]}"#,
        );
        let cleaner = GroqCleaner::at(endpoint);
        let dict = vec!["Sotto".to_owned(), "Wayland".to_owned()];

        let cleaned = cleaner.clean(&sample_request(&dict)).expect("cleanup");
        handle.join().expect("server thread");

        assert_eq!(cleaned, "We are building Sotto.");
        let captured = rx.recv().expect("captured request");
        assert_eq!(captured.authorization.as_deref(), Some("Bearer gsk_test"));
        assert_eq!(captured.body["model"], "qwen/qwen3.8-27b");
        assert_eq!(captured.body["temperature"], 0.0);
        assert_eq!(captured.body["reasoning_effort"], "none");
        let system = captured.body["messages"][0]["content"]
            .as_str()
            .expect("system prompt");
        assert!(system.contains("Sotto, Wayland"));
    }

    #[test]
    fn non_qwen3_models_omit_reasoning_effort() {
        let (endpoint, rx, handle) = mock(
            200,
            r#"{"choices":[{"message":{"content":"We are building Sotto."}}]}"#,
        );
        let cleaner = GroqCleaner::at(endpoint);
        let dict = Vec::new();
        let mut req = sample_request(&dict);
        req.model = "llama-3.1-8b-instant";

        cleaner.clean(&req).expect("cleanup");
        handle.join().expect("server thread");

        let captured = rx.recv().expect("captured request");
        assert!(captured.body.get("reasoning_effort").is_none());
    }

    #[test]
    fn missing_key_fails_without_making_a_network_call() {
        let cleaner = GroqCleaner::at("http://127.0.0.1:1/never");
        let dict = Vec::new();
        let mut req = sample_request(&dict);
        req.api_key = "   ";

        let err = cleaner.clean(&req).expect_err("missing key");
        assert!(matches!(err, CleanupError::MissingKey));
    }

    #[test]
    fn empty_reply_from_provider_is_an_error() {
        let (endpoint, _rx, handle) = mock(200, r#"{"choices":[{"message":{"content":"   "}}]}"#);
        let cleaner = GroqCleaner::at(endpoint);
        let dict = Vec::new();

        let err = cleaner.clean(&sample_request(&dict)).expect_err("empty reply");
        handle.join().expect("server thread");
        assert!(matches!(err, CleanupError::EmptyReply));
    }

    #[test]
    fn rate_limit_and_rejected_map_to_typed_errors() {
        let (endpoint_429, _rx1, h1) = mock(429, r#"{"error":"slow down"}"#);
        let dict = Vec::new();
        let err_429 = GroqCleaner::at(endpoint_429)
            .clean(&sample_request(&dict))
            .expect_err("429");
        h1.join().expect("server thread");
        assert!(matches!(err_429, CleanupError::RateLimited));

        let (endpoint_401, _rx2, h2) = mock(401, r#"{"error":"bad key"}"#);
        let err_401 = GroqCleaner::at(endpoint_401)
            .clean(&sample_request(&dict))
            .expect_err("401");
        h2.join().expect("server thread");
        assert!(matches!(err_401, CleanupError::Rejected));
    }
}
