//! A game's store page details (description, developer, publisher, year,
//! artwork), fetched from the store it came from, the way launchers show
//! them. Cached in the profile (games/<id>.store.toml, <id>-art.jpg plus
//! more screenshots as <id>-art-<n>.jpg) and refreshed monthly.
//!
//! GOG and Steam have public APIs (their screenshots cycle in the Games
//! pane). Games with neither (never sold there, or delisted) get their
//! description from Wikipedia (credited) and developer, publisher, year and
//! box art from the LaunchBox games database; their pictures are the
//! player's own unlock screenshots. (LaunchBox only serves thumbnails, and
//! its wide art is often fan-made.)

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use windows_sys::Win32::Networking::WinHttp::*;

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct StoreInfo {
    pub description: String,
    pub developer: String,
    pub publisher: String,
    pub year: Option<u16>,
    /// Wide artwork for the Games pane.
    #[serde(default)]
    pub art_url: String,
    /// Box art (portrait), for list thumbnails.
    #[serde(default)]
    pub cover_url: String,
    /// More screenshots to cycle through after `art_url`.
    #[serde(default)]
    pub shots: Vec<String>,
    /// Where the description came from, when it needs crediting.
    #[serde(default)]
    pub description_source: String,
    /// When this was fetched (unix seconds).
    pub fetched: u64,
    /// What fetched it (older caches are refreshed).
    #[serde(default)]
    pub version: u32,
}

const MONTH: u64 = 30 * 86_400;
const VERSION: u32 = 4;
/// Most screenshots kept per game.
pub const MAX_SHOTS: usize = 8;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn cache_path(profile: &Path, game: &str) -> PathBuf {
    profile.join("games").join(format!("{game}.store.toml"))
}

pub fn art_path(profile: &Path, game: &str) -> PathBuf {
    profile.join("games").join(format!("{game}-art.jpg"))
}

/// Screenshot `n` (0 is the main art, `art_path`).
pub fn shot_path(profile: &Path, game: &str, n: usize) -> PathBuf {
    if n == 0 { art_path(profile, game) } else { profile.join("games").join(format!("{game}-art-{n}.jpg")) }
}

/// Downloads the art, extra screenshots and box art for fetched details.
pub fn download_art(profile: &Path, game: &str, info: &StoreInfo) {
    let urls: Vec<&String> = std::iter::once(&info.art_url).chain(info.shots.iter()).filter(|u| !u.is_empty()).take(MAX_SHOTS).collect();
    for (n, url) in urls.iter().enumerate() {
        if let Ok(bytes) = get(url) {
            let _ = std::fs::write(shot_path(profile, game, n), bytes);
        }
    }
    // Drop leftovers from an earlier, longer set.
    for n in urls.len().max(1)..MAX_SHOTS + 4 {
        let _ = std::fs::remove_file(shot_path(profile, game, n));
    }
    if !info.cover_url.is_empty() {
        if let Ok(bytes) = get(&info.cover_url) {
            let _ = std::fs::write(cover_path(profile, game), bytes);
        }
    }
}

/// The cached details, if any.
pub fn cached(profile: &Path, game: &str) -> Option<StoreInfo> {
    toml::from_str(&std::fs::read_to_string(cache_path(profile, game)).ok()?).ok()
}

pub fn is_fresh(info: &StoreInfo) -> bool {
    info.version == VERSION && now().saturating_sub(info.fetched) < MONTH
}

pub fn cover_path(profile: &Path, game: &str) -> PathBuf {
    profile.join("games").join(format!("{game}-cover.jpg"))
}

