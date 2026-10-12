//! Achievement definitions loaded from `games/*.toml` next to the DLL.
//!
//! Each file describes one game build: how to recognise its exe, and its
//! achievements as memory conditions. See `definitions/README.md`.

pub mod award;
pub mod rules;

use rules::{Condition, Memory};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
// (Unknown fields are ignored: newer game files may add some.)
struct File {
    game: GameInfo,
    /// For engine conditions (quest / faction).
    #[serde(default)]
    engine: Option<rules::Engine>,
    build: Vec<Build>,
    #[serde(default)]
    achievement: Vec<Achievement>,
}

#[derive(Deserialize)]
// (Unknown fields are ignored: newer game files may add some.)
struct GameInfo {
    id: String,
    title: String,
    subtitle: Option<String>,
    /// Shown in the Games blade.
    developer: Option<String>,
    publisher: Option<String>,
    year: Option<u16>,
    description: Option<String>,
    /// Artwork for the Games pane when the game has no store page to take
    /// it from.
    art_url: Option<String>,
    /// Where official achievement icons can be fetched ("steam:<appid>":
    /// that app's public achievement list, matched by `steam_name`).
    icons_from: Option<String>,
}

/// Where the running game came from, from tell-tale files next to its exe
/// (shown as a badge, like the dashboard's "XBOX 360").
pub fn detect_store() -> &'static str {
    // (The UI preview tool passes one in.)
    match std::env::var("AH_STORE").as_deref() {
        Ok("GOG") => return "GOG",
        Ok("Steam") => return "Steam",
        _ => {}
    }
    let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) else { return "PC" };
    let exe = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|n| n.to_string_lossy().to_lowercase())).unwrap_or_default();
    let has = |d: &Path, f: &dyn Fn(&str) -> bool| {
        fs::read_dir(d).into_iter().flatten().flatten().any(|e| f(&e.file_name().to_string_lossy().to_lowercase()))
    };
    let path = dir.to_string_lossy().to_lowercase();
    let dirs = [Some(dir.clone()), dir.parent().map(Path::to_path_buf)];
    const EMULATORS: [&str; 18] = [
        "retroarch", "dolphin", "pcsx2", "rpcs3", "cemu", "yuzu", "ryujinx", "duckstation", "ppsspp", "xemu", "xenia", "mame",
        "mgba", "snes9x", "project64", "citra", "melonds", "flycast",
    ];
    if EMULATORS.iter().any(|e| exe.contains(e)) {
        "Emulator"
    } else if dirs.iter().flatten().any(|d| has(d, &|n| n.starts_with("goggame-") && n.ends_with(".info"))) {
        "GOG"
    } else if path.contains("steamapps") || has(&dir, &|n| n == "steam_api.dll" || n == "steam_api64.dll" || n == "steam_appid.txt") {
        "Steam"
    } else if path.contains("epic games") || dirs.iter().flatten().any(|d| d.join(".egstore").is_dir()) {
        "Epic"
    } else {
        "PC"
    }
}

/// Identifies an exe build by its PE header.
#[derive(Deserialize)]
// (Unknown fields are ignored: newer game files may add some.)
struct Build {
    #[allow(dead_code)] // documentation for humans; matching uses the header
    exe: Option<String>,
    timestamp: u32,
    image_size: u32,
}

#[derive(Deserialize)]
// (Unknown fields are ignored: newer game files may add some.)
pub struct Achievement {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub points: u32,
    /// Name and description stay hidden until unlocked.
    #[serde(default)]
    pub secret: bool,
    /// Unlocks automatically once every other achievement is unlocked.
    #[serde(default)]
    pub platinum: bool,
    /// PNG path, relative to the definitions folder.
    icon: Option<String>,
    /// This achievement's name in the `icons_from` Steam app's list.
    pub steam_name: Option<String>,
    /// Its Steam API name, for games that report unlocks to Steam.
    pub steam_api: Option<String>,
    /// Empty = listed, but not trackable yet.
    #[serde(default)]
    when: Vec<Condition>,
    /// Its condition came from record mode, not the definition.
    #[serde(skip)]
    pub learned: bool,
}

impl Achievement {
    /// Whether the overlay can currently detect this achievement.
    pub fn trackable(&self) -> bool {
        self.platinum || !self.when.is_empty() || self.steam_api.is_some()
    }
}

