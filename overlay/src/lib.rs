//! In-game overlay: loaded into the game process, it tracks achievements by
//! reading the game's memory directly and draws unlock pop-ups and the
//! achievements panel on top of the game's own frames.
//!
//! The same DLL works on Windows and under Proton/Wine.

#[path = "../../common/anticheat.rs"]
mod anticheat;
mod crash;
mod gameicon;
mod games;
mod gate;
mod input;
mod media;
mod sound;
mod steam;
mod storeinfo;
mod profile;
mod recorder;
mod social;
mod requests;
mod toast;
mod ui;

use hudhook::hooks::dx9::ImguiDx9Hooks;
use hudhook::imgui::{Context, FontConfig, FontGlyphRanges, FontId, FontSource, Io, TextureId, Ui};
use hudhook::windows::Win32::Foundation::HINSTANCE;
use hudhook::windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use hudhook::windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use hudhook::{Hudhook, ImguiRenderLoop, MessageFilter, RenderContext};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use games::Game;
use input::Hotkeys;
use media::Media;
use profile::Profile;
use toast::Toasts;
use ui::Panel;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Appends a line to overlay.log next to the DLL (for troubleshooting).
pub fn log(dir: &Path, msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("overlay.log")) {
        let _ = writeln!(f, "{msg}");
    }
}

pub struct Fonts {
    /// Big light blade headings (Selawik Light).
    pub heading: FontId,
    /// Metro's tab row (Selawik Light).
    pub nav: FontId,
    /// Blades: the big title ("Select a Game") and the tab labels.
    pub display: FontId,
    pub tab: FontId,
    pub large: FontId,
    pub title: FontId,
    pub body: FontId,
    pub small_bold: FontId,
    /// The same set in an installed theme pack's font (Blades only).
    pub themed: Option<Box<Fonts>>,
}

// SAFETY: FontIds are only created and used on the game's render thread,
// inside hudhook's render loop; they're never dereferenced elsewhere.
unsafe impl Send for Fonts {}
unsafe impl Sync for Fonts {}

/// A captured game frame (RGBA8).
struct Frame {
    rgba: Arc<Vec<u8>>,
    width: u32,
    height: u32,
}

struct Overlay {
    game: Option<Game>,
    profile: Profile,
    toasts: Toasts,
    fonts: Option<Fonts>,
    last_poll: Instant,
    dir: PathBuf,
    /// Shared with the input thread, which toggles it.
    panel_open: Arc<AtomicBool>,
    /// Up/down presses from the input thread, consumed by the panel.
    nav: Arc<AtomicI32>,
    panel: Panel,
    media: Media,
    /// Achievements unlocked since the last frame capture.
    capture_queue: Vec<String>,
    /// The unlock screenshot came out blank (a loading screen or fade):
    /// when to try again, and how many tries so far.
    shot_retry: Option<(Instant, u32)>,
    mouse_down: bool,
    /// Pause: the panel's open state last frame, a request to capture the
    /// game frame to freeze on, the frame waiting for upload, its texture,
    /// and whether the game is being held.
    was_open: bool,
    want_pause_frame: bool,
    pending_frame: Option<Frame>,
    pause_tex: Option<(TextureId, u32, u32)>,
    paused: bool,
    /// Running inside the UI preview tool (ah-preview.exe), not a game.
    preview: bool,
    /// An installed theme pack's font (kept for the life of the process).
    theme_font: Option<&'static [u8]>,
    /// The real mouse position (imgui gets it in the panel's layout units).
    mouse_real: Option<[f32; 2]>,
    /// Hidden while the controller or arrow keys are in use, back when the
    /// mouse moves; the pad activity count last seen.
    pointer_shown: bool,
    pad_seen: u32,
    /// `debug_engine=true`: log what engine conditions see every 15 s.
    debug_engine: bool,
    last_engine_log: Instant,
    /// Official icons finished downloading.
    icons_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
    /// Store details arriving from the background fetch.
    store_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<(String, storeinfo::StoreInfo)>>>,
}

/// A white arrow pointer with a dark outline at `p` (screen pixels).
fn draw_pointer(ui: &hudhook::imgui::Ui, p: [f32; 2]) {
    let s = (ui.io().display_size[1] / 1080.0).max(0.7) * 22.0;
    let pts = [[0.0, 0.0], [0.0, 1.0], [0.27, 0.76], [0.45, 1.12], [0.58, 1.06], [0.40, 0.70], [0.72, 0.70]];
    let poly: Vec<[f32; 2]> = pts.iter().map(|q| [p[0] + q[0] * s, p[1] + q[1] * s]).collect();
    let fg = ui.get_foreground_draw_list();
    // Filled as a fan of convex pieces: body and stem.
    fg.add_triangle(poly[0], poly[1], poly[6], [1.0, 1.0, 1.0, 1.0]).filled(true).build();
    fg.add_triangle(poly[2], poly[3], poly[5], [1.0, 1.0, 1.0, 1.0]).filled(true).build();
    fg.add_triangle(poly[3], poly[4], poly[5], [1.0, 1.0, 1.0, 1.0]).filled(true).build();
    let mut outline = poly.clone();
    outline.push(poly[0]);
    fg.add_polyline(outline, [0.0, 0.0, 0.0, 0.9]).thickness(1.5).build();
}

