//! Every Dictation, its Recording, and the failures that happened along the way.
//!
//! [`DictationStore`] is the seam: a small interface over rows, with two
//! adapters — [`MemoryStore`] and [`SqliteStore`] — that are held to the same
//! conformance tests so they cannot drift. [`History`] adds the one thing a
//! row cannot hold by itself: the WAV on disk, written atomically and removed
//! with its row.
//!
//! A dictation is recorded before it is transcribed, so a network failure
//! leaves a Recording that Retry can send again. The user never has to repeat
//! the sentence.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::{
    config::{describe, ProviderId},
    dictation::{Attempt, Captured},
    failure::{Failure, FailureKind},
    transcribe::model_for,
};

/// How the finished text reached the focused app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Injection {
    /// There was no text to insert, or the user cancelled.
    NotAttempted,
    Inserted,
    Blocked,
}

impl Injection {
    pub fn as_str(self) -> &'static str {
        match self {
            Injection::NotAttempted => "not_attempted",
            Injection::Inserted => "inserted",
            Injection::Blocked => "blocked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "not_attempted" => Some(Injection::NotAttempted),
            "inserted" => Some(Injection::Inserted),
            "blocked" => Some(Injection::Blocked),
            _ => None,
        }
    }
}

/// One Dictation as History shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// Unix milliseconds. The UI formats it in the reader's locale.
    pub created_at_ms: u64,
    /// When the attempt finished, or `None` while it is still running.
    pub finished_at_ms: Option<u64>,
    pub duration_ms: u64,
    pub provider: String,
    pub model: String,
    pub transcript: String,
    pub cleaned: String,
    pub injection: Injection,
    /// The reason this dictation failed, if it did. A cleanup warning also
    /// lives here: the text landed, but the user should know what happened.
    pub failure: Option<Failure>,
    pub recording_path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the history database at {} could not be opened: {source}", path.display())]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },
    #[error("the history database could not be read: {source}")]
    Read {
        #[source]
        source: rusqlite::Error,
    },
    #[error("the history database could not be written: {source}")]
    Write {
        #[source]
        source: rusqlite::Error,
    },
    #[error("the history database was left locked by a panicking thread")]
    Locked,
    #[error("dictation {id} is already in history")]
    Duplicate { id: String },
    #[error("dictation {id} is not in history")]
    Missing { id: String },
    #[error("the history directory at {} could not be created: {source}", path.display())]
    Directory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the permissions on {} could not be restricted: {source}", path.display())]
    Permissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the recording at {} could not be written: {source}", path.display())]
    RecordingWrite {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the recording at {} could not be read: {source}", path.display())]
    RecordingRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the recording at {} could not be removed: {source}", path.display())]
    RecordingRemove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the system clock is before 1970: {source}")]
    Clock {
        #[source]
        source: std::time::SystemTimeError,
    },
    #[error("history is unavailable: {detail}")]
    Unavailable { detail: String },
}

/// The seam History is built on. Adapters only have to answer these questions.
pub trait DictationStore: Send + Sync {
    fn insert(&self, entry: &Entry) -> Result<(), StoreError>;
    fn update(&self, entry: &Entry) -> Result<(), StoreError>;
    fn get(&self, id: &str) -> Result<Option<Entry>, StoreError>;
    /// Newest first. `query` matches the transcript, cleaned text, provider or
    /// model, case-insensitively; an empty query lists everything.
    fn list(&self, query: &str) -> Result<Vec<Entry>, StoreError>;
    /// Returns the row it removed, so the caller can remove the Recording too.
    fn delete(&self, id: &str) -> Result<Option<Entry>, StoreError>;
    fn delete_all(&self) -> Result<Vec<Entry>, StoreError>;
    /// Entries older than `cutoff_ms`, oldest first.
    fn before(&self, cutoff_ms: u64) -> Result<Vec<Entry>, StoreError>;
}

/// A store for tests. The conformance tests run it beside SQLite so the two
/// adapters cannot drift.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: Mutex<Vec<Entry>>,
}

#[cfg(test)]
impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Vec<Entry>>, StoreError> {
        self.entries.lock().map_err(|_| StoreError::Locked)
    }
}

#[cfg(test)]
impl DictationStore for MemoryStore {
    fn insert(&self, entry: &Entry) -> Result<(), StoreError> {
        let mut entries = self.lock()?;
        if entries.iter().any(|existing| existing.id == entry.id) {
            return Err(StoreError::Duplicate {
                id: entry.id.clone(),
            });
        }
        entries.push(entry.clone());
        Ok(())
    }

