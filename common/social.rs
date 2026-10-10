//! Online profiles and friends (shared by the desktop app and the overlay).
//!
//! Accounts live on the Achievement Hub website. The desktop app links to an
//! account with a one-time code from the website's Account page and keeps
//! the resulting app token in `social.toml`. With it, the app and the
//! overlay publish this player's public profile (`snapshot`: gamertag,
//! Gamerscore, games with progress, recent unlocks, what's being played)
//! and fetch their friends'. Nothing else leaves the computer.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// The website (until a domain is bought). AH_SITE overrides it (testing).
pub const SITE: &str = "https://achievementhub.pages.dev";

pub fn site() -> String {
    std::env::var("AH_SITE").unwrap_or_else(|_| SITE.to_string())
}

/// The link to the player's account.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Social {
    pub token: String,
    #[serde(default)]
    pub gamertag: String,
}

pub fn load(profile: &Path) -> Option<Social> {
    std::fs::read_to_string(profile.join("social.toml")).ok().and_then(|t| toml::from_str::<Social>(&t).ok()).filter(|s| !s.token.is_empty())
}

pub fn save(profile: &Path, s: &Social) {
    if let Ok(t) = toml::to_string(s) {
        let _ = std::fs::write(profile.join("social.toml"), t);
    }
}

pub fn forget(profile: &Path) {
    let _ = std::fs::remove_file(profile.join("social.toml"));
}

/// Where each call goes.
pub fn link_url() -> String {
    format!("{}/api/app/link", site())
}
pub fn progress_url() -> String {
    format!("{}/api/app/progress", site())
}
pub fn friends_url() -> String {
    format!("{}/api/app/friends", site())
}
pub fn unlink_url() -> String {
    format!("{}/api/app/unlink", site())
}

#[derive(Deserialize)]
struct Summary {
    id: String,
    title: String,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    total_points: u32,
    #[serde(default)]
    last_played: u64,
    #[serde(default)]
    achievement: Vec<SummaryAch>,
}

#[derive(Deserialize)]
struct SummaryAch {
    id: String,
    name: String,
    #[serde(default)]
    points: u32,
}

/// The public profile, as published.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Profile {
    pub gamertag: String,
    #[serde(default)]
    pub gamerscore: u32,
    #[serde(default)]
    pub unlocked: u32,
    /// Profile look, from the website.
    #[serde(default)]
    pub avatar: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub motto: String,
    /// The game being played right now (the overlay sets it).
    #[serde(default)]
    pub playing: Option<String>,
    #[serde(default)]
    pub games: Vec<GameProgress>,
    #[serde(default)]
    pub recent: Vec<RecentUnlock>,
    /// When the player's computer sent it (unix seconds), for "online now".
    #[serde(default)]
    pub sent: u64,
}

impl Profile {
    /// Sent within the last 15 minutes (so `playing` is current).
    pub fn online(&self, now: u64) -> bool {
        self.sent + 15 * 60 >= now
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct GameProgress {
    pub id: String,
    pub title: String,
    pub unlocked: u32,
    pub total: u32,
    pub points: u32,
    pub total_points: u32,
    pub last_played: u64,
    /// The achievements unlocked (ids), so the website can show which ones
    /// are confirmed to work.
    #[serde(default)]
    pub ids: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct RecentUnlock {
    pub title: String,
    pub name: String,
    pub points: u32,
    pub time: u64,
}

/// Builds the public profile from the profile folder.
pub fn snapshot(profile: &Path, playing: Option<&str>) -> Profile {
    let gamertag = std::fs::read_to_string(profile.join("profile.toml"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("gamertag = ").map(|v| v.trim().trim_matches('"').to_string())))
        .unwrap_or_default();
    let unlocks: Vec<(String, String, u64)> = std::fs::read_to_string(profile.join("unlocks.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.to_string(), it.next().and_then(|t| t.parse().ok()).unwrap_or(0)))
        })
        .collect();
    let summaries: HashMap<String, Summary> = std::fs::read_dir(profile.join("games"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "toml") && !e.file_name().to_string_lossy().ends_with(".store.toml"))
        .filter_map(|e| toml::from_str::<Summary>(&std::fs::read_to_string(e.path()).ok()?).ok())
        .map(|s| (s.id.clone(), s))
        .collect();
    let full = |s: &Summary| match &s.subtitle {
        Some(sub) => format!("{} ({sub})", s.title),
        None => s.title.clone(),
    };
    let mut games: Vec<GameProgress> = summaries
        .values()
        .map(|s| {
            let mine: Vec<&SummaryAch> = s.achievement.iter().filter(|a| unlocks.iter().any(|(g, id, _)| *g == s.id && *id == a.id)).collect();
            GameProgress {
                id: s.id.clone(),
                title: full(s),
                unlocked: mine.len() as u32,
                total: s.achievement.len() as u32,
                points: mine.iter().map(|a| a.points).sum(),
                total_points: s.total_points,
                last_played: s.last_played,
                ids: mine.iter().map(|a| a.id.clone()).collect(),
            }
        })
        .collect();
    games.sort_by(|a, b| b.last_played.cmp(&a.last_played));
    games.truncate(60);
    let mut recent: Vec<RecentUnlock> = unlocks
        .iter()
        .filter_map(|(g, id, t)| {
            let s = summaries.get(g)?;
            let a = s.achievement.iter().find(|a| a.id == *id)?;
            Some(RecentUnlock { title: full(s), name: a.name.clone(), points: a.points, time: *t })
        })
        .collect();
    recent.sort_by(|a, b| b.time.cmp(&a.time));
    recent.truncate(30);
    let gamerscore = games.iter().map(|g| g.points).sum();
    let unlocked = games.iter().map(|g| g.unlocked).sum();
    Profile {
        gamertag,
        avatar: String::new(),
        color: String::new(),
        motto: String::new(),
        gamerscore,
        unlocked,
        playing: playing.map(|p| summaries.get(p).map_or(p.to_string(), full)),
        games,
        recent,
        sent: now(),
    }
}

#[derive(Deserialize)]
struct FriendsReply {
    #[serde(default)]
    friends: Vec<Profile>,
}

/// Friends' profiles from the website's reply.
pub fn parse_friends(body: &[u8]) -> Vec<Profile> {
    serde_json::from_slice::<FriendsReply>(body).map(|r| r.friends).unwrap_or_default()
}

/// The website's error message in a reply, if any.
pub fn error_in(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body).ok()?.get("error")?.as_str().map(String::from)
}