/// True only inside ah-preview.exe, the tool that renders the panel to a
/// screenshot for checking the look. Never true in a game.
fn preview_mode() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("ah-preview.exe")))
        .unwrap_or(false)
}

/// Mouse position (client coordinates of the foreground window) and left
/// button. Read directly because many games don't get normal mouse messages.
fn poll_mouse() -> Option<([f32; 2], bool)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow};
    // SAFETY: plain Win32 queries with valid out pointers.
    unsafe {
        let mut p = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut p) == 0 || ScreenToClient(GetForegroundWindow(), &mut p) == 0 {
            return None;
        }
        Some(([p.x as f32, p.y as f32], GetAsyncKeyState(VK_LBUTTON as i32) < 0))
    }
}

/// Names of every module loaded in this process.
fn loaded_modules() -> Vec<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32,
    };
    let mut out = Vec::new();
    // SAFETY: Toolhelp snapshot of our own process, correctly sized struct.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut me: MODULEENTRY32W = std::mem::zeroed();
        me.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut ok = Module32FirstW(snap, &mut me) != 0;
        while ok {
            let end = me.szModule.iter().position(|&c| c == 0).unwrap_or(me.szModule.len());
            out.push(String::from_utf16_lossy(&me.szModule[..end]));
            ok = Module32NextW(snap, &mut me) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Anti-cheat present in this game (loaded modules or files in its folder)?
/// If so the overlay must not hook anything: it could get the player banned.
fn anti_cheat_present() -> Option<&'static str> {
    let modules = loaded_modules();
    anticheat::scan_modules(modules.iter().map(String::as_str)).or_else(|| {
        let mut buf = [0u16; 1024];
        // SAFETY: NULL module = the game exe.
        let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
        let exe = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
        exe.parent().and_then(anticheat::scan_dir)
    })
}

/// Polls hotkeys on its own thread, so the panel can be toggled even while
/// the game isn't presenting frames.
fn spawn_input_thread(mut hotkeys: Hotkeys, open: Arc<AtomicBool>, nav: Arc<AtomicI32>, dir: PathBuf, debug: bool) {
    let mut last_buttons = 0u32;
    let mut last_codes: Vec<u32> = Vec::new();
    let mut logged = 0;
    std::thread::spawn(move || loop {
        let was_open = open.load(Ordering::Relaxed);
        let p = hotkeys.poll(was_open);
        // debug_input=true: log controller buttons as they change.
        let codes: Vec<u32> = input::PAD_CODES.iter().map(|c| c.load(Ordering::Relaxed)).collect();
        if debug && codes != last_codes && logged < 200 {
            log(&dir, &format!("input: xinput slot results {codes:?} (0 = connected, 1167 = not connected)"));
            last_codes = codes;
            logged += 1;
        }
        let buttons = input::PAD_BUTTONS.load(Ordering::Relaxed);
        if debug && buttons != last_buttons && logged < 200 {
            log(&dir, &format!("input: controller buttons {buttons:#06x}{}", if p.toggle { " -> toggle" } else { "" }));
            last_buttons = buttons;
            logged += 1;
        }
        if p.toggle || p.close {
            let now_open = !was_open && p.toggle;
            open.store(now_open, Ordering::Relaxed);
            // On close, keep the game's input blocked until the button or key
            // that closed the panel is released (below).
            if now_open {
                gate::BLOCK.store(true, Ordering::Relaxed);
            }
            log(&dir, if now_open { "panel opened" } else { "panel closed" });
        } else if was_open && p.nav != 0 {
            nav.fetch_add(p.nav, Ordering::Relaxed);
        }
        if !open.load(Ordering::Relaxed) && !p.held {
            gate::BLOCK.store(false, Ordering::Relaxed);
        }
        std::thread::sleep(Duration::from_millis(16));
    });
}

/// Reads `key=value` overlay settings from achievement-hub.ini (shared with
/// ah-launch, which ignores keys it doesn't know).
fn setting(dir: &Path, key: &str) -> Option<String> {
    std::fs::read_to_string(dir.join("achievement-hub.ini"))
        .ok()?
        .lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        .map(|(_, v)| v.trim().to_string())
}