    fn update(&self, entry: &Entry) -> Result<(), StoreError> {
        let mut entries = self.lock()?;
        let slot = entries
            .iter_mut()
            .find(|existing| existing.id == entry.id)
            .ok_or_else(|| StoreError::Missing {
                id: entry.id.clone(),
            })?;
        *slot = entry.clone();
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<Entry>, StoreError> {
        Ok(self.lock()?.iter().find(|entry| entry.id == id).cloned())
    }

    fn list(&self, query: &str) -> Result<Vec<Entry>, StoreError> {
        let needle = query.trim().to_lowercase();
        let mut entries: Vec<Entry> = self
            .lock()?
            .iter()
            .filter(|entry| {
                needle.is_empty()
                    || entry.transcript.to_lowercase().contains(&needle)
                    || entry.cleaned.to_lowercase().contains(&needle)
                    || entry.provider.to_lowercase().contains(&needle)
                    || entry.model.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        entries.sort_by(|left, right| {
            right
                .created_at_ms
                .cmp(&left.created_at_ms)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(entries)
    }

    fn delete(&self, id: &str) -> Result<Option<Entry>, StoreError> {
        let mut entries = self.lock()?;
        let index = entries.iter().position(|entry| entry.id == id);
        Ok(index.map(|index| entries.remove(index)))
    }

    fn delete_all(&self) -> Result<Vec<Entry>, StoreError> {
        Ok(std::mem::take(&mut *self.lock()?))
    }

    fn before(&self, cutoff_ms: u64) -> Result<Vec<Entry>, StoreError> {
        let mut entries: Vec<Entry> = self
            .lock()?
            .iter()
            .filter(|entry| entry.created_at_ms < cutoff_ms)
            .cloned()
            .collect();
        entries.sort_by_key(|entry| entry.created_at_ms);
        Ok(entries)
    }
}

const COLUMNS: &str = "id, created_at_ms, finished_at_ms, duration_ms, provider, model, transcript, cleaned, injection, failure_kind, failure_detail, failure_cause, recording_path";

/// The durable adapter. One connection behind a mutex is enough for a desktop
/// app that writes once per dictation.
pub struct SqliteStore {
    connection: Mutex<rusqlite::Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Directory {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let connection = rusqlite::Connection::open(path).map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        // The database holds every transcript; it is as private as the config.
        restrict(path, 0o600)?;
        Self::prepare(connection, path)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let connection =
            rusqlite::Connection::open_in_memory().map_err(|source| StoreError::Open {
                path: PathBuf::from(":memory:"),
                source,
            })?;
        Self::prepare(connection, Path::new(":memory:"))
    }

    fn prepare(connection: rusqlite::Connection, path: &Path) -> Result<Self, StoreError> {
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|source| StoreError::Open {
                path: path.to_path_buf(),
                source,
            })?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 CREATE TABLE IF NOT EXISTS dictations (
                     id TEXT PRIMARY KEY,
                     created_at_ms INTEGER NOT NULL,
                     finished_at_ms INTEGER,
                     duration_ms INTEGER NOT NULL,
                     provider TEXT NOT NULL,
                     model TEXT NOT NULL,
                     transcript TEXT NOT NULL,
                     cleaned TEXT NOT NULL,
                     injection TEXT NOT NULL,
                     failure_kind TEXT,
                     failure_detail TEXT,
                     failure_cause TEXT,
                     recording_path TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS dictations_created_at
                     ON dictations (created_at_ms DESC);",
            )
            .map_err(|source| StoreError::Open {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, rusqlite::Connection>, StoreError> {
        self.connection.lock().map_err(|_| StoreError::Locked)
    }
}

impl DictationStore for SqliteStore {
    fn insert(&self, entry: &Entry) -> Result<(), StoreError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO dictations (id, created_at_ms, finished_at_ms, duration_ms, provider, model, transcript, cleaned, injection, failure_kind, failure_detail, failure_cause, recording_path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                rusqlite::params![
                    entry.id,
                    entry.created_at_ms as i64,
                    entry.finished_at_ms.map(|value| value as i64),
                    entry.duration_ms as i64,
                    entry.provider,
                    entry.model,
                    entry.transcript,
                    entry.cleaned,
                    entry.injection.as_str(),
                    entry.failure.as_ref().map(|failure| failure.kind.as_str()),
                    entry.failure.as_ref().map(|failure| failure.detail.as_str()),
                    entry.failure.as_ref().and_then(|failure| failure.cause.as_deref()),
                    entry.recording_path,
                ],
            )
            .map(|_| ())
            .map_err(|source| match source {
                rusqlite::Error::SqliteFailure(error, _)
                    if error.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    StoreError::Duplicate {
                        id: entry.id.clone(),
                    }
                }
                other => StoreError::Write { source: other },
            })
    }

    fn update(&self, entry: &Entry) -> Result<(), StoreError> {
        let connection = self.lock()?;
        let changed = connection
            .execute(
                "UPDATE dictations
                 SET created_at_ms = ?2, finished_at_ms = ?3, duration_ms = ?4,
                     provider = ?5, model = ?6, transcript = ?7, cleaned = ?8,
                     injection = ?9, failure_kind = ?10, failure_detail = ?11,
                     failure_cause = ?12, recording_path = ?13
                 WHERE id = ?1",
                rusqlite::params![
                    entry.id,
                    entry.created_at_ms as i64,
                    entry.finished_at_ms.map(|value| value as i64),
                    entry.duration_ms as i64,
                    entry.provider,
                    entry.model,
                    entry.transcript,
                    entry.cleaned,
                    entry.injection.as_str(),
                    entry.failure.as_ref().map(|failure| failure.kind.as_str()),
                    entry
                        .failure
                        .as_ref()
                        .map(|failure| failure.detail.as_str()),
                    entry
                        .failure
                        .as_ref()
                        .and_then(|failure| failure.cause.as_deref()),
                    entry.recording_path,
                ],
            )
            .map_err(|source| StoreError::Write { source })?;
        if changed == 0 {
            return Err(StoreError::Missing {
                id: entry.id.clone(),
            });
        }
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<Entry>, StoreError> {
        let connection = self.lock()?;
        connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM dictations WHERE id = ?1"),
                rusqlite::params![id],
                row_to_entry,
            )
            .optional()
            .map_err(|source| StoreError::Read { source })
    }

    fn list(&self, query: &str) -> Result<Vec<Entry>, StoreError> {
        let connection = self.lock()?;
        let needle = query.trim();
        let sql = if needle.is_empty() {
            format!("SELECT {COLUMNS} FROM dictations ORDER BY created_at_ms DESC, id DESC")
        } else {
            format!(
                "SELECT {COLUMNS} FROM dictations
                 WHERE instr(lower(transcript), lower(?1)) > 0
                    OR instr(lower(cleaned), lower(?1)) > 0
                    OR instr(lower(provider), lower(?1)) > 0
                    OR instr(lower(model), lower(?1)) > 0
                 ORDER BY created_at_ms DESC, id DESC"
            )
        };
        let mut statement = connection
            .prepare(&sql)
            .map_err(|source| StoreError::Read { source })?;
        let rows = if needle.is_empty() {
            statement.query_map([], row_to_entry)
        } else {
            statement.query_map(rusqlite::params![needle], row_to_entry)
        }
        .map_err(|source| StoreError::Read { source })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| StoreError::Read { source })
    }

    fn delete(&self, id: &str) -> Result<Option<Entry>, StoreError> {
        let entry = self.get(id)?;
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM dictations WHERE id = ?1",
                rusqlite::params![id],
            )
            .map_err(|source| StoreError::Write { source })?;
        Ok(entry)
    }

    fn delete_all(&self) -> Result<Vec<Entry>, StoreError> {
        let entries = self.list("")?;
        let connection = self.lock()?;
        connection
            .execute("DELETE FROM dictations", [])
            .map_err(|source| StoreError::Write { source })?;
        Ok(entries)
    }

    fn before(&self, cutoff_ms: u64) -> Result<Vec<Entry>, StoreError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {COLUMNS} FROM dictations WHERE created_at_ms < ?1 ORDER BY created_at_ms ASC"
            ))
            .map_err(|source| StoreError::Read { source })?;
        let rows = statement
            .query_map(rusqlite::params![cutoff_ms as i64], row_to_entry)
            .map_err(|source| StoreError::Read { source })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|source| StoreError::Read { source })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct CorruptRow(String);

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let injection: String = row.get(8)?;
    let injection = Injection::parse(&injection).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(CorruptRow(format!(
                "unknown injection outcome `{injection}`"
            ))),
        )
    })?;

    let failure_kind: Option<String> = row.get(9)?;
    let failure = match failure_kind {
        None => None,
        Some(kind) => {
            let kind = FailureKind::parse(&kind).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    Box::new(CorruptRow(format!("unknown failure kind `{kind}`"))),
                )
            })?;
            let detail: String = row.get(10)?;
            let cause: Option<String> = row.get(11)?;
            Some(Failure {
                kind,
                detail,
                cause,
            })
        }
    };

    Ok(Entry {
        id: row.get(0)?,
        created_at_ms: row.get::<_, i64>(1)? as u64,
        finished_at_ms: row.get::<_, Option<i64>>(2)?.map(|value| value as u64),
        duration_ms: row.get::<_, i64>(3)? as u64,
        provider: row.get(4)?,
        model: row.get(5)?,
        transcript: row.get(6)?,
        cleaned: row.get(7)?,
        injection,
        failure,
        recording_path: row.get(12)?,
    })
}

