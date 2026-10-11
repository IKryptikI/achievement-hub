//! The player's profile, shared by every game: gamertag, every unlock, and a
//! summary of each game played (for the Home and Games blades).
//!
//! Lives in `Documents/My Games/Achievement Hub`. Under Wine/Proton that is
//! the *Linux* user's Documents folder (reached through the Z: drive), so
//! every game — each in its own Wine prefix, even inside Heroic's Flatpak
//! sandbox — shares one profile. On Windows it is the user's Documents.
//!
//!   profile.toml        gamertag
//!   unlocks.txt         game_id achievement_id unix_time   (append-only)
//!   games/<id>.toml     title, totals, achievement names, last played
//!   screenshots/<id>/   unlock screenshots

use crate::games::Game;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Everything the panel shows about an achievement, saved per game so any
/// game's list can be browsed, not just the one running.
#[derive(Serialize, Deserialize, Clone)]
pub struct AchievementSummary {
    pub id: String,
    pub name: String,
    pub points: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub platinum: bool,
    /// False if the overlay can't detect it yet.
    #[serde(default = "yes")]
    pub trackable: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GameSummary {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    pub total_points: u32,
    pub last_played: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Who wrote the description, when it needs crediting ("Wikipedia").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_source: Option<String>,
    /// GOG, Steam, Epic, Emulator or PC.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub store: String,
    pub achievement: Vec<AchievementSummary>,
}

impl GameSummary {
    pub fn full_title(&self) -> String {
        match &self.subtitle {
            Some(s) => format!("{} ({s})", self.title),
            None => self.title.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ProfileFile {
    gamertag: String,
    /// Dashboard design: "blades", "metro" or "classic".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    dashboard: String,
    /// The player chose to help map achievements (record mode). Off unless
    /// they turn it on in the app.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    mapper: bool,
    /// Gamer picture: "gp:<preset>" or a letter/emoji (from the website or
    /// picked in the overlay).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    avatar: String,
}

pub struct Profile {
    pub dir: PathBuf,
    pub gamertag: String,
    /// Dashboard design picked by the player (shared with the desktop app).
    pub dashboard: String,
    /// Opted in to record mode (see ProfileFile::mapper).
    pub mapper: bool,
    /// Gamer picture (see ProfileFile::avatar).
    pub avatar: String,
    /// (game, achievement) -> unlock time.
    unlocked: HashMap<(String, String), u64>,
    pub games: HashMap<String, GameSummary>,
}

/// Unlock as shown in the Home blade's recent list.
pub struct Recent<'a> {
    pub game: &'a GameSummary,
    pub achievement: &'a AchievementSummary,
    pub when: u64,
}

fn read_unlocks(path: &Path) -> Vec<(String, String, u64)> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.to_string(), it.next()?.to_string(), it.next().and_then(|t| t.parse().ok()).unwrap_or(0)))
        })
        .collect()
}