impl Overlay {
    /// None if this exe build has no achievement definition: then the overlay
    /// stays completely dormant (only vetted games are ever hooked).
    fn new(dll_dir: PathBuf) -> Option<Overlay> {
        let logf = |m: &str| log(&dll_dir, m);
        let preview = preview_mode();
        let debug_engine = setting(&dll_dir, "debug_engine").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let mut def_dirs: Vec<PathBuf> = profile::synced_definitions_dir().into_iter().filter(|_| !preview).collect();
        def_dirs.push(dll_dir.join("games"));
        let game = if preview { games::detect_any(&def_dirs, &logf) } else { games::detect(&def_dirs, &logf) };
        if game.is_none() {
            logf("no achievement definition for this exe build; overlay stays off");
            return None;
        }
        logf(&format!("overlay loaded; game: {}", game.as_ref().map_or("unsupported".into(), |g| g.full_title())));

        let mut profile = Profile::open(&dll_dir, &logf);
        let mut game = game;
        if let Some(g) = game.as_mut().filter(|_| !preview) {
            // Record mode: conditions learned earlier, and (for players who
            // opted in) watching memory while anything is still unmapped.
            let build = games::exe_build().unwrap_or_default();
            let learned = g.apply_learned(recorder::load(&profile.dir, g.id(), build));
            if learned > 0 {
                logf(&format!("record mode: {learned} achievements use learned conditions"));
            }
            let unmapped = g.achievements().iter().any(|a| !a.trackable() || a.learned);
            // Only for players who chose to help map (the app's "Help map
            // achievements" option, or record_mode=true in the ini).
            let opted_in = profile.mapper || setting(&dll_dir, "record_mode").is_some_and(|v| v.eq_ignore_ascii_case("true"));
            if unmapped && opted_in {
                let logd = dll_dir.clone();
                recorder::start(g.id(), move |m| log(&logd, m));
            }
            let exe_dir = steam::exe_path().parent().map(Path::to_path_buf).unwrap_or_default();
            let linked = g.link_steam(&steam::local_schema(&exe_dir));
            if linked > 0 {
                logf(&format!("steam: {linked} achievements follow the game's own Steam unlocks"));
                let known = g.achievements().iter().filter_map(|a| a.steam_api.clone()).collect();
                let logd = dll_dir.clone();
                steam::start(known, move |m| log(&logd, m));
            }
        }
        let mut icons_rx = None;
        if let Some(g) = game.as_mut() {
            let dir = profile.icon_dir(g.id());
            g.icon_cache = Some(dir.clone());
            let wanted: Vec<(String, String)> =
                g.achievements().iter().map(|a| (a.id.clone(), a.steam_name.clone().unwrap_or_else(|| a.name.clone()))).collect();
            let have = profile.cached_icons(g.id()).len();
            if let Some(source) = g.icons_from.clone() {
                if have < wanted.len() {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let logd = dll_dir.clone();
                    std::thread::spawn(move || {
                        let result = match source.split_once(':') {
                            Some(("steam", id)) => storeinfo::fetch_steam_icons(id, &wanted, &dir),
                            Some(("exophase", slug)) => storeinfo::fetch_exophase_icons(slug, &wanted, &dir),
                            _ => Err(format!("unknown icon source {source}")),
                        };
                        match result {
                        Ok(n) => {
                            log(&logd, &format!("achievement icons: {n} of {} from {source}", wanted.len()));
                            let _ = tx.send(());
                        }
                        Err(e) => log(&logd, &format!("achievement icons: {e}")),
                        }
                    });
                    icons_rx = Some(std::sync::Mutex::new(rx));
                }
            }
        }
        let mut toasts = Toasts::default();
        let media = Media::default();
        // Store page details for the running game: from the cache, and
        // fetched in the background when missing or a month old.
        let mut store_rx = None;
        if let Some(g) = &game {
            let cached = storeinfo::cached(&profile.dir, g.id());
            if preview && !profile.games.contains_key(g.id()) {
                // (The preview doesn't write the profile; show the game anyway.)
                profile.touch_game_in_memory(g);
            }
            if let Some(info) = &cached {
                profile.apply_store_info(g.id(), info, !preview);
            }
            // Details for the running game, then any other game in the
            // library that has none yet (or a month-old set), one at a
            // time in the background.
            let mut todo: Vec<(String, String, String, Option<u16>, bool)> = Vec::new();
            if cached.as_ref().is_none_or(|c| !storeinfo::is_fresh(c)) {
                todo.push((g.id().to_string(), g.store.to_string(), g.title().to_string(), g.year, true));
            }
            for (id, sum) in &profile.games {
                // (Store games refresh from their store page while they run.)
                let by_title = !matches!(sum.store.as_str(), "GOG" | "Steam") || storeinfo::cached(&profile.dir, id).is_none();
                if id != g.id() && by_title && storeinfo::cached(&profile.dir, id).is_none_or(|c| !storeinfo::is_fresh(&c)) {
                    todo.push((id.clone(), sum.store.clone(), sum.title.clone(), sum.year, false));
                }
            }
            if !todo.is_empty() && !preview {
                let (tx, rx) = std::sync::mpsc::channel();
                let pdir = profile.dir.clone();
                let logd = dll_dir.clone();
                std::thread::spawn(move || {
                    for (id, store, title, year, running) in todo {
                        match storeinfo::fetch(&store, &title, year, running) {
                            Ok(info) => {
                                let _ = std::fs::write(storeinfo::cache_path(&pdir, &id), toml::to_string(&info).unwrap_or_default());
                                storeinfo::download_art(&pdir, &id, &info);
                                log(&logd, &format!("store details for {title}: {} screenshots", 1 + info.shots.len()));
                                let _ = tx.send((id, info));
                            }
                            Err(e) => log(&logd, &format!("store details for {title}: {e}")),
                        }
                    }
                });
                store_rx = Some(std::sync::Mutex::new(rx));
            }
        }
        // Art named by the definition (games without a store page).
        if let Some((id, url)) = game.as_ref().and_then(|g| g.art_url.clone().map(|u| (g.id().to_string(), u))) {
            let path = storeinfo::art_path(&profile.dir, &id);
            if !path.exists() {
                let logd = dll_dir.clone();
                std::thread::spawn(move || match storeinfo::get(&url) {
                    Ok(bytes) => {
                        let _ = std::fs::write(&path, bytes);
                        log(&logd, "game art downloaded (shows next time the panel loads it)");
                    }
                    Err(e) => log(&logd, &format!("game art: {e}")),
                });
            }
        }
        // Store artwork for every game that has it.
        for id in profile.games.keys() {
            media.load_art(id, &profile.dir);
        }
        // Online profile: share progress (and what's being played).
        if preview && std::env::var("AH_FRIENDS_DEMO").is_ok() {
            social::demo();
        }
        // (Preview: sample counts on two Fallout 3 counters, to see the bars.)
        if let (true, Some(g)) = (preview && std::env::var("AH_PROGRESS_DEMO").is_ok(), game.as_mut()) {
            let locked: Vec<String> = g.achievements().iter().filter(|a| profile.unlocked_at(g.id(), &a.id).is_none()).take(2).map(|a| a.id.clone()).collect();
            for (id, n) in locked.into_iter().zip([(54, 300), (7, 10)]) {
                g.progress.insert(id, n);
            }
        }
        if !preview {
            let logd = dll_dir.clone();
            social::start(profile.dir.clone(), game.as_ref().map(|g| g.id().to_string()), move |m| log(&logd, m));
        }
        // An installed Blades theme pack (the player's own files).
        let theme_dir = profile.dir.join("themes").join("blades");
        let mut theme_font = None;
        if theme_dir.join("theme.toml").exists() {
            logf(&format!("theme pack: {}", theme_dir.display()));
            theme_font = std::fs::read(theme_dir.join("font.ttf")).ok().map(|b| &*Box::leak(b.into_boxed_slice()));
            media.load_theme(theme_dir);
        }
        if preview {
            // The built-in sounds, to listen to outside a game.
            sound::export(&dll_dir.join("sounds-preview"));
        } else {
            // (Settings screen, else the game's ini.)
            let on = profile.settings.sounds && setting(&dll_dir, "sounds").is_none_or(|v| !v.eq_ignore_ascii_case("off"));
            let theme_dir = profile.dir.join("themes").join("blades");
            let theme_dir = theme_dir.join("theme.toml").exists().then_some(theme_dir);
            let custom = sound::init(&profile.dir, theme_dir.as_deref(), on);
            sound::set_volume(profile.settings.volume);
            toast::POPUPS_TOP.store(profile.settings.popups == "top", Ordering::Relaxed);
            logf(&format!("sounds: {}{}", if on { "on" } else { "off" }, if custom.is_empty() { String::new() } else { format!(" (yours: {})", custom.join(", ")) }));
        }
        if preview {
            // Look only: read the profile, never write it; no toasts.
            if let Ok(skin) = std::env::var("AH_SKIN") {
                profile.dashboard = skin;
            }
            let all = profile.games.keys().map(|id| (id.clone(), profile.game_icon_path(id))).collect();
            media.preload_game_icons(all);
            if let Some(g) = &game {
                let unlocked = g.achievements().iter().filter(|a| profile.unlocked_at(g.id(), &a.id).is_some()).map(|a| a.id.clone()).collect();
                let icons = g.achievements().iter().filter_map(|a| g.icon_path(a).map(|p| (a.id.clone(), p))).collect();
                media.load_game(g.id(), profile.screenshots_dir(g.id()), icons, unlocked);
            }
        } else {
            profile.migrate(&dll_dir.join("achievements.txt"), &logf);
        }
        match &game {
            _ if preview => {}
            Some(g) => {
                profile.touch_game(g);
                // Game icon: found once, saved to the profile for the Games blade.
                match gameicon::find() {
                    Some(icon) => {
                        let _ = icon.save(profile.game_icon_path(g.id()));
                        media.set_game_icon(g.id(), icon);
                    }
                    None => logf("no game icon found"),
                }
                let (got, total, pts) = profile.games.get(g.id()).map(|s| profile.game_progress(s)).unwrap_or((0, 0, 0));
                toasts.push_info(
                    &format!("Signed in as {}", profile.gamertag),
                    &format!("{}  ·  {got}/{total}  ·  {pts}/{} G", g.full_title(), g.total_points()),
                    Some(g.id()),
                );
                let icons = g.achievements().iter().filter_map(|a| g.icon_path(a).map(|p| (a.id.clone(), p))).collect();
                let unlocked = g
                    .achievements()
                    .iter()
                    .filter(|a| profile.unlocked_at(g.id(), &a.id).is_some())
                    .map(|a| a.id.clone())
                    .collect();
                media.load_game(g.id(), profile.screenshots_dir(g.id()), icons, unlocked);
            }
            None => toasts.push_info(&format!("Signed in as {}", profile.gamertag), "No achievements for this game yet", None),
        }
        // Icons of every other game played, for the Games blade.
        let others = profile
            .games
            .keys()
            .filter(|id| !preview && game.as_ref().is_none_or(|g| g.id() != id.as_str()))
            .map(|id| (id.clone(), profile.game_icon_path(id)))
            .collect();
        media.preload_game_icons(others);
        if preview {
            let mut panel = Panel::default();
            panel.blade = std::env::var("AH_BLADE").ok().and_then(|b| ui::Blade::from_name(&b));
            // AH_SETTINGS=1 opens the Settings screen.
            if std::env::var("AH_SETTINGS").is_ok_and(|v| !v.is_empty()) {
                panel.open_settings();
            }
            // AH_FILTER=locked shows only locked achievements in Now Playing.
            if std::env::var("AH_FILTER").is_ok_and(|f| f == "locked") {
                panel.preview_filter(2);
            }
            return Some(Overlay {
                game,
                profile,
                toasts,
                fonts: None,
                last_poll: Instant::now(),
                dir: dll_dir,
                panel_open: Arc::new(AtomicBool::new(true)),
                nav: Arc::new(AtomicI32::new(0)),
                panel,
                media,
                capture_queue: Vec::new(),
                shot_retry: None,
                mouse_down: false,
                was_open: false,
                want_pause_frame: false,
                pending_frame: None,
                pause_tex: None,
                paused: false,
                preview,
                theme_font,
                mouse_real: None,
                pointer_shown: true,
                pad_seen: 0,
                store_rx,
                icons_rx,
                debug_engine,
                last_engine_log: Instant::now(),
            });
        }

        let hotkeys = Hotkeys::new(
            &Some(profile.settings.hotkey.clone()).filter(|k| !k.is_empty()).or_else(|| setting(&dll_dir, "hotkey")).unwrap_or(input::DEFAULT_KEY.into()),
            &setting(&dll_dir, "pad_hotkey").unwrap_or(input::DEFAULT_PAD.into()),
        );
        logf(&format!("panel hotkey: {}", hotkeys.label));
        logf(&format!("input gate on: {}", gate::install().join(", ")));
        let panel_open = Arc::new(AtomicBool::new(false));
        let nav = Arc::new(AtomicI32::new(0));
        let debug = setting(&dll_dir, "debug_input").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        logf(&format!("controller: {}", input::describe(&hotkeys)));
        spawn_input_thread(hotkeys, Arc::clone(&panel_open), Arc::clone(&nav), dll_dir.clone(), debug);

        let panel = Panel::default();
        Some(Overlay {
            game,
            profile,
            toasts,
            fonts: None,
            last_poll: Instant::now(),
            dir: dll_dir,
            panel_open,
            nav,
            panel,
            media,
            capture_queue: Vec::new(),
            shot_retry: None,
            mouse_down: false,
            was_open: false,
            want_pause_frame: false,
            pending_frame: None,
            pause_tex: None,
            paused: false,
            preview,
            theme_font,
            mouse_real: None,
            pointer_shown: true,
            pad_seen: 0,
            store_rx,
            icons_rx,
            debug_engine,
            last_engine_log: Instant::now(),
        })
    }