/// The WAVs that sit beside the rows. Files are named by the dictation id, so
/// deleting a row has one obvious file to remove.
#[derive(Debug, Clone)]
pub struct RecordingStore {
    directory: PathBuf,
}

impl RecordingStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    pub fn path_for(&self, id: &str) -> PathBuf {
        self.directory.join(format!("{id}.wav"))
    }

    pub fn write(&self, id: &str, wav: &[u8]) -> Result<PathBuf, StoreError> {
        fs::create_dir_all(&self.directory).map_err(|source| StoreError::Directory {
            path: self.directory.clone(),
            source,
        })?;
        restrict(&self.directory, 0o700)?;

        let path = self.path_for(id);
        let temporary = path.with_extension("wav.tmp");
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file =
                options
                    .open(&temporary)
                    .map_err(|source| StoreError::RecordingWrite {
                        path: temporary.clone(),
                        source,
                    })?;
            file.write_all(wav)
                .and_then(|()| file.sync_all())
                .map_err(|source| StoreError::RecordingWrite {
                    path: temporary.clone(),
                    source,
                })?;
            fs::rename(&temporary, &path).map_err(|source| StoreError::RecordingWrite {
                path: path.clone(),
                source,
            })?;
            Ok(path)
        })();

        if result.is_err() {
            // best-effort: the failure is already on its way to the caller; a
            // half-written temp file is not worth a second failure.
            if let Err(error) = fs::remove_file(&temporary) {
                if error.kind() != io::ErrorKind::NotFound {
                    eprintln!(
                        "history: could not remove the half-written recording {}: {error}",
                        temporary.display()
                    );
                }
            }
        }

        result
    }

    pub fn read(&self, path: &Path) -> Result<Vec<u8>, StoreError> {
        fs::read(path).map_err(|source| StoreError::RecordingRead {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn remove(&self, path: &Path) -> Result<(), StoreError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StoreError::RecordingRemove {
                path: path.to_path_buf(),
                source,
            }),
        }
    }
}

