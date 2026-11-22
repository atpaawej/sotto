//! One vocabulary for every failure the user can see.
//!
//! A failure is not a string. It is a [`FailureKind`], a short reason derived
//! from that kind in exactly one place, a detail sentence for Settings, and the
//! cause chain for the log. The Pill, Settings and History all read the same
//! struct, so a new provider error cannot invent its own wording.
//!
//! ADR-0001: every error is either handled deliberately or propagated. This
//! module is the "propagated" half: it turns typed errors from the microphone,
//! provider, cleanup call, keyboard bridge and store into something a person
//! can act on.

use std::fmt;

use serde::ser::{Serialize, SerializeStruct, Serializer};

use crate::{
    audio::AudioError, cleanup::CleanupError, config::describe, dictation::DictationError,
    transcribe::TranscribeError,
};

/// The eight user-facing failures, plus one for the local store.
///
/// Storage is included because "the recording could not be saved" is a real
/// failure with a real recovery (free disk space, fix permissions), and
/// mislabelling it as a provider problem would send the user to the wrong
/// place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// No key, or the provider refused the key it was given.
    KeyRejected,
    /// The provider is rate limiting this key.
    RateLimited,
    /// The provider answered, but not with usable text.
    ProviderError,
    /// The provider could not be reached.
    Network,
    /// The provider took too long to answer.
    Timeout,
    /// The microphone could not be opened or stopped.
    MicDenied,
    /// The global shortcut could not be bound.
    ShortcutRefused,
    /// The finished text could not be put into the focused app.
    InjectionBlocked,
    /// The recording or its history row could not be written.
    StorageFailed,
}

impl FailureKind {
    pub const ALL: [FailureKind; 9] = [
        FailureKind::KeyRejected,
        FailureKind::RateLimited,
        FailureKind::ProviderError,
        FailureKind::Network,
        FailureKind::Timeout,
        FailureKind::MicDenied,
        FailureKind::ShortcutRefused,
        FailureKind::InjectionBlocked,
        FailureKind::StorageFailed,
    ];

    /// The one place a failure becomes the short sentence the Pill shows.
    ///
    /// It says what the user can do, not what the code did. The full sentence
    /// and the cause chain live in [`Failure::detail`] and [`Failure::cause`].
    pub fn reason(self) -> &'static str {
        match self {
            FailureKind::KeyRejected => "Check your API key",
            FailureKind::RateLimited => "Rate limited. Try again shortly.",
            FailureKind::ProviderError => "The provider failed",
            FailureKind::Network => "No connection to the provider",
            FailureKind::Timeout => "The provider timed out",
            FailureKind::MicDenied => "Microphone unavailable",
            FailureKind::ShortcutRefused => "Shortcut unavailable",
            FailureKind::InjectionBlocked => "Text could not be inserted",
            FailureKind::StorageFailed => "History is unavailable",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            FailureKind::KeyRejected => "key_rejected",
            FailureKind::RateLimited => "rate_limited",
            FailureKind::ProviderError => "provider_error",
            FailureKind::Network => "network",
            FailureKind::Timeout => "timeout",
            FailureKind::MicDenied => "mic_denied",
            FailureKind::ShortcutRefused => "shortcut_refused",
            FailureKind::InjectionBlocked => "injection_blocked",
            FailureKind::StorageFailed => "storage_failed",
        }
    }

    /// Reads the value written by [`FailureKind::as_str`].
    ///
    /// An unknown value is a corrupt row, not a reason to guess at a category.
    pub fn parse(value: &str) -> Option<Self> {
        FailureKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == value)
    }
}

/// A failure as every surface sees it.
///
/// The short reason is derived on serialization from [`FailureKind::reason`],
/// so there is no second copy of that vocabulary to drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    /// The full sentence for Settings: what happened, in the provider's words.
    pub detail: String,
    /// The cause chain for the log, when the detail alone does not carry it.
    pub cause: Option<String>,
}