    /// Record mode: the player says they just earned achievement `i`.
    fn record(&mut self, i: usize) {
        let Some(game) = self.game.as_mut() else { return };
        let id = game.achievements()[i].id.clone();
        let target = recorder::target_in(&game.achievements()[i].description);
        let build = games::exe_build().unwrap_or_default();
        match recorder::capture(&self.profile.dir, game.id(), &id, target, build) {
            Ok((condition, recordings, left)) => {
                log(&self.dir, &format!("record mode: {id} recording {recordings}, {left} candidates left"));
                if left == 1 {
                    game.apply_learned(vec![(id.clone(), condition)]);
                }
                if self.profile.unlock(game.id(), &id) {
                    log(&self.dir, &format!("unlocked {id}"));
                    self.toasts.push_unlock(i);
                    self.capture_queue.push(id);
                } else {
                    let body = if left == 1 { "Mapped. It will unlock by itself from now on.".to_string() } else { format!("{left} possibilities left; record it again to narrow it down.") };
                    self.toasts.push_info("Recorded", &body, None);
                }
            }
            Err(e) => {
                log(&self.dir, &format!("record mode: {id}: {e}"));
                self.toasts.push_info("Couldn't record", &e, None);
            }
        }
    }

    fn poll(&mut self) {
        let Some(game) = self.game.as_mut() else { return };
        let mut newly = Vec::new();
        let mut met = game.poll();
        for call in steam::take_seen() {
            log(&self.dir, &format!("steam: game called {call}"));
        }
        for n in games::award::take_new() {
            log(&self.dir, &format!("engine: the game awarded achievement {n}"));
        }
        for api in steam::take() {
            log(&self.dir, &format!("steam: game reported {api}"));
            match game.by_steam_api(&api) {
                Some(i) => met.push(i),
                None => log(&self.dir, &format!("steam: game reported {api}, which no achievement here has as steam_api")),
            }
        }
        for i in met {
            if self.profile.unlock(game.id(), &game.achievements()[i].id) {
                newly.push(i);
            }
        }
        // Platinum: every other achievement unlocked.
        if !newly.is_empty() {
            let all = game.achievements();
            let rest_done = |p: usize| {
                all.iter()
                    .enumerate()
                    .all(|(j, a)| j == p || a.platinum || self.profile.unlocked_at(game.id(), &a.id).is_some())
            };
            let ready: Vec<usize> =
                all.iter().enumerate().filter(|(p, a)| a.platinum && rest_done(*p)).map(|(p, _)| p).collect();
            for p in ready {
                if self.profile.unlock(game.id(), &all[p].id) {
                    newly.push(p);
                }
            }
        }
        if !newly.is_empty() {
            social::kick();
        }
        for i in newly {
            let a = &game.achievements()[i];
            log(&self.dir, &format!("unlocked {}", a.id));
            self.toasts.push_unlock(i);
            if self.profile.settings.screenshots {
                self.capture_queue.push(a.id.clone());
            }
        }
    }
}