pub struct Game {
    id: String,
    title: String,
    subtitle: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub year: Option<u16>,
    pub description: Option<String>,
    /// GOG, Steam, Epic, Emulator or PC.
    pub store: &'static str,
    pub icons_from: Option<String>,
    pub art_url: Option<String>,
    /// Official icons fetched into the profile (<id>.jpg), if any.
    pub icon_cache: Option<PathBuf>,
    achievements: Vec<Achievement>,
    dir: PathBuf,
    memory: Memory,
    /// Counting achievements: (now, target), refreshed every poll.
    pub progress: std::collections::HashMap<String, (i64, i64)>,
}

impl Game {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    /// "Resident Evil 2 (Leon)"-style name for one-line contexts.
    pub fn full_title(&self) -> String {
        match &self.subtitle {
            Some(s) => format!("{} ({s})", self.title),
            None => self.title.clone(),
        }
    }

    pub fn achievements(&self) -> &[Achievement] {
        &self.achievements
    }

    /// Fills in Steam API names the definition doesn't give, from a schema
    /// (API name, display name) matched on the achievement's name. Returns
    /// how many achievements have one.
    pub fn link_steam(&mut self, schema: &[(String, String)]) -> usize {
        let norm = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        for a in self.achievements.iter_mut().filter(|a| a.steam_api.is_none()) {
            let want = norm(a.steam_name.as_deref().unwrap_or(&a.name));
            a.steam_api = schema.iter().find(|(_, shown)| norm(shown) == want).map(|(api, _)| api.clone());
        }
        self.achievements.iter().filter(|a| a.steam_api.is_some()).count()
    }

    /// Gives achievements the definition can't track yet the conditions
    /// record mode learned for them.
    pub fn apply_learned(&mut self, learned: Vec<(String, rules::Condition)>) -> usize {
        let mut n = 0;
        for (id, c) in learned {
            if let Some(a) = self.achievements.iter_mut().find(|a| a.id == id && (a.when.is_empty() || a.learned) && a.steam_api.is_none()) {
                a.when = vec![c];
                a.learned = true;
                n += 1;
            }
        }
        n
    }

    /// The achievement with this Steam API name.
    pub fn by_steam_api(&self, api: &str) -> Option<usize> {
        self.achievements.iter().position(|a| a.steam_api.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(api)))
    }

    pub fn total_points(&self) -> u32 {
        self.achievements.iter().map(|a| a.points).sum()
    }

    /// Where captured icons for this game are stored:
    /// `icons/<game id>/<achievement id>.png` in the definitions folder.
    pub fn icon_file(&self, a: &Achievement) -> PathBuf {
        self.dir.join("icons").join(&self.id).join(format!("{}.png", a.id))
    }

    /// Icon for an achievement: the one the definition names, otherwise a
    /// captured icon if one exists.
    pub fn icon_path(&self, a: &Achievement) -> Option<PathBuf> {
        match &a.icon {
            Some(p) => Some(self.dir.join(p)),
            None => Some(self.icon_file(a))
                .filter(|p| p.exists())
                .or_else(|| self.icon_cache.as_ref().map(|d| d.join(format!("{}.jpg", a.id))).filter(|p| p.exists())),
        }
    }

    /// Called ~10 times a second; returns achievements whose conditions all
    /// hold right now, as indexes into `achievements()`. Platinum entries are
    /// handled by the caller (they depend on what's unlocked, not memory).
    /// What engine conditions see right now (for `debug_engine` logging):
    /// the player's factions and the stages of the quests conditions use.
    pub fn engine_report(&mut self) -> Option<String> {
        self.memory.engine?;
        self.memory.begin_poll();
        let factions = self.memory.player_factions();
        let mut quests: Vec<String> = self.achievements.iter().flat_map(|a| a.when.iter().filter_map(|c| c.quest_name())).map(String::from).collect();
        quests.sort();
        quests.dedup();
        let stages: Vec<String> = quests.iter().map(|q| format!("{q}={}", self.memory.quest_stage(q).map_or("?".into(), |s| s.to_string()))).collect();
        let level = self.memory.player_value(rules::PlayerValue::Level);
        let karma = self.memory.player_value(rules::PlayerValue::Karma);
        self.memory.end_poll();
        Some(format!(
            "engine: level {level:?} karma {karma:?} factions [{}] quests [{}]",
            factions.iter().map(|(id, r)| format!("{id:08x}:{r}")).collect::<Vec<_>>().join(" "),
            stages.join(" ")
        ))
    }

    pub fn poll(&mut self) -> Vec<usize> {
        self.memory.begin_poll();
        let mem = &mut self.memory;
        // Counting achievements: how far along each is (for the list).
        self.progress = self.achievements.iter().filter_map(|a| Some((a.id.clone(), a.when.iter().find_map(|c| c.progress(mem))?))).collect();
        let met: Vec<bool> = self
            .achievements
            .iter()
            .map(|a| !a.platinum && !a.when.is_empty() && a.when.iter().all(|c| c.holds(mem)))
            .collect();
        self.memory.end_poll();
        met.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect()
    }
}

