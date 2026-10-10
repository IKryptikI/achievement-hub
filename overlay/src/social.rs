//! Online profiles in the overlay: publishes the player's profile (with the
//! game being played) when the overlay starts, after unlocks and every few
//! minutes, and keeps friends' profiles for the Friends blade. Everything
//! runs on one background thread. The identity is made by the desktop app.

#[path = "../../common/social.rs"]
#[allow(dead_code)]
pub mod shared;

use shared::Profile;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Friends' profiles and a status line for the Friends blade.
static STATE: Mutex<Option<(Vec<Profile>, String)>> = Mutex::new(None);
/// Set after an unlock: publish soon.
static KICK: AtomicBool = AtomicBool::new(false);
const EVERY: Duration = Duration::from_secs(180);

fn set(friends: Option<Vec<Profile>>, status: &str) {
    if let Ok(mut s) = STATE.lock() {
        let entry = s.get_or_insert_with(Default::default);
        if let Some(f) = friends {
            entry.0 = f;
        }
        entry.1 = status.to_string();
    }
}

/// Friends' profiles and the current status.
pub fn friends() -> (Vec<Profile>, String) {
    STATE.lock().ok().and_then(|s| s.clone()).unwrap_or_else(|| (Vec::new(), "Link your account in the Achievement Hub app (Friends) to see friends here.".into()))
}

/// Sample friends for the UI preview tool (AH_FRIENDS_DEMO=1).
pub fn demo() {
    let now = shared::now();
    let mk = |tag: &str, g: u32, playing: Option<&str>, sent: u64, latest: (&str, &str)| Profile {
        gamertag: tag.into(),
        gamerscore: g,
        playing: playing.map(String::from),
        sent,
        recent: vec![shared::RecentUnlock { title: latest.1.into(), name: latest.0.into(), points: 20, time: now }],
        ..Default::default()
    };
    set(
        Some(vec![
            mk("Bingus", 1240, Some("Project Zomboid"), now, ("Big Trouble in Big Town", "Fallout 3")),
            mk("Sidra", 655, None, now - 3 * 86_400, ("Damaged Goods", "Saw")),
            mk("Kodeman", 3210, Some("Silent Hill 3"), now - 60, ("Let's Do This", "Silent Hill 3")),
        ]),
        "",
    );
}

/// Something worth sharing happened (an unlock).
pub fn kick() {
    KICK.store(true, Ordering::Relaxed);
}

pub fn start(profile: PathBuf, playing: Option<String>, log: impl Fn(&str) + Send + 'static) {
    std::thread::spawn(move || {
        if shared::load(&profile).is_none() {
            set(None, "Link your account in the Achievement Hub app (Friends) to see friends here.");
            return;
        }
        set(None, "Connecting…");
        let mut logged = false;
        loop {
            // (Re-read each time: the app may have linked or unlinked.)
            let Some(me) = shared::load(&profile) else {
                set(Some(Vec::new()), "Link your account in the Achievement Hub app (Friends) to see friends here.");
                return;
            };
            let p = shared::snapshot(&profile, playing.as_deref());
            let body = serde_json::to_string(&p).unwrap_or_default();
            if let (Err(e), false) = (crate::storeinfo::call("PUT", &shared::progress_url(), Some(&body), &me.token), logged) {
                log(&format!("profile: couldn't publish ({e})"));
                logged = true;
            }
            match crate::storeinfo::call("GET", &shared::friends_url(), None, &me.token) {
                Ok(reply) => match shared::error_in(&reply) {
                    Some(e) => set(None, &e),
                    None => {
                        let list = shared::parse_friends(&reply);
                        let status = if list.is_empty() { "No friends yet. Add some by gamertag on the Achievement Hub website." } else { "" };
                        set(Some(list), status);
                    }
                },
                Err(_) => set(None, "Couldn't reach the Achievement Hub website (offline?)."),
            }
            let since = Instant::now();
            while since.elapsed() < EVERY {
                std::thread::sleep(Duration::from_secs(2));
                if KICK.swap(false, Ordering::Relaxed) {
                    // Let a burst of unlocks settle first.
                    std::thread::sleep(Duration::from_secs(5));
                    break;
                }
            }
        }
    });
}
