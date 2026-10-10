//! Achievement Hub desktop app: map your games, see which have achievement
//! sets, install the overlay, and request new games. Games using anti-cheat
//! are blocked outright.

mod catalogue;
mod covers;
mod dbsync;
mod friends;
mod library;
mod mappings;
mod requests;
mod safety;
mod scan;

use catalogue::Definition;
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};
use library::{Library, Mapped};
use safety::{Safety, Verdict};
use scan::Found;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};

const BG: Color32 = Color32::from_rgb(16, 21, 30);
const PANEL: Color32 = Color32::from_rgb(27, 40, 56);
const CARD: Color32 = Color32::from_rgb(22, 32, 45);
const CARD_HOVER: Color32 = Color32::from_rgb(39, 54, 73);
const GREEN: Color32 = Color32::from_rgb(124, 191, 30);
const GREEN_DARK: Color32 = Color32::from_rgb(68, 120, 15);
const RED: Color32 = Color32::from_rgb(214, 72, 64);
const AMBER: Color32 = Color32::from_rgb(222, 170, 60);
const TEXT: Color32 = Color32::from_rgb(199, 213, 224);
const MUTED: Color32 = Color32::from_rgb(143, 152, 160);
const INK: Color32 = Color32::from_rgb(10, 15, 20);

#[derive(PartialEq, Clone, Copy)]
enum View {
    Library,
    Add,
    Supported,
    Request,
    Friends,
}

/// What a detected game is, from the player's point of view.
#[derive(Clone)]
enum Status {
    Blocked(String),
    Checking,
    /// Can be added, but the overlay won't be installed.
    Unverified(String),
    /// Has an achievement set (number of achievements).
    Supported(usize),
    /// Can be mapped; no achievement set yet.
    Mappable,
}

struct Hub {
    view: View,
    library: Library,
    found: Option<Vec<Found>>,
    scan_rx: Option<Receiver<Vec<Found>>>,
    browse_rx: Option<Receiver<Option<PathBuf>>>,
    safety: Safety,
    defs: Vec<Definition>,
    /// Cover thumbnails by game folder: loaded, or None while loading/absent.
    covers: HashMap<PathBuf, Option<egui::TextureHandle>>,
    cover_tx: std::sync::mpsc::Sender<(PathBuf, Option<PathBuf>, Option<String>)>,
    cover_rx: Receiver<(PathBuf, Option<egui::ColorImage>)>,
    /// Status per game folder (checking a game means reading its files).
    statuses: std::cell::RefCell<HashMap<PathBuf, Status>>,
    search: String,
    /// (message, good news?)
    message: Option<(String, bool)>,
    request: requests::Request,
    request_rx: Option<Receiver<Result<(), String>>>,
    unlocks: HashMap<String, usize>,
    /// The online game database check started at launch.
    sync_rx: Option<Receiver<Result<Vec<String>, String>>>,
    /// Record-mode mappings being shared.
    share_rx: Option<Receiver<Result<usize, String>>>,
    /// This player's identity, friends' profiles, and calls in flight.
    social: Option<friends::Social>,
    friend_profiles: Vec<friends::Profile>,
    friends_rx: Option<Receiver<Result<Vec<friends::Profile>, String>>>,
    publish_rx: Option<Receiver<Result<String, String>>>,
    publish_status: String,
    /// The link code being typed, and the link call in flight.
    add_code: String,
    link_rx: Option<Receiver<Result<friends::Social, String>>>,
    /// "Forgot your website password?": the new one being typed.
    new_password: String,
    show_reset: bool,
    social_started: bool,
}

/// Unlock counts per game id, from the shared profile.
fn unlock_counts() -> HashMap<String, usize> {
    let mut m = HashMap::new();
    let text = std::fs::read_to_string(library::profile_dir().join("unlocks.txt")).unwrap_or_default();
    for game in text.lines().filter_map(|l| l.split_whitespace().next()) {
        *m.entry(game.to_string()).or_default() += 1;
    }
    m
}

/// Whether the player opted in to helping map achievements (profile.toml).
fn mapper() -> bool {
    std::fs::read_to_string(library::profile_dir().join("profile.toml")).is_ok_and(|t| t.lines().any(|l| l.trim() == "mapper = true"))
}