/// The store's id for the running game: GOG's product id (goggame-<id>.info)
/// or Steam's app id (steam_appid.txt), next to the exe or one folder up.
fn store_id(store: &str) -> Option<String> {
    // (The UI preview tool passes one in.)
    if let Ok(id) = std::env::var("AH_STORE_ID") {
        return Some(id);
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    if store == "Steam" {
        if let Some(id) = steam_appid_from_manifest(&dir) {
            return Some(id);
        }
    }
    let dirs = [Some(dir.clone()), dir.parent().map(Path::to_path_buf)];
    for d in dirs.iter().flatten() {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            match store {
                "GOG" => {
                    if let Some(id) = name.strip_prefix("goggame-").and_then(|n| n.strip_suffix(".info")) {
                        return Some(id.to_string());
                    }
                }
                "Steam" if name == "steam_appid.txt" => {
                    return std::fs::read_to_string(e.path()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
                }
                _ => {}
            }
        }
    }
    None
}

/// A Steam game's app id from Steam's install record: the
/// appmanifest_<id>.acf in <library>/steamapps whose installdir is the
/// folder the game is in (every Steam game has one; steam_appid.txt is rare).
fn steam_appid_from_manifest(game_dir: &Path) -> Option<String> {
    let mut common_child = game_dir;
    let steamapps = loop {
        let parent = common_child.parent()?;
        if parent.file_name().is_some_and(|n| n.eq_ignore_ascii_case("common")) {
            break parent.parent()?;
        }
        common_child = parent;
    };
    let installdir = common_child.file_name()?.to_string_lossy().to_lowercase();
    for e in std::fs::read_dir(steamapps).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("appmanifest_").and_then(|n| n.strip_suffix(".acf")) else { continue };
        let text = std::fs::read_to_string(e.path()).unwrap_or_default();
        let dir_matches = text.lines().any(|l| {
            let l = l.trim();
            l.starts_with("\"installdir\"") && l.trim_start_matches("\"installdir\"").trim().trim_matches('"').to_lowercase() == installdir
        });
        if dir_matches {
            return Some(id.to_string());
        }
    }
    None
}

/// Fetches a game's details (blocking): from its store when it's the
/// running game and the store knows it, otherwise by title.
/// `year` (when known) tells remakes and re-releases apart.
pub fn fetch(store: &str, title: &str, year: Option<u16>, running: bool) -> Result<StoreInfo, String> {
    let id = if running { store_id(store) } else { None };
    let from_store = match (store, id) {
        ("GOG", Some(id)) => get(&format!("https://api.gog.com/v2/games/{id}?locale=en-US")).and_then(|b| parse_gog(&b)),
        ("Steam", Some(id)) => get(&format!("https://store.steampowered.com/api/appdetails?appids={id}&l=english")).and_then(|b| parse_steam(&b, &id)),
        _ => Err("no store page".into()),
    };
    let mut info = match from_store {
        Ok(i) => i,
        // Not on a store (or delisted): the game's facts and screenshots
        // from LaunchBox, its description from Wikipedia.
        Err(_) => {
            let mut i = launchbox(title, year).or_else(|_| steam_by_title(title))?;
            if let Some(d) = wikipedia(title) {
                i.description = d;
                i.description_source = "Wikipedia".into();
            }
            i
        }
    };
    info.fetched = now();
    info.version = VERSION;
    Ok(info)
}

/// Lowercase letters and digits only, for comparing titles.
fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Text between `start` and the following `end`, from `from` onward.
fn between<'a>(t: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = t.find(start)? + start.len();
    let j = t[i..].find(end)? + i;
    Some(&t[i..j])
}

fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ").replace("&amp;", "&").replace("&#39;", "'")
}

/// A Steam store game with exactly this title (for copies bought
/// elsewhere).
fn steam_by_title(title: &str) -> Result<StoreInfo, String> {
    let body = get(&format!("https://store.steampowered.com/api/storesearch/?term={}&cc=us&l=english", urlencode(title)))?;
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let item = v["items"].as_array().and_then(|a| a.iter().find(|i| i["name"].as_str().is_some_and(|n| norm(n) == norm(title)))).ok_or("not on Steam")?;
    let id = item["id"].as_u64().ok_or("not on Steam")?.to_string();
    parse_steam(&get(&format!("https://store.steampowered.com/api/appdetails?appids={id}&l=english"))?, &id)
}

