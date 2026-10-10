//! "Request a game": sends the request to the Achievement Hub website, which
//! adds it to the public requests list (and counts it as your vote when this
//! computer is linked to an account).

use crate::friends::social;
use crate::library::profile_dir;
use serde::Serialize;

#[derive(Serialize, Clone, Default)]
pub struct Request {
    pub game: String,
    pub store: String,
    pub exe: String,
    pub anticheat: String,
    pub gamertag: String,
    pub note: String,
}

pub fn enabled() -> bool {
    true
}

/// POSTs JSON to the website's app API, as the linked account if there is
/// one. Ok(()) when it was accepted.
pub fn post(path: &str, body: &serde_json::Value) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let mut req = agent.post(&format!("{}{path}", social::site())).header("Content-Type", "application/json");
    let auth = social::load(&profile_dir()).map(|s| format!("Bearer {}", s.token));
    if let Some(a) = &auth {
        req = req.header("Authorization", a);
    }
    let mut resp = req.send(&body.to_string()).map_err(|e| format!("couldn't reach the Achievement Hub website: {e}"))?;
    let status = resp.status();
    let text = resp.body_mut().read_to_vec().unwrap_or_default();
    if status.is_success() {
        Ok(())
    } else {
        Err(social::error_in(&text).unwrap_or_else(|| format!("the website said {status}")))
    }
}

/// Sends the request; Ok(()) when the website accepted it.
pub fn send(req: &Request) -> Result<(), String> {
    post("/api/app/request", &serde_json::to_value(req).map_err(|e| e.to_string())?)
}