fn set_mapper(on: bool) {
    let path = library::profile_dir().join("profile.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with("mapper = ")).map(String::from).collect();
    if on {
        // (Top-level keys must come before any [table].)
        let at = lines.iter().position(|l| l.starts_with('[')).unwrap_or(lines.len());
        lines.insert(at, "mapper = true".into());
    }
    let _ = std::fs::write(path, lines.join("\n") + "\n");
}

fn gamertag() -> String {
    std::fs::read_to_string(library::profile_dir().join("profile.toml"))
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("gamertag = ").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_default()
}

impl Hub {
    fn new(cc: &eframe::CreationContext<'_>) -> Hub {
        let mut v = egui::Visuals::dark();
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.selection.bg_fill = GREEN_DARK;
        v.selection.stroke = Stroke::new(1.0, GREEN);
        v.hyperlink_color = GREEN;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, GREEN);
        cc.egui_ctx.set_visuals(v);

        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(scan::installed());
        });
        let request = requests::Request { gamertag: gamertag(), ..Default::default() };
        let (sync_tx, sync_rx) = channel();
        std::thread::spawn(move || {
            let _ = sync_tx.send(dbsync::sync());
        });
        let (cover_tx, cover_rx) = covers::spawn_loader();
        Hub {
            view: View::Library,
            library: Library::load(),
            found: None,
            scan_rx: Some(rx),
            browse_rx: None,
            safety: Safety::new(&library::profile_dir()),
            defs: catalogue::all(),
            covers: HashMap::new(),
            cover_tx,
            cover_rx,
            statuses: Default::default(),
            search: String::new(),
            message: None,
            request,
            request_rx: None,
            unlocks: unlock_counts(),
            sync_rx: Some(sync_rx),
            share_rx: None,
            social: friends::social::load(&library::profile_dir()),
            friend_profiles: Vec::new(),
            friends_rx: None,
            publish_rx: None,
            publish_status: String::new(),
            add_code: String::new(),
            link_rx: None,
            new_password: String::new(),
            show_reset: false,
            social_started: false,
        }
    }

    fn status(&self, f: &Found) -> Status {
        if let Some(s) = self.statuses.borrow().get(&f.dir) {
            return s.clone();
        }
        let s = self.status_uncached(f);
        // (Still-checking ones are looked at again next frame.)
        if !matches!(s, Status::Checking) {
            self.statuses.borrow_mut().insert(f.dir.clone(), s.clone());
        }
        s
    }

    fn status_uncached(&self, f: &Found) -> Status {
        match self.safety.check(f) {
            Verdict::Blocked(why) => Status::Blocked(why),
            Verdict::Checking => Status::Checking,
            Verdict::Unverified(why) => Status::Unverified(why),
            Verdict::Clear => match catalogue::for_exes(&self.defs, &f.game_exes).first() {
                Some(d) => Status::Supported(d.achievement.len()),
                None => Status::Mappable,
            },
        }
    }

    /// A game's cover thumbnail; decoded (and downloaded if needed) in the
    /// background, so scrolling never waits on it.
    fn cover(&mut self, ctx: &egui::Context, f: &Found) -> Option<egui::TextureHandle> {
        while let Ok((key, img)) = self.cover_rx.try_recv() {
            let tex = img.map(|i| ctx.load_texture(key.to_string_lossy(), i, Default::default()));
            self.covers.insert(key, tex);
        }
        if let Some(t) = self.covers.get(&f.dir) {
            return t.clone();
        }
        if f.cover.is_none() && f.cover_url.is_none() {
            return None;
        }
        self.covers.insert(f.dir.clone(), None);
        let _ = self.cover_tx.send((f.dir.clone(), f.cover.clone(), f.cover_url.clone()));
        None
    }

    fn add(&mut self, f: &Found) {
        match self.status(f) {
            Status::Blocked(why) => {
                self.message = Some((format!("{} can't be added: it {why}. Achievement Hub never runs in games with anti-cheat.", f.name), false));
            }
            Status::Checking => self.message = Some(("Still checking this game for anti-cheat. Try again in a moment.".into(), false)),
            _ => {
                if !self.library.contains(f) {
                    self.library.game.push(Mapped::from_found(f));
                    self.library.save();
                }
                self.message = Some((format!("Added {} to your library.", f.name), true));
                self.view = View::Library;
            }
        }
    }
}

/// A coloured status chip.
fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.22))
        .stroke(Stroke::new(1.0, color))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| ui.label(RichText::new(text).color(color).size(12.0).strong()));
}

/// Xbox 360 style blade tab.
fn blade(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    let text = RichText::new(label).strong().size(13.0).color(if active { INK } else { TEXT });
    let button = egui::Button::new(text)
        .fill(if active { GREEN } else { PANEL })
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(2))
        .min_size(Vec2::new(130.0, 32.0));
    ui.add(button).clicked()
}

