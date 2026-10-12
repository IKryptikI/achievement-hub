//! Reads the shared profile the overlay writes (Documents/My Games/
//! Achievement Hub): gamertag, unlocks, per-game summaries, icons and
//! screenshots. Read-only here: only the overlay records unlocks.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Clone)]
pub struct AchievementSummary {
    pub id: String,
    pub name: String,
    pub points: u32,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub category: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    #[allow(dead_code)]
    pub platinum: bool,
    #[serde(default = "yes")]
    pub trackable: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Clone)]
pub struct GameSummary {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    pub total_points: u32,
    pub last_played: u64,
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

pub struct Recent {
    pub game: String,
    pub achievement: AchievementSummary,
    pub when: u64,
}

#[derive(Default)]
pub struct Profile {
    pub dir: PathBuf,
    pub gamertag: String,
    unlocked: HashMap<(String, String), u64>,
    pub games: Vec<GameSummary>,
}

impl Profile {
    pub fn load() -> Profile {
        let dir = crate::library::profile_dir();
        let gamertag = fs::read_to_string(dir.join("profile.toml"))
            .ok()
            .and_then(|t| t.lines().find_map(|l| l.strip_prefix("gamertag = ").map(|v| v.trim_matches('"').to_string())))
            .unwrap_or_else(|| "Player".into());
        let unlocked = fs::read_to_string(dir.join("unlocks.txt"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                Some(((it.next()?.to_string(), it.next()?.to_string()), it.next().and_then(|t| t.parse().ok()).unwrap_or(0)))
            })
            .collect();
        let mut games: Vec<GameSummary> = fs::read_dir(dir.join("games"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .filter_map(|e| toml::from_str(&fs::read_to_string(e.path()).ok()?).ok())
            .collect();
        games.sort_by_key(|g| std::cmp::Reverse(g.last_played));
        Profile { dir, gamertag, unlocked, games }
    }

    pub fn unlocked_at(&self, game: &str, id: &str) -> Option<u64> {
        self.unlocked.get(&(game.to_string(), id.to_string())).copied()
    }

    /// (unlocked, total, points earned)
    pub fn progress(&self, g: &GameSummary) -> (usize, usize, u32) {
        let got: Vec<_> = g.achievement.iter().filter(|a| self.unlocked_at(&g.id, &a.id).is_some()).collect();
        (got.len(), g.achievement.len(), got.iter().map(|a| a.points).sum())
    }

    pub fn gamerscore(&self) -> u32 {
        self.games.iter().map(|g| self.progress(g).2).sum()
    }

    pub fn total_unlocked(&self) -> usize {
        self.games.iter().map(|g| self.progress(g).0).sum()
    }

    pub fn recent(&self, n: usize) -> Vec<Recent> {
        let mut v: Vec<Recent> = self
            .unlocked
            .iter()
            .filter_map(|((g, a), &when)| {
                let game = self.games.iter().find(|x| &x.id == g)?;
                let achievement = game.achievement.iter().find(|x| &x.id == a)?.clone();
                Some(Recent { game: game.title.clone(), achievement, when })
            })
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.when));
        v.truncate(n);
        v
    }

    pub fn game_icon(&self, game: &str) -> PathBuf {
        self.dir.join("games").join(format!("{game}.png"))
    }

    pub fn achievement_icon(&self, game: &str, id: &str) -> PathBuf {
        self.dir.join("screenshots").join(game).join(format!("{id}.icon.png"))
    }
}

/// "5 Oct 2026" (UTC).
pub fn format_date(unix: u64) -> String {
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
    match now().saturating_sub(unix) / 86_400 {
        0 => "today".into(),
        1 => "yesterday".into(),
        d @ 2..=6 => format!("{d} days ago"),
        _ => format_date(unix),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
