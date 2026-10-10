//! Sharing record-mode findings: an achievement the player pinned down with
//! record mode (one candidate left) can be sent to the website, where it's
//! reviewed and published to the online game database for everyone.
//! A `<achievement>.shared` file next to the mapping marks it as sent.

use crate::catalogue::Definition;
use crate::library::profile_dir;
use crate::requests;
use std::fs;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Ready {
    pub game: String,
    pub game_title: String,
    pub achievement: String,
    pub name: String,
    pub recordings: u64,
    json: String,
    path: PathBuf,
}

/// Pinned mappings not shared yet, for achievements the database can't
/// track yet.
pub fn ready(defs: &[Definition]) -> Vec<Ready> {
    let mut out = Vec::new();
    for game_dir in fs::read_dir(profile_dir().join("mapping")).into_iter().flatten().flatten() {
        let game = game_dir.file_name().to_string_lossy().into_owned();
        let Some(def) = defs.iter().find(|d| d.game.id == game) else { continue };
        for f in fs::read_dir(game_dir.path()).into_iter().flatten().flatten() {
            let path = f.path();
            if path.extension().is_none_or(|x| x != "json") || path.with_extension("shared").exists() {
                continue;
            }
            let Ok(json) = fs::read_to_string(&path) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else { continue };
            if v["candidates"].as_array().is_none_or(|c| c.len() != 1) {
                continue;
            }
            let achievement = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            let Some(a) = def.achievement.iter().find(|a| a.id == achievement) else { continue };
            if !a.when.is_empty() || a.steam_api.is_some() {
                continue; // already tracked in the database
            }
            out.push(Ready {
                game: game.clone(),
                game_title: def.game.title.clone(),
                achievement,
                name: a.name.clone(),
                recordings: v["recordings"].as_u64().unwrap_or(1),
                json,
                path,
            });
        }
    }
    out.sort_by(|a, b| (&a.game_title, &a.name).cmp(&(&b.game_title, &b.name)));
    out
}

/// Sends each mapping to the website; returns how many went through.
pub fn share(list: &[Ready], gamertag: &str) -> Result<usize, String> {
    let mut sent = 0;
    for r in list {
        let data: serde_json::Value = serde_json::from_str(&r.json).map_err(|e| e.to_string())?;
        let body = serde_json::json!({ "game": r.game, "achievement": r.achievement, "gamertag": gamertag, "data": data });
        requests::post("/api/app/mapping", &body).map_err(|e| format!("{} - {}: {e}", r.game_title, r.name))?;
        let _ = fs::write(r.path.with_extension("shared"), "");
        sent += 1;
    }
    Ok(sent)
}