impl eframe::App for Hub {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.social_started {
            self.social_started = true;
            self.refresh_social();
        }
        if let Some(rx) = &self.sync_rx {
            if let Ok(result) = rx.try_recv() {
                self.sync_rx = None;
                match result {
                    Ok(changed) if !changed.is_empty() => {
                        self.defs = catalogue::all();
                        self.statuses.borrow_mut().clear();
                        self.message = Some((format!("Game database updated: {} game file(s) new or changed.", changed.len()), true));
                    }
                    Ok(_) => {}
                    // Offline or not published yet: the built-in games still work.
                    Err(e) => eprintln!("game database: {e}"),
                }
            }
        }
        if let Some(rx) = &self.scan_rx {
            if let Ok(found) = rx.try_recv() {
                self.found = Some(found);
                self.scan_rx = None;
            }
        }
        if let Some(rx) = &self.browse_rx {
            if let Ok(pick) = rx.try_recv() {
                self.browse_rx = None;
                if let Some(exe) = pick {
                    let f = scan::from_exe(&exe);
                    self.add(&f);
                }
            }
        }
        if let Some(rx) = &self.request_rx {
            if let Ok(result) = rx.try_recv() {
                self.request_rx = None;
                self.message = Some(match result {
                    Ok(()) => (format!("Request for {} sent. Thanks!", self.request.game), true),
                    Err(e) => (e, false),
                });
                if self.message.as_ref().is_some_and(|m| m.1) {
                    self.request.note.clear();
                }
            }
        }
        // Keep repainting while background work (scan, Steam checks) runs.
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        egui::Panel::top("top").frame(egui::Frame::new().fill(PANEL).inner_margin(egui::Margin::symmetric(20, 14))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("ACHIEVEMENT HUB").color(GREEN).strong().size(15.0));
                ui.add_space(24.0);
                for (view, label) in [(View::Library, "LIBRARY"), (View::Add, "ADD GAME"), (View::Supported, "SUPPORTED GAMES"), (View::Request, "REQUEST A GAME"), (View::Friends, "FRIENDS")] {
                    if blade(ui, label, self.view == view) {
                        self.view = view;
                        self.message = None;
                    }
                }
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(20))).show(ui, |ui| {
            if let Some((msg, good)) = &self.message {
                let color = if *good { GREEN } else { AMBER };
                egui::Frame::new().fill(color.gamma_multiply(0.15)).stroke(Stroke::new(1.0, color)).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(msg).color(TEXT));
                });
                ui.add_space(12.0);
            }
            match self.view {
                View::Library => self.library_view(ui, &ctx),
                View::Add => self.add_view(ui, &ctx),
                View::Supported => self.supported_view(ui),
                View::Request => self.request_view(ui),
                View::Friends => self.friends_view(ui),
            }
        });
    }
}