impl ImguiRenderLoop for Overlay {
    fn initialize<'a>(&'a mut self, ctx: &mut Context, _: &'a mut dyn RenderContext) {
        // Fonts are rasterised at the size the panel will be scaled to on
        // this screen (its layout is 700 units tall), so text stays sharp.
        let screen_h = {
            let h = ctx.io().display_size[1];
            if h > 0.0 {
                h
            } else {
                // SAFETY: plain system metric query.
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(windows_sys::Win32::UI::WindowsAndMessaging::SM_CYSCREEN) as f32 }
            }
        };
        let scale = (screen_h * 0.92 / 700.0).max(0.5);
        ui::theme::set_font_scale(scale);
        log(&self.dir, &format!("ui scale {scale:.2} (screen height {screen_h})"));
        // Latin, Latin-1 and Latin Extended-A, plus typographic punctuation
        // (curly quotes, dashes, ellipsis, bullet) that store text uses.
        const RANGES: &[u32] = &[0x0020, 0x017F, 0x2010, 0x2027, 0x2030, 0x203A, 0x20AC, 0x20AC, 0x2122, 0x2122, 0];
        let font = move |data: &'static [u8], size: f32| FontSource::TtfData {
            data,
            size_pixels: size * scale,
            config: Some(FontConfig { oversample_h: 2, glyph_ranges: FontGlyphRanges::from_slice(RANGES), ..FontConfig::default() }),
        };
        // Selawik: Microsoft's open-source (OFL) stand-in for Segoe UI.
        let light = include_bytes!("../assets/selawkl.ttf");
        let semibold = include_bytes!("../assets/selawksb.ttf");
        let bold = include_bytes!("../assets/selawkb.ttf");
        let regular = include_bytes!("../assets/selawk.ttf");
        let heading = ctx.fonts().add_font(&[font(light, 44.0)]);
        let nav = ctx.fonts().add_font(&[font(light, 34.0)]);
        let display = ctx.fonts().add_font(&[font(regular, 40.0)]);
        let tab = ctx.fonts().add_font(&[font(regular, 29.0)]);
        let large = ctx.fonts().add_font(&[font(semibold, 30.0)]);
        let title = ctx.fonts().add_font(&[font(semibold, 21.0)]);
        let body = ctx.fonts().add_font(&[font(regular, 17.0)]);
        let small_bold = ctx.fonts().add_font(&[font(bold, 14.0)]);
        // A theme pack's font: one face, at the same sizes.
        let themed = self.theme_font.map(|f| {
            let mut add = |size: f32| ctx.fonts().add_font(&[font(f, size)]);
            Box::new(Fonts {
                heading: add(44.0),
                nav: add(34.0),
                display: add(40.0),
                tab: add(29.0),
                large: add(30.0),
                title: add(21.0),
                body: add(17.0),
                small_bold: add(14.0),
                themed: None,
            })
        });
        self.fonts = Some(Fonts { heading, nav, display, tab, large, title, body, small_bold, themed });
        // Called again whenever the renderer is rebuilt; old textures are gone.
        self.media.reset();
        self.pause_tex = None;
        self.paused = false;
        log(&self.dir, "renderer hooked; drawing overlay");
        ctx.set_ini_filename(None);
    }

    fn before_render<'a>(&'a mut self, ctx: &mut Context, render_context: &'a mut dyn RenderContext) {
        self.media.upload(render_context);
        // Upload the frozen game frame; reuse one texture to avoid leaking.
        if let Some(frame) = self.pending_frame.take() {
            let (w, h) = (frame.width, frame.height);
            let tex = match self.pause_tex {
                Some((t, tw, th)) if tw == w && th == h => render_context.replace_texture(t, &frame.rgba, w, h).map(|_| t),
                _ => render_context.load_texture(&frame.rgba, w, h),
            };
            if let Ok(t) = tex {
                self.pause_tex = Some((t, w, h));
                self.paused = self.panel_open.load(Ordering::Relaxed);
            }
        }
        gate::install_message_hook();
        // Fullscreen games hide the OS cursor; draw our own while the panel is up.
        let open = self.panel_open.load(Ordering::Relaxed);
        let io = ctx.io_mut();
        // The panel is laid out small and scaled up: give imgui the mouse in
        // layout coordinates, and draw the pointer ourselves.
        io.mouse_draw_cursor = false;
        hudhook::EXTERNAL_MOUSE_POS.store(open, Ordering::Relaxed);
        if open && !self.preview {
            // Games often confine the cursor to an area (Alt+Tab lifts it);
            // free it while the panel is up so it can reach everything.
            // SAFETY: plain Win32 call; a null rect removes the confinement.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::ClipCursor(std::ptr::null());
            }
            if let Some((pos, down)) = poll_mouse() {
                let pad = input::PAD_ACTIVITY.load(Ordering::Relaxed);
                if pad != self.pad_seen {
                    self.pad_seen = pad;
                    self.pointer_shown = false;
                }
                if self.mouse_real.is_some_and(|old| (old[0] - pos[0]).abs() + (old[1] - pos[1]).abs() > 2.0) || down {
                    self.pointer_shown = true;
                }
                self.mouse_real = Some(pos);
                // Hidden pointer: park it off the panel so hover highlights
                // don't fight the controller's selection.
                io.add_mouse_pos_event(if self.pointer_shown { self.panel.to_layout(pos) } else { [-10_000.0, -10_000.0] });
                if down != self.mouse_down {
                    io.add_mouse_button_event(hudhook::imgui::MouseButton::Left, down);
                    self.mouse_down = down;
                }
            }
        }
    }

    fn render(&mut self, ui: &mut Ui) {
        // Official icons finished downloading: show them.
        if let Some(rx) = &self.icons_rx {
            if rx.lock().ok().and_then(|r| r.try_recv().ok()).is_some() {
                if let Some(g) = &self.game {
                    self.media.add_icons(g.id(), self.profile.cached_icons(g.id()));
                }
                self.icons_rx = None;
            }
        }
        // Store details finished fetching: show them.
        if let Some(rx) = &self.store_rx {
            while let Some((id, info)) = rx.lock().ok().and_then(|r| r.try_recv().ok()) {
                self.profile.apply_store_info(&id, &info, !self.preview);
                self.media.load_art(&id, &self.profile.dir);
            }
        }
        if self.debug_engine && self.last_engine_log.elapsed() >= Duration::from_secs(15) {
            self.last_engine_log = Instant::now();
            if let Some(report) = self.game.as_mut().and_then(|g| g.engine_report()) {
                log(&self.dir, &report);
            }
        }
        if !self.preview && self.last_poll.elapsed() >= POLL_INTERVAL {
            self.last_poll = Instant::now();
            self.poll();
        }
        let open = self.panel_open.load(Ordering::Relaxed);
        if open != self.was_open {
            sound::play(if open { sound::Sfx::Open } else { sound::Sfx::Close });
        }
        if open && !self.was_open {
            // Freeze on the current game frame: capture it, then hold.
            self.want_pause_frame = !self.preview;
            self.panel.opened();
        }
        if !open {
            self.paused = false;
            self.want_pause_frame = false;
            input::MODAL.store(false, Ordering::Relaxed);
        }
        self.was_open = open;

        let focused = poll_mouse().is_some();
        let (keys, pad) = self.panel.handle_input(focused);
        let Some(fonts) = &self.fonts else { return };

        if open && self.paused {
            if let Some((tex, _, _)) = self.pause_tex {
                let size = ui.io().display_size;
                let bg = ui.get_background_draw_list();
                bg.add_image(tex, [0.0, 0.0], size).build();
                let dim = 0.55 * self.panel.open_amount();
                bg.add_rect([0.0, 0.0], size, [0.0, 0.0, 0.0, dim]).filled(true).build();
            }
        }

        if open {
            // A gamer picture set on the website, and pictures downloaded.
            if let Some(avatar) = social::take_avatar().filter(|a| *a != self.profile.avatar && !self.preview) {
                self.profile.set_avatar(&avatar);
            }
            for (tag, path) in social::take_pictures() {
                self.media.load_picture(&tag, path);
            }
            self.panel.navigate(self.nav.swap(0, Ordering::Relaxed));
            self.panel.draw(ui, fonts, &self.media, self.game.as_ref(), &self.profile, keys, pad);
            if let Some(why) = self.panel.record_refused.take() {
                log(&self.dir, &format!("record mode: {why}"));
            }
            if let Some(i) = self.panel.record_request.take() {
                self.record(i);
            }
            if let Some(s) = self.panel.settings_request.take() {
                toast::POPUPS_TOP.store(s.popups == "top", Ordering::Relaxed);
                if self.preview {
                    self.profile.settings = s;
                } else {
                    log(&self.dir, &format!("settings: {s:?}"));
                    self.profile.set_settings(s);
                }
            }
            if let Some(skin) = self.panel.skin_request.take().filter(|_| !self.preview) {
                self.profile.set_dashboard(skin.id());
                log(&self.dir, &format!("dashboard design: {}", skin.id()));
            }
        } else {
            self.nav.store(0, Ordering::Relaxed);
        }
        if let (true, Some(p)) = (open && !self.preview && self.pointer_shown, self.mouse_real) {
            draw_pointer(ui, p);
        }
        let Some(fonts) = &self.fonts else { return };
        self.toasts.draw(ui, fonts, &self.media, self.game.as_ref());
    }

    fn wants_frame_capture(&mut self) -> bool {
        (!self.capture_queue.is_empty() && self.shot_due()) || self.want_pause_frame
    }

    fn on_frame_captured(&mut self, rgba: Vec<u8>, width: u32, height: u32) {
        let rgba = Arc::new(rgba);
        if self.want_pause_frame {
            self.want_pause_frame = false;
            self.pending_frame = Some(Frame { rgba: Arc::clone(&rgba), width, height });
        }
        if self.capture_queue.is_empty() || !self.shot_due() {
            return;
        }
        // A loading screen, fade or title card makes a poor keepsake: wait a
        // moment and take another (keeping the last try if it stays blank).
        let tries = self.shot_retry.map_or(0, |(_, n)| n);
        if tries < SHOT_RETRIES && looks_blank(&rgba, width, height) {
            if tries == 0 {
                log(&self.dir, "unlock screenshot: the screen is blank (loading screen?), trying again shortly");
            }
            self.shot_retry = Some((Instant::now() + Duration::from_millis(1000), tries + 1));
            return;
        }
        self.shot_retry = None;
        let ids = std::mem::take(&mut self.capture_queue);
        if !ids.is_empty() {
            log(&self.dir, &format!("captured {width}x{height} screenshot for {}", ids.join(", ")));
            // Achievements without hand-made art get an icon from this frame.
            let needs_icon = self
                .game
                .as_ref()
                .map(|g| g.achievements().iter().filter(|a| ids.contains(&a.id) && g.icon_path(a).is_none()).map(|a| a.id.clone()).collect())
                .unwrap_or_default();
            if let Some(g) = &self.game {
                self.media.save_shot(g.id(), self.profile.screenshots_dir(g.id()), ids, needs_icon, rgba.to_vec(), width, height);
            }
        }
    }

    fn wants_hold(&mut self) -> bool {
        self.paused && self.panel_open.load(Ordering::Relaxed)
    }

    fn message_filter(&self, _io: &Io) -> MessageFilter {
        // Like Steam's overlay: while the panel is open the game doesn't get
        // mouse/keyboard window messages.
        if self.panel_open.load(Ordering::Relaxed) {
            MessageFilter::InputKeyboard | MessageFilter::InputMouse
        } else {
            MessageFilter::empty()
        }
    }
}