/// PE timestamp and image size of the process's main executable.
pub fn exe_build() -> Option<(u32, u32)> {
    use hudhook::windows::Win32::System::LibraryLoader::GetModuleHandleW;
    // SAFETY: the main module's headers are always mapped and readable.
    unsafe {
        let base = GetModuleHandleW(None).ok()?.0 as *const u8;
        if *(base as *const u16) != 0x5A4D {
            return None;
        }
        let pe = base.add(*(base.add(0x3C) as *const u32) as usize);
        if *(pe as *const u32) != 0x0000_4550 {
            return None;
        }
        Some((*(pe.add(8) as *const u32), *(pe.add(24 + 56) as *const u32)))
    }
}

/// Finds the definition file matching the running exe. Problems with
/// individual files are reported through `log` and the file is skipped.
///
/// `dirs` in order of preference: a definition in an earlier folder wins
/// over one with the same file name in a later one (the profile's copies,
/// synced from the online database, over the ones installed with the game).
pub fn detect(dirs: &[PathBuf], log: &dyn Fn(&str)) -> Option<Game> {
    detect_inner(dirs, log, false)
}

/// The UI preview tool: the first valid definition, whatever the exe.
pub fn detect_any(dirs: &[PathBuf], log: &dyn Fn(&str)) -> Option<Game> {
    detect_inner(dirs, log, true)
}

fn detect_inner(dirs: &[PathBuf], log: &dyn Fn(&str), any_build: bool) -> Option<Game> {
    let (timestamp, image_size) = exe_build()?;
    log(&format!("exe build: timestamp {timestamp:#010x}, image size {image_size:#x}"));
    let mut by_name: std::collections::BTreeMap<std::ffi::OsString, PathBuf> = Default::default();
    for d in dirs.iter().rev() {
        for e in fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("toml")) {
                by_name.insert(e.file_name(), p);
            }
        }
    }
    if by_name.is_empty() {
        log(&format!("no definitions in {}", dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(" or ")));
        return None;
    }
    let paths: Vec<PathBuf> = by_name.into_values().collect();
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let file: File = match fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str(&t).map_err(|e| e.to_string()))
        {
            Ok(f) => f,
            Err(e) => {
                log(&format!("{name}: {e}"));
                continue;
            }
        };
        // (The preview tool can pick a definition: AH_DEF=part of its file name.)
        if any_build && std::env::var("AH_DEF").is_ok_and(|want| !name.contains(&want)) {
            continue;
        }
        if !any_build && !file.build.iter().any(|b| b.timestamp == timestamp && b.image_size == image_size) {
            continue;
        }
        let mut seen = std::collections::HashSet::new();
        if let Some(bad) = file.achievement.iter().find_map(|a| {
            if !seen.insert(a.id.as_str()) {
                return Some(format!("duplicate achievement id {}", a.id));
            }
            a.when
                .iter()
                .find_map(|c| c.validate().err().or_else(|| (c.uses_engine() && file.engine.is_none()).then(|| "quest/faction conditions need an [engine] block".to_string())))
                .map(|e| format!("achievement {}: {e}", a.id))
        }) {
            log(&format!("{name}: {bad}"));
            continue;
        }
        let trackable = file.achievement.iter().filter(|a| a.trackable()).count();
        log(&format!(
            "{name}: {} achievements ({trackable} trackable) for {}",
            file.achievement.len(),
            file.game.title
        ));
        // Achievements the game's own scripts award (Fallout 3).
        if let Some(e) = &file.engine {
            if e.catch_add_achievement && !any_build {
                match award::install("AddAchievement", e.extract_args_rva) {
                    Ok(m) => log(&format!("engine: {m}")),
                    Err(m) => log(&format!("engine: couldn't catch AddAchievement ({m}); those achievements won't unlock")),
                }
            }
        }
        return Some(Game {
            id: file.game.id,
            title: file.game.title,
            subtitle: file.game.subtitle,
            developer: file.game.developer,
            publisher: file.game.publisher,
            year: file.game.year,
            description: file.game.description,
            store: detect_store(),
            icons_from: file.game.icons_from,
            art_url: file.game.art_url,
            icon_cache: None,
            achievements: file.achievement,
            // (Hand-made icons sit next to the installed copies.)
            dir: dirs.last().cloned().unwrap_or_default(),
            memory: Memory::with_engine(file.engine),
            progress: std::collections::HashMap::new(),
        });
    }
    None
}
