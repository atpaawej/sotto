//! The user's settings, and the only module that knows where they live.
//!
//! One JSON file, written atomically, with mode 0600 because it holds API keys.
//! A malformed file is an error the user is shown, never a silent reset:
//! deleting someone's keys and dictionary because one byte was wrong is not an
//! acceptable recovery.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const CONFIG_FILE_NAME: &str = "config.json";

/// Bumped when a file written by an older build needs a one-time rewrite.
/// Version 2 made Groq the default provider, because it is the one wired up.
const CURRENT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub trigger_key: String,
    pub active_provider: ProviderId,
    pub providers: Providers,
    pub cleanup: Cleanup,
    pub dictionary: Vec<String>,
    pub retention_days: Option<u32>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            trigger_key: "ControlRight".to_owned(),
            active_provider: ProviderId::default(),
            providers: Providers::default(),
            cleanup: Cleanup::default(),
            dictionary: Vec::new(),
            retention_days: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    ElevenLabs,
    #[default]
    Groq,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Providers {
    pub elevenlabs_key: Option<String>,
    pub groq_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cleanup {
    pub enabled: bool,
    pub fillers: bool,
    pub spoken_punctuation: bool,
    pub whitespace: bool,
    pub dictionary: bool,
}

impl Default for Cleanup {
    fn default() -> Self {
        Self {
            enabled: true,
            fillers: true,
            spoken_punctuation: true,
            whitespace: true,
            dictionary: true,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("no home directory to place the config file in")]
    NoHome,
    #[error("the config path has no parent directory: {}", path.display())]
    NoParent { path: PathBuf },
    #[error("no config file at {}", path.display())]
    Missing { path: PathBuf },
    #[error("could not read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{} is not valid config: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not encode the config: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("could not write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Joins an error with its causes, the form used in logs and in the window.
pub fn describe(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        message.push_str(": ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

/// Reads and writes one config file.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$XDG_CONFIG_HOME/sotto/config.json`, or `~/.config/sotto/config.json`.
    pub fn default_path() -> Result<PathBuf, ConfigError> {
        let base = dirs::config_dir().ok_or(ConfigError::NoHome)?;
        Ok(base.join("sotto").join(CONFIG_FILE_NAME))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Config, ConfigError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(ConfigError::Missing {
                    path: self.path.clone(),
                })
            }
            Err(source) => {
                return Err(ConfigError::Read {
                    path: self.path.clone(),
                    source,
                })
            }
        };

        serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
            path: self.path.clone(),
            source,
        })
    }

    /// Loads the config, or writes the defaults when there is no file yet.
    ///
    /// A file that exists but cannot be read or parsed is returned as an error
    /// and left alone: the user's keys are worth more than a clean start.
    pub fn load_or_init(&self) -> Result<Config, ConfigError> {
        match self.load() {
            Ok(config) => self.migrate(config),
            Err(ConfigError::Missing { .. }) => {
                let config = Config::default();
                self.save(&config)?;
                Ok(config)
            }
            Err(error) => Err(error),
        }
    }

    /// Brings an older file up to the current shape, once.
    ///
    /// Only fields whose meaning changed are touched. A file that already has a
    /// version at least this new is returned exactly as the user left it.
    fn migrate(&self, mut config: Config) -> Result<Config, ConfigError> {
        if config.version >= CURRENT_VERSION {
            return Ok(config);
        }

        let had_no_choice = config.active_provider == ProviderId::ElevenLabs
            && config.providers.elevenlabs_key.is_none();
        if had_no_choice {
            config.active_provider = ProviderId::Groq;
        }
        config.version = CURRENT_VERSION;

        eprintln!(
            "config: migrated {} to version {CURRENT_VERSION} (provider {})",
            self.path.display(),
            if had_no_choice { "now groq" } else { "kept" }
        );
        self.save(&config)?;
        Ok(config)
    }

    /// Writes the config atomically: a private temp file in the same directory,
    /// then a rename, so a crash never leaves a half-written file.
    pub fn save(&self, config: &Config) -> Result<(), ConfigError> {
        let parent = self.path.parent().ok_or_else(|| ConfigError::NoParent {
            path: self.path.clone(),
        })?;

        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;

        let mut body =
            serde_json::to_vec_pretty(config).map_err(|source| ConfigError::Encode { source })?;
        body.push(b'\n');

        let temporary = self.path.with_extension("json.tmp");
        self.write_private(&temporary, &body)?;
        fs::rename(&temporary, &self.path).map_err(|source| ConfigError::Write {
            path: self.path.clone(),
            source,
        })
    }

    fn write_private(&self, path: &Path, body: &[u8]) -> Result<(), ConfigError> {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        let mut file = options.open(path).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;

        file.write_all(body)
            .and_then(|()| file.sync_all())
            .map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let store = ConfigStore::at(dir.path().join("sotto").join(CONFIG_FILE_NAME));
        (dir, store)
    }

    #[test]
    fn a_saved_config_loads_back_unchanged() {
        let (_dir, store) = store();
        let mut config = Config::default();
        config.providers.elevenlabs_key = Some("sk-test".to_owned());
        config.dictionary = vec!["Sotto".to_owned(), "Wayland".to_owned()];
        config.cleanup.fillers = false;

        store.save(&config).expect("save");
        let loaded = store.load().expect("load");

        assert_eq!(loaded, config);
    }

    #[cfg(unix)]
    #[test]
    fn the_config_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, store) = store();
        store.save(&Config::default()).expect("save");

        let mode = fs::metadata(store.path())
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(mode, 0o600, "config holds API keys, so only the owner reads it");
    }

    #[test]
    fn a_malformed_file_is_an_error_and_is_not_overwritten() {
        let (_dir, store) = store();
        fs::create_dir_all(store.path().parent().expect("parent")).expect("mkdir");
        let broken = "{\n  \"version\": 1,\n  \"trigger_key\": \n";
        fs::write(store.path(), broken).expect("write broken config");

        let error = store.load_or_init().expect_err("malformed config must not be accepted");

        assert!(matches!(error, ConfigError::Parse { .. }), "got {error:?}");
        assert_eq!(
            fs::read_to_string(store.path()).expect("read back"),
            broken,
            "the user's file must survive a failed load"
        );
    }

    #[test]
    fn a_missing_file_is_created_with_defaults() {
        let (_dir, store) = store();
        assert!(matches!(store.load(), Err(ConfigError::Missing { .. })));

        let config = store.load_or_init().expect("init");

        assert_eq!(config, Config::default());
        assert!(store.path().exists(), "the defaults are written, not just returned");
        assert_eq!(store.load().expect("reload"), config);
    }

    #[test]
    fn a_version_one_file_without_an_elevenlabs_key_moves_to_groq() {
        let (_dir, store) = store();
        fs::create_dir_all(store.path().parent().expect("parent")).expect("mkdir");
        fs::write(
            store.path(),
            r#"{"version":1,"active_provider":"eleven_labs"}"#,
        )
        .expect("write v1 config");

        let config = store.load_or_init().expect("migrate");

        assert_eq!(config.active_provider, ProviderId::Groq);
        assert_eq!(config.version, CURRENT_VERSION);
        assert_eq!(
            store.load().expect("reload").active_provider,
            ProviderId::Groq,
            "the migration is written, not just returned"
        );
    }

    #[test]
    fn a_version_one_file_with_an_elevenlabs_key_keeps_the_users_choice() {
        let (_dir, store) = store();
        fs::create_dir_all(store.path().parent().expect("parent")).expect("mkdir");
        fs::write(
            store.path(),
            r#"{"version":1,"active_provider":"eleven_labs","providers":{"elevenlabs_key":"sk-test"}}"#,
        )
        .expect("write v1 config");

        let config = store.load_or_init().expect("migrate");

        assert_eq!(config.active_provider, ProviderId::ElevenLabs);
        assert_eq!(config.providers.elevenlabs_key.as_deref(), Some("sk-test"));
    }

    #[test]
    fn a_current_file_is_left_alone() {
        let (_dir, store) = store();
        let config = Config {
            active_provider: ProviderId::ElevenLabs,
            ..Config::default()
        };
        store.save(&config).expect("save");
        let before = fs::read_to_string(store.path()).expect("read");

        let loaded = store.load_or_init().expect("load");

        assert_eq!(loaded.active_provider, ProviderId::ElevenLabs);
        assert_eq!(fs::read_to_string(store.path()).expect("reread"), before);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults_instead_of_failing() {
        let (_dir, store) = store();
        fs::create_dir_all(store.path().parent().expect("parent")).expect("mkdir");
        fs::write(store.path(), "{\"trigger_key\": \"F9\"}\n").expect("write partial config");

        let config = store.load().expect("a partial file still loads");

        assert_eq!(config.trigger_key, "F9");
        assert_eq!(config.cleanup, Cleanup::default());
    }
}
