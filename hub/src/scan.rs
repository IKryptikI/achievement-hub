//! Finds installed games: Steam libraries and Heroic (GOG and Epic).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Store {
    Steam,
    Gog,
    Epic,
    Other,
}

impl Store {
    pub fn label(self) -> &'static str {
        match self {
            Store::Steam => "Steam",
            Store::Gog => "GOG",
            Store::Epic => "Epic",
            Store::Other => "Other",
        }
    }
}

/// An installed game, as found on disk.
#[derive(Clone, Debug)]
pub struct Found {
    pub name: String,
    pub store: Store,
    /// Steam app id, GOG product id or Epic app name.
    pub store_id: String,
    pub dir: PathBuf,
    /// What to start (a store launcher may be in front of the real exe).
    pub launch: Option<PathBuf>,
    /// The exe(s) the game actually runs as (what the overlay goes into).
    pub game_exes: Vec<PathBuf>,
    /// Portrait cover art, if available locally.
    pub cover: Option<PathBuf>,
    /// Where to download cover art from when there's none locally.
    pub cover_url: Option<String>,
    /// Heroic app id (GOG/Epic), for configuring Heroic.
    pub heroic_app: Option<String>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default()
}

/// Exe names that are never the game itself.
fn is_helper(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "unins", "setup", "redist", "vcredist", "dxsetup", "directx", "crash", "report", "launcher", "helper", "updater",
        "install", "easyanticheat", "battleye", "ue4prereq", "dotnet", "cefsharp", "webhelper", "7z", "python", "support",
        "ah-launch", "uninstall", "config", "settings", "patch", "gui", "original", "update", "editor", "tool",
        "server", "dedicated", "benchmark", "mge", "mwse", "skse", "f4se", "loader",
    ]
    .iter()
    .any(|h| n.contains(h))
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

/// Best guess at a game's main exe: among non-helper exes, prefer one named
/// like the game or its folder ("Morrowind.exe" for "...Morrowind"), else
/// the largest.
pub fn guess_exe(dir: &Path, game_name: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, depth: u32, out: &mut Vec<(PathBuf, u64)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth > 0 {
                    walk(&p, depth - 1, out);
                }
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe"))
                && !p.file_name().is_some_and(|n| is_helper(&n.to_string_lossy()))
            {
                out.push((p, e.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    let mut all = Vec::new();
    walk(dir, 4, &mut all);
    let names = [norm(game_name), dir.file_name().map(|n| norm(&n.to_string_lossy())).unwrap_or_default()];
    let named = |p: &Path| {
        let stem = p.file_stem().map(|n| norm(&n.to_string_lossy())).unwrap_or_default();
        stem.len() >= 3 && names.iter().any(|n| !n.is_empty() && (n.contains(&stem) || stem.contains(n.as_str())))
    };
    let pick = |c: Vec<&(PathBuf, u64)>| c.into_iter().max_by_key(|(_, s)| *s).map(|(p, _)| p.clone());
    pick(all.iter().filter(|(p, _)| named(p)).collect()).or_else(|| pick(all.iter().collect()))
}

// ── Steam ───────────────────────────────────────────────────────────────

/// Values of `"key" "value"` pairs in a VDF/ACF text file (flat lookup).
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let needle = format!("\"{key}\"");
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix(&needle)?.trim();
            Some(rest.trim_matches('"').replace("\\\\", "\\"))
        })
        .collect()
}

/// Windows registry values under `key` (`reg query`, so no extra crates):
/// (subkey, value name, data).
#[cfg(windows)]
fn reg_query(key: &str, recursive: bool) -> Vec<(String, String, String)> {
    use std::os::windows::process::CommandExt;
    let mut args = vec!["query", key];
    if recursive {
        args.push("/s");
    }
    let Ok(out) = std::process::Command::new("reg").args(&args).creation_flags(0x0800_0000).output() else { return Vec::new() };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut current = String::new();
    let mut rows = Vec::new();
    for line in text.lines() {
        if line.starts_with("HKEY_") {
            current = line.trim().to_string();
        } else if let Some((name, rest)) = line.trim().split_once("    REG_") {
            let data = rest.split_once("    ").map(|(_, d)| d.trim().to_string()).unwrap_or_default();
            rows.push((current.clone(), name.trim().to_string(), data));
        }
    }
    rows
}

