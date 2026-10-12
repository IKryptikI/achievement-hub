//! The player's mapped games (library.toml in the profile folder) and
//! installing the launcher + overlay into a game.

use crate::catalogue::{self, Definition};
use crate::scan::{Found, Store};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// The shared profile folder, same as the overlay's:
/// ~/Documents/My Games/Achievement Hub (Documents on Windows too).
pub fn profile_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    let dir = home.join("Documents").join("My Games").join("Achievement Hub");
    let _ = fs::create_dir_all(&dir);
    dir
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Mapped {
    pub name: String,
    pub store: Store,
    pub store_id: String,
    pub dir: PathBuf,
    pub launch: Option<PathBuf>,
    pub game_exes: Vec<PathBuf>,
    pub heroic_app: Option<String>,
    pub added: u64,
    /// The overlay has been installed into the game folder.
    #[serde(default)]
    pub overlay_installed: bool,
}

impl Mapped {
    pub fn from_found(f: &Found) -> Mapped {
        Mapped {
            name: f.name.clone(),
            store: f.store,
            store_id: f.store_id.clone(),
            dir: f.dir.clone(),
            launch: f.launch.clone(),
            game_exes: f.game_exes.clone(),
            heroic_app: f.heroic_app.clone(),
            added: now(),
            overlay_installed: false,
        }
    }

    /// Same game? (store + id, or same folder for manually added games)
    pub fn same(&self, f: &Found) -> bool {
        if self.store == f.store && !self.store_id.is_empty() {
            self.store_id == f.store_id
        } else {
            self.dir == f.dir
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Serialize, Deserialize, Default)]
pub struct Library {
    #[serde(default)]
    pub game: Vec<Mapped>,
}

impl Library {
    fn path() -> PathBuf {
        profile_dir().join("library.toml")
    }

    pub fn load() -> Library {
        fs::read_to_string(Self::path()).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = fs::write(Self::path(), text);
        }
    }

    pub fn contains(&self, f: &Found) -> bool {
        self.game.iter().any(|g| g.same(f))
    }
}

/// Where ah-launch.exe and overlay.dll come from: a `payload` folder next to
/// the hub, or the build output when running from the source tree.
fn payload_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let candidates = [
        exe_dir.join("payload"),
        exe_dir.join("../i686-pc-windows-gnu/release"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/i686-pc-windows-gnu/release"),
    ];
    candidates.into_iter().find(|d| d.join("overlay.dll").exists() && d.join("version.dll").exists())
}