impl Hub {
    fn library_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.library.game.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Your library is empty").size(22.0).color(TEXT).strong());
                ui.label(RichText::new("Add the games you play, and Achievement Hub will set them up.").color(MUTED));
                ui.add_space(12.0);
                if ui.add(egui::Button::new(RichText::new("ADD A GAME").strong().color(INK)).fill(GREEN)).clicked() {
                    self.view = View::Add;
                }
            });
            return;
        }
        let mut remove = None;
        let mut install = None;
        let mut uninstall = None;
        let mut request = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, g) in self.library.game.clone().iter().enumerate() {
                let found = Found {
                    name: g.name.clone(),
                    store: g.store,
                    store_id: g.store_id.clone(),
                    dir: g.dir.clone(),
                    launch: g.launch.clone(),
                    game_exes: g.game_exes.clone(),
                    cover: self.found.as_ref().and_then(|f| f.iter().find(|x| g.same(x)).and_then(|x| x.cover.clone())),
                    // (Before the scan finishes, the store's art still works.)
                    cover_url: self.found.as_ref().and_then(|f| f.iter().find(|x| g.same(x)).and_then(|x| x.cover_url.clone())).or_else(|| match g.store {
                        scan::Store::Steam => Some(format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/library_600x900.jpg", g.store_id)),
                        scan::Store::Gog => Some(format!("gog:{}", g.store_id)),
                        _ => None,
                    }),
                    heroic_app: g.heroic_app.clone(),
                };
                let status = self.status(&found);
                let def = catalogue::for_exes(&self.defs, &g.game_exes).first().map(|d| (d.game.id.clone(), d.achievement.len(), d.total_points(), d.trackable()));
                let cover = self.cover(ctx, &found);
                egui::Frame::new().fill(CARD).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        match cover {
                            Some(t) => {
                                ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::new(80.0, 120.0)));
                            }
                            None => {
                                let (r, _) = ui.allocate_exact_size(Vec2::new(80.0, 120.0), egui::Sense::hover());
                                ui.painter().rect_filled(r, 2.0, PANEL);
                            }
                        }
                        ui.add_space(10.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&g.name).size(22.0).strong().color(Color32::WHITE));
                            ui.horizontal(|ui| {
                                chip(ui, g.store.label(), MUTED);
                                match &status {
                                    Status::Blocked(why) => chip(ui, &format!("BLOCKED: {why}"), RED),
                                    Status::Checking => chip(ui, "CHECKING ANTI-CHEAT…", AMBER),
                                    Status::Unverified(_) => chip(ui, "OVERLAY UNAVAILABLE: CAN'T VERIFY ANTI-CHEAT", AMBER),
                                    Status::Supported(_) => chip(ui, "ACHIEVEMENTS READY", GREEN),
                                    Status::Mappable => chip(ui, "NOT AVAILABLE YET", MUTED),
                                }
                                if g.overlay_installed {
                                    chip(ui, "OVERLAY INSTALLED", GREEN);
                                }
                            });
                            if let Some((id, n, pts, trackable)) = &def {
                                let got = self.unlocks.get(id).copied().unwrap_or(0);
                                ui.label(RichText::new(format!("{got} of {n} achievements · {pts} G · {trackable} tracked so far")).color(MUTED));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Remove").clicked() {
                                remove = Some(i);
                            }
                            if g.overlay_installed && ui.button("Remove overlay").clicked() {
                                uninstall = Some(i);
                            }
                            match &status {
                                Status::Supported(_) => {
                                    let label = if g.overlay_installed { "Reinstall overlay" } else { "Install overlay" };
                                    let button = egui::Button::new(RichText::new(label).strong().color(INK)).fill(GREEN);
                                    if ui.add(button).clicked() {
                                        install = Some(i);
                                    }
                                }
                                Status::Mappable => {
                                    if ui.button("Request achievements").clicked() {
                                        request = Some(i);
                                    }
                                }
                                _ => {}
                            }
                        });
                    });
                });
                ui.add_space(8.0);
            }
        });
        if let Some(i) = remove {
            let g = self.library.game.remove(i);
            self.library.save();
            self.message = Some((format!("Removed {} from your library.", g.name), true));
        }
        if let Some(i) = uninstall {
            let g = self.library.game[i].clone();
            self.message = Some(match library::uninstall_overlay(&g) {
                Ok(msg) => {
                    self.library.game[i].overlay_installed = false;
                    self.library.save();
                    (msg, true)
                }
                Err(e) => (format!("Couldn't remove the overlay: {e}"), false),
            });
        }
        if let Some(i) = request {
            let g = self.library.game[i].clone();
            self.request.game = g.name.clone();
            self.request.store = format!("{} {}", g.store.label(), g.store_id).trim().to_string();
            self.request.exe = g
                .game_exes
                .iter()
                .map(|e| {
                    let name = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    match catalogue::exe_build(e) {
                        Some((t, s, x64)) => format!("{name} {t:08x}/{s:x} {}", if x64 { "64-bit" } else { "32-bit" }),
                        None => name,
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            self.view = View::Request;
            self.message = Some((format!("Achievements aren't available for {} yet. Send a request and we'll look at it.", g.name), true));
        }
        if let Some(i) = install {
            let g = self.library.game[i].clone();
            let defs = catalogue::for_exes(&self.defs, &g.game_exes);
            self.message = Some(match library::install_overlay_proxy(&g, &defs) {
                Ok(msg) => {
                    self.library.game[i].overlay_installed = true;
                    self.library.save();
                    (msg, true)
                }
                Err(e) => (format!("Couldn't install the overlay: {e}"), false),
            });
        }
    }

    /// Every game with an achievement set, and whether you have it.
    fn supported_view(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Supported games").size(20.0).strong().color(Color32::WHITE));
        ui.label(RichText::new("Games with achievement sets. A set matches one exact version of the game; other versions show here as soon as they're supported.").color(MUTED));
        ui.add_space(10.0);
        let found = self.found.clone().unwrap_or_default();
        let mut add = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for d in &self.defs {
                let pts: u32 = d.achievement.iter().map(|a| a.points).sum();
                let tracked = d.trackable();
                let yours = found.iter().find(|f| catalogue::for_exes(std::slice::from_ref(d), &f.game_exes).first().is_some()).cloned();
                egui::Frame::new().fill(CARD).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            let title = match &d.game.subtitle {
                                Some(s) => format!("{} ({s})", d.game.title),
                                None => d.game.title.clone(),
                            };
                            ui.label(RichText::new(title).size(17.0).strong().color(Color32::WHITE));
                            let mut who = d.game.developer.clone().unwrap_or_default();
                            if let Some(y) = d.game.year {
                                who = if who.is_empty() { y.to_string() } else { format!("{who}, {y}") };
                            }
                            if !who.is_empty() {
                                ui.label(RichText::new(who).color(MUTED));
                            }
                            ui.horizontal(|ui| {
                                chip(ui, &format!("{} ACHIEVEMENTS", d.achievement.len()), GREEN);
                                chip(ui, &format!("{pts} G"), GREEN);
                                if tracked < d.achievement.len() {
                                    chip(ui, &format!("{tracked} TRACKED SO FAR"), AMBER);
                                }
                                for b in &d.build {
                                    let exe = b.exe.clone().unwrap_or_else(|| "exe".into());
                                    chip(ui, &format!("{exe} {:08x}", b.timestamp), MUTED);
                                }
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &yours {
                            Some(f) if self.library.contains(f) => {
                                ui.label(RichText::new("In your library").color(GREEN));
                            }
                            Some(f) => {
                                if ui.add(egui::Button::new(RichText::new("Add").strong().color(INK)).fill(GREEN).min_size(Vec2::new(70.0, 26.0))).clicked() {
                                    add = Some(f.clone());
                                }
                                ui.label(RichText::new("Installed").color(GREEN));
                            }
                            None => {
                                ui.label(RichText::new("Not installed (or a different version)").color(MUTED));
                            }
                        });
                    });
                });
                ui.add_space(6.0);
            }
        });
        if let Some(f) = add {
            self.add(&f);
        }
    }

    fn add_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Installed games").size(20.0).strong().color(Color32::WHITE));
            ui.add_space(16.0);
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search…").desired_width(240.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let browsing = self.browse_rx.is_some();
                if ui.add_enabled(!browsing, egui::Button::new("Browse for a game's .exe…")).clicked() {
                    let (tx, rx) = channel();
                    std::thread::spawn(move || {
                        let pick = rfd::FileDialog::new().add_filter("Game", &["exe", "EXE"]).set_title("Pick the game's .exe").pick_file();
                        let _ = tx.send(pick);
                    });
                    self.browse_rx = Some(rx);
                }
            });
        });
        ui.label(RichText::new("Found in your Steam and Heroic (GOG/Epic) libraries. Games with anti-cheat are blocked to keep your accounts safe.").color(MUTED));
        ui.add_space(10.0);

        let Some(found) = self.found.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Scanning your libraries…");
            });
            return;
        };
        let q = self.search.to_lowercase();
        let mut to_add = None;
        let shown: Vec<&Found> = found.iter().filter(|f| q.is_empty() || f.name.to_lowercase().contains(&q)).collect();
        // Only the rows on screen are built each frame.
        let row_h = 116.0;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, shown.len(), |ui, range| {
            for f in shown[range].iter().copied() {
                let status = self.status(f);
                let in_library = self.library.contains(f);
                let cover = self.cover(ctx, f);
                let resp = egui::Frame::new().fill(CARD).inner_margin(8).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        match cover {
                            Some(t) => {
                                ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::new(64.0, 96.0)));
                            }
                            None => {
                                let (r, _) = ui.allocate_exact_size(Vec2::new(64.0, 96.0), egui::Sense::hover());
                                ui.painter().rect_filled(r, 2.0, PANEL);
                            }
                        }
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&f.name).size(20.0).strong().color(Color32::WHITE));
                            ui.horizontal(|ui| {
                                chip(ui, f.store.label(), MUTED);
                                match &status {
                                    Status::Blocked(why) => chip(ui, &format!("BLOCKED: {why}"), RED),
                                    Status::Checking => chip(ui, "CHECKING…", AMBER),
                                    Status::Unverified(why) => chip(ui, &format!("UNVERIFIED: {why}"), AMBER),
                                    Status::Supported(n) => chip(ui, &format!("{n} ACHIEVEMENTS"), GREEN),
                                    Status::Mappable => chip(ui, "NO SET YET", MUTED),
                                }
                            });
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if in_library {
                                ui.label(RichText::new("In library").color(GREEN));
                            } else {
                                let ok = matches!(status, Status::Supported(_) | Status::Mappable | Status::Unverified(_));
                                let button = egui::Button::new(RichText::new("Add").strong().color(if ok { INK } else { MUTED }))
                                    .fill(if ok { GREEN } else { PANEL })
                                    .min_size(Vec2::new(70.0, 26.0));
                                if ui.add_enabled(ok, button).clicked() {
                                    to_add = Some(f.clone());
                                }
                            }
                        });
                    });
                });
                if resp.response.hovered() {
                    ui.painter().rect_stroke(resp.response.rect, 0.0, Stroke::new(1.0, CARD_HOVER), egui::StrokeKind::Inside);
                }
                let used = resp.response.rect.height();
                ui.add_space((row_h - used).max(0.0));
            }
        });
        if let Some(f) = to_add {
            self.add(&f);
        }
    }

    /// Publishes this player's profile and refreshes friends', in the
    /// background (when the app is linked to an account).
    fn refresh_social(&mut self) {
        let Some(s) = self.social.clone() else { return };
        if self.publish_rx.is_none() {
            let (tx, rx) = channel();
            let s2 = s.clone();
            std::thread::spawn(move || {
                let _ = tx.send(friends::publish(&s2));
            });
            self.publish_rx = Some(rx);
            self.publish_status = "Updating your profile…".into();
        }
        if self.friends_rx.is_none() {
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let _ = tx.send(friends::fetch(&s));
            });
            self.friends_rx = Some(rx);
        }
    }

    fn friends_view(&mut self, ui: &mut egui::Ui) {
        if let Some(rx) = &self.publish_rx {
            if let Ok(r) = rx.try_recv() {
                self.publish_rx = None;
                self.publish_status = match r {
                    Ok(tag) => {
                        if let Some(s) = self.social.as_mut().filter(|s| s.gamertag != tag) {
                            friends::use_gamertag(&tag);
                            s.gamertag = tag;
                            friends::social::save(&library::profile_dir(), s);
                        }
                        "Your profile is up to date.".into()
                    }
                    Err(e) => format!("Couldn't update your profile: {e}"),
                };
            }
        }
        if let Some(rx) = &self.friends_rx {
            if let Ok(r) = rx.try_recv() {
                self.friends_rx = None;
                match r {
                    Ok(p) => self.friend_profiles = p,
                    Err(e) => self.publish_status = e,
                }
            }
        }
        if let Some(rx) = &self.link_rx {
            if let Ok(r) = rx.try_recv() {
                self.link_rx = None;
                match r {
                    Ok(s) => {
                        self.message = Some((format!("Linked to {}. Your progress now shows on your profile.", s.gamertag), true));
                        self.social = Some(s);
                        self.add_code.clear();
                        self.refresh_social();
                    }
                    Err(e) => self.message = Some((e, false)),
                }
            }
        }
        let site = friends::social::site();
        ui.label(RichText::new("Friends").size(20.0).strong().color(Color32::WHITE));
        ui.add_space(8.0);
        let Some(me) = self.social.clone() else {
            // Not linked yet: a code from the website links it.
            egui::Frame::new().fill(PANEL).inner_margin(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Link your Achievement Hub account").size(17.0).strong().color(Color32::WHITE));
                ui.label(RichText::new("Your Gamerscore and unlocks show on your profile, and your friends show up here and in the overlay.").color(MUTED));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("1.");
                    ui.hyperlink_to("Create an account or log in on the website", format!("{site}/signup.html"));
                });
                ui.horizontal(|ui| {
                    ui.label("2.");
                    ui.hyperlink_to("Open Account → Link the app and get a code", format!("{site}/account.html"));
                });
                ui.horizontal(|ui| {
                    ui.label("3.");
                    ui.add(egui::TextEdit::singleline(&mut self.add_code).desired_width(140.0).hint_text("Link code").char_limit(6));
                    let busy = self.link_rx.is_some();
                    let ok = self.add_code.trim().len() == 6 && !busy;
                    if ui.add_enabled(ok, egui::Button::new(if busy { "Linking…" } else { "Link" })).clicked() {
                        let (tx, rx) = channel();
                        let code = self.add_code.clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(friends::link(&code));
                        });
                        self.link_rx = Some(rx);
                    }
                });
            });
            return;
        };
        egui::Frame::new().fill(PANEL).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Signed in as").color(MUTED));
                ui.label(RichText::new(&me.gamertag).size(17.0).strong().color(GREEN));
                ui.hyperlink_to("Your profile", format!("{site}/profile.html?u={}", me.gamertag));
                ui.hyperlink_to("Add friends", format!("{site}/friends.html"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Unlink").clicked() {
                        friends::unlink(&me);
                        self.social = None;
                        self.friend_profiles.clear();
                    }
                    if ui.small_button("Refresh").clicked() {
                        self.refresh_social();
                    }
                });
            });
            if !self.publish_status.is_empty() {
                ui.label(RichText::new(&self.publish_status).color(MUTED));
            }
            if ui.link("Forgot your website password?").clicked() {
                self.show_reset = !self.show_reset;
            }
            if self.show_reset {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_password).password(true).desired_width(220.0).hint_text("New password (8+ characters)"));
                    if ui.add_enabled(self.new_password.len() >= 8, egui::Button::new("Set password")).clicked() {
                        self.message = Some(match friends::reset_password(&me, &self.new_password) {
                            Ok(()) => ("Website password changed. Log in on the website with it.".into(), true),
                            Err(e) => (format!("Couldn't change it: {e}"), false),
                        });
                        self.new_password.clear();
                        self.show_reset = false;
                    }
                });
            }
        });
        ui.add_space(12.0);
        if self.friend_profiles.is_empty() {
            ui.label(RichText::new(if self.friends_rx.is_some() { "Loading friends…" } else { "No friends yet. Add them by gamertag on the website." }).color(MUTED));
            return;
        }
        let now = friends::social::now();
        let mut list = self.friend_profiles.clone();
        list.sort_by(|a, b| (b.online(now), b.gamerscore).cmp(&(a.online(now), a.gamerscore)));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for p in &list {
                egui::Frame::new().fill(CARD).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if p.online(now) { "●" } else { "○" }).color(if p.online(now) { GREEN } else { MUTED }));
                        ui.label(RichText::new(&p.gamertag).size(17.0).strong().color(Color32::WHITE));
                        ui.label(RichText::new(format!("{} G", p.gamerscore)).color(GREEN));
                        if !p.motto.is_empty() {
                            ui.label(RichText::new(&p.motto).color(MUTED));
                        }
                    });
                    match (&p.playing, p.online(now)) {
                        (Some(game), true) => {
                            ui.label(RichText::new(format!("Playing {game}")).color(GREEN));
                        }
                        (_, true) => {
                            ui.label(RichText::new("Online").color(GREEN));
                        }
                        _ => {}
                    }
                    for r in p.recent.iter().take(3) {
                        ui.label(RichText::new(format!("{}  -  {} ({} G)", r.title, r.name, r.points)).color(TEXT));
                    }
                });
                ui.add_space(8.0);
            }
        });
    }

    /// Record-mode findings ready to share (shown above the request form).
    fn share_panel(&mut self, ui: &mut egui::Ui) {
        // Mapping is opt-in: record mode only runs for players who choose it.
        egui::Frame::new().fill(PANEL).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let mut on = mapper();
            if ui.checkbox(&mut on, RichText::new("Help map achievements (record mode)").strong().color(Color32::WHITE)).changed() {
                set_mapper(on);
            }
            ui.label(RichText::new(
                "Off by default. When on, the overlay watches games that still have unmapped achievements, and pressing A on one right after you earn it records what changed. Takes effect next time you start a game.",
            ).color(MUTED));
            ui.hyperlink_to("Want to help more? Apply to be a mapper", format!("{}/mappers.html", friends::social::site()));
        });
        ui.add_space(12.0);
        if !mapper() {
            return;
        }
        if let Some(rx) = &self.share_rx {
            if let Ok(result) = rx.try_recv() {
                self.share_rx = None;
                self.message = Some(match result {
                    Ok(n) => (format!("Shared {n} mapping(s). Thanks! Once checked, they're published for everyone."), true),
                    Err(e) => (format!("Couldn't share: {e}"), false),
                });
            }
        }
        let ready = mappings::ready(&self.defs);
        if ready.is_empty() {
            return;
        }
        egui::Frame::new().fill(PANEL).stroke(Stroke::new(1.0, GREEN)).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Share your mappings").size(17.0).strong().color(Color32::WHITE));
            ui.label(
                RichText::new("You pinned these down with record mode. Sharing sends what you found so they can unlock automatically for everyone.").color(MUTED),
            );
            ui.add_space(6.0);
            for r in &ready {
                ui.label(RichText::new(format!(
                    "{}  -  {}  ({} recording{})",
                    r.game_title,
                    r.name,
                    r.recordings,
                    if r.recordings == 1 { "" } else { "s" }
                ))
                .color(TEXT));
            }
            ui.add_space(8.0);
            let sending = self.share_rx.is_some();
            let ok = requests::enabled() && !sending;
            let button = egui::Button::new(RichText::new(if sending { "Sharing…" } else { "Share" }).strong().color(if ok { INK } else { MUTED }))
                .fill(if ok { GREEN } else { PANEL })
                .min_size(Vec2::new(120.0, 30.0));
            if ui.add_enabled(ok, button).clicked() {
                let (tx, rx) = channel();
                let tag = gamertag();
                std::thread::spawn(move || {
                    let _ = tx.send(mappings::share(&ready, &tag));
                });
                self.share_rx = Some(rx);
            }
        });
        ui.add_space(16.0);
    }

    fn request_view(&mut self, ui: &mut egui::Ui) {
        self.share_panel(ui);
        ui.label(RichText::new("Request a game").size(20.0).strong().color(Color32::WHITE));
        ui.label(RichText::new("Tell us which game you'd like achievements for. Requests go to the Achievement Hub wishlist.").color(MUTED));
        ui.add_space(12.0);

        // Quick-pick from installed games fills in store/exe details.
        if let Some(found) = self.found.clone() {
            egui::ComboBox::from_label("Pick an installed game (optional)")
                .selected_text(if self.request.game.is_empty() { "Choose…".to_string() } else { self.request.game.clone() })
                .width(320.0)
                .show_ui(ui, |ui| {
                    for f in &found {
                        if ui.selectable_label(self.request.game == f.name, &f.name).clicked() {
                            self.request.game = f.name.clone();
                            self.request.store = format!("{} {}", f.store.label(), f.store_id).trim().to_string();
                            self.request.exe = f
                                .game_exes
                                .iter()
                                .map(|e| {
                                    let name = e.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                    match catalogue::exe_build(e) {
                                        Some((t, s, x64)) => format!("{name} {t:08x}/{s:x} {}", if x64 { "64-bit" } else { "32-bit" }),
                                        None => name,
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.request.anticheat = match self.safety.check(f) {
                                Verdict::Blocked(why) => why,
                                Verdict::Checking => "not checked yet".into(),
                                Verdict::Unverified(why) => format!("unverified: {why}"),
                                Verdict::Clear => "none detected".into(),
                            };
                        }
                    }
                });
            ui.add_space(8.0);
        }
        egui::Grid::new("request").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Game");
            ui.add(egui::TextEdit::singleline(&mut self.request.game).desired_width(400.0));
            ui.end_row();
            ui.label("Store");
            ui.add(egui::TextEdit::singleline(&mut self.request.store).desired_width(400.0).hint_text("Steam, GOG, Epic…"));
            ui.end_row();
            ui.label("Gamertag");
            ui.add(egui::TextEdit::singleline(&mut self.request.gamertag).desired_width(400.0).hint_text("optional"));
            ui.end_row();
            ui.label("Note");
            ui.add(egui::TextEdit::multiline(&mut self.request.note).desired_width(400.0).desired_rows(3).hint_text("What would you like to earn?"));
            ui.end_row();
        });
        ui.add_space(12.0);
        let sending = self.request_rx.is_some();
        let ready = requests::enabled() && !self.request.game.trim().is_empty() && !sending;
        let button = egui::Button::new(RichText::new(if sending { "Sending…" } else { "Send request" }).strong().color(if ready { INK } else { MUTED }))
            .fill(if ready { GREEN } else { PANEL })
            .min_size(Vec2::new(160.0, 32.0));
        if ui.add_enabled(ready, button).clicked() {
            let (tx, rx) = channel();
            let req = self.request.clone();
            std::thread::spawn(move || {
                let _ = tx.send(requests::send(&req));
            });
            self.request_rx = Some(rx);
        }
        if !requests::enabled() {
            ui.label(RichText::new("Requests aren't connected yet.").color(AMBER));
        }
    }
}