/// The LaunchBox games database's entry for a PC game: developer,
/// publisher, year and box art (not its art or community-written overview).
fn launchbox(title: &str, year: Option<u16>) -> Result<StoreInfo, String> {
    let page = String::from_utf8_lossy(&get(&format!("https://gamesdb.launchbox-app.com/games/results/{}", urlencode(title)))?).into_owned();
    // Result cards: link, then <h3>title</h3> and <p>platform</p>.
    let cards: Vec<(String, String, String)> = page
        .split("href=\"/games/details/")
        .skip(1)
        .filter_map(|c| {
            let link = c.split('"').next()?.to_string();
            let name = strip_tags(between(c, "<h3", "</h3>")?.split_once('>')?.1);
            let platform = strip_tags(between(c, "<p", "</p>")?.split_once('>')?.1);
            Some((link, name, platform))
        })
        .collect();
    let want = norm(title);
    // Exact title, PC versions first; with a year, the one released then
    // (not a remake of the same name).
    let mut candidates: Vec<&(String, String, String)> = cards.iter().filter(|(_, n, p)| norm(n) == want && p == "Windows").collect();
    candidates.extend(cards.iter().filter(|(_, n, p)| norm(n) == want && p != "Windows"));
    let field_of = |page: &str, label: &str| -> String { between(page, &format!("{label} </dt>"), "</dd>").map(strip_tags).unwrap_or_default() };
    let year_of = |page: &str| -> Option<u16> { field_of(page, "Release Date").split(|c: char| !c.is_ascii_digit()).filter(|p| p.len() == 4).last().and_then(|y| y.parse().ok()) };
    let mut found = None;
    for (link, _, _) in candidates.iter().take(4) {
        let page = String::from_utf8_lossy(&get(&format!("https://gamesdb.launchbox-app.com/games/details/{link}"))?).into_owned();
        let y = year_of(&page);
        if year.is_none() || y.zip(year).is_some_and(|(a, b)| a.abs_diff(b) <= 1) {
            found = Some(page);
            break;
        }
    }
    let page = found.ok_or("not in the LaunchBox database")?;
    let field = |label: &str| field_of(&page, label);
    let first = |s: String| s.split(',').next().unwrap_or_default().trim().to_string();
    let year = year_of(&page);
    // Box art: <img src="URL" ... alt="Title - Box - Front (Region) - WxH">.
    let cover = page.split("<img src=\"").skip(1).find_map(|part| {
        let url = part.split('"').next()?;
        let alt = between(part, "alt=\"", "\"")?;
        (alt.contains("- Box - Front (North America)") || alt.contains("- Box - Front (World)")).then(|| url.replace(".com//", ".com/"))
    });
    Ok(StoreInfo {
        description: String::new(),
        developer: first(field("Developers")),
        publisher: first(field("Publishers")),
        year,
        art_url: String::new(),
        cover_url: cover.unwrap_or_default(),
        shots: Vec::new(),
        description_source: String::new(),
        fetched: 0,
        version: 0,
    })
}

/// The opening of a game's English Wikipedia article (its exact title or
/// "<title> (video game)", else a search result with a matching title).
fn wikipedia(title: &str) -> Option<String> {
    let summary = |page: &str| -> Option<String> {
        let body = get(&format!("https://en.wikipedia.org/api/rest_v1/page/summary/{}", urlencode(&page.replace(' ', "_")))).ok()?;
        let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let is_game = v["description"].as_str().is_some_and(|d| d.to_lowercase().contains("video game"));
        (v["type"] == "standard" && is_game).then(|| v["extract"].as_str().unwrap_or_default().to_string()).filter(|e| e.len() > 40)
    };
    if let Some(s) = summary(title).or_else(|| summary(&format!("{title} (video game)"))) {
        return Some(s);
    }
    let body = get(&format!("https://en.wikipedia.org/w/api.php?action=query&list=search&format=json&srlimit=5&srsearch={}", urlencode(&format!("{title} video game")))).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let want = norm(title);
    v["query"]["search"].as_array()?.iter().filter_map(|r| r["title"].as_str()).filter(|t| norm(t).starts_with(&want)).find_map(summary)
}