/// Copies `src` to `dst` via a temporary file, so a running game keeps the
/// copy it already loaded.
fn swap_copy(src: &Path, dst: &Path) -> std::io::Result<()> {
    let tmp = dst.with_extension("new");
    fs::copy(src, &tmp)?;
    // (Windows won't replace a DLL a running game has loaded: leave it be.)
    fs::rename(&tmp, dst).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Installs the launcher, overlay, achievement definitions and settings into
/// a game folder. Returns a message for the player.
/// Sandboxed (Flatpak) launchers can't see ~/Documents by default, so the
/// overlay inside their games couldn't reach the profile. Grant each one
/// that's installed access to just the profile folder (user-level setting).
fn share_profile_with_flatpaks() {
    if cfg!(windows) {
        return;
    }
    let profile = profile_dir();
    for app in ["com.usebottles.bottles", "com.heroicgameslauncher.hgl", "net.lutris.Lutris", "com.valvesoftware.Steam"] {
        let installed = std::process::Command::new("flatpak").args(["info", app]).output().is_ok_and(|o| o.status.success());
        if installed {
            let _ = std::process::Command::new("flatpak")
                .args(["override", "--user", &format!("--filesystem={}", profile.display()), app])
                .status();
        }
    }
}

/// The overlay's file name when installed as a mod-loader plugin.
const ASI_NAME: &str = "AchievementHub.asi";

/// DLLs we can stand in for, in order of preference (see proxy/).
const PROXIES: [&str; 3] = ["version.dll", "winmm.dll", "dinput8.dll"];

/// Is the file at `p` one of our stand-in DLLs?
fn is_our_proxy(p: &Path) -> bool {
    fs::read(p).is_ok_and(|d| d.windows(23).any(|w| w == b"AchievementHub-proxy-v1"))
}

/// The game's Wine prefix (Linux), if we can find it.
fn wine_prefix(game: &Mapped) -> Option<PathBuf> {
    match game.store {
        Store::Steam => {
            // <library>/steamapps/common/<dir> -> <library>/steamapps/compatdata/<appid>/pfx
            let steamapps = game.dir.ancestors().find(|a| a.file_name().is_some_and(|n| n == "steamapps"))?;
            Some(steamapps.join("compatdata").join(&game.store_id).join("pfx")).filter(|p| p.is_dir())
        }
        Store::Gog | Store::Epic => {
            let app = game.heroic_app.as_ref()?;
            let cfg = crate::scan::heroic_config()?;
            let text = fs::read_to_string(cfg.join("GamesConfig").join(format!("{app}.json"))).ok()?;
            let json: serde_json::Value = serde_json::from_str(&text).ok()?;
            Some(PathBuf::from(json[app]["winePrefix"].as_str()?)).map(|p| if p.join("pfx").is_dir() { p.join("pfx") } else { p }).filter(|p| p.join("user.reg").exists())
        }
        Store::Other => bottles_prefix(&game.dir),
    }
}

/// The Bottles bottle that has a program in `dir` (its folder is the prefix).
fn bottles_prefix(dir: &Path) -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let roots = [home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"), home.join(".local/share/bottles/bottles")];
    let want = format!("folder: {}", dir.display());
    roots.iter().flat_map(|r| fs::read_dir(r).into_iter().flatten().flatten()).map(|e| e.path()).find(|b| {
        b.join("user.reg").exists() && fs::read_to_string(b.join("bottle.yml")).is_ok_and(|t| t.lines().any(|l| l.trim() == want || l.trim().starts_with(&format!("{want}/"))))
    })
}

/// Tells Wine to use the game folder's `dll` (not its own) for this exe
/// only, by editing the prefix's user.reg (Wine must not be running in it).
fn set_wine_override(prefix: &Path, exe: &str, dll: &str, on: bool) -> Result<(), String> {
    let reg = prefix.join("user.reg");
    let text = fs::read_to_string(&reg).map_err(|e| format!("reading the Wine prefix: {e}"))?;
    let key = format!("[Software\\\\Wine\\\\AppDefaults\\\\{exe}\\\\DllOverrides]");
    let name = dll.trim_end_matches(".dll");
    let line = format!("\"{name}\"=\"native,builtin\"");
    let mut out = String::new();
    let mut in_key = false;
    let mut done = false;
    for l in text.lines() {
        if l.starts_with('[') {
            if in_key && on && !done {
                out.push_str(&line);
                out.push('\n');
                done = true;
            }
            in_key = l.starts_with(&key);
        }
        if in_key && l.starts_with(&format!("\"{name}\"=")) {
            if on && !done {
                out.push_str(&line);
                out.push('\n');
                done = true;
            }
            continue;
        }
        out.push_str(l);
        out.push('\n');
    }
    if in_key && on && !done {
        out.push_str(&line);
        out.push('\n');
        done = true;
    }
    if on && !done {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        out.push_str(&format!("\n{key} {secs}\n{line}\n"));
    }
    let backup = prefix.join("user.reg.ah-backup");
    if !backup.exists() {
        let _ = fs::copy(&reg, &backup);
    }
    fs::write(&reg, out).map_err(|e| format!("writing the Wine prefix: {e}"))
}

/// Installs the overlay so it loads however the game is started: a
/// stand-in for a DLL the game imports, plus (on Linux) a per-exe Wine
/// override. Returns a message for the player.
pub fn install_overlay_proxy(game: &Mapped, defs: &[&Definition]) -> Result<String, String> {
    let exe = game.game_exes.first().ok_or("couldn't work out the game's exe")?;
    if catalogue::exe_build(exe).is_some_and(|(_, _, x64)| x64) {
        return Err("this is a 64-bit game; 64-bit support is coming soon".into());
    }
    let dir = exe.parent().ok_or("bad game path")?;
    share_profile_with_flatpaks();
    let payload = payload_dir().ok_or("overlay files not found (build overlay + proxies first)")?;
    let write_defs = || -> Result<(), String> {
        let games_dir = dir.join("games");
        fs::create_dir_all(&games_dir).map_err(|e| e.to_string())?;
        for d in defs {
            fs::write(games_dir.join(&d.file), &d.text).map_err(|e| format!("writing {}: {e}", d.file))?;
        }
        Ok(())
    };
    // Modded game with an ASI loader already (it loads every .asi next to
    // the game): ride along as a plugin and leave the mod setup alone.
    let has_asi = fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("asi")) && e.file_name() != ASI_NAME
    });
    if has_asi {
        swap_copy(&payload.join("overlay.dll"), &dir.join(ASI_NAME)).map_err(|e| format!("copying the overlay: {e}"))?;
        write_defs()?;
        return Ok(format!("Installed as a plugin for this game's mod loader. Start {} however you usually do; the overlay comes with it.", game.name));
    }
    let imports = catalogue::exe_imports(exe);
    // The game ships its own Bink video DLL: stand in for that (keeping the
    // original as binkw32_ah.dll). Wine always loads a DLL the game ships,
    // so this needs no Wine settings at all.
    let bink = dir.join("binkw32.dll");
    // Only when our stand-in covers every Bink function this game calls
    // (newer Bink versions add some); a missing one stops the game loading.
    let bink_covered = || {
        let have = catalogue::exported_names(&payload.join("binkw32.dll"));
        catalogue::imported_functions(exe, "binkw32.dll").iter().all(|f| have.contains(f))
    };
    if imports.iter().any(|i| i == "binkw32.dll") && bink.exists() && bink_covered() {
        let original = dir.join("binkw32_ah.dll");
        if !is_our_proxy(&bink) {
            fs::rename(&bink, &original).map_err(|e| format!("setting aside binkw32.dll: {e}"))?;
        }
        if !original.exists() {
            return Err("binkw32.dll was replaced earlier but the original is missing".into());
        }
        swap_copy(&payload.join("binkw32.dll"), &bink).map_err(|e| format!("copying binkw32.dll: {e}"))?;
        swap_copy(&payload.join("overlay.dll"), &dir.join("overlay.dll")).map_err(|e| format!("copying the overlay: {e}"))?;
        write_defs()?;
        return Ok(format!("Installed. Start {} however you like; the overlay comes with it, no extra settings needed.", game.name));
    }
    let proxy = PROXIES
        .iter()
        .find(|p| imports.contains(&p.to_string()) && (!dir.join(p).exists() || is_our_proxy(&dir.join(p))))
        .ok_or("this game doesn't load a DLL we can attach through (or a mod already uses it)")?;
    swap_copy(&payload.join(proxy), &dir.join(proxy)).map_err(|e| format!("copying {proxy}: {e}"))?;
    swap_copy(&payload.join("overlay.dll"), &dir.join("overlay.dll")).map_err(|e| format!("copying the overlay: {e}"))?;
    write_defs()?;
    let ini = dir.join("achievement-hub.ini");
    if !ini.exists() {
        let _ = fs::write(&ini, format!("; Achievement Hub — {}\n; Shortcuts: hotkey=shift+f2, pad_hotkey=guide|back+start\n", game.name));
    }
    let exe_name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if cfg!(windows) {
        return Ok(format!("Installed. Start {} however you like; the overlay comes with it.", game.name));
    }
    match wine_prefix(game) {
        Some(pfx) => {
            set_wine_override(&pfx, &exe_name, proxy, true)?;
            Ok(format!("Installed. Start {} however you like (Steam, Heroic, a shortcut); the overlay comes with it. If the game was running, restart it.", game.name))
        }
        None => Ok(format!(
            "Installed. This game's Wine prefix wasn't found, so add WINEDLLOVERRIDES=\"{}=n,b\" to how you launch it.",
            proxy.trim_end_matches(".dll")
        )),
    }
}

