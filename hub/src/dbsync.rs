//! The online game database: every game definition lives in a public
//! repository with an index (file name + SHA-256). On start the app fetches
//! the index, downloads any definition that's new or changed into the
//! profile's `definitions` folder, and refreshes the copies inside games the
//! overlay is installed in. Definitions built into the app are the fallback
//! when offline.

use crate::library::{Library, profile_dir};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

/// Where the database is published (index.json and the .toml files).
const BASE: &str = "https://raw.githubusercontent.com/IKryptikI/achievement-hub-games/main";

/// The database location (AH_DB_BASE overrides it, for testing).
fn base() -> String {
    std::env::var("AH_DB_BASE").unwrap_or_else(|_| BASE.to_string())
}

#[derive(Deserialize)]
struct Index {
    games: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    file: String,
    sha256: String,
}

/// The profile folder downloaded definitions live in.
pub fn dir() -> PathBuf {
    profile_dir().join("definitions")
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    let mut resp = ureq::get(url).call().map_err(|e| e.to_string())?;
    resp.body_mut().with_config().limit(4 << 20).read_to_vec().map_err(|e| e.to_string())
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A file name the index may use (no paths).
fn safe_name(f: &str) -> bool {
    f.ends_with(".toml") && !f.contains(['/', '\\']) && !f.starts_with('.')
}

/// Brings the local copy up to date. Returns the file names that changed.
pub fn sync() -> Result<Vec<String>, String> {
    let index: Index = serde_json::from_slice(&get(&format!("{}/index.json", base()))?).map_err(|e| format!("index: {e}"))?;
    let dir = dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut changed = Vec::new();
    for e in index.games.iter().filter(|e| safe_name(&e.file)) {
        let path = dir.join(&e.file);
        if fs::read(&path).is_ok_and(|b| sha(&b) == e.sha256) {
            continue;
        }
        let bytes = get(&format!("{}/definitions/{}", base(), e.file))?;
        if sha(&bytes) != e.sha256 {
            return Err(format!("{}: download didn't match the index", e.file));
        }
        // Only keep files that parse, so a bad upload can't break a game.
        let text = String::from_utf8(bytes).map_err(|_| format!("{}: not text", e.file))?;
        toml::from_str::<crate::catalogue::Definition>(&text).map_err(|err| format!("{}: {err}", e.file))?;
        fs::write(&path, &text).map_err(|e| e.to_string())?;
        changed.push(e.file.clone());
    }
    if !changed.is_empty() {
        refresh_installed(&changed);
    }
    Ok(changed)
}

/// Copies updated definitions into the `games` folder of every game that has
/// the overlay installed and already carries that file.
fn refresh_installed(changed: &[String]) {
    for g in Library::load().game.iter().filter(|g| g.overlay_installed) {
        for exe in &g.game_exes {
            let Some(games) = exe.parent().map(|p| p.join("games")) else { continue };
            for f in changed {
                if games.join(f).exists() {
                    let _ = fs::copy(dir().join(f), games.join(f));
                }
            }
        }
    }
}