fn parse_gog(body: &[u8]) -> Result<StoreInfo, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let e = &v["_embedded"];
    let developer = e["developers"].as_array().and_then(|d| d.first()).and_then(|d| d["name"].as_str()).unwrap_or_default().to_string();
    let publisher = e["publisher"]["name"].as_str().unwrap_or_default().to_string();
    let year = e["product"]["globalReleaseDate"].as_str().and_then(|d| d.get(..4)).and_then(|y| y.parse().ok());
    let description = first_paragraph(v["description"].as_str().unwrap_or_default());
    // Galaxy's background is the crisp one (the store's fades into grey).
    let link = |k: &str| v["_links"][k]["href"].as_str().unwrap_or_default().to_string();
    let art_url = Some(link("galaxyBackgroundImage")).filter(|s| !s.is_empty()).unwrap_or_else(|| link("backgroundImage"));
    let shots = e["screenshots"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s["_links"]["self"]["href"].as_str().or(s["formatterTemplateHref"].as_str())).map(|u| u.replace("_{formatter}", "")).take(MAX_SHOTS - 1).collect())
        .unwrap_or_default();
    Ok(StoreInfo { description, developer, publisher, year, art_url, cover_url: link("boxArtImage"), shots, description_source: String::new(), fetched: 0, version: 0 })
}

fn parse_steam(body: &[u8], id: &str) -> Result<StoreInfo, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let d = &v[id]["data"];
    if d.is_null() {
        return Err("Steam has no details for this app".into());
    }
    let first = |k: &str| d[k].as_array().and_then(|a| a.first()).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let year = d["release_date"]["date"].as_str().and_then(|s| s.split(|c: char| !c.is_ascii_digit()).filter(|p| p.len() == 4).last()).and_then(|y| y.parse().ok());
    Ok(StoreInfo {
        description: first_paragraph(d["about_the_game"].as_str().or(d["short_description"].as_str()).unwrap_or_default()),
        developer: first("developers"),
        publisher: first("publishers"),
        year,
        // A real in-game shot (the header image is mostly logo).
        art_url: d["screenshots"][0]["path_full"].as_str().or(d["header_image"].as_str()).unwrap_or_default().to_string(),
        cover_url: format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{id}/library_600x900.jpg"),
        shots: d["screenshots"].as_array().map(|a| a.iter().skip(1).take(MAX_SHOTS - 1).filter_map(|s| s["path_full"].as_str().map(String::from)).collect()).unwrap_or_default(),
        description_source: String::new(),
        fetched: 0,
        version: 0,
    })
}

/// The first readable paragraph of a store description: HTML stripped,
/// entities decoded, non-Latin paragraphs (other languages) skipped, and
/// symbols our fonts lack dropped.
fn first_paragraph(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for c in html.chars() {
        match (in_tag, c) {
            (false, '<') => {
                in_tag = true;
                tag.clear();
            }
            (true, '>') => {
                in_tag = false;
                let t = tag.to_lowercase();
                if t.starts_with("br") || t.starts_with("/p") || t.starts_with("p") || t.starts_with("/h") || t.starts_with("/div") || t.starts_with("li") {
                    text.push_str("\n\n");
                }
            }
            (true, c) => tag.push(c),
            (false, c) => text.push(c),
        }
    }
    let text = text
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace(['™', '®', '©'], "");
    let latin = |p: &str| p.chars().filter(|c| c.is_alphabetic()).all(|c| (c as u32) < 0x0250);
    let paragraphs: Vec<String> = text
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| p.len() > 40 && latin(p))
        // Store notices, not the game.
        .filter(|p| !p.to_lowercase().starts_with("publisher update") && !p.to_lowercase().contains("this is the product page"))
        .collect();
    // One paragraph, or two if the first is short.
    let mut out = String::new();
    for p in paragraphs {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&p);
        if out.len() >= 160 {
            break;
        }
    }
    if out.len() > 600 {
        let cut = out[..600].rfind(". ").map(|i| i + 1).unwrap_or(600);
        out.truncate(cut);
    }
    // A sentence introducing a list we dropped ("...includes:") goes too.
    if out.trim_end().ends_with(':') {
        match out.trim_end().rfind(". ") {
            Some(i) => out.truncate(i + 1),
            None => {
                out = out.trim_end().trim_end_matches(':').to_string();
                out.push('.');
            }
        }
    }
    out
}