impl Failure {
    pub fn new(kind: FailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            cause: None,
        }
    }

    pub fn with_cause(
        kind: FailureKind,
        detail: impl Into<String>,
        cause: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            detail: detail.into(),
            cause: Some(cause.into()),
        }
    }

    /// A failure from the local store, with the store's cause chain attached.
    pub fn storage(error: &dyn std::error::Error) -> Self {
        let detail = error.to_string();
        let cause = describe(error);
        Self::with_cause(FailureKind::StorageFailed, detail, cause)
    }

    /// The cause chain, or the detail when there is no separate chain.
    pub fn cause_chain(&self) -> &str {
        self.cause.as_deref().unwrap_or(&self.detail)
    }

    /// Writes the failure to the log once, with everything needed to diagnose
    /// it. Surfaces call this before they show the short reason.
    pub fn log(&self) {
        eprintln!(
            "failure[{}]: {} | {}",
            self.kind.as_str(),
            self.detail,
            self.cause_chain()
        );
    }

    /// The short reason the Pill shows.
    pub fn reason(&self) -> &'static str {
        self.kind.reason()
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl Serialize for Failure {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Failure", 4)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("reason", self.reason())?;
        state.serialize_field("detail", &self.detail)?;
        state.serialize_field("cause", &self.cause)?;
        state.end()
    }
}

/// Maps a provider failure, including a timeout hidden inside reqwest's error.
pub fn from_transcribe(error: TranscribeError) -> Failure {
    let kind = match &error {
        TranscribeError::MissingKey | TranscribeError::Rejected => FailureKind::KeyRejected,
        TranscribeError::RateLimited => FailureKind::RateLimited,
        TranscribeError::Unavailable { .. }
        | TranscribeError::Malformed { .. }
        | TranscribeError::Request { .. } => FailureKind::ProviderError,
        TranscribeError::Network { source } if source.is_timeout() => FailureKind::Timeout,
        TranscribeError::Network { .. } => FailureKind::Network,
    };
    let detail = match &error {
        TranscribeError::MissingKey => {
            "No API key is stored for this provider. Add one in Settings.".to_owned()
        }
        other => other.to_string(),
    };
    Failure::with_cause(kind, detail, describe(&error))
}

/// Maps a microphone failure. Every variant here means the user cannot be
/// heard, so they share one reason and keep the specific cause in the detail.
pub fn from_audio(error: AudioError) -> Failure {
    let detail = error.to_string();
    Failure::with_cause(FailureKind::MicDenied, detail, describe(&error))
}

/// Maps the composite dictation error to the vocabulary.
pub fn from_dictation(error: DictationError) -> Failure {
    match error {
        DictationError::Audio(inner) => from_audio(inner),
        DictationError::Transcribe(inner) => from_transcribe(inner),
        other => {
            let detail = other.to_string();
            Failure::with_cause(FailureKind::ProviderError, detail, describe(&other))
        }
    }
}

/// Maps the second LLM call. A cleanup failure is non-fatal: the raw
/// Transcript is inserted, and this failure is shown as the warning.
pub fn from_cleanup(error: CleanupError) -> Failure {
    let kind = match &error {
        CleanupError::MissingKey | CleanupError::Rejected => FailureKind::KeyRejected,
        CleanupError::RateLimited => FailureKind::RateLimited,
        CleanupError::Unavailable { .. }
        | CleanupError::Malformed { .. }
        | CleanupError::EmptyReply
        | CleanupError::Request { .. } => FailureKind::ProviderError,
        CleanupError::Network { source } if source.is_timeout() => FailureKind::Timeout,
        CleanupError::Network { .. } => FailureKind::Network,
    };
    let detail = match &error {
        CleanupError::MissingKey => {
            "No API key is stored for cleanup. Add one in Settings.".to_owned()
        }
        other => other.to_string(),
    };
    Failure::with_cause(kind, detail, describe(&error))
}

/// The settings file could not be read, so there is no key to dictate with.
/// The fix is in Settings, which is why it shares the key reason.
pub fn settings(problem: impl Into<String>) -> Failure {
    Failure::new(
        FailureKind::KeyRejected,
        format!(
            "Your settings could not be read, so no API key is available: {}",
            problem.into()
        ),
    )
}