impl Profile {
    /// Opens (creating if needed) the shared profile. `fallback` is used if
    /// the Documents folder can't be found or written.
    pub fn open(fallback: &Path, log: &dyn Fn(&str)) -> Profile {
        let dir = documents_dir()
            .map(|d| d.join("My Games").join("Achievement Hub"))
            .filter(|d| fs::create_dir_all(d.join("games")).is_ok())
            .unwrap_or_else(|| {
                log("profile: Documents folder unavailable (a sandboxed launcher without access to it?), using the game folder");
                fallback.to_path_buf()
            });
        log(&format!("profile: {}", dir.display()));

        let file = fs::read_to_string(dir.join("profile.toml")).ok().and_then(|t| toml::from_str::<ProfileFile>(&t).ok()).unwrap_or_else(|| {
            let file = ProfileFile { gamertag: default_gamertag(), dashboard: String::new(), mapper: false, avatar: String::new() };
            let _ = fs::write(dir.join("profile.toml"), toml::to_string(&file).unwrap_or_default());
            file
        });
        let (gamertag, dashboard, mapper, avatar) = (file.gamertag, file.dashboard, file.mapper, file.avatar);

        let unlocked = read_unlocks(&dir.join("unlocks.txt")).into_iter().map(|(g, a, t)| ((g, a), t)).collect();

        let games = fs::read_dir(dir.join("games"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| toml::from_str::<GameSummary>(&fs::read_to_string(e.path()).ok()?).ok())
            .map(|g| (g.id.clone(), g))
            .collect();

        Profile { dir, gamertag, dashboard, mapper, avatar, unlocked, games }
    }

    /// Saves the dashboard design choice to profile.toml.
    pub fn set_dashboard(&mut self, name: &str) {
        self.dashboard = name.to_string();
        self.save();
    }

    /// Saves the gamer picture to profile.toml.
    pub fn set_avatar(&mut self, avatar: &str) {
        self.avatar = avatar.to_string();
        self.save();
    }

    fn save(&self) {
        let file = ProfileFile { gamertag: self.gamertag.clone(), dashboard: self.dashboard.clone(), mapper: self.mapper, avatar: self.avatar.clone() };
        let _ = fs::write(self.dir.join("profile.toml"), toml::to_string(&file).unwrap_or_default());
    }

    /// Imports unlocks from an older per-game `achievements.txt`, then
    /// renames it so it isn't imported twice.
    pub fn migrate(&mut self, old: &Path, log: &dyn Fn(&str)) {
        if !old.exists() {
            return;
        }
        let mut n = 0;
        for (g, a, t) in read_unlocks(old) {
            if !self.unlocked.contains_key(&(g.clone(), a.clone())) {
                self.record(&g, &a, t);
                n += 1;
            }
        }
        let _ = fs::rename(old, old.with_extension("txt.migrated"));
        log(&format!("profile: imported {n} unlocks from {}", old.display()));
    }

    fn record(&mut self, game: &str, id: &str, when: u64) {
        self.unlocked.insert((game.to_string(), id.to_string()), when);
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(self.dir.join("unlocks.txt")) {
            let _ = writeln!(f, "{game} {id} {when}");
        }
    }

    /// Records an unlock; returns false if it was already unlocked.
    pub fn unlock(&mut self, game: &str, id: &str) -> bool {
        if self.unlocked.contains_key(&(game.to_string(), id.to_string())) {
            return false;
        }
        self.record(game, id, now());
        true
    }

    pub fn unlocked_at(&self, game: &str, id: &str) -> Option<u64> {
        self.unlocked.get(&(game.to_string(), id.to_string())).copied()
    }

    /// Saves the running game's summary (and marks it as just played).
    /// Adds the game's summary without saving it (the preview tool).
    pub fn touch_game_in_memory(&mut self, game: &Game) {
        self.touch_game_inner(game, false);
    }

    pub fn touch_game(&mut self, game: &Game) {
        self.touch_game_inner(game, true);
    }

    fn touch_game_inner(&mut self, game: &Game, write: bool) {
        let summary = GameSummary {
            id: game.id().to_string(),
            title: game.title().to_string(),
            subtitle: game.subtitle().map(String::from),
            total_points: game.total_points(),
            last_played: now(),
            developer: game.developer.clone(),
            publisher: game.publisher.clone(),
            year: game.year,
            description: game.description.clone(),
            description_source: None,
            store: game.store.to_string(),
            achievement: game
                .achievements()
                .iter()
                .map(|a| AchievementSummary {
                    id: a.id.clone(),
                    name: a.name.clone(),
                    points: a.points,
                    description: a.description.clone(),
                    category: a.category.clone(),
                    secret: a.secret,
                    platinum: a.platinum,
                    trackable: a.trackable(),
                })
                .collect(),
        };
        let mut summary = summary;
        // The store's details (when fetched) win over the definition's.
        if let Some(info) = crate::storeinfo::cached(&self.dir, &summary.id) {
            let set = |v: &str| (!v.is_empty()).then(|| v.to_string());
            if let Some(d) = set(&info.description) {
                summary.description = Some(d);
                summary.description_source = set(&info.description_source);
            }
            summary.developer = set(&info.developer).or(summary.developer);
            summary.publisher = set(&info.publisher).or(summary.publisher);
            summary.year = info.year.or(summary.year);
        }
        if write {
            if let Ok(text) = toml::to_string(&summary) {
                let _ = fs::write(self.dir.join("games").join(format!("{}.toml", summary.id)), text);
            }
        }
        self.games.insert(summary.id.clone(), summary);
    }

    /// Fills a game's summary with its store page details (they win over
    /// the definition's), saving it unless `write` is false (preview).
    pub fn apply_store_info(&mut self, game: &str, info: &crate::storeinfo::StoreInfo, write: bool) {
        let Some(s) = self.games.get_mut(game) else { return };
        let set = |v: &str| (!v.is_empty()).then(|| v.to_string());
        if let Some(d) = set(&info.description) {
            s.description = Some(d);
            s.description_source = set(&info.description_source);
        }
        if let Some(d) = set(&info.developer) {
            s.developer = Some(d);
        }
        if let Some(p) = set(&info.publisher) {
            s.publisher = Some(p);
        }
        if info.year.is_some() {
            s.year = info.year;
        }
        if write {
            if let Ok(text) = toml::to_string(&*s) {
                let _ = fs::write(self.dir.join("games").join(format!("{game}.toml")), text);
            }
        }
    }

    /// Where a game's official achievement icons are cached.
    pub fn icon_dir(&self, game: &str) -> PathBuf {
        self.dir.join("icons").join(game)
    }

    /// Cached official icons for a game, as (achievement id, path).
    pub fn cached_icons(&self, game: &str) -> Vec<(String, PathBuf)> {
        fs::read_dir(self.icon_dir(game))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension()? != "jpg" {
                    return None;
                }
                Some((p.file_stem()?.to_string_lossy().into_owned(), p.clone()))
            })
            .collect()
    }

    /// (unlocked, total, points earned) for one game.
    pub fn game_progress(&self, g: &GameSummary) -> (usize, usize, u32) {
        let got: Vec<_> = g.achievement.iter().filter(|a| self.unlocked_at(&g.id, &a.id).is_some()).collect();
        (got.len(), g.achievement.len(), got.iter().map(|a| a.points).sum())
    }

    /// Gamerscore across every game.
    pub fn gamerscore(&self) -> u32 {
        self.games.values().map(|g| self.game_progress(g).2).sum()
    }

    pub fn total_unlocked(&self) -> usize {
        self.games.values().map(|g| self.game_progress(g).0).sum()
    }

    /// Games, most recently played first.
    pub fn games_by_recent(&self) -> Vec<&GameSummary> {
        let mut v: Vec<_> = self.games.values().collect();
        v.sort_by_key(|g| std::cmp::Reverse(g.last_played));
        v
    }

    /// Most recent unlocks across all games.
    pub fn recent(&self, n: usize) -> Vec<Recent<'_>> {
        let mut v: Vec<Recent> = self
            .unlocked
            .iter()
            .filter_map(|((g, a), &when)| {
                let game = self.games.get(g)?;
                let achievement = game.achievement.iter().find(|x| &x.id == a)?;
                Some(Recent { game, achievement, when })
            })
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.when));
        v.truncate(n);
        v
    }

    /// Saved icon for a game (PNG), next to its summary.
    pub fn game_icon_path(&self, game: &str) -> PathBuf {
        self.dir.join("games").join(format!("{game}.png"))
    }

    pub fn screenshots_dir(&self, game: &str) -> PathBuf {
        self.dir.join("screenshots").join(game)
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn default_gamertag() -> String {
    // Under Proton the Windows user is "steamuser"; prefer the Linux user.
    env("USER").or_else(|| env("USERNAME")).filter(|u| u != "steamuser").unwrap_or_else(|| "Player".into())
}

/// The user's Documents folder; under Wine, the Linux one.
/// Where the desktop app keeps game definitions synced from the online
/// database (read before the profile is opened, to find the game).
pub fn synced_definitions_dir() -> Option<PathBuf> {
    documents_dir().map(|d| d.join("My Games").join("Achievement Hub").join("definitions"))
}

fn documents_dir() -> Option<PathBuf> {
    if let Some(home) = wine_unix_home() {
        return unix_to_dos(&format!("{home}/Documents"));
    }
    env("USERPROFILE").map(|p| PathBuf::from(p).join("Documents"))
}

/// $HOME of the Linux user if we're running under Wine (Wine passes the
/// Unix environment through).
fn wine_unix_home() -> Option<String> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    // SAFETY: lookup only; ntdll is always loaded.
    let is_wine = unsafe {
        let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
        !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some()
    };
    if !is_wine {
        return None;
    }
    // Wine doesn't always pass HOME through, but it sets WINEHOMEDIR to the
    // Unix home as an NT path ("\\??\\unix/home/user").
    env("HOME")
        .filter(|h| h.starts_with('/'))
        .or_else(|| env("WINEHOMEDIR").and_then(|w| w.find("/").map(|i| w[i..].to_string())))
        .or_else(|| env("USER").map(|u| format!("/home/{u}")))
}

/// Converts a Unix path to a Windows path via Wine's Z: drive (mapped to
/// "/" by default in Wine and Proton prefixes). None if that doesn't work.
fn unix_to_dos(unix: &str) -> Option<PathBuf> {
    let p = PathBuf::from(format!("Z:{}", unix.replace('/', "\\")));
    p.exists().then_some(p)
}

/// Formats unix seconds as e.g. "5 Oct 2026" (UTC).
pub fn format_date(unix: u64) -> String {
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

/// "today", "yesterday", "3 days ago", or a date.
pub fn format_ago(unix: u64) -> String {
    let days = now().saturating_sub(unix) / 86_400;
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=6 => format!("{days} days ago"),
        _ => format_date(unix),
    }
}