fn steam_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let from_reg = reg_query(r"HKCU\Software\Valve\Steam", false).into_iter().find(|(_, n, _)| n == "SteamPath").map(|(_, _, d)| PathBuf::from(d));
        let default = std::env::var_os("ProgramFiles(x86)").map(|p| PathBuf::from(p).join("Steam"));
        return from_reg.into_iter().chain(default).find(|p| p.join("steamapps").is_dir());
    }
    #[allow(unreachable_code)]
    [home().join(".local/share/Steam"), home().join(".steam/steam")].into_iter().find(|p| p.join("steamapps").is_dir())
}

/// GOG Galaxy's installed games (Windows; from its registry entries).
fn galaxy_games() -> Vec<Found> {
    #[cfg(windows)]
    {
        let rows = reg_query(r"HKLM\SOFTWARE\WOW6432Node\GOG.com\Games", true);
        let mut by_key: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (k, n, d) in rows {
            by_key.entry(k).or_default().insert(n.to_lowercase(), d);
        }
        return by_key
            .into_values()
            .filter_map(|v| {
                let dir = PathBuf::from(v.get("path")?);
                let name = v.get("gamename").cloned().or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))?;
                let id = v.get("gameid").cloned().unwrap_or_default();
                let exe = v.get("exe").map(PathBuf::from).filter(|p| p.exists()).or_else(|| guess_exe(&dir, &name));
                dir.is_dir().then(|| Found {
                    name,
                    store: Store::Gog,
                    store_id: id.clone(),
                    dir,
                    launch: exe.clone(),
                    game_exes: exe.into_iter().collect(),
                    cover: None,
                    cover_url: Some(format!("gog:{id}")),
                    heroic_app: None,
                })
            })
            .collect();
    }
    #[allow(unreachable_code)]
    Vec::new()
}

/// Steam tools that aren't games.
fn steam_tool(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("proton") || n.starts_with("steam linux runtime") || n.starts_with("steamworks") || n.contains("redistributable")
}

fn steam_games() -> Vec<Found> {
    let Some(root) = steam_root() else { return Vec::new() };
    let text = fs::read_to_string(root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    let mut libs: Vec<PathBuf> = vdf_values(&text, "path").into_iter().map(PathBuf::from).collect();
    if libs.is_empty() {
        libs.push(root.clone());
    }
    let mut out = Vec::new();
    for lib in libs {
        let Ok(rd) = fs::read_dir(lib.join("steamapps")) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let fname = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if !(fname.starts_with("appmanifest_") && fname.ends_with(".acf")) {
                continue;
            }
            let acf = fs::read_to_string(&p).unwrap_or_default();
            let (Some(appid), Some(name), Some(installdir)) = (
                vdf_values(&acf, "appid").into_iter().next(),
                vdf_values(&acf, "name").into_iter().next(),
                vdf_values(&acf, "installdir").into_iter().next(),
            ) else {
                continue;
            };
            if steam_tool(&name) {
                continue;
            }
            let dir = lib.join("steamapps/common").join(&installdir);
            if !dir.is_dir() {
                continue;
            }
            let exe = guess_exe(&dir, &name);
            let cover = steam_cover(&root.join("appcache/librarycache").join(&appid));
            let cover_url = Some(format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{appid}/library_600x900.jpg"));
            out.push(Found {
                name,
                store: Store::Steam,
                store_id: appid,
                dir,
                launch: exe.clone(),
                game_exes: exe.into_iter().collect(),
                cover,
                cover_url,
                heroic_app: None,
            });
        }
    }
    out
}

// ── Heroic (GOG / Epic) ─────────────────────────────────────────────────

/// Heroic's config folder (Flatpak or native).
pub fn heroic_config() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("heroic"));
    [home().join(".var/app/com.heroicgameslauncher.hgl/config/heroic"), home().join(".config/heroic")]
        .into_iter()
        .chain(appdata)
        .find(|p| p.is_dir())
}

#[derive(Deserialize)]
struct GogInstalled {
    installed: Vec<GogEntry>,
}

#[derive(Deserialize)]
struct GogEntry {
    #[serde(rename = "appName")]
    app_name: String,
    install_path: String,
    #[serde(default)]
    is_dlc: bool,
}

#[derive(Deserialize)]
struct GogInfo {
    name: String,
    #[serde(rename = "playTasks", default)]
    play_tasks: Vec<GogTask>,
}

#[derive(Deserialize)]
struct GogTask {
    #[serde(default)]
    category: String,
    #[serde(rename = "isPrimary", default)]
    is_primary: bool,
    #[serde(default)]
    path: String,
}

