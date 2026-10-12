//! Updates the app in place from the latest GitHub release.
//!
//! On start (in the background) it asks GitHub for the latest release. If
//! it's newer than this version, it downloads the file for this kind of
//! install, checks it against the SHA-256 GitHub publishes for it, and puts
//! it in place:
//!
//! - Linux AppImage: the AppImage file is replaced (the running copy keeps
//!   working until it exits).
//! - Windows folder (AchievementHub.exe + payload/): the running exe is
//!   moved aside to AchievementHub.exe.old (Windows allows renaming a
//!   running exe, not overwriting it), the new exe and payload files are
//!   written, and the .old file is removed on the next start.
//!
//! Running from a build folder (development) never updates itself. The
//! overlays in games are brought up to date separately on every start
//! (library::refresh_overlays). AH_UPDATE_URL points the check somewhere
//! else (for testing); AH_NO_UPDATE turns it off.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const LATEST: &str = "https://api.github.com/repos/IKryptikI/achievement-hub/releases/latest";

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    /// "sha256:<hex>" (GitHub fills this in for uploaded files).
    #[serde(default)]
    digest: Option<String>,
}

/// How this copy was installed (what an update replaces).
enum Install {
    AppImage(PathBuf),
    WindowsFolder(PathBuf),
}

fn install() -> Option<Install> {
    if let Some(p) = std::env::var_os("APPIMAGE") {
        return Some(Install::AppImage(PathBuf::from(p)));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let named = exe.file_name()?.to_string_lossy().eq_ignore_ascii_case("AchievementHub.exe");
    (cfg!(windows) && named && dir.join("payload").is_dir()).then(|| Install::WindowsFolder(exe))
}

/// "v0.3.1" -> (0, 3, 1).
fn parse(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut resp = agent
        .get(url)
        .header("User-Agent", &format!("AchievementHub/{VERSION}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("couldn't check for updates: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("update check failed ({})", resp.status()));
    }
    resp.body_mut().with_config().limit(200 << 20).read_to_vec().map_err(|e| e.to_string())
}

/// What happened, for the app to show (None: nothing to say).
pub enum Outcome {
    /// Updated to this version; restart to use it.
    Updated(String),
    Failed(String),
}

/// Checks for a newer release and installs it. Blocking: run it on a thread.
pub fn run() -> Option<Outcome> {
    if std::env::var_os("AH_NO_UPDATE").is_some() {
        return None;
    }
    cleanup();
    let install = install()?;
    let url = std::env::var("AH_UPDATE_URL").unwrap_or(LATEST.into());
    let release: Release = match get(&url).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
        Ok(r) => r,
        Err(_) => return None, // offline: try again next start, quietly
    };
    let (Some(new), Some(ours)) = (parse(&release.tag_name), parse(VERSION)) else { return None };
    if new <= ours {
        return None;
    }
    let version = release.tag_name.trim_start_matches('v').to_string();
    let want = match install {
        Install::AppImage(_) => "AchievementHub-x86_64.AppImage",
        Install::WindowsFolder(_) => "AchievementHub-windows.zip",
    };
    let result = (|| {
        let asset = release.assets.iter().find(|a| a.name == want).ok_or(format!("the {version} release has no {want}"))?;
        let expected = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).ok_or("the release doesn't list a checksum; not installing it")?;
        let bytes = get(&asset.browser_download_url)?;
        let actual: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err("the download didn't match its checksum; not installing it".to_string());
        }
        match &install {
            Install::AppImage(path) => replace_appimage(path, &bytes),
            Install::WindowsFolder(exe) => replace_windows(exe, &bytes),
        }
    })();
    Some(match result {
        Ok(()) => Outcome::Updated(version),
        Err(e) => Outcome::Failed(format!("Couldn't update to {version}: {e}")),
    })
}

/// Writes next to the target, then renames over it (all or nothing).
fn put(target: &Path, bytes: &[u8], executable: bool) -> Result<(), String> {
    let tmp = target.with_extension("update");
    std::fs::write(&tmp, bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = executable;
    std::fs::rename(&tmp, target).map_err(|e| format!("replacing {}: {e}", target.display()))
}

fn replace_appimage(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if !bytes.starts_with(b"\x7fELF") {
        return Err("the download isn't an AppImage".into());
    }
    put(path, bytes, true)
}

fn replace_windows(exe: &Path, zip_bytes: &[u8]) -> Result<(), String> {
    let dir = exe.parent().ok_or("bad install folder")?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| format!("reading the download: {e}"))?;
    // Everything out first (so a bad file stops us before anything changes).
    let mut files = Vec::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        let Some(rel) = f.enclosed_name() else { continue };
        // Inside the zip everything sits in AchievementHub-windows/.
        let rel: PathBuf = rel.components().skip(1).collect();
        let mut data = Vec::new();
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        files.push((rel, data));
    }
    if !files.iter().any(|(p, _)| p == Path::new("AchievementHub.exe")) {
        return Err("the download has no AchievementHub.exe".into());
    }
    // The running exe can be renamed but not overwritten.
    let old = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("moving the old version aside: {e}"))?;
    for (rel, data) in &files {
        let target = dir.join(rel);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = put(&target, data, false) {
            if rel == Path::new("AchievementHub.exe") {
                let _ = std::fs::rename(&old, exe); // put the old one back
            }
            return Err(e);
        }
    }
    Ok(())
}

/// Removes what an earlier Windows update left behind.
fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
}

/// Starts the updated copy and quits this one.
pub fn restart() {
    let target = match install() {
        Some(Install::AppImage(p)) => p,
        Some(Install::WindowsFolder(exe)) => exe,
        None => return,
    };
    if std::process::Command::new(&target).spawn().is_ok() {
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert!(super::parse("v0.3.1") > super::parse("0.3.0"));
        assert!(super::parse("v1.0") > super::parse("0.9.9"));
        assert_eq!(super::parse("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(super::parse("nightly"), None);
    }
}