/// A failure from the keyboard bridge, which reports plain sentences.
pub fn injection(reason: impl Into<String>) -> Failure {
    let detail = reason.into();
    Failure::new(FailureKind::InjectionBlocked, detail)
}

/// A failure from the global shortcut listener.
pub fn shortcut(problem: impl Into<String>) -> Failure {
    let detail = problem.into();
    Failure::new(FailureKind::ShortcutRefused, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_kind_has_a_short_reason_and_no_two_share_one() {
        let mut seen = HashSet::new();
        for kind in FailureKind::ALL {
            let reason = kind.reason();
            assert!(!reason.is_empty(), "{kind:?} has no pill reason");
            assert!(
                reason.len() <= 48,
                "{kind:?} reason is too long for the pill"
            );
            assert!(seen.insert(reason), "{kind:?} reuses another kind's reason");
        }
    }

    #[test]
    fn every_kind_round_trips_through_its_stored_name() {
        for kind in FailureKind::ALL {
            assert_eq!(FailureKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(FailureKind::parse("not_a_kind"), None);
    }

    #[test]
    fn the_serialized_failure_carries_the_reason_from_the_one_mapping() {
        let failure = Failure::new(FailureKind::RateLimited, "Groq is rate limiting this key");
        let json = serde_json::to_value(&failure).expect("serialize");

        assert_eq!(json["kind"], "rate_limited");
        assert_eq!(json["reason"], FailureKind::RateLimited.reason());
        assert_eq!(json["detail"], "Groq is rate limiting this key");
        assert!(json["cause"].is_null());
    }

    #[test]
    fn a_rejected_key_and_a_missing_key_are_the_same_user_problem() {
        let rejected = from_transcribe(TranscribeError::Rejected);
        let missing = from_transcribe(TranscribeError::MissingKey);

        assert_eq!(rejected.kind, FailureKind::KeyRejected);
        assert_eq!(missing.kind, FailureKind::KeyRejected);
        assert!(missing.detail.contains("Add one in Settings"));
    }

    #[test]
    fn rate_limiting_and_provider_errors_stay_distinct() {
        assert_eq!(
            from_transcribe(TranscribeError::RateLimited).kind,
            FailureKind::RateLimited
        );
        assert_eq!(
            from_transcribe(TranscribeError::Unavailable {
                status: 503,
                message: "upstream".to_owned(),
            })
            .kind,
            FailureKind::ProviderError
        );
        assert_eq!(
            from_transcribe(TranscribeError::Malformed {
                detail: "html".to_owned(),
            })
            .kind,
            FailureKind::ProviderError
        );
    }

    #[test]
    fn a_cleanup_failure_uses_the_same_vocabulary_as_a_transcription_failure() {
        assert_eq!(
            from_cleanup(CleanupError::RateLimited).kind,
            FailureKind::RateLimited
        );
        assert_eq!(
            from_cleanup(CleanupError::EmptyReply).kind,
            FailureKind::ProviderError
        );
    }

    #[test]
    fn an_injection_failure_is_its_own_kind_with_the_bridge_detail() {
        let failure = injection("the keyboard bridge is not running");
        assert_eq!(failure.kind, FailureKind::InjectionBlocked);
        assert_eq!(failure.reason(), "Text could not be inserted");
        assert_eq!(failure.detail, "the keyboard bridge is not running");
    }

    #[test]
    fn a_shortcut_refusal_is_its_own_kind() {
        let failure = shortcut("/dev/input/event* is not readable");
        assert_eq!(failure.kind, FailureKind::ShortcutRefused);
        assert_eq!(failure.reason(), "Shortcut unavailable");
    }

    #[test]
    fn a_store_failure_keeps_the_cause_chain() {
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let failure = Failure::storage(&error);

        assert_eq!(failure.kind, FailureKind::StorageFailed);
        assert!(failure.detail.contains("denied"));
        assert!(failure.cause_chain().contains("denied"));
    }
}
