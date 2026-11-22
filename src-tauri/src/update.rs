//! The update check: ask GitHub for the newest release, and install it when
//! the user asks — one click from Settings, or one shell command through
//! scripts/install-sotto.sh.

use std::{
    fs,
    path::{Path, PathBuf},
    process,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const REPO: &str = "atpaawej/typeless";

/// What the window needs to show about an update.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// The running version.
    pub current: String,
    /// The newest published version, when the check succeeded.
    pub latest: Option<String>,
    /// True when `latest` is newer than `current`.
    pub available: bool,
    /// The release page URL.
    pub url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: Option<String>,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

/// Asks GitHub for the newest release and reports whether it is newer.
pub fn check(current: &str) -> Result<UpdateStatus, String> {
    Ok(status_for(&fetch_release()?, current))
}

fn status_for(release: &Release, current: &str) -> UpdateStatus {
    let latest = version_of(&release.tag_name);
    UpdateStatus {
        available: version_is_newer(&latest, current),
        current: current.to_owned(),
        latest: Some(latest),
        url: release.html_url.clone(),
    }
}

/// Downloads the newest `.deb` and installs it with sudo, the way the
/// installer script does.
pub fn install_latest() -> Result<(), String> {
    let release = fetch_release()?;
    let deb = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(".deb"))
        .ok_or_else(|| "the latest release has no .deb to install".to_owned())?;
    let checksum = release
        .assets
        .iter()
        .find(|asset| asset.name == format!("{}.sha256", deb.name));

    let dir = stage_dir()?;
    let path = dir.join(&deb.name);
    download(&deb.browser_download_url, &path)?;
    if let Some(checksum) = checksum {
        let expected = fetch_text(&checksum.browser_download_url)?;
        verify_sha256(&path, &expected)?;
    } else {
        eprintln!("update: the release ships no checksum for {}", deb.name);
    }
    run_sudo_install(&path)
}

fn fetch_release() -> Result<Release, String> {
    let response = reqwest::blocking::Client::builder()
        .user_agent(format!("sotto/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("could not build the request: {error}"))?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .send()
        .map_err(|error| format!("could not reach the GitHub API: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "the GitHub API answered {status}; no release is published yet"
        ));
    }
    let body = response
        .text()
        .map_err(|error| format!("could not read the release data: {error}"))?;
    serde_json::from_str::<Release>(&body).map_err(|error| format!("could not parse the release data: {error}"))
}

fn fetch_text(url: &str) -> Result<String, String> {
    let response =
        reqwest::blocking::get(url).map_err(|error| format!("could not download {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "the download from {url} failed with status {}",
            response.status()
        ));
    }
    response
        .text()
        .map_err(|error| format!("could not read the checksum: {error}"))
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    let mut response =
        reqwest::blocking::get(url).map_err(|error| format!("could not download {url}: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "the download from {url} failed with status {}",
            response.status()
        ));
    }
    let mut file = fs::File::create(to)
        .map_err(|error| format!("could not write {}: {error}", to.display()))?;
    response
        .copy_to(&mut file)
        .map_err(|error| format!("could not write {}: {error}", to.display()))?;
    Ok(())
}

/// Compares the file's digest with the first token of the checksum text, the
/// way `sha256sum -c` reads its input.
fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let actual: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let expected = expected
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "the checksum of {} does not match the release's: expected {expected}, got {actual}",
            path.display()
        ))
    }
}

fn run_sudo_install(path: &Path) -> Result<(), String> {
    let status = match process::Command::new("pkexec")
        .arg("dpkg")
        .arg("-i")
        .arg(path)
        .status()
    {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => process::Command::new("sudo")
            .arg("dpkg")
            .arg("-i")
            .arg(path)
            .status()
            .map_err(|sudo_error| format!("could not run dpkg: {sudo_error}"))?,
        Err(error) => return Err(format!("could not run pkexec: {error}")),
    };
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "dpkg exited with {status} and did not install the update"
        ))
    }
}

fn stage_dir() -> Result<PathBuf, String> {
    let dir = dirs::cache_dir()
        .ok_or_else(|| "there is no cache directory to stage the update in".to_owned())?
        .join("sotto");
    fs::create_dir_all(&dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

/// "v0.1.0" → "0.1.0"; anything without a leading v is its own version.
fn version_of(tag: &str) -> String {
    tag.strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag)
        .to_owned()
}

/// True when `candidate` is a strictly newer version than `current`.
fn version_is_newer(candidate: &str, current: &str) -> bool {
    let candidate = parse_version(&version_of(candidate));
    let current = parse_version(&version_of(current));
    let len = candidate.len().max(current.len());
    for index in 0..len {
        let newer = candidate.get(index).copied().unwrap_or(0);
        let older = current.get(index).copied().unwrap_or(0);
        if newer != older {
            return newer > older;
        }
    }
    false
}

/// "0.1.0" → [0, 1, 0]; a suffix like "-beta" drops off its segment.
fn parse_version(text: &str) -> Vec<u32> {
    text.split('.')
        .filter_map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(version_is_newer("1.0.0", "0.9.9"));
        assert!(version_is_newer("v0.2.0", "0.1.9"));
        assert!(version_is_newer("0.1.0", "v0.0.9"));
        assert!(!version_is_newer("1.0.0", "1.0.0"));
        assert!(!version_is_newer("0.1.0", "0.2.0"));
        assert!(!version_is_newer("", "1.0.0"));
    }

    #[test]
    fn the_checksum_file_must_match_the_downloaded_deb() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("sotto_0.1.0_amd64.deb");
        fs::write(&path, b"payload").expect("write the fake deb");
        let digest: String = Sha256::digest(b"payload")
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();

        verify_sha256(&path, &format!("{digest}  sotto_0.1.0_amd64.deb")).expect("matching hash");

        let error =
            verify_sha256(&path, "0".repeat(64).as_str()).expect_err("a wrong hash is an error");
        assert!(error.contains("does not match"), "got {error}");
    }

    #[test]
    fn the_latest_release_is_reported_with_its_assets() {
        let release = Release {
            tag_name: "v0.2.0".to_owned(),
            html_url: Some("https://github.com/atpaawej/typeless/releases/tag/v0.2.0".to_owned()),
            assets: vec![ReleaseAsset {
                name: "sotto_0.2.0_amd64.deb".to_owned(),
                browser_download_url: "https://example.com/sotto_0.2.0_amd64.deb".to_owned(),
            }],
        };

        let status = status_for(&release, "0.1.0");
        assert!(status.available);
        assert_eq!(status.latest.as_deref(), Some("0.2.0"));
        assert_eq!(
            status.url.as_deref(),
            Some("https://github.com/atpaawej/typeless/releases/tag/v0.2.0")
        );

        assert!(!status_for(&release, "0.2.0").available, "equal is not newer");
        assert!(!status_for(&release, "0.3.0").available, "newer is not newer");
    }
}