/// Brings the overlay files in every game it's installed in up to date
/// with this app's copies (after an update). Only replaces files that are
/// ours and differ; a game that's running keeps its copy until next time.
/// Returns the names of the games updated.
pub fn refresh_overlays(games: &[Mapped]) -> Vec<String> {
    let Some(payload) = payload_dir() else { return Vec::new() };
    let read = |p: &Path| fs::read(p).ok();
    let mut updated = Vec::new();
    for g in games.iter().filter(|g| g.overlay_installed) {
        let Some(dir) = g.game_exes.first().and_then(|e| e.parent()) else { continue };
        let mut pairs: Vec<(PathBuf, PathBuf)> = vec![(payload.join("overlay.dll"), dir.join("overlay.dll")), (payload.join("overlay.dll"), dir.join(ASI_NAME))];
        for p in PROXIES.iter().chain(["binkw32.dll"].iter()) {
            pairs.push((payload.join(p), dir.join(p)));
        }
        let mut changed = false;
        for (from, to) in pairs {
            // Only files we put there (the overlay, or our stand-in DLLs).
            let ours = to.file_name().is_some_and(|n| n == "overlay.dll" || n == ASI_NAME) || is_our_proxy(&to);
            if !to.exists() || !ours {
                continue;
            }
            let (Some(new), Some(old)) = (read(&from), read(&to)) else { continue };
            if new != old && swap_copy(&from, &to).is_ok() {
                changed = true;
            }
        }
        if changed {
            updated.push(g.name.clone());
        }
    }
    updated
}

/// Removes everything `install_overlay_proxy` added.
pub fn uninstall_overlay(game: &Mapped) -> Result<String, String> {
    let exe = game.game_exes.first().ok_or("couldn't work out the game's exe")?;
    let dir = exe.parent().ok_or("bad game path")?;
    let exe_name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    for p in PROXIES {
        if is_our_proxy(&dir.join(p)) {
            fs::remove_file(dir.join(p)).map_err(|e| format!("removing {p}: {e}"))?;
            if let Some(pfx) = wine_prefix(game) {
                set_wine_override(&pfx, &exe_name, p, false)?;
            }
        }
    }
    let bink = dir.join("binkw32.dll");
    if is_our_proxy(&bink) && dir.join("binkw32_ah.dll").exists() {
        fs::remove_file(&bink).map_err(|e| format!("removing binkw32.dll: {e}"))?;
        fs::rename(dir.join("binkw32_ah.dll"), &bink).map_err(|e| format!("restoring binkw32.dll: {e}"))?;
    }
    let _ = fs::remove_file(dir.join("overlay.dll"));
    let _ = fs::remove_file(dir.join(ASI_NAME));
    Ok(format!("Removed the overlay from {}. Your achievements stay in your profile.", game.name))
}