/// Downloads a Steam app's achievement icons from its public achievements
/// page into `dir` as <our id>.jpg, matching by name. Returns how many.
pub fn fetch_steam_icons(appid: &str, wanted: &[(String, String)], dir: &Path) -> Result<usize, String> {
    let page = get(&format!("https://steamcommunity.com/stats/{appid}/achievements?l=english"))?;
    let page = String::from_utf8_lossy(&page);
    let _ = std::fs::create_dir_all(dir);
    let mut got = 0;
    // Each entry: <div class="achieveRow ..."> ... <img src="URL"> ... <h3>NAME</h3>
    for row in page.split("<div class=\"achieveRow").skip(1) {
        let Some(img) = row.split("<img src=\"").nth(1).and_then(|r| r.split('"').next()) else { continue };
        let Some(name) = row.split("<h3>").nth(1).and_then(|r| r.split("</h3>").next()) else { continue };
        let name = name.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"");
        for (id, _) in wanted.iter().filter(|(_, n)| *n == name) {
            let path = dir.join(format!("{id}.jpg"));
            if path.exists() {
                got += 1;
                continue;
            }
            if let Ok(bytes) = get(img) {
                if std::fs::write(&path, bytes).is_ok() {
                    got += 1;
                }
            }
        }
    }
    Ok(got)
}

/// The same from an Exophase achievement page (for console-only lists):
/// each award's name and its image, taken at the large size.
pub fn fetch_exophase_icons(slug: &str, wanted: &[(String, String)], dir: &Path) -> Result<usize, String> {
    let page = get(&format!("https://www.exophase.com/game/{slug}/achievements/"))?;
    let page = String::from_utf8_lossy(&page);
    let _ = std::fs::create_dir_all(dir);
    let mut got = 0;
    for item in page.split("<li").skip(1).filter(|i| i.split('>').next().is_some_and(|h| h.contains("award"))) {
        let Some(img) = item.split("src=\"").find(|p| p.contains("/awards/")).and_then(|p| p.split('"').next()) else { continue };
        let Some(name) = item.split("<a").nth(1).and_then(|a| a.split('>').nth(1)).and_then(|a| a.split('<').next()) else { continue };
        let name = name.trim().replace("&amp;", "&").replace("&#039;", "'").replace("&#39;", "'").replace("&quot;", "\"");
        let img = img.replace("/s/", "/l/");
        for (id, _) in wanted.iter().filter(|(_, n)| *n == name) {
            let path = dir.join(format!("{id}.jpg"));
            if path.exists() || get(&img).ok().and_then(|b| std::fs::write(&path, b).ok()).is_some() {
                got += 1;
            }
        }
    }
    Ok(got)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// HTTPS GET with WinHTTP (follows redirects), returning the body.
pub fn get(url: &str) -> Result<Vec<u8>, String> {
    request("GET", url, None, "")
}

/// An authorized call with an optional JSON body (the Achievement Hub
/// website API). `auth` is the bearer token.
pub fn call(method: &str, url: &str, body: Option<&str>, auth: &str) -> Result<Vec<u8>, String> {
    request(method, url, body, &format!("Authorization: Bearer {auth}\r\n"))
}

fn request(method: &str, url: &str, body: Option<&str>, extra_headers: &str) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("https only")?;
    let (host, path) = rest.split_once('/').map(|(h, p)| (h, format!("/{p}"))).unwrap_or((rest, "/".into()));
    // SAFETY: WinHTTP calls with valid NUL-terminated strings; every handle
    // is closed before returning.
    unsafe {
        let session = WinHttpOpen(wide("AchievementHub/1.0").as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, std::ptr::null(), std::ptr::null(), 0);
        if session.is_null() {
            return Err("couldn't start WinHTTP".into());
        }
        let connect = WinHttpConnect(session, wide(host).as_ptr(), INTERNET_DEFAULT_HTTPS_PORT as u16, 0);
        let request = if connect.is_null() {
            std::ptr::null_mut()
        } else {
            WinHttpOpenRequest(connect, wide(method).as_ptr(), wide(&path).as_ptr(), std::ptr::null(), std::ptr::null(), std::ptr::null(), WINHTTP_FLAG_SECURE)
        };
        let result = (|| {
            if request.is_null() {
                return Err("couldn't reach the store".to_string());
            }
            let headers = wide(&format!("{}{extra_headers}", if body.is_some() { "Content-Type: application/json\r\n" } else { "" }));
            let (hdr, hdr_len) = if headers.len() > 1 { (headers.as_ptr(), u32::MAX) } else { (std::ptr::null(), 0) };
            let data = body.unwrap_or_default().as_bytes();
            let sent = WinHttpSendRequest(request, hdr, hdr_len, data.as_ptr().cast(), data.len() as u32, data.len() as u32, 0);
            if sent == 0 || WinHttpReceiveResponse(request, std::ptr::null_mut()) == 0 {
                return Err("couldn't reach the store (offline?)".to_string());
            }
            let mut body = Vec::new();
            loop {
                let mut buf = [0u8; 16384];
                let mut n = 0u32;
                if WinHttpReadData(request, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n) == 0 || n == 0 {
                    break;
                }
                body.extend_from_slice(&buf[..n as usize]);
            }
            Ok(body)
        })();
        if !request.is_null() {
            WinHttpCloseHandle(request);
        }
        if !connect.is_null() {
            WinHttpCloseHandle(connect);
        }
        WinHttpCloseHandle(session);
        result
    }
}