/// What the session needs after a recording is saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRecording {
    pub id: String,
}

/// What Retry needs: the Recording exactly as it was captured, the row that
/// receives the result, and the provider to transcribe it with.
#[derive(Debug, Clone)]
pub struct RetrySource {
    pub captured: Captured,
    /// Different from the source row when retrying with the other provider:
    /// the first result stays in History beside the new one.
    pub id: String,
    /// `None` means the active provider; `Some` forces a specific one.
    pub provider: Option<ProviderId>,
}

/// The session's view of History. Small on purpose: the state machine does not
/// know about SQLite, WAV files or retention.
pub trait HistoryPort {
    fn record(&mut self, captured: &Captured) -> Result<StoredRecording, Failure>;
    fn complete(
        &mut self,
        id: &str,
        attempt: &Attempt,
        injection: Injection,
        failure: Option<&Failure>,
    ) -> Result<(), Failure>;
    fn fail(&mut self, id: &str, failure: &Failure) -> Result<(), Failure>;
    fn retry_source(&mut self, id: &str) -> Result<RetrySource, Failure>;
    /// Re-transcribes the Recording with the other provider. The original
    /// result is kept; the retry is a new row that shares the Recording.
    fn retry_other(&mut self, id: &str) -> Result<RetrySource, Failure>;
}

/// The durable History: rows in [`SqliteStore`], WAVs in [`RecordingStore`].
pub struct History {
    store: Arc<dyn DictationStore>,
    recordings: RecordingStore,
    /// Set when the database could not be opened. The app still runs; every
    /// History operation returns this reason instead of pretending otherwise.
    unavailable: Option<String>,
}

impl History {
    /// Opens `$XDG_DATA_HOME/sotto/history.sqlite3` and its recordings folder.
    /// A failure is kept as a readable reason rather than preventing startup.
    pub fn open_default() -> Self {
        match Self::try_open_default() {
            Ok(history) => history,
            Err(error) => {
                let detail = describe(&error);
                eprintln!("history: {detail}");
                Self::unavailable(detail)
            }
        }
    }

    fn try_open_default() -> Result<Self, StoreError> {
        let base = dirs::data_dir().ok_or_else(|| StoreError::Unavailable {
            detail: "there is no data directory to keep history in".to_owned(),
        })?;
        let root = base.join("sotto");
        let store = SqliteStore::open(&root.join("history.sqlite3"))?;
        Ok(Self {
            store: Arc::new(store),
            recordings: RecordingStore::new(root.join("recordings")),
            unavailable: None,
        })
    }

    fn unavailable(detail: String) -> Self {
        Self {
            store: Arc::new(UnavailableStore {
                detail: detail.clone(),
            }),
            recordings: RecordingStore::new(std::env::temp_dir().join("sotto-history-unavailable")),
            unavailable: Some(detail),
        }
    }

    /// Used by tests, which pair a memory store with a real recordings folder.
    #[cfg(test)]
    pub fn with_store(store: Arc<dyn DictationStore>, recordings: RecordingStore) -> Self {
        Self {
            store,
            recordings,
            unavailable: None,
        }
    }

    pub fn record(&self, captured: &Captured) -> Result<StoredRecording, Failure> {
        if let Some(detail) = &self.unavailable {
            return Err(Failure::new(FailureKind::StorageFailed, detail.clone()));
        }

        let id = uuid::Uuid::new_v4().to_string();
        let created_at_ms = now_ms().map_err(|error| Failure::storage(&error))?;
        let path = self
            .recordings
            .write(&id, &captured.wav)
            .map_err(|error| Failure::storage(&error))?;

        let entry = Entry {
            id: id.clone(),
            created_at_ms,
            finished_at_ms: None,
            duration_ms: captured.duration_ms,
            provider: String::new(),
            model: String::new(),
            transcript: String::new(),
            cleaned: String::new(),
            injection: Injection::NotAttempted,
            failure: None,
            recording_path: path.display().to_string(),
        };

        if let Err(error) = self.store.insert(&entry) {
            let cleanup = self.recordings.remove(&path);
            let detail = match cleanup {
                Ok(()) => error.to_string(),
                Err(cleanup_error) => {
                    format!("{error}; the recording could not be removed either: {cleanup_error}")
                }
            };
            return Err(Failure::with_cause(
                FailureKind::StorageFailed,
                detail,
                describe(&error),
            ));
        }

        Ok(StoredRecording { id })
    }

