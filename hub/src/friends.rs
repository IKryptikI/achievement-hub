//! The desktop app's link to the player's website account: linking with a
//! code, publishing their profile, and fetching friends (common/social.rs).

#[path = "../../common/social.rs"]
#[allow(dead_code)]
pub mod social;

use crate::library::profile_dir;
pub use social::{Profile, Social};

fn read(mut resp: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>, String> {
    let status = resp.status();
    let body = resp.body_mut().with_config().limit(8 << 20).read_to_vec().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(social::error_in(&body).unwrap_or_else(|| format!("the website said {status}")));
    }
    Ok(body)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).build().into()
}

const OFFLINE: &str = "couldn't reach the Achievement Hub website";

/// Links this computer to the account the code came from.
pub fn link(code: &str) -> Result<Social, String> {
    let body = serde_json::json!({ "code": code.trim() }).to_string();
    let resp = agent().post(&social::link_url()).header("Content-Type", "application/json").send(&body).map_err(|e| format!("{OFFLINE}: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&read(resp)?).map_err(|e| e.to_string())?;
    let s = Social { token: v["token"].as_str().unwrap_or_default().to_string(), gamertag: v["gamertag"].as_str().unwrap_or_default().to_string() };
    if s.token.is_empty() {
        return Err("the website didn't send a link".into());
    }
    social::save(&profile_dir(), &s);
    use_gamertag(&s.gamertag);
    Ok(s)
}

/// Shows the account's gamertag in the overlay too (profile.toml).
pub fn use_gamertag(tag: &str) {
    if tag.is_empty() {
        return;
    }
    let path = profile_dir().join("profile.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with("gamertag = ")).map(String::from).collect();
    lines.insert(0, format!("gamertag = {tag:?}"));
    let _ = std::fs::write(path, lines.join("\n") + "\n");
}

/// Sets a new website password (the linked app proves it's the owner).
pub fn reset_password(s: &Social, new_password: &str) -> Result<(), String> {
    let body = serde_json::json!({ "new_password": new_password }).to_string();
    let resp = agent()
        .post(&format!("{}/api/app/reset-password", social::site()))
        .header("Content-Type", "application/json")
        .header("Authorization", &format!("Bearer {}", s.token))
        .send(&body)
        .map_err(|e| format!("{OFFLINE}: {e}"))?;
    read(resp).map(|_| ())
}

pub fn unlink(s: &Social) {
    let _ = agent().post(&social::unlink_url()).header("Authorization", &format!("Bearer {}", s.token)).send_empty();
    social::forget(&profile_dir());
}

/// Publishes the current profile. Returns the account's gamertag.
pub fn publish(s: &Social) -> Result<String, String> {
    let p = social::snapshot(&profile_dir(), None);
    let body = serde_json::to_string(&p).map_err(|e| e.to_string())?;
    let resp = agent()
        .put(&social::progress_url())
        .header("Content-Type", "application/json")
        .header("Authorization", &format!("Bearer {}", s.token))
        .send(&body)
        .map_err(|e| format!("{OFFLINE}: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&read(resp)?).map_err(|e| e.to_string())?;
    Ok(v["gamertag"].as_str().unwrap_or(&s.gamertag).to_string())
}

/// Friends' profiles, and your own look (gamer picture etc.).
pub fn fetch(s: &Social) -> Result<(Vec<Profile>, Option<social::Look>), String> {
    let resp = agent().get(&social::friends_url()).header("Authorization", &format!("Bearer {}", s.token)).call().map_err(|e| format!("{OFFLINE}: {e}"))?;
    let body = read(resp)?;
    Ok((social::parse_friends(&body), social::parse_me(&body)))
}

/// A player's uploaded gamer picture, from the shared cache (downloading it
/// the first time), ready to show. None if they have none.
pub fn picture(gamertag: &str, avatar: &str) -> Option<eframe::egui::ColorImage> {
    let (path, url) = social::picture(&profile_dir(), gamertag, avatar)?;
    if !path.exists() {
        let mut resp = agent().get(&url).call().ok()?;
        let bytes = resp.body_mut().with_config().limit(1 << 20).read_to_vec().ok()?;
        social::save_picture(&path, &bytes).ok()?;
    }
    let img = image::open(&path).ok()?.resize_to_fill(96, 96, image::imageops::FilterType::Triangle).into_rgba8();
    Some(eframe::egui::ColorImage::from_rgba_unmultiplied([96, 96], img.as_raw()))
}
