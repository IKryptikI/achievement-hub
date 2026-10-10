//! Unlocked achievements, saved as plain text lines `game_id achievement_id
//! unix_time` next to the DLL. The hub app will read and sync this later.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::games::Achievement;

pub struct Store {
    path: PathBuf,
    /// (game, achievement) -> unlock time (unix seconds).
    unlocked: HashMap<(String, String), u64>,
}

impl Store {
    pub fn open(path: PathBuf) -> Store {
        let unlocked = fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace();
                let key = (it.next()?.to_string(), it.next()?.to_string());
                Some((key, it.next().and_then(|t| t.parse().ok()).unwrap_or(0)))
            })
            .collect();
        Store { path, unlocked }
    }

    /// Records an unlock; returns false if it was already unlocked.
    pub fn unlock(&mut self, game: &str, id: &str) -> bool {
        let key = (game.to_string(), id.to_string());
        if self.unlocked.contains_key(&key) {
            return false;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.unlocked.insert(key, now);
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{game} {id} {now}");
        }
        true
    }

    /// Unlock time in unix seconds, if unlocked.
    pub fn unlocked_at(&self, game: &str, id: &str) -> Option<u64> {
        self.unlocked.get(&(game.to_string(), id.to_string())).copied()
    }

    pub fn progress(&self, game: &str, all: &[Achievement]) -> (usize, usize) {
        (all.iter().filter(|a| self.unlocked_at(game, &a.id).is_some()).count(), all.len())
    }
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