    pub fn complete(
        &self,
        id: &str,
        attempt: &Attempt,
        injection: Injection,
        failure: Option<&Failure>,
    ) -> Result<(), Failure> {
        let mut entry = self
            .store
            .get(id)
            .map_err(|error| Failure::storage(&error))?
            .ok_or_else(|| {
                Failure::new(
                    FailureKind::StorageFailed,
                    format!("dictation {id} was deleted before it finished"),
                )
            })?;

        entry.provider = attempt.provider.clone();
        entry.model = attempt.model.clone();
        entry.transcript = attempt.transcript.clone();
        entry.cleaned = attempt.cleaned.clone();
        entry.injection = injection;
        entry.finished_at_ms = Some(now_ms().map_err(|error| Failure::storage(&error))?);
        entry.failure = failure.cloned().or_else(|| attempt.cleanup_failure.clone());

        self.store
            .update(&entry)
            .map_err(|error| Failure::storage(&error))
    }

    pub fn fail(&self, id: &str, failure: &Failure) -> Result<(), Failure> {
        let mut entry = self
            .store
            .get(id)
            .map_err(|error| Failure::storage(&error))?
            .ok_or_else(|| {
                Failure::new(
                    FailureKind::StorageFailed,
                    format!("dictation {id} was deleted before it finished"),
                )
            })?;

        entry.failure = Some(failure.clone());
        entry.injection = Injection::NotAttempted;
        entry.finished_at_ms = Some(now_ms().map_err(|error| Failure::storage(&error))?);
        self.store
            .update(&entry)
            .map_err(|error| Failure::storage(&error))
    }

    fn captured_for(&self, entry: &Entry) -> Result<Captured, Failure> {
        if entry.recording_path.is_empty() {
            return Err(Failure::new(
                FailureKind::StorageFailed,
                "that dictation has no recording to retry".to_owned(),
            ));
        }

        let wav = self
            .recordings
            .read(Path::new(&entry.recording_path))
            .map_err(|error| Failure::storage(&error))?;

        Ok(Captured {
            wav,
            duration_ms: entry.duration_ms,
            stop_ms: 0,
            encode_ms: 0,
            silent: false,
        })
    }

    pub fn retry_source(&self, id: &str) -> Result<RetrySource, Failure> {
        let entry = self.entry(id)?;
        Ok(RetrySource {
            captured: self.captured_for(&entry)?,
            id: entry.id,
            provider: None,
        })
    }

    /// Sends a stored Recording to the other provider, keeping the first
    /// result: the retry is a new row that shares the same Recording. Both
    /// results stay in History, each named with the provider that produced it.
    pub fn retry_other(&self, id: &str) -> Result<RetrySource, Failure> {
        let entry = self.entry(id)?;
        let Some(provider) = ProviderId::parse(&entry.provider) else {
            return Err(Failure::new(
                FailureKind::StorageFailed,
                format!("dictation {id} has no known provider to retry with"),
            ));
        };
        let other = provider.other();
        let captured = self.captured_for(&entry)?;

        let retried = Entry {
            id: uuid::Uuid::new_v4().to_string(),
            created_at_ms: now_ms().map_err(|error| Failure::storage(&error))?,
            finished_at_ms: None,
            duration_ms: entry.duration_ms,
            provider: other.as_str().to_owned(),
            model: model_for(other).to_owned(),
            transcript: String::new(),
            cleaned: String::new(),
            injection: Injection::NotAttempted,
            failure: None,
            recording_path: entry.recording_path.clone(),
        };
        self.store
            .insert(&retried)
            .map_err(|error| Failure::storage(&error))?;

        Ok(RetrySource {
            captured,
            id: retried.id,
            provider: Some(other),
        })
    }

    pub fn entry(&self, id: &str) -> Result<Entry, Failure> {
        self.store
            .get(id)
            .map_err(|error| Failure::storage(&error))?
            .ok_or_else(|| {
                Failure::new(
                    FailureKind::StorageFailed,
                    "that dictation is no longer in history".to_owned(),
                )
            })
    }

    /// The WAV for one entry, for the Play button in History.
    pub fn recording(&self, id: &str) -> Result<Vec<u8>, Failure> {
        let entry = self.entry(id)?;
        if entry.recording_path.is_empty() {
            return Err(Failure::new(
                FailureKind::StorageFailed,
                "that dictation has no recording to play".to_owned(),
            ));
        }
        self.recordings
            .read(Path::new(&entry.recording_path))
            .map_err(|error| Failure::storage(&error))
    }

    pub fn list(&self, query: &str) -> Result<Vec<Entry>, Failure> {
        self.store
            .list(query)
            .map_err(|error| Failure::storage(&error))
    }

    pub fn delete(&self, id: &str) -> Result<(), Failure> {
        let entry = self
            .store
            .get(id)
            .map_err(|error| Failure::storage(&error))?;
        if let Some(entry) = entry {
            self.remove_recording(&entry)?;
            self.store
                .delete(id)
                .map_err(|error| Failure::storage(&error))?;
        }
        Ok(())
    }