/// `hub --scan`: print what was detected and each game's status.
fn print_scan() {
    let profile = library::profile_dir();
    let safety = Safety::new(&profile);
    let defs = catalogue::all();
    let found = scan::installed();
    // First pass starts Steam lookups; give them a moment.
    for f in &found {
        safety.check(f);
    }
    std::thread::sleep(std::time::Duration::from_secs(6));
    for f in &found {
        let verdict = match safety.check(f) {
            Verdict::Blocked(why) => format!("BLOCKED ({why})"),
            Verdict::Checking => "checking".into(),
            Verdict::Unverified(why) => format!("unverified ({why})"),
            Verdict::Clear => match catalogue::for_exes(&defs, &f.game_exes).first() {
                Some(d) => format!("achievements: {}", d.game.id),
                None => "can map".into(),
            },
        };
        let exes: Vec<String> = f.game_exes.iter().filter_map(|e| e.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
        println!("{:38} {:6} {:50} {}", f.name.chars().take(38).collect::<String>(), f.store.label(), verdict, exes.join(","));
    }
}

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--test-request") {
        let req = requests::Request {
            game: "TEST from Achievement Hub (safe to delete)".into(),
            store: "test".into(),
            note: "Connection test".into(),
            ..Default::default()
        };
        match requests::send(&req) {
            Ok(()) => println!("request sent: sheet said ok"),
            Err(e) => println!("request failed: {e}"),
        }
        return Ok(());
    }
    if std::env::args().any(|a| a == "--profile-json") {
        println!("{}", serde_json::to_string_pretty(&friends::social::snapshot(&library::profile_dir(), None)).unwrap_or_default());
        return Ok(());
    }
        // `hub --link CODE`: link this computer to a website account.
    if let Some(i) = std::env::args().position(|a| a == "--link") {
        match std::env::args().nth(i + 1).map(|c| friends::link(&c)) {
            Some(Ok(s)) => {
                println!("linked to {}", s.gamertag);
                match friends::publish(&s) {
                    Ok(_) => println!("profile published"),
                    Err(e) => println!("publish: {e}"),
                }
                match friends::fetch(&s) {
                    Ok(f) => println!("friends: {}", f.iter().map(|p| format!("{} ({} G)", p.gamertag, p.gamerscore)).collect::<Vec<_>>().join(", ")),
                    Err(e) => println!("friends: {e}"),
                }
            }
            Some(Err(e)) => println!("link: {e}"),
            None => println!("usage: hub --link CODE"),
        }
        return Ok(());
    }
        if std::env::args().any(|a| a == "--mappings") {
        for r in mappings::ready(&catalogue::all()) {
            println!("{} / {} ({} recordings)", r.game, r.achievement, r.recordings);
        }
        return Ok(());
    }
        if std::env::args().any(|a| a == "--sync") {
        match dbsync::sync() {
            Ok(changed) => println!("game database: {} updated {:?}", changed.len(), changed),
            Err(e) => println!("game database: {e}"),
        }
        return Ok(());
    }
        if std::env::args().any(|a| a == "--scan") {
        print_scan();
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("Achievement Hub").with_inner_size([1100.0, 720.0]).with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native("Achievement Hub", options, Box::new(|cc| Ok(Box::new(Hub::new(cc)))))
}