fn module_dir(hmodule: HINSTANCE) -> PathBuf {
    let mut buf = [0u16; 1024];
    // SAFETY: buffer is valid for its length.
    let n = unsafe { GetModuleFileNameW(Some(hmodule.into()), &mut buf) } as usize;
    let path = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    path.parent().map(PathBuf::from).unwrap_or_default()
}

/// # Safety
/// Called by the Windows loader.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(hmodule: HINSTANCE, reason: u32, _: *mut std::ffi::c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        let dir = module_dir(hmodule);
        let dir2 = dir.clone();
        let panic_dir = dir.clone();
        std::panic::set_hook(Box::new(move |info| log(&panic_dir, &format!("panic: {info}"))));
        crash::install(dir.clone());
        let hmodule_raw = hmodule.0 as usize;
        std::thread::spawn(move || {
            let hmodule = HINSTANCE(hmodule_raw as _);
            // Safety first: never hook a game with anti-cheat.
            if let Some(ac) = anti_cheat_present() {
                log(&dir, &format!("anti-cheat detected ({ac}); overlay disabled, nothing hooked"));
                return;
            }
            let Some(overlay) = Overlay::new(dir.clone()) else { return };
            if let Err(e) = Hudhook::builder()
                .with::<ImguiDx9Hooks>(overlay)
                .with_hmodule(hmodule)
                .build()
                .apply()
                .inspect(|_| {
                    log(&dir2, "hooks installed; waiting for the game to draw a frame");
                    // The game may load dinput.dll after we're injected; retry briefly.
                    let dir3 = dir2.clone();
                    std::thread::spawn(move || {
                        for _ in 0..30 {
                            let result = gate::install_dinput();
                            if result != "no dinput.dll loaded" {
                                log(&dir3, &format!("input gate: {result}"));
                                return;
                            }
                            std::thread::sleep(Duration::from_secs(1));
                        }
                        log(&dir3, "input gate: game never loaded dinput.dll");
                    });
                })
            {
                log(&dir2, &format!("couldn't hook the renderer: {e:?}"));
                hudhook::eject();
            }
        });
    }
    1
}

