//! Anti-cheat blacklist. A game is blocked if ANY source says it uses
//! anti-cheat, because injecting into such a game can get a player banned:
//!
//! 1. files in the game folder (EAC, BattlEye, GameGuard, ...);
//! 2. the community "Are We Anti-Cheat Yet" database (Heroic ships a copy;
//!    we bundle one too);
//! 3. for Steam games, Steam's own "Valve Anti-Cheat enabled" flag, which
//!    catches VAC games that leave no files behind.

#[path = "../../common/anticheat.rs"]
#[allow(dead_code)]
mod anticheat;

use crate::scan::{Found, Store};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// No anti-cheat found by any source.
    Clear,
    /// Uses anti-cheat: never add, never inject.
    Blocked(String),
    /// Still waiting on the Steam store check.
    Checking,
    /// Couldn't be verified (e.g. delisted from the Steam store): can be
    /// added to the library, but the overlay is never installed.
    Unverified(String),
}

/// Steam's answer about Valve Anti-Cheat for one app.
#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
enum Vac {
    Pending,
    Yes,
    No,
    /// The store doesn't know the app (delisted).
    Unknown,
}

#[derive(Deserialize)]
struct AwacyEntry {
    name: String,
    #[serde(default)]
    anticheats: Vec<String>,
}

/// Lower-case letters and digits only, for matching titles across sources.
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

pub struct Safety {
    /// Normalized title -> anti-cheat names.
    awacy: HashMap<String, Vec<String>>,
    /// Steam app id -> what Steam says about VAC.
    vac: Arc<Mutex<HashMap<String, Vac>>>,
    cache_file: PathBuf,
}

impl Safety {
    pub fn new(profile_dir: &std::path::Path) -> Safety {
        // Prefer Heroic's copy (kept up to date by Heroic), else the bundled one.
        let heroic = crate::scan::heroic_config().and_then(|c| std::fs::read_to_string(c.join("areweanticheatyet.json")).ok());
        let text = heroic.unwrap_or_else(|| include_str!("../data/areweanticheatyet.json").to_string());
        let awacy = serde_json::from_str::<Vec<AwacyEntry>>(&text)
            .unwrap_or_default()
            .into_iter()
            .filter(|e| !e.anticheats.is_empty())
            .map(|e| (norm(&e.name), e.anticheats))
            .collect();
        let cache_file = profile_dir.join("vac-cache.json");
        let vac: HashMap<String, Vac> =
            std::fs::read_to_string(&cache_file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Safety { awacy, vac: Arc::new(Mutex::new(vac)), cache_file }
    }

    /// Checks a game against every source. May return `Checking` while the
    /// Steam lookup runs in the background (call again later).
    pub fn check(&self, game: &Found) -> Verdict {
        if let Some(ac) = anticheat::scan_dir(&game.dir) {
            return Verdict::Blocked(format!("{ac} files found in the game folder"));
        }
        if let Some(list) = self.awacy.get(&norm(&game.name)) {
            return Verdict::Blocked(format!("uses {}", list.join(", ")));
        }
        if game.store == Store::Steam {
            return match self.steam_vac(&game.store_id) {
                Vac::Yes => Verdict::Blocked("uses Valve Anti-Cheat".into()),
                Vac::No => Verdict::Clear,
                Vac::Pending => Verdict::Checking,
                Vac::Unknown => Verdict::Unverified("not on the Steam store anymore, so its anti-cheat can't be verified".into()),
            };
        }
        Verdict::Clear
    }

    /// VAC flag for a Steam app: cached, or looked up in the background.
    fn steam_vac(&self, appid: &str) -> Vac {
        let Ok(mut vac) = self.vac.lock() else { return Vac::Pending };
        match vac.get(appid) {
            Some(v) => return *v,
            None => {
                vac.insert(appid.to_string(), Vac::Pending);
            }
        }
        drop(vac);
        let (vac, appid, cache_file) = (Arc::clone(&self.vac), appid.to_string(), self.cache_file.clone());
        std::thread::spawn(move || {
            let url = format!("https://store.steampowered.com/api/appdetails?appids={appid}&filters=categories");
            // Until Steam answers, the game stays unverified (never "clear").
            // Retry every 30 s while offline.
            loop {
                let result = ureq::get(&url)
                    .call()
                    .ok()
                    .and_then(|mut r| r.body_mut().read_to_string().ok())
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .map(|v| match v[&appid]["success"].as_bool() {
                        Some(true) => {
                            let vac = v[&appid]["data"]["categories"]
                                .as_array()
                                .is_some_and(|cats| cats.iter().any(|c| c["id"].as_u64() == Some(8)));
                            if vac { Vac::Yes } else { Vac::No }
                        }
                        _ => Vac::Unknown,
                    });
                if let Some(answer) = result {
                    let Ok(mut map) = vac.lock() else { return };
                    map.insert(appid, answer);
                    let done: HashMap<&String, Vac> = map.iter().filter(|(_, v)| **v != Vac::Pending).map(|(k, v)| (k, *v)).collect();
                    let _ = std::fs::write(&cache_file, serde_json::to_string_pretty(&done).unwrap_or_default());
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
        });
        Vac::Pending
    }
}