    pub fn delete_all(&self) -> Result<(), Failure> {
        let entries = self
            .store
            .list("")
            .map_err(|error| Failure::storage(&error))?;
        self.store
            .delete_all()
            .map(|_| ())
            .map_err(|error| Failure::storage(&error))?;

        // Two results can share one Recording. Remove each distinct file once,
        // after no row can still point at it.
        let mut removed = std::collections::HashSet::new();
        for entry in &entries {
            if entry.recording_path.is_empty() || !removed.insert(entry.recording_path.as_str()) {
                continue;
            }
            self.recordings
                .remove(Path::new(&entry.recording_path))
                .map_err(|error| Failure::storage(&error))?;
        }
        Ok(())
    }

    /// Deletes rows and recordings older than `days`. `None` means keep
    /// everything. Returns how many dictations were removed.
    pub fn enforce_retention(&self, days: Option<u32>) -> Result<usize, Failure> {
        let Some(days) = days else {
            return Ok(0);
        };
        let now = now_ms().map_err(|error| Failure::storage(&error))?;
        let cutoff = now.saturating_sub(u64::from(days) * 86_400_000);

        let expired = self
            .store
            .before(cutoff)
            .map_err(|error| Failure::storage(&error))?;
        for entry in &expired {
            self.remove_recording(entry)?;
            self.store
                .delete(&entry.id)
                .map_err(|error| Failure::storage(&error))?;
        }
        Ok(expired.len())
    }

    fn remove_recording(&self, entry: &Entry) -> Result<(), Failure> {
        if entry.recording_path.is_empty() {
            return Ok(());
        }
        // A retry with the other provider shares this Recording. Keep the file
        // until the last entry that points at it is gone.
        let still_used = self
            .store
            .list("")
            .map_err(|error| Failure::storage(&error))?
            .iter()
            .any(|other| other.id != entry.id && other.recording_path == entry.recording_path);
        if still_used {
            return Ok(());
        }
        self.recordings
            .remove(Path::new(&entry.recording_path))
            .map_err(|error| Failure::storage(&error))
    }
}

impl HistoryPort for Arc<History> {
    fn record(&mut self, captured: &Captured) -> Result<StoredRecording, Failure> {
        (**self).record(captured)
    }

    fn complete(
        &mut self,
        id: &str,
        attempt: &Attempt,
        injection: Injection,
        failure: Option<&Failure>,
    ) -> Result<(), Failure> {
        (**self).complete(id, attempt, injection, failure)
    }

    fn fail(&mut self, id: &str, failure: &Failure) -> Result<(), Failure> {
        (**self).fail(id, failure)
    }

    fn retry_source(&mut self, id: &str) -> Result<RetrySource, Failure> {
        (**self).retry_source(id)
    }

    fn retry_other(&mut self, id: &str) -> Result<RetrySource, Failure> {
        (**self).retry_other(id)
    }
}

struct UnavailableStore {
    detail: String,
}

impl DictationStore for UnavailableStore {
    fn insert(&self, _entry: &Entry) -> Result<(), StoreError> {
        Err(self.error())
    }
    fn update(&self, _entry: &Entry) -> Result<(), StoreError> {
        Err(self.error())
    }
    fn get(&self, _id: &str) -> Result<Option<Entry>, StoreError> {
        Err(self.error())
    }
    fn list(&self, _query: &str) -> Result<Vec<Entry>, StoreError> {
        Err(self.error())
    }
    fn delete(&self, _id: &str) -> Result<Option<Entry>, StoreError> {
        Err(self.error())
    }
    fn delete_all(&self) -> Result<Vec<Entry>, StoreError> {
        Err(self.error())
    }
    fn before(&self, _cutoff_ms: u64) -> Result<Vec<Entry>, StoreError> {
        Err(self.error())
    }
}