#[cfg(test)]
mod tests {
    /// Network: `cargo test -- --ignored lookups --nocapture` (under Wine).
    #[test]
    #[ignore]
    fn lookups() {
        for title in ["Silent Hill 3", "Silent Hill 2", "Transformers: The Game", "Saw", "Spider-Man: Shattered Dimensions"] {
            match super::fetch("PC", title, if title == "Silent Hill 2" { Some(2002) } else { None }, false) {
                Ok(i) => println!(
                    "{title}: {} / {} / {:?}; {} shots; cover {}; desc ({}) {}",
                    i.developer,
                    i.publisher,
                    i.year,
                    1 + i.shots.len(),
                    !i.cover_url.is_empty(),
                    i.description_source,
                    i.description.chars().take(90).collect::<String>()
                ),
                Err(e) => println!("{title}: ERROR {e}"),
            }
        }
    }
}

#[cfg(test)]
mod prefetch {
    /// Fills the profile's store cache for one game, as the overlay would:
    /// AH_PROFILE=<dir> AH_GAME=<id> AH_TITLE=.. AH_YEAR=.. cargo test -- --ignored prefetch
    #[test]
    #[ignore]
    fn prefetch() {
        let dir = std::path::PathBuf::from(std::env::var("AH_PROFILE").unwrap());
        let (id, title) = (std::env::var("AH_GAME").unwrap(), std::env::var("AH_TITLE").unwrap());
        let year = std::env::var("AH_YEAR").ok().and_then(|y| y.parse().ok());
        let store = std::env::var("AH_STORE").unwrap_or("PC".into());
        let info = super::fetch(&store, &title, year, std::env::var("AH_STORE_ID").is_ok()).unwrap();
        std::fs::write(super::cache_path(&dir, &id), toml::to_string(&info).unwrap()).unwrap();
        super::download_art(&dir, &id, &info);
        println!("{title}: {} shots", 1 + info.shots.len());
    }
}
