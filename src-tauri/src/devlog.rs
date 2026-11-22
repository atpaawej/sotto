//! Step-by-step dev-mode pipeline logging.
//!
//! Enabled automatically in debug builds (`cfg!(debug_assertions)`) and in
//! release builds when `SOTTO_LOG=1` (or `true`/`yes`) is set. Never logs API keys.

use std::{
    fmt::Display,
    sync::OnceLock,
    time::Instant,
};

static START: OnceLock<Instant> = OnceLock::new();
static ENABLED: OnceLock<bool> = OnceLock::new();

pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        cfg!(debug_assertions)
            || std::env::var("SOTTO_LOG")
                .is_ok_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
    })
}

pub fn step(stage: &str, message: impl Display) {
    if !enabled() {
        return;
    }
    let elapsed_ms = START.get_or_init(Instant::now).elapsed().as_millis();
    eprintln!("[sotto][{elapsed_ms:>7}ms][{stage}] {message}");
}
