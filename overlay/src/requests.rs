//! "Request a game" from inside the game: posts to the Achievement Hub
//! website (as the linked account, if the app is linked) with WinHTTP.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

pub struct Request {
    pub game: String,
    pub store: String,
    pub exe: String,
    pub gamertag: String,
    pub note: String,
}

/// Sends in the background; the receiver yields the result once. `profile`
/// is the profile folder (for the account link).
pub fn send(req: Request, profile: Option<PathBuf>) -> Receiver<Result<(), String>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let body = serde_json::json!({
            "game": req.game,
            "store": req.store,
            "exe": req.exe,
            "anticheat": "none detected (sent from the overlay)",
            "gamertag": req.gamertag,
            "note": req.note,
        })
        .to_string();
        let token = profile.and_then(|p| crate::social::shared::load(&p)).map(|s| s.token).unwrap_or_default();
        let url = format!("{}/api/app/request", crate::social::shared::site());
        let result = crate::storeinfo::call("POST", &url, Some(&body), &token).and_then(|reply| match crate::social::shared::error_in(&reply) {
            Some(e) => Err(e),
            None => Ok(()),
        });
        let _ = tx.send(result);
    });
    rx
}