impl UnavailableStore {
    fn error(&self) -> StoreError {
        StoreError::Unavailable {
            detail: self.detail.clone(),
        }
    }
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
        StoreError::Permissions {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> Result<(), StoreError> {
    Ok(())
}

fn now_ms() -> Result<u64, StoreError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .map_err(|source| StoreError::Clock { source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::Timings;

    fn entry(id: &str, created_at_ms: u64, transcript: &str) -> Entry {
        Entry {
            id: id.to_owned(),
            created_at_ms,
            finished_at_ms: Some(created_at_ms + 500),
            duration_ms: 2_000,
            provider: "groq".to_owned(),
            model: "whisper-large-v3-turbo".to_owned(),
            transcript: transcript.to_owned(),
            cleaned: transcript.to_owned(),
            injection: Injection::Inserted,
            failure: None,
            recording_path: format!("/tmp/{id}.wav"),
        }
    }

    /// One contract, run against every adapter. A change to one store that the
    /// other does not match fails here.
    fn conformance(store: &dyn DictationStore) {
        store.delete_all().expect("a clean store");

        let mut older = entry("older", 1_000, "Hello from the older dictation");
        older.provider = "groq".to_owned();
        let mut newer = entry("newer", 2_000, "A newer sentence about Wayland");
        newer.failure = Some(Failure::with_cause(
            FailureKind::Network,
            "the provider could not be reached",
            "connection refused",
        ));
        newer.injection = Injection::Blocked;

        store.insert(&older).expect("insert older");
        store.insert(&newer).expect("insert newer");

        assert!(
            matches!(store.insert(&older), Err(StoreError::Duplicate { .. })),
            "an id is written once"
        );
        assert_eq!(store.list("").expect("list after duplicate").len(), 2);

        assert_eq!(store.get("newer").expect("get"), Some(newer.clone()));
        assert_eq!(store.get("missing").expect("get missing"), None);

        let all = store.list("").expect("list");
        assert_eq!(
            all.iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["newer", "older"],
            "newest first"
        );

        let matched = store.list("wayland").expect("search");
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "newer");

        assert!(
            store.list("%").expect("literal search").is_empty(),
            "search is literal text, not a LIKE pattern"
        );

        let mut updated = newer.clone();
        updated.cleaned = "A cleaner newer sentence".to_owned();
        updated.injection = Injection::Inserted;
        updated.failure = None;
        store.update(&updated).expect("update");
        assert_eq!(store.get("newer").expect("get updated"), Some(updated));

        assert!(matches!(
            store.update(&entry("missing", 3_000, "nothing")),
            Err(StoreError::Missing { .. })
        ));

        let expired = store.before(1_500).expect("before");
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, "older");

        let removed = store.delete("newer").expect("delete").expect("a row");
        assert_eq!(removed.id, "newer");
        assert_eq!(store.delete("newer").expect("delete again"), None);

        let remaining = store.delete_all().expect("delete all");
        assert_eq!(remaining.len(), 1);
        assert!(store.list("").expect("empty list").is_empty());
    }

    #[test]
    fn the_memory_store_meets_the_history_contract() {
        conformance(&MemoryStore::new());
    }

    #[test]
    fn the_sqlite_store_meets_the_history_contract() {
        conformance(&SqliteStore::open_in_memory().expect("open in-memory sqlite"));
    }

    #[cfg(unix)]
    #[test]
    fn the_history_database_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("history.sqlite3");
        let store = SqliteStore::open(&path).expect("open");
        drop(store);

        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "history holds every transcript");
    }

    #[test]
    fn a_recording_is_written_privately_and_read_back() {
        let dir = tempfile::tempdir().expect("temp dir");
        let recordings = RecordingStore::new(dir.path().join("recordings"));

        let path = recordings.write("dictation", b"RIFFfake").expect("write");
        assert_eq!(recordings.read(&path).expect("read"), b"RIFFfake");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);

            let directory_mode = fs::metadata(dir.path().join("recordings"))
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(directory_mode, 0o700);
        }

        recordings.remove(&path).expect("remove");
        assert!(!path.exists());
        recordings
            .remove(&path)
            .expect("removing a missing file is fine");
    }

    fn history() -> (tempfile::TempDir, History) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = Arc::new(MemoryStore::new());
        let recordings = RecordingStore::new(dir.path().join("recordings"));
        (dir, History::with_store(store, recordings))
    }

    fn captured() -> Captured {
        Captured {
            wav: b"RIFFfake".to_vec(),
            duration_ms: 2_000,
            stop_ms: 1,
            encode_ms: 2,
            silent: false,
        }
    }

    fn attempt(transcript: &str) -> Attempt {
        Attempt {
            transcript: transcript.to_owned(),
            cleaned: transcript.to_owned(),
            cleanup_failure: None,
            cleanup_model: None,
            provider: "groq".to_owned(),
            model: "whisper-large-v3-turbo".to_owned(),
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

    #[test]
    fn recording_then_completing_keeps_the_wav_and_the_words() {
        let (_dir, history) = history();
        let stored = history.record(&captured()).expect("record");
        assert!(Path::new(&history.list("").expect("list")[0].recording_path).exists());

        history
            .complete(
                &stored.id,
                &attempt("Hello there."),
                Injection::Inserted,
                None,
            )
            .expect("complete");

        let entries = history.list("").expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].transcript, "Hello there.");
        assert_eq!(entries[0].injection, Injection::Inserted);
        assert!(entries[0].failure.is_none());
    }

    #[test]
    fn a_failed_dictation_keeps_its_recording_for_retry() {
        let (_dir, history) = history();
        let stored = history.record(&captured()).expect("record");
        let failure = Failure::new(FailureKind::Network, "offline");

        history.fail(&stored.id, &failure).expect("fail");
        let entries = history.list("").expect("list");
        assert_eq!(
            entries[0].failure.as_ref().map(|f| f.kind),
            Some(FailureKind::Network)
        );

        let source = history.retry_source(&stored.id).expect("retry source");
        assert_eq!(source.captured.wav, b"RIFFfake");
        assert_eq!(source.captured.duration_ms, 2_000);
    }

    #[test]
    fn deleting_one_entry_removes_its_recording() {
        let (_dir, history) = history();
        let stored = history.record(&captured()).expect("record");
        let path = history.list("").expect("list")[0].recording_path.clone();
        assert!(Path::new(&path).exists());

        history.delete(&stored.id).expect("delete");
        assert!(history.list("").expect("list").is_empty());
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn deleting_everything_removes_every_recording() {
        let (_dir, history) = history();
        let first = history.record(&captured()).expect("record");
        let second = history.record(&captured()).expect("record");
        assert_ne!(first.id, second.id);
        let paths: Vec<String> = history
            .list("")
            .expect("list")
            .into_iter()
            .map(|entry| entry.recording_path)
            .collect();

        history.delete_all().expect("delete all");
        assert!(history.list("").expect("list").is_empty());
        for path in paths {
            assert!(!Path::new(&path).exists());
        }
    }

    #[test]
    fn retention_removes_old_rows_and_their_recordings() {
        let (_dir, history) = history();
        let old = history.record(&captured()).expect("record");
        let old_path = history.list("").expect("list")[0].recording_path.clone();
        let mut row = history.list("").expect("list")[0].clone();
        row.created_at_ms = 1;
        history.store.update(&row).expect("age the row");

        history.enforce_retention(Some(30)).expect("retention");
        assert!(history.list("").expect("list").is_empty());
        assert!(!Path::new(&old_path).exists());
        assert!(history.retry_source(&old.id).is_err());
    }

    #[test]
    fn a_cleanup_warning_is_stored_while_the_inserted_text_is_kept() {
        let (_dir, history) = history();
        let stored = history.record(&captured()).expect("record");
        let mut attempt = attempt("um raw text");
        attempt.cleanup_failure = Some(Failure::new(FailureKind::RateLimited, "slow down"));

        history
            .complete(&stored.id, &attempt, Injection::Inserted, None)
            .expect("complete");

        let entry = &history.list("").expect("list")[0];
        assert_eq!(entry.injection, Injection::Inserted);
        assert_eq!(
            entry.failure.as_ref().map(|failure| failure.kind),
            Some(FailureKind::RateLimited)
        );
    }

    #[test]
    fn an_unavailable_history_says_so_instead_of_pretending() {
        let history = History::unavailable("the disk is read-only".to_owned());
        let error = history.record(&captured()).expect_err("no store");
        assert_eq!(error.kind, FailureKind::StorageFailed);
        assert!(error.detail.contains("read-only"));
        assert!(history.list("").is_err());
    }

    /// One recording, two providers: the first result stays beside the retry.
    fn two_results() -> (tempfile::TempDir, History, String, String) {
        let (dir, history) = history();
        let stored = history.record(&captured()).expect("record");
        history
            .complete(
                &stored.id,
                &attempt("Groq heard this."),
                Injection::Inserted,
                None,
            )
            .expect("complete first result");

        let source = history
            .retry_other(&stored.id)
            .expect("retry with the other provider");
        let mut retried = attempt("ElevenLabs heard that.");
        retried.provider = "eleven_labs".to_owned();
        retried.model = "scribe_v3".to_owned();
        history
            .complete(&source.id, &retried, Injection::Inserted, None)
            .expect("complete retry");

        (dir, history, stored.id, source.id)
    }

    #[test]
    fn retrying_with_the_other_provider_keeps_both_named_results() {
        let (_dir, history, original, retried) = two_results();
        let entries = history.list("").expect("list");

        assert_eq!(entries.len(), 2);
        let first = entries.iter().find(|entry| entry.id == original).expect("original");
        let second = entries.iter().find(|entry| entry.id == retried).expect("retry");
        assert_eq!(first.provider, "groq");
        assert_eq!(first.cleaned, "Groq heard this.");
        assert_eq!(second.provider, "eleven_labs");
        assert_eq!(second.cleaned, "ElevenLabs heard that.");
        assert_eq!(
            first.recording_path, second.recording_path,
            "both results read the same Recording"
        );
        assert!(Path::new(&first.recording_path).exists());
    }

    #[test]
    fn an_other_provider_retry_can_be_retried_back() {
        let (_dir, history, _original, retried) = two_results();

        let source = history.retry_other(&retried).expect("retry back");

        assert_eq!(source.provider, Some(ProviderId::Groq));
        let entries = history.list("").expect("list");
        assert_eq!(entries.len(), 3);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.id == source.id)
                .expect("third result")
                .provider,
            "groq"
        );
    }

    #[test]
    fn deleting_one_result_keeps_the_recording_for_the_other() {
        let (_dir, history, original, retried) = two_results();
        let path = history.entry(&original).expect("entry").recording_path.clone();

        history.delete(&original).expect("delete first result");

        assert!(
            Path::new(&path).exists(),
            "the other result still owns the Recording"
        );
        assert_eq!(
            history.retry_source(&retried).expect("retry source").captured.wav,
            b"RIFFfake"
        );

        history.delete(&retried).expect("delete second result");
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn delete_all_removes_a_shared_recording_once() {
        let (_dir, history, original, _retried) = two_results();
        let path = history.entry(&original).expect("entry").recording_path.clone();

        history.delete_all().expect("delete all");

        assert!(history.list("").expect("list").is_empty());
        assert!(!Path::new(&path).exists());
    }

    #[test]
    fn retrying_an_entry_without_a_known_provider_is_a_failure() {
        let (_dir, history) = history();
        let stored = history.record(&captured()).expect("record");
        let mut row = history.entry(&stored.id).expect("entry");
        row.provider = String::new();
        history.store.update(&row).expect("write the row back");

        let error = history.retry_other(&stored.id).expect_err("no provider");

        assert!(error.detail.contains("no known provider"));
    }
}