fn gog_games(cfg: &Path) -> Vec<Found> {
    let Ok(text) = fs::read_to_string(cfg.join("gog_store/installed.json")) else { return Vec::new() };
    let Ok(list) = serde_json::from_str::<GogInstalled>(&text) else { return Vec::new() };
    list.installed
        .into_iter()
        .filter(|g| !g.is_dlc && g.app_name != "gog-redist")
        .filter_map(|g| {
            let dir = PathBuf::from(&g.install_path);
            let info: Option<GogInfo> = fs::read_to_string(dir.join(format!("goggame-{}.info", g.app_name)))
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            let name = info.as_ref().map(|i| i.name.clone()).or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))?;
            let tasks = info.map(|i| i.play_tasks).unwrap_or_default();
            let primary = tasks.iter().find(|t| t.is_primary && !t.path.is_empty()).map(|t| dir.join(&t.path));
            let mut game_exes: Vec<PathBuf> =
                tasks.iter().filter(|t| t.category == "game" && !t.path.is_empty()).map(|t| dir.join(&t.path)).collect();
            if game_exes.is_empty() {
                game_exes.extend(primary.clone().or_else(|| guess_exe(&dir, &name)));
            }
            let launch = primary.or_else(|| game_exes.first().cloned());
            let cover = Some(dir.join(format!("goggame-{}.ico", g.app_name))).filter(|p| p.exists());
            let cover_url = Some(format!("gog:{}", g.app_name));
            Some(Found {
                name,
                store: Store::Gog,
                store_id: g.app_name.clone(),
                dir,
                launch,
                game_exes,
                cover,
                cover_url,
                heroic_app: Some(g.app_name),
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct EpicEntry {
    title: String,
    install_path: String,
    #[serde(default)]
    executable: String,
    #[serde(default)]
    is_dlc: bool,
}

fn epic_games(cfg: &Path) -> Vec<Found> {
    let Ok(text) = fs::read_to_string(cfg.join("legendaryConfig/legendary/installed.json")) else { return Vec::new() };
    let Ok(map) = serde_json::from_str::<HashMap<String, EpicEntry>>(&text) else { return Vec::new() };
    map.into_iter()
        .filter(|(_, e)| !e.is_dlc)
        .map(|(app, e)| {
            let dir = PathBuf::from(&e.install_path);
            let exe = Some(dir.join(&e.executable)).filter(|p| !e.executable.is_empty() && p.exists()).or_else(|| guess_exe(&dir, &e.title));
            Found {
                name: e.title,
                store: Store::Epic,
                store_id: app.clone(),
                dir,
                launch: exe.clone(),
                game_exes: exe.into_iter().collect(),
                cover: None,
                cover_url: None,
                heroic_app: Some(app),
            }
        })
        .collect()
}

/// Every installed game we can find, sorted by name.
pub fn installed() -> Vec<Found> {
    let mut all = steam_games();
    if let Some(cfg) = heroic_config() {
        all.extend(gog_games(&cfg));
        all.extend(epic_games(&cfg));
    }
    // GOG Galaxy (Windows), unless Heroic already listed the same folder.
    for g in galaxy_games() {
        if !all.iter().any(|a| a.dir == g.dir) {
            all.push(g);
        }
    }
    all.sort_by_key(|g| g.name.to_lowercase());
    all
}

/// A game added by picking its exe directly.
pub fn from_exe(exe: &Path) -> Found {
    let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    // Name it after the first folder up that isn't a generic build folder
    // ("Binaries/Win32" in Unreal games and the like).
    const GENERIC: [&str; 12] = ["binaries", "binary", "bin", "win32", "win64", "x86", "x64", "system", "game", "exe", "release", "retail"];
    let name = dir
        .ancestors()
        .filter_map(|a| a.file_name().map(|n| n.to_string_lossy().into_owned()))
        .find(|n| !GENERIC.contains(&n.to_lowercase().as_str()))
        .unwrap_or_else(|| "Game".into());
    Found {
        name,
        store: Store::Other,
        store_id: String::new(),
        dir,
        launch: Some(exe.to_path_buf()),
        game_exes: vec![exe.to_path_buf()],
        cover: None,
        cover_url: None,
        heroic_app: None,
    }
}

/// Steam's cached portrait art for an app: in the app's folder, or (newer
/// clients) in a hashed subfolder.
fn steam_cover(dir: &Path) -> Option<PathBuf> {
    let direct = dir.join("library_600x900.jpg");
    if direct.exists() {
        return Some(direct);
    }
    fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).map(|p| p.join("library_600x900.jpg")).find(|p| p.exists())
}