/// Tries at a blank unlock screenshot before keeping it anyway (about a
/// second apart).
const SHOT_RETRIES: u32 = 6;

impl Overlay {
    /// Whether a waiting unlock screenshot may be taken now.
    fn shot_due(&self) -> bool {
        self.shot_retry.is_none_or(|(at, _)| Instant::now() >= at)
    }
}

/// True if a frame is (nearly) one flat color: a loading screen, a fade to
/// black or white, a title card. Samples a grid of pixels and checks how
/// many sit close to the most common brightness.
fn looks_blank(rgba: &[u8], width: u32, height: u32) -> bool {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return false;
    }
    let (cols, rows) = (48, 27);
    let mut luma: Vec<i32> = Vec::with_capacity(cols * rows);
    for r in 0..rows {
        for c in 0..cols {
            let (x, y) = ((c * 2 + 1) * w / (cols * 2), (r * 2 + 1) * h / (rows * 2));
            let i = (y * w + x) * 4;
            // (Channel order doesn't matter much for brightness.)
            luma.push((2 * rgba[i] as i32 + 5 * rgba[i + 1] as i32 + rgba[i + 2] as i32) / 8);
        }
    }
    let mut sorted = luma.clone();
    sorted.sort_unstable();
    let median = sorted[sorted.len() / 2];
    let near = luma.iter().filter(|&&l| (l - median).abs() <= 12).count();
    near * 100 >= luma.len() * 94
}

#[cfg(test)]
mod blank_tests {
    use super::looks_blank;

    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        let f = &f;
        (0..h).flat_map(|y| (0..w).flat_map(move |x| [f(x, y), f(x, y), f(x, y), 255])).collect::<Vec<u8>>()
    }

    #[test]
    fn title_card_and_fades_are_blank() {
        // White with a line of dark text in the middle ("THREE YEARS LATER").
        let card = frame(320, 180, |x, y| if (85..95).contains(&y) && (110..210).contains(&x) && x % 3 != 0 { 40 } else { 250 });
        assert!(looks_blank(&card, 320, 180));
        assert!(looks_blank(&frame(320, 180, |_, _| 0), 320, 180));
    }

    #[test]
    fn gameplay_is_not_blank() {
        // Sky over ground, with detail.
        let scene = frame(320, 180, |x, y| if y < 80 { 150 + (x % 40) as u8 } else { 60 + ((x * 7 + y * 3) % 90) as u8 });
        assert!(!looks_blank(&scene, 320, 180));
    }
}
