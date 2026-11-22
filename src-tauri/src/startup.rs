//! Start at login: the XDG autostart file in the user's own config.
//!
//! The file is the state — the desktop reads it at login — so enable and
//! disable are both just writes and removals, and both survive a re-login
//! because nothing else can change them.

use std::{fs, path::Path};

/// The autostart entry's file name.
pub const AUTOSTART_FILE: &str = "Sotto.desktop";

/// `$XDG_CONFIG_HOME/autostart/Sotto.desktop`.
pub fn autostart_path() -> Result<std::path::PathBuf, String> {
    let base =
        dirs::config_dir().ok_or_else(|| "no home directory to place the autostart file in".to_owned())?;
    Ok(base.join("autostart").join(AUTOSTART_FILE))
}

/// Writes the autostart entry when `enabled`, and removes it when not.
pub fn set_start_at_login(enabled: bool) -> Result<(), String> {
    apply(&autostart_path()?, enabled)
}

/// The autostart file exists, so the desktop starts Sotto at login.
pub fn is_start_at_login_enabled() -> bool {
    autostart_path().map(|path| path.exists()).unwrap_or(false)
}

fn apply(path: &Path, enabled: bool) -> Result<(), String> {
    if enabled {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        fs::write(path, AUTOSTART_CONTENTS)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("could not remove {}: {error}", path.display())),
        }
    }
}

const AUTOSTART_CONTENTS: &str = "\
[Desktop Entry]
Type=Application
Name=Sotto
GenericName=Dictation
Comment=Hold a key, speak, and clean text lands where your cursor is
Exec=sotto
Icon=io.github.atpaawej.Sotto
Terminal=false
Categories=Utility;Accessibility;
StartupWMClass=Sotto
X-GNOME-Autostart-enabled=true";

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_to(dir: &std::path::Path, enabled: bool) -> Result<(), String> {
        apply(&dir.join("autostart").join(AUTOSTART_FILE), enabled)
    }

    #[test]
    fn enabling_writes_the_file_and_disabling_removes_it() {
        let dir = tempfile::tempdir().expect("a temp dir");

        apply_to(dir.path(), true).expect("enable");
        let path = dir.path().join("autostart").join(AUTOSTART_FILE);
        assert!(path.exists(), "enabling writes the file the desktop reads at login");
        assert!(
            fs::read_to_string(&path).expect("read back").contains("Exec=sotto"),
            "the entry runs the installed app by name"
        );

        apply_to(dir.path(), true).expect("enabling twice is idempotent");
        assert!(path.exists());

        apply_to(dir.path(), false).expect("disable");
        assert!(!path.exists(), "disabling removes the file");

        apply_to(dir.path(), false).expect("disabling twice is idempotent");
        assert!(!path.exists());
    }
}
