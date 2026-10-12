//! The overlay panel, Xbox 360 "blades" style: the blades tile the panel,
//! the active one opens wide and glossy green while the others fold into
//! silver tabs at the edges with sideways labels. Everything animates:
//! opening, blade switches, hover, selection, scrolling.

pub mod anim;
pub mod list;
pub mod theme;

use crate::games::Game;
use crate::input;
use crate::media::{Kind, Media};
use crate::profile::{format_ago, Profile};
use crate::Fonts;
use anim::{ease_out_back, ease_out_cubic, lerp, lerp_color, Anim};
use hudhook::imgui::{Condition, DrawListMut, MouseButton, StyleColor, StyleVar, Ui, WindowFlags};
use list::AchList;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use theme::*;
use theme::p as pal;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Blade {
    Home,
    NowPlaying,
    Games,
    Requests,
    Friends,
}

/// Blades in order: tab label and heading.
impl Blade {
    /// By tab label or a short name ("now", "games"...), for the preview tool.
    pub fn from_name(name: &str) -> Option<Blade> {
        let n = name.to_ascii_lowercase();
        BLADES.iter().find(|(_, label, _)| label.replace(' ', "").starts_with(&n.replace(' ', ""))).map(|(b, ..)| *b)
    }
}

const BLADES: [(Blade, &str, &str); 5] = [
    (Blade::Home, "home", "Welcome back"),
    (Blade::NowPlaying, "now playing", "Now Playing"),
    (Blade::Games, "games", "My Games"),
    (Blade::Requests, "requests", "Request a Game"),
    (Blade::Friends, "friends", "Friends"),
];

/// The open blade's body (between its label strip and right edge) and the
/// tops of its header and footer bands, in screen coordinates.
struct BladeArea {
    left: f32,
    right: f32,
    header: f32,
    footer: f32,
}

/// Where a design puts the blade's contents.
struct Layout {
    pad_x: f32,
    top: f32,
    /// Space kept below the contents for the button hints.
    footer: f32,
    /// Blades: the heading and gamer card go in the header band, and the
    /// hints in the footer band, in the dashboard's arrangement.
    blades: Option<BladeArea>,
}

/// Blades Home menu: four items plus the game tile.
const HOME_ITEMS: usize = 6;

/// The Settings screen's rows.
const SETTING_ROWS: usize = 6;
const VOLUMES: [u8; 4] = [25, 50, 75, 100];
const HOTKEYS: [(&str, &str); 5] = [("shift+f2", "Shift+F2"), ("f1", "F1"), ("f9", "F9"), ("home", "Home"), ("insert", "Insert")];

/// Width of a folded blade (and of the active blade's label strip).
const TAB: f32 = 58.0;

/// Why a game is being requested (the first line of the note).
const REASONS: [&str; 4] =
    ["Add achievements for this game", "Add more achievements to this game", "Report a broken achievement", "Something else"];

/// Edge detection for keys and controller buttons read once per frame.
#[derive(Default)]
struct Edges {
    prev_keys: u64,
    prev_pad: u32,
}

const KEYS: [u16; 7] = [b'Q' as u16, b'E' as u16, b'F' as u16, 0x0D /* Enter */, 0x1B /* Esc */, 0x08 /* Backspace */, b'D' as u16];
const KEY_Q: u64 = 1;
const KEY_E: u64 = 2;
const KEY_F: u64 = 4;
const KEY_ENTER: u64 = 8;
const KEY_ESC: u64 = 16;
const KEY_BACK: u64 = 32;
const KEY_D: u64 = 64;
const PAD_A: u32 = 0x1000;
const PAD_B: u32 = 0x2000;
const PAD_X: u32 = 0x4000;
const PAD_Y: u32 = 0x8000;
const PAD_LB: u32 = 0x0100;
const PAD_RB: u32 = 0x0200;

impl Edges {
    /// Returns (keys newly pressed bitmask over KEYS, pad buttons newly pressed).
    fn update(&mut self, focused: bool) -> (u64, u32) {
        let mut keys = 0u64;
        if focused {
            for (i, &k) in KEYS.iter().enumerate() {
                // SAFETY: plain key state query (the real one, not the game's).
                if unsafe { GetAsyncKeyState(k as i32) } < 0 {
                    keys |= 1 << i;
                }
            }
        }
        let pad = input::PAD_BUTTONS.load(Ordering::Relaxed);
        let out = (keys & !self.prev_keys, pad & !self.prev_pad);
        self.prev_keys = keys;
        self.prev_pad = pad;
        out
    }
}

#[derive(Default)]
pub struct Panel {
    pub blade: Option<Blade>,
    edges: Edges,
    anim: Anim,
    /// Blade shown last frame and the direction of the last switch.
    shown_blade: Option<Blade>,
    blade_dir: f32,
    /// The running game's list.
    now: AchList,
    /// Games blade: selection, the game opened (if any) and its list.
    games_sel: usize,
    games_order: Vec<String>,
    games_scroll_pending: bool,
    games_scroll_target: Option<f32>,
    open_game: Option<String>,
    opened: AchList,
    /// Requests blade: highlighted row (reasons, then Send), chosen reason,
    /// typed note, request in flight and the last result.
    req_sel: usize,
    req_reason: usize,
    req_note: String,
    /// (Mutex only so the overlay is Sync, as hudhook requires.)
    req_rx: Option<std::sync::Mutex<Receiver<Result<(), String>>>>,
    req_status: Option<(String, bool)>,
    /// Blades Home: the highlighted menu item.
    home_sel: usize,
    /// Record mode: the achievement (index) the player says they just
    /// earned (handled by the caller).
    pub record_request: Option<usize>,
    /// Why an A press didn't record (for the log).
    pub record_refused: Option<String>,
    /// A dashboard design the player just picked (saved by the caller).
    pub skin_request: Option<Skin>,
    /// The Settings screen: open on the highlighted row.
    settings_row: Option<usize>,
    /// Settings the player just changed (saved and applied by the caller).
    pub settings_request: Option<crate::profile::Settings>,
    /// Where the panel sits and how much it's scaled (for mapping the mouse).
    view: ([f32; 2], f32),
    /// Design shown last frame (to animate a change).
    shown_skin: Option<Skin>,
}

impl Panel {
    /// Preview tool: step the Now Playing filter (all, unlocked, locked).
    pub fn preview_filter(&mut self, steps: usize) {
        for _ in 0..steps {
            self.now.cycle_filter();
        }
    }

    pub fn open_settings(&mut self) {
        crate::sound::play(crate::sound::Sfx::Select);
        self.settings_row = Some(0);
        self.anim.set("settings_t", 0.0);
    }

    /// Changes the highlighted setting one step (`dir` -1 or +1).
    fn change_setting(&mut self, row: usize, dir: i32, current: &crate::profile::Settings, skin: Skin) {
        let mut s = self.settings_request.clone().unwrap_or_else(|| current.clone());
        let step = |len: usize, at: usize| ((at as i64 + dir as i64).rem_euclid(len as i64)) as usize;
        match row {
            0 => s.sounds = !s.sounds,
            1 => s.volume = VOLUMES[step(VOLUMES.len(), VOLUMES.iter().position(|v| *v == s.volume).unwrap_or(3))],
            2 => s.popups = if s.popups == "top" { "bottom".into() } else { "top".into() },
            3 => s.screenshots = !s.screenshots,
            4 => s.hotkey = HOTKEYS[step(HOTKEYS.len(), HOTKEYS.iter().position(|(k, _)| *k == s.hotkey).unwrap_or(0))].0.into(),
            _ => {
                self.skin_request = Some(if dir > 0 { skin.next() } else { skin.next().next() });
                return;
            }
        }
        crate::sound::set_enabled(s.sounds);
        crate::sound::set_volume(s.volume);
        crate::sound::play(crate::sound::Sfx::Select);
        self.settings_request = Some(s);
    }

    /// Keys and buttons while Settings is open: up/down (via `navigate`)
    /// pick a row, left/right or A change it, B closes.
    fn settings_input(&mut self, keys: u64, pad: u32, current: &crate::profile::Settings, skin: Skin) {
        let Some(row) = self.settings_row else { return };
        let h = input::HNAV.swap(0, Ordering::Relaxed);
        let dir = if h != 0 { h.signum() } else if keys & KEY_ENTER != 0 || pad & PAD_A != 0 { 1 } else { 0 };
        if dir != 0 {
            self.change_setting(row, dir, current, skin);
        } else if keys & (KEY_ESC | KEY_BACK) != 0 || pad & PAD_B != 0 {
            crate::sound::play(crate::sound::Sfx::Back);
            self.settings_row = None;
        }
    }

    /// The Settings screen, over a dimmed panel.
    fn draw_settings(&mut self, ui: &Ui, fonts: &Fonts, profile: &Profile, skin: Skin, origin: [f32; 2], width: f32, height: f32) {
        let Some(sel) = self.settings_row else { return };
        let s = self.settings_request.clone().unwrap_or_else(|| profile.settings.clone());
        let t = ease_out_cubic(self.anim.timer("settings_t", 0.25));
        let hotkey = HOTKEYS.iter().find(|(k, _)| *k == s.hotkey).map_or("Shift+F2", |(_, l)| l);
        let rows: [(&str, String, &str); SETTING_ROWS] = [
            ("Sounds", if s.sounds { "On" } else { "Off" }.into(), ""),
            ("Volume", format!("{}%", s.volume), ""),
            ("Pop-ups appear", if s.popups == "top" { "At the top" } else { "At the bottom" }.into(), ""),
            ("Unlock screenshots", if s.screenshots { "On" } else { "Off" }.into(), "A picture of the moment you earn each achievement"),
            ("Panel shortcut", hotkey.into(), "Takes effect next time a game starts"),
            ("Dashboard design", skin.name().into(), ""),
        ];
        let (row_h, gap) = (54.0, 8.0);
        let size = [620.0f32.min(width - 40.0), 96.0 + SETTING_ROWS as f32 * (row_h + gap) + 44.0];
        let min = [origin[0] + (width - size[0]) / 2.0, origin[1] + (height - size[1]) / 2.0 + (1.0 - t) * 30.0];
        let max = [min[0] + size[0], min[1] + size[1]];
        let mouse = ui.io().mouse_pos;
        let clicked = ui.is_mouse_clicked(MouseButton::Left);
        let mut change = None;
        let dl = ui.get_window_draw_list();
        dl.add_rect(origin, [origin[0] + width, origin[1] + height], [0.0, 0.0, 0.0, 0.55 * t]).filled(true).build();
        dl.add_rect([min[0], min[1] + 6.0], [max[0], max[1] + 10.0], [0.0, 0.0, 0.0, 0.35 * t]).filled(true).rounding(16.0).build();
        gradient(&dl, min, max, [0.13, 0.16, 0.20, 0.97 * t], [0.06, 0.08, 0.10, 0.97 * t]);
        dl.add_rect(min, max, with_alpha(GREEN, 0.85 * t)).rounding(14.0).thickness(2.0).build();
        {
            let _f = ui.push_font(fonts.title);
            dl.add_text([min[0] + 28.0, min[1] + 24.0], col(with_alpha(WHITE, t)), "Settings");
        }
        for (i, (label, value, note)) in rows.iter().enumerate() {
            let a = [min[0] + 20.0, min[1] + 76.0 + i as f32 * (row_h + gap)];
            let b = [max[0] - 20.0, a[1] + row_h];
            let hover = mouse[0] >= a[0] && mouse[0] < b[0] && mouse[1] >= a[1] && mouse[1] < b[1];
            if hover && ui.io().mouse_delta != [0.0, 0.0] {
                self.settings_row = Some(i);
            }
            if hover && clicked {
                change = Some(i);
            }
            let on = self.anim.to(("setting", i), (sel == i) as u8 as f32, 16.0, 0.0);
            dl.add_rect(a, b, [1.0, 1.0, 1.0, (0.05 + 0.10 * on) * t]).filled(true).rounding(10.0).build();
            if on > 0.01 {
                dl.add_rect(a, b, with_alpha(GREEN_LIGHT, on * t)).rounding(10.0).thickness(2.0).build();
            }
            let _f = ui.push_font(fonts.body);
            let ty = if note.is_empty() { a[1] + (row_h - ui.text_line_height()) / 2.0 } else { a[1] + 7.0 };
            dl.add_text([a[0] + 16.0, ty], col(with_alpha(WHITE, t)), label);
            if !note.is_empty() {
                dl.add_text([a[0] + 16.0, ty + 22.0], col([1.0, 1.0, 1.0, 0.5 * t]), note);
            }
            let v = format!("<  {value}  >");
            let w = ui.calc_text_size(&v)[0];
            dl.add_text([b[0] - 16.0 - w, a[1] + (row_h - ui.text_line_height()) / 2.0], col(with_alpha(if on > 0.5 { GREEN_LIGHT } else { WHITE }, t)), &v);
        }
        {
            let _f = ui.push_font(fonts.body);
            let hint = "Up/down choose  ·  left/right or A change  ·  B back";
            let w = ui.calc_text_size(hint)[0];
            dl.add_text([min[0] + (size[0] - w) / 2.0, max[1] - 34.0], col([1.0, 1.0, 1.0, 0.55 * t]), hint);
        }
        if let Some(i) = change {
            self.settings_row = Some(i);
            self.change_setting(i, 1, &profile.settings, skin);
        }
    }

    /// Call when the panel opens: everything animates in afresh.
    pub fn opened(&mut self) {
        self.anim.reset();
        self.now.reset();
        self.opened.reset();
        self.games_scroll_pending = true;
    }

    /// A real screen position in the panel's layout coordinates.
    pub fn to_layout(&self, p: [f32; 2]) -> [f32; 2] {
        let (o, f) = self.view;
        if f <= 0.0 {
            return p;
        }
        [o[0] + (p[0] - o[0]) / f, o[1] + (p[1] - o[1]) / f]
    }

    /// 0→1 as the panel opens (for the background dim).
    pub fn open_amount(&self) -> f32 {
        ease_out_cubic(self.anim.peek("open", 0.0))
    }

    /// Up/down from the D-pad, stick or arrow keys.
    pub fn navigate(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        crate::sound::play(crate::sound::Sfx::Move);
        if let Some(row) = self.settings_row.as_mut() {
            *row = (*row as i64 + delta as i64).clamp(0, SETTING_ROWS as i64 - 1) as usize;
            return;
        }
        match (self.blade(), &self.open_game) {
            (Blade::NowPlaying, _) => self.now.navigate(delta),
            (Blade::Games, Some(_)) => self.opened.navigate(delta),
            (Blade::Home, _) if skin() == Skin::Blades => {
                self.home_sel = (self.home_sel as i64 + delta as i64).clamp(0, HOME_ITEMS as i64 - 1) as usize;
            }
            (Blade::Requests, _) => {
                self.req_sel = (self.req_sel as i64 + delta as i64).clamp(0, REASONS.len() as i64) as usize;
            }
            (Blade::Games, None) if !self.games_order.is_empty() => {
                let n = self.games_order.len() as i64;
                self.games_sel = (self.games_sel as i64 + delta as i64).clamp(0, n - 1) as usize;
                self.games_scroll_pending = true;
            }
            _ => {}
        }
    }

    fn blade(&self) -> Blade {
        self.blade.unwrap_or(Blade::NowPlaying)
    }

    fn blade_index(&self) -> usize {
        BLADES.iter().position(|(b, ..)| *b == self.blade()).unwrap_or(0)
    }

    fn switch(&mut self, step: i32) {
        let n = BLADES.len() as i32;
        self.blade = Some(BLADES[((self.blade_index() as i32 + step).rem_euclid(n)) as usize].0);
    }

    fn open(&mut self, game_id: String) {
        crate::sound::play(crate::sound::Sfx::Select);
        self.open_game = Some(game_id);
        self.opened = AchList::default();
        self.anim.set("open_game_t", 0.0);
    }

    /// Reads key/button presses. Call once per frame.
    pub fn handle_input(&mut self, focused: bool) -> (u64, u32) {
        self.edges.update(focused)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: Option<&Game>, profile: &Profile, keys: u64, pad: u32) {
        self.anim.begin(ui.io().delta_time);
        // Typing a note: letters are text, not shortcuts.
        let keys = if ui.io().want_text_input { keys & (KEY_ESC | KEY_ENTER) } else { keys };
        let mut skin = self.skin_request.unwrap_or_else(|| Skin::from_id(&profile.dashboard));
        if keys & KEY_D != 0 || pad & PAD_Y != 0 {
            skin = skin.next();
            self.skin_request = Some(skin);
        }
        if self.shown_skin.is_some_and(|s| s != skin) {
            crate::sound::play(crate::sound::Sfx::Switch);
            // New design: let everything sweep in again, without re-dimming.
            self.anim.reset();
            self.anim.set("open", 1.0);
            self.anim.set("skin_t", 0.0);
            self.now.reset();
            self.opened.reset();
        }
        self.shown_skin = Some(skin);
        set_skin(skin);
        // Themed Blades: everything in the theme pack's font, if it has one.
        let themed_blades = skin == Skin::Blades && media.get(Kind::Theme, "", "blade-open").is_some();
        let fonts: &Fonts = match &fonts.themed {
            Some(t) if themed_blades => t,
            _ => fonts,
        };
        // The Settings screen takes the controls while it's open.
        let in_settings = self.settings_row.is_some();
        if in_settings {
            self.settings_input(keys, pad, &profile.settings, skin);
        }
        let (keys, pad) = if in_settings { (0, 0) } else { (keys, pad) };
        if keys & KEY_Q != 0 || pad & PAD_LB != 0 {
            self.switch(-1);
        }
        if keys & KEY_E != 0 || pad & PAD_RB != 0 {
            self.switch(1);
        }
        // Stick / D-pad / arrows left-right.
        let h = input::HNAV.swap(0, Ordering::Relaxed);
        if h != 0 {
            self.switch(h.signum());
        }
        let in_game_list = self.blade() == Blade::Games && self.open_game.is_some();
        if keys & KEY_F != 0 || pad & PAD_X != 0 {
            if self.blade() == Blade::NowPlaying || in_game_list {
                crate::sound::play(crate::sound::Sfx::Move);
            }
            match self.blade() {
                Blade::NowPlaying => self.now.cycle_filter(),
                Blade::Games if in_game_list => self.opened.cycle_filter(),
                Blade::Home => self.open_settings(),
                _ => {}
            }
        }
        // Record mode: A on a locked, unmapped achievement of this game.
        if self.blade() == Blade::NowPlaying && (keys & KEY_ENTER != 0 || pad & PAD_A != 0) {
            match game.and_then(|g| g.achievements().get(self.now.selected).map(|a| (g, a))) {
                Some((g, a)) if !crate::recorder::watching(g.id()) => self.record_refused = Some(format!("A on {}: record mode isn't running", a.id)),
                Some((_, a)) if a.trackable() => self.record_refused = Some(format!("A on {}: already mapped", a.id)),
                Some(_) => {
                    crate::sound::play(crate::sound::Sfx::Select);
                    self.record_request = Some(self.now.selected);
                }
                None => self.record_refused = Some(format!("A with nothing selected ({})", self.now.selected)),
            }
        }
        if self.blade() == Blade::Games && !in_game_list && (keys & KEY_ENTER != 0 || pad & PAD_A != 0) {
            if let Some(id) = self.games_order.get(self.games_sel).cloned() {
                self.open(id);
            }
        }
        if in_game_list && (keys & (KEY_ESC | KEY_BACK) != 0 || pad & PAD_B != 0) {
            self.close_game();
        }
        // While a game's list is open, B/Esc mean "back", not "close".
        input::MODAL.store((self.blade() == Blade::Games && self.open_game.is_some()) || self.settings_row.is_some(), Ordering::Relaxed);
        if self.settings_row.is_some() && self.shown_blade.is_some_and(|b| b != self.blade()) {
            self.settings_row = None;
        }

        // Blade change: remember the direction for the slide.
        if self.shown_blade != Some(self.blade()) {
            let idx = |b: Blade| BLADES.iter().position(|(x, ..)| *x == b).unwrap_or(0) as f32;
            self.blade_dir = self.shown_blade.map_or(0.0, |old| (idx(self.blade()) - idx(old)).signum());
            if self.shown_blade.is_some() {
                self.anim.set("blade_t", 0.0);
                crate::sound::play(crate::sound::Sfx::Switch);
            }
            self.shown_blade = Some(self.blade());
        }

        let [w, h] = ui.io().display_size;
        // Every design fills the screen like the dashboard, so switching
        // between them doesn't jump. It's laid out on a canvas 700 units
        // tall and scaled to the real size afterwards (see scale_panel).
        let (real_w, real_h) = (w, h);
        let f = (real_h / 700.0).max(0.5);
        let (width, height) = (real_w / f, real_h / f);
        let open = ease_out_cubic(self.anim.timer("open", 0.3)) * ease_out_cubic(self.anim.timer("skin_t", 0.35));

        let _r = ui.push_style_var(StyleVar::WindowRounding(0.0));
        let _p = ui.push_style_var(StyleVar::WindowPadding([0.0, 0.0]));
        let _s = ui.push_style_var(StyleVar::ItemSpacing([8.0, 6.0]));
        let _bw = ui.push_style_var(StyleVar::WindowBorderSize(0.0));
        let _ss = ui.push_style_var(StyleVar::ScrollbarSize(8.0));
        let _sr = ui.push_style_var(StyleVar::ScrollbarRounding(4.0));
        let _fr = ui.push_style_var(StyleVar::FrameRounding(6.0));
        let _fp = ui.push_style_var(StyleVar::FramePadding([10.0, 7.0]));
        let _bg = ui.push_style_color(StyleColor::WindowBg, [0.0, 0.0, 0.0, 0.0]);
        let _cb = ui.push_style_color(StyleColor::ChildBg, [0.0, 0.0, 0.0, 0.0]);
        let _sb = ui.push_style_color(StyleColor::ScrollbarBg, [0.0, 0.0, 0.0, 0.0]);
        let _sg = ui.push_style_color(StyleColor::ScrollbarGrab, with_alpha(pal().text, 0.3));
        let _sh = ui.push_style_color(StyleColor::ScrollbarGrabHovered, with_alpha(pal().text, 0.55));
        let _sa = ui.push_style_color(StyleColor::ScrollbarGrabActive, pal().text);
        let _f1 = ui.push_style_color(StyleColor::FrameBg, pal().card);
        let _f2 = ui.push_style_color(StyleColor::FrameBgHovered, with_alpha(pal().card, 1.3));
        let _f3 = ui.push_style_color(StyleColor::FrameBgActive, with_alpha(pal().card, 1.5));
        let _t1 = ui.push_style_color(StyleColor::Text, pal().text);
        let _t2 = ui.push_style_color(StyleColor::TextDisabled, pal().faint);
        let _t3 = ui.push_style_color(StyleColor::TextSelectedBg, with_alpha(pal().accent, 0.35));

        // Rise into place while opening.
        let corner = [(w - real_w) / 2.0, (h - real_h) / 2.0 + (1.0 - open) * 36.0];
        self.view = (corner, f);
        ui.window("##ah_panel")
            .position(corner, Condition::Always)
            .size([width, height], Condition::Always)
            .flags(WindowFlags::NO_DECORATION | WindowFlags::NO_SAVED_SETTINGS | WindowFlags::NO_MOVE | WindowFlags::NO_SCROLLBAR | WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS)
            .build(|| {
                panel_window(ui);
                fade(open, || {
                    let origin = ui.cursor_screen_pos();
                    match skin {
                        Skin::Blades => {
                            // An installed theme pack draws with its own art.
                            let themed = media.get(Kind::Theme, "", "blade-open").is_some();
                            let t = |name: &str| media.get(Kind::Theme, "", name).map(|(t, _)| t);
                            set_theme_textures(themed.then(|| ThemeTextures {
                                focus: t("focus"),
                                card: media.get(Kind::Theme, "", "card"),
                                glyphs: [t("glyph-a"), t("glyph-b"), t("glyph-x"), t("glyph-y")],
                                scale: height / 576.0 * 0.42,
                            }));
                            let area = if themed {
                                self.blades_themed(ui, fonts, media, origin, width, height)
                            } else {
                                self.blades(ui, fonts, media, origin, width, height)
                            };
                            let (left, right) = (area.left, area.right);
                            let top = area.header - origin[1] + 16.0;
                            let footer = origin[1] + height - area.footer + 14.0;
                            let layout = Layout { pad_x: 26.0, top, footer, blades: Some(area) };
                            self.inside(ui, fonts, media, game, profile, keys, pad, [left, origin[1]], right - left, height, layout);
                        }
                        Skin::Metro => {
                            self.metro_frame(ui, fonts, media, profile, origin, width, height);
                            let layout = Layout { pad_x: 48.0, top: 100.0, footer: 56.0, blades: None };
                            self.inside(ui, fonts, media, game, profile, keys, pad, origin, width, height, layout);
                        }
                        Skin::Classic => {
                            self.classic_frame(ui, fonts, media, profile, origin, width, height);
                            let layout = Layout { pad_x: 24.0, top: 134.0, footer: 56.0, blades: None };
                            self.inside(ui, fonts, media, game, profile, keys, pad, origin, width, height, layout);
                        }
                    }
                    self.draw_settings(ui, fonts, profile, skin, origin, width, height);
                })
            });
        scale_panel(corner, f);
    }

    /// Draws the blades: silver pages folded at the edges, curved as if
    /// seen from inside a drum, each with a raised tab for its label, and
    /// the open blade with its header, glowing body and footer. Returns
    /// where the contents go.
    fn blades(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, origin: [f32; 2], width: f32, height: f32) -> BladeArea {
        let n = BLADES.len();
        let active = self.blade_index();
        // Left edges, relative to the panel: folded blades sit at the edges.
        // On open they fan out from the right.
        let xs: Vec<f32> = (0..=n)
            .map(|i| {
                if i == n {
                    return width;
                }
                let target = if i <= active { i as f32 * TAB } else { width - (n - i) as f32 * TAB };
                let start = width - (n - i) as f32 * TAB;
                self.anim.to(("blade_x", i), target, 18.0, start)
            })
            .collect();
        let top = origin[1];
        let bottom = origin[1] + height;

        // Tabs are clickable (the folded ones switch to that blade).
        let mut hover = [false; BLADES.len()];
        for i in 0..n {
            ui.set_cursor_screen_pos([origin[0] + xs[i], top]);
            if ui.invisible_button(format!("##blade{i}"), [TAB, height]) {
                self.blade = Some(BLADES[i].0);
            }
            hover[i] = ui.is_item_hovered();
        }

        // Geometry. The drum: an edge bows toward its side of the screen,
        // most in the middle, more the further out it is.
        let bulge = height * 0.035;
        let half = width / 2.0;
        let edge = |base: f32, y: f32, y0: f32, y1: f32| -> f32 {
            let t = ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
            let barrel = 1.0 - (2.0 * t - 1.0).powi(2);
            let w = ((base - half).abs() / half).clamp(0.0, 1.0).powf(0.7);
            let x = base + (base - half).signum() * bulge * w * barrel;
            origin[0] + x.clamp(0.0, width)
        };
        // Rounded page ends: how far an edge pulls in near the top/bottom.
        const R: f32 = 16.0;
        let corner = |y: f32, y0: f32, y1: f32| -> f32 {
            let d = (y - y0).min(y1 - y);
            if d >= R {
                0.0
            } else {
                let k = R - d.max(0.0);
                R - (R * R - k * k).max(0.0).sqrt()
            }
        };
        // The raised label tab: 0..1 along the page edge.
        const PROTRUDE: f32 = 13.0;
        // The open blade's own strip barely sticks out (as on the 360), so
        // the tab beside it keeps its label clear.
        const PROTRUDE_OPEN: f32 = 5.0;
        let label_w: Vec<f32> = {
            let _f = ui.push_font(fonts.tab);
            BLADES.iter().map(|(_, l, _)| ui.calc_text_size(l)[0]).collect()
        };
        let tab_zone = |i: usize, y0: f32, depth: f32| -> (f32, f32) {
            let za = y0 + 34.0 + 10.0 * depth;
            (za, za + label_w[i] + 46.0)
        };
        let tab = |y: f32, (za, zb): (f32, f32)| smoothstep(za - 16.0, za + 2.0, y) * (1.0 - smoothstep(zb - 2.0, zb + 16.0, y));
        let step = 4.0;

        // Colours (sampled from the real dashboard).
        // Vivid, glossy greens (the dashboard on a CRT, not a compressed
        // video of it).
        const FRAME_TOP: [f32; 4] = [0.36, 0.78, 0.12, 1.0];
        const FRAME_BOTTOM: [f32; 4] = [0.27, 0.66, 0.07, 1.0];
        const BODY_TOP: [f32; 4] = [0.30, 0.71, 0.09, 1.0];
        const BODY_BOTTOM: [f32; 4] = [0.38, 0.80, 0.15, 1.0];
        const GLOW: [f32; 4] = [0.62, 0.97, 0.34, 1.0];
        const SWIRL: [f32; 4] = [0.08, 0.36, 0.02, 1.0];
        const STRIP_TOP: [f32; 4] = [0.66, 0.95, 0.44, 1.0];
        const STRIP_BOTTOM: [f32; 4] = [0.42, 0.82, 0.20, 1.0];
        const SILVER_LEAD: [f32; 4] = [0.985, 0.985, 0.99, 1.0];
        const SILVER_TRAIL: [f32; 4] = [0.72, 0.725, 0.73, 1.0];
        const LABEL: [f32; 4] = [0.50, 0.51, 0.53, 1.0];
        const STRIP_LABEL: [f32; 4] = [0.12, 0.42, 0.04, 1.0];

        // Far pages first: the left stack from the inside out (outer tabs
        // overlap inner ones), then the open blade, then the right stack
        // from the inside out.
        let mut order: Vec<usize> = (0..active).rev().collect();
        order.push(active);
        order.extend(active + 1..n);
        let mut area = BladeArea { left: 0.0, right: 0.0, header: 0.0, footer: 0.0 };
        let time = ui.time() as f32;
        for i in order {
            let on = self.anim.to(("blade_on", i), (i == active) as u8 as f32, 16.0, (i == active) as u8 as f32);
            let hv = self.anim.to(("blade_hover", i), hover[i] as u8 as f32, 14.0, 0.0);
            let depth = (i as f32 - active as f32).abs();
            let depth = self.anim.to(("blade_depth", i), depth, 18.0, depth);
            let inset = (depth * 4.0).min(16.0);
            let (y0, y1) = (top + inset, bottom - inset);
            let zone = tab_zone(i, y0, depth);
            let left_side = i < active;
            let (bl, br) = (xs[i], xs[i + 1].max(xs[i] + TAB));

            // Silver shading runs from the trailing side (darker) to the
            // leading side (bright); the open blade is its frame colour.
            let (lead, trail) = (lerp_color(SILVER_LEAD, WHITE, 0.5 * hv), lerp_color(SILVER_TRAIL, SILVER_LEAD, 0.15 * hv));
            let mut slices = Vec::new();
            let mut y = y0;
            while y <= y1 + 0.01 {
                let yy = y.min(y1);
                let c = corner(yy, y0, y1);
                let p = if i == active { PROTRUDE_OPEN } else { PROTRUDE } * tab(yy, zone);
                let mut xl = edge(bl, yy, top, bottom) + c;
                let mut xr = edge(br, yy, top, bottom) - c;
                if left_side {
                    xr += p;
                } else {
                    xl -= p;
                }
                let v = 1.0 - 0.12 * ((yy - top) / height);
                let shade = |c: [f32; 4]| [c[0] * v, c[1] * v, c[2] * v, c[3]];
                let frame = lerp_color(FRAME_TOP, FRAME_BOTTOM, (yy - top) / height);
                let (cl, cr) = if left_side { (shade(trail), shade(lead)) } else { (shade(lead), shade(trail)) };
                slices.push(Slice { y: yy, xl, xr, cl: lerp_color(cl, frame, on), cr: lerp_color(cr, frame, on) });
                y += step;
            }

            // Shadow on the page underneath, along the leading edge(s).
            {
                let dl = ui.get_window_draw_list();
                let lead_edge: Vec<[f32; 2]> =
                    slices.iter().map(|s| if left_side { [s.xr + 3.0, s.y] } else { [s.xl - 3.0, s.y] }).collect();
                dl.add_polyline(lead_edge.clone(), [0.0, 0.0, 0.0, 0.10]).thickness(8.0).build();
                dl.add_polyline(lead_edge, [0.0, 0.0, 0.0, 0.22]).thickness(3.0).build();
            }
            fill_slices(ui, &slices);
            {
                // Bright rim on the leading edge.
                let dl = ui.get_window_draw_list();
                let rim: Vec<[f32; 2]> = slices.iter().map(|s| if left_side { [s.xr - 1.0, s.y] } else { [s.xl + 1.0, s.y] }).collect();
                dl.add_polyline(rim, [1.0, 1.0, 1.0, 0.75 * (1.0 - on) + 0.25 * on]).thickness(1.5).build();
            }

            if on > 0.01 {
                // The open blade: a label strip, the header and footer bands
                // (the frame) and the glowing body between them.
                let header = top + height * 0.118;
                let footer = top + height * 0.826;
                let strip = |y: f32| edge(bl, y, top, bottom) + corner(y, y0, y1) + TAB;
                let right = |y: f32| edge(br, y, top, bottom) - corner(y, y0, y1) - 3.0;
                let mut strip_slices = Vec::new();
                let mut body = Vec::new();
                let mut y = y0;
                while y <= y1 + 0.01 {
                    let yy = y.min(y1);
                    let s = slices[(((yy - y0) / step) as usize).min(slices.len() - 1)];
                    let c = with_alpha(lerp_color(STRIP_TOP, STRIP_BOTTOM, (yy - top) / height), on);
                    strip_slices.push(Slice { y: yy, xl: s.xl, xr: strip(yy), cl: c, cr: c });
                    y += step;
                }
                const BR: f32 = 26.0;
                let mut y = header;
                while y <= footer + 0.01 {
                    let yy = y.min(footer);
                    let d = (yy - header).min(footer - yy);
                    let c = if d >= BR { 0.0 } else { BR - (BR * BR - (BR - d).powi(2)).max(0.0).sqrt() };
                    let col_y = with_alpha(lerp_color(BODY_TOP, BODY_BOTTOM, (yy - header) / (footer - header)), on);
                    body.push(Slice { y: yy, xl: strip(yy) + c, xr: right(yy) - c, cl: col_y, cr: col_y });
                    y += step;
                }
                fill_slices(ui, &strip_slices);
                fill_slices(ui, &body);
                let bx0 = strip(header);
                let bx1 = right(header);
                let bw = bx1 - bx0;
                let bh = footer - header;
                {
                    // The swirl: bright patches drifting at the sides.
                    with_clip([bx0, header], [bx1, footer], || {
                        // The dashboard's turning spiral: light ribbons, and
                        // the same spiral half a turn on in shadow.
                        if let Some((tex, _)) = media.get(Kind::Swirl, "", "") {
                            let c = [bx0 + bw * 0.55, header + bh * 0.48];
                            let half = bw.max(bh) * 0.95;
                            let quad = |a: f32| {
                                let (s, k) = a.sin_cos();
                                let p = |x: f32, y: f32| [c[0] + (x * k - y * s) * half, c[1] + (x * s + y * k) * half];
                                (p(-1.0, -1.0), p(1.0, -1.0), p(1.0, 1.0), p(-1.0, 1.0))
                            };
                            let angle = time * std::f32::consts::TAU / 24.0;
                            let dl = ui.get_window_draw_list();
                            let (a, b, cc, d) = quad(angle);
                            dl.add_image_quad(tex, a, b, cc, d).col([1.0, 1.0, 1.0, 0.30 * on]).build();
                            let (a, b, cc, d) = quad(angle + std::f32::consts::PI);
                            dl.add_image_quad(tex, a, b, cc, d).col([0.0, 0.25, 0.0, 0.22 * on]).build();
                        }
                        let drift = (time * 0.25).sin() * 0.03;
                        radial_glow([bx0 + bw * (0.18 + drift), header + bh * 0.45], [bw * 0.34, bh * 0.62], with_alpha(GLOW, 0.45 * on));
                        radial_glow([bx0 + bw * (0.84 - drift), header + bh * 0.30], [bw * 0.30, bh * 0.55], with_alpha(GLOW, 0.40 * on));
                        radial_glow([bx0 + bw * 0.52, header + bh * 0.78], [bw * 0.40, bh * 0.30], with_alpha(GLOW, 0.35 * on));
                        radial_glow([bx0 + bw * (0.48 + drift), header + bh * 0.25], [bw * 0.22, bh * 0.20], with_alpha(SWIRL, 0.45 * on));
                        radial_glow([bx0 + bw * 0.62, header + bh * 0.60], [bw * 0.18, bh * 0.22], with_alpha(SWIRL, 0.30 * on));
                    });
                }
                {
                    // Engraved lines where the body meets the frame, and the
                    // strip's edge.
                    // Glossy bands: a bright sheen on the header and footer,
                    // the body set in with a dark lip above and a light one
                    // below, and the strip's seam.
                    let dl = ui.get_window_draw_list();
                    let (fx0, fx1) = (bx0 + 6.0, bx1 - 6.0);
                    // Whitewash glass over the header and footer bands.
                    dl.add_rect([fx0, y0 + 4.0], [fx1, header - 3.0], [1.0, 1.0, 1.0, 0.16 * on]).filled(true).rounding(BR * 0.7).build();
                    dl.add_rect([fx0, footer + 3.0], [fx1, y1 - 4.0], [1.0, 1.0, 1.0, 0.16 * on]).filled(true).rounding(BR * 0.7).build();
                    gradient(&dl, [fx0 + 8.0, y0 + 5.0], [fx1 - 8.0, header - (header - y0) * 0.45], [1.0, 1.0, 1.0, 0.34 * on], [1.0, 1.0, 1.0, 0.0]);
                    gradient(&dl, [fx0 + 8.0, footer + 4.0], [fx1 - 8.0, footer + (y1 - footer) * 0.35], [1.0, 1.0, 1.0, 0.30 * on], [1.0, 1.0, 1.0, 0.0]);
                    dl.add_line([fx0 + BR, y0 + 4.5], [fx1 - BR, y0 + 4.5], [1.0, 1.0, 1.0, 0.65 * on]).thickness(1.2).build();
                    dl.add_line([fx0 + BR, footer + 3.5], [fx1 - BR, footer + 3.5], [1.0, 1.0, 1.0, 0.65 * on]).thickness(1.2).build();
                    let lip: Vec<[f32; 2]> = body.iter().map(|s| [s.xl, s.y]).collect();
                    let lip_r: Vec<[f32; 2]> = body.iter().map(|s| [s.xr, s.y]).collect();
                    dl.add_polyline(lip, [0.05, 0.25, 0.0, 0.35 * on]).thickness(1.5).build();
                    dl.add_polyline(lip_r, [1.0, 1.0, 1.0, 0.30 * on]).thickness(1.5).build();
                    dl.add_line([bx0 + BR, header + 0.5], [bx1 - BR, header + 0.5], [0.05, 0.25, 0.0, 0.45 * on]).thickness(2.0).build();
                    dl.add_line([bx0 + BR, footer - 0.5], [bx1 - BR, footer - 0.5], [1.0, 1.0, 1.0, 0.55 * on]).thickness(1.5).build();
                    let seam: Vec<[f32; 2]> = strip_slices.iter().map(|s| [s.xr, s.y]).collect();
                    dl.add_polyline(seam, [0.05, 0.30, 0.0, 0.40 * on]).thickness(1.5).build();
                }
                if i == active {
                    area = BladeArea { left: bx0, right: bx1, header, footer };
                }
            }

            // Sideways label, on the tab (or the open blade's strip).
            let _f = ui.push_font(fonts.tab);
            // Centre it in the part of the tab left uncovered: neighbours
            // drawn on top tuck their own label tabs over this one.
            let (za, _) = zone;
            let yl = za + 40.0;
            let (mut vl, mut vr) = (edge(bl, yl, top, bottom), edge(br, yl, top, bottom));
            match i.cmp(&active) {
                std::cmp::Ordering::Less => {
                    vl += if i > 0 { PROTRUDE } else { 0.0 };
                    vr += if i + 1 == active { -PROTRUDE_OPEN } else { PROTRUDE };
                }
                std::cmp::Ordering::Greater => {
                    vl -= PROTRUDE;
                    vr -= if i + 1 < n { PROTRUDE } else { 0.0 };
                }
                std::cmp::Ordering::Equal => {
                    vl -= PROTRUDE_OPEN;
                    vr = vl + PROTRUDE_OPEN + TAB;
                }
            }
            let x_mid = (vl + vr) / 2.0;
            let color = lerp_color(lerp_color(LABEL, [0.15, 0.16, 0.17, 1.0], hv), STRIP_LABEL, on);
            sideways_text(ui, [x_mid + 1.0, za + 18.0 + label_w[i] / 2.0], color, BLADES[i].1);
        }
        area
    }

    /// Blades from an installed theme pack: the pack's own pieces placed as
    /// on the dashboard (a 720x576 layout, scaled to the panel). Earlier
    /// blades stack from the left and later ones against the right, 35
    /// units apart; the open blade follows the left stack and its body runs
    /// to the right stack.
    fn blades_themed(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, origin: [f32; 2], width: f32, height: f32) -> BladeArea {
        let n = BLADES.len();
        let active = self.blade_index();
        let (sx, sy) = (width / 720.0, height / 576.0);
        let x = |c: f32| origin[0] + c * sx;
        let y = |c: f32| origin[1] + c * sy;
        let tex = |name: &str| media.get(Kind::Theme, "", name).map(|(t, _)| t);
        // Pieces land on whole pixels so neighbours meet without gaps or
        // doubled-up glass.
        let snap = |v: f32| v.round();
        // (Sampling half a texel inside the edges: stretched art otherwise
        // blends in the opposite edge, which showed as lines on the tabs.)
        let draw = |name: &str, x0: f32, y0: f32, w: f32, h: f32, alpha: f32| {
            if let Some((t, [tw, th])) = media.get(Kind::Theme, "", name) {
                let (hx, hy) = (0.5 / tw, 0.5 / th);
                ui.get_window_draw_list()
                    .add_image(t, [snap(x(x0)), snap(y(y0))], [snap(x(x0 + w)), snap(y(y0 + h))])
                    .uv_min([hx, hy])
                    .uv_max([1.0 - hx, 1.0 - hy])
                    .col([1.0, 1.0, 1.0, alpha])
                    .build();
            }
        };
        // The same, sampling half a texel in from the edges that join
        // another piece (so the texture's border pixels don't show as lines).
        let draw_joined = |name: &str, x0: f32, y0: f32, w: f32, h: f32, inset_l: bool, inset_r: bool| {
            if let Some((t, [tw, th])) = media.get(Kind::Theme, "", name) {
                let half = 0.5 / tw;
                let hy = 0.5 / th;
                ui.get_window_draw_list()
                    .add_image(t, [snap(x(x0)), snap(y(y0))], [snap(x(x0 + w)), snap(y(y0 + h))])
                    .uv_min([if inset_l { half } else { 0.0 }, hy])
                    .uv_max([if inset_r { 1.0 - half } else { 1.0 }, 1.0 - hy])
                    .build();
            }
        };
        // Each blade glides between its place in the left stack and in the
        // right stack.
        let xs: Vec<f32> = (0..n)
            .map(|i| {
                // (The outermost right blade sits a little further out so the
                // label before it has room.)
                let right = |i: usize| 668.0 - 35.0 * (n - 1 - i) as f32;
                let target = if i <= active { 18.0 + 35.0 * i as f32 } else { right(i) };
                let start = right(i);
                self.anim.to(("tblade_x", i), target, 18.0, start)
            })
            .collect();
        let blade_x = |i: usize| xs[i];
        // The body spans from the open blade's strip to the nearest right
        // blade, its curved right edge tucked 12 units behind that blade and
        // the glass running on under it (as the skin places them).
        let body_l = blade_x(active) + 52.0;
        let near_r = if active + 1 < n { blade_x(active + 1) } else { 680.0 };
        // Nearer blades use the taller tab art.
        let size = |dist: usize| match dist {
            1 => 3,
            2 => 2,
            _ => 1,
        };

        // Clicking a blade's tab opens it.
        for i in 0..n {
            if i == active {
                continue;
            }
            let bx = blade_x(i);
            let (l, r) = if i < active { (bx + 29.0, bx + 64.0) } else { (bx + 12.0, bx + 47.0) };
            ui.set_cursor_screen_pos([x(l), y(0.0)]);
            if ui.invisible_button(format!("##tblade{i}"), [((r - l) * sx).max(1.0), height]) {
                self.blade = Some(BLADES[i].0);
            }
        }

        // The body: the turning background frames, each blade's colour
        // (cross-fading on a switch), the glass bands and the sheen.
        let frames = (0..100).take_while(|k| tex(&format!("swirl-{k:02}")).is_some()).count();
        if frames > 0 {
            let f = ((ui.time() * 15.0) as usize) % frames;
            draw(&format!("swirl-{f:02}"), 18.0, 0.0, 702.0, 576.0, 1.0);
        }
        // A colour per blade, like the dashboard (red is made from orange).
        const COLOURS: [&str; 5] = ["body-orange", "body-green", "body-blue", "body-purple", "body-red"];
        for (i, c) in COLOURS.iter().enumerate() {
            let on = self.anim.to(("tblade_on", i), (i == active) as u8 as f32, 14.0, (i == active) as u8 as f32);
            if on > 0.01 {
                draw(c, 18.0, 0.0, 702.0, 576.0, on);
            }
        }
        // Glass and sheen begin where the open blade's strip ends, with
        // their own rounded edges there (the strip is see-through, and a hard
        // clip showed as a line), and the glass runs on under the right-hand
        // blades.
        // They fade in across the strip's edge rather than starting at a
        // hard edge, which showed as a line either way.
        let start = blade_x(active) + 56.0;
        let ramp = 22.0;
        let glass_r = near_r + 61.0;
        // A piece whose left edge fades from clear to full.
        let faded = |name: &str, x0: f32, y0: f32, w: f32, h: f32| {
            let mark = vtx_mark();
            draw_joined(name, x0, y0, w, h, true, true);
            let (l, r) = (x(x0), x(x0 + w));
            fade_verts_x(mark, l, r);
        };
        with_clip([x(start), y(0.0)], [x(glass_r), y(576.0)], || {
            for (row, y0) in [("top", 0.0), ("bottom", 512.0)] {
                faded(&format!("glass-{row}-middle"), start, y0, ramp, 64.0);
                draw_joined(&format!("glass-{row}-middle"), start + ramp, y0, glass_r - start - ramp - 16.0, 64.0, true, true);
                draw_joined(&format!("glass-{row}-right"), glass_r - 16.0, y0, 16.0, 64.0, true, false);
            }
            faded("sheen-middle", start, 0.0, ramp, 576.0);
            draw_joined("sheen-middle", start + ramp, 0.0, near_r - start - ramp + 16.0, 576.0, true, true);
            draw_joined("sheen-right", near_r + 16.0, 0.0, 64.0, 576.0, true, false);
        });

        // The blades: left runner, left blades outside-in, the open blade,
        // right blades outside-in, right runner.
        draw("runner-left", -61.0, 0.0, 128.0, 576.0, 1.0);
        for i in 0..active {
            draw(&format!("blade-left-{}", size(active - i)), blade_x(i), 0.0, 80.0, 576.0, 1.0);
        }
        draw("blade-open", blade_x(active), 0.0, 80.0, 576.0, 1.0);
        for i in (active + 1..n).rev() {
            draw(&format!("blade-right-{}", size(i - active)), blade_x(i), 0.0, 80.0, 576.0, 1.0);
        }
        // The rim sits a little past the last blade, so the last blade's
        // bright edge shows as a gap before it like between the other tabs,
        // and the blade and rim read as one big final tab.
        draw("runner-right", 678.0, 0.0, 128.0, 576.0, 1.0);

        // Labels, sideways on each tab; nearer tabs' labels sit higher.
        let _f = ui.push_font(fonts.tab);
        for (i, (_, label, _)) in BLADES.iter().enumerate() {
            let dist = i.abs_diff(active);
            let bx = blade_x(i);
            // Centred on the part of the tab left showing (measured from the
            // art), starting just below its notch: 104 on the open blade,
            // 124, 132, 140, 148 moving outwards (as on the dashboard).
            // (Open blade: on its coloured strip. Right blades: on the
            // raised tab at their left edge. Measured from the dashboard.)
            // The skin's own placement: the letters' tops face x = blade +
            // 61 (left and open blades) or + 43 (right ones); the label
            // starts at y 155 on the open blade and 7 lower per step out.
            let anchor = if i <= active { bx + 59.0 } else if i == n - 1 { bx + 45.0 } else { bx + 39.0 };
            if !(-10.0..760.0).contains(&anchor) {
                continue;
            }
            let [tw, th] = ui.calc_text_size(label);
            let cx = (x(anchor) - th / 2.0 - origin[0]) / sx;
            let top = y([155.0, 163.0, 170.0, 177.0][dist.min(3)]);
            let color = if i == active { [0.22, 0.23, 0.25, 1.0] } else { [0.42, 0.43, 0.45, 1.0] };
            let spacing = th * 0.06;
            let tw = tw + spacing * (label.chars().count().saturating_sub(1)) as f32;
            sideways_text_spaced(ui, [x(cx), top + tw / 2.0], color, label, spacing);
        }
        // Contents stay inside the glass's flat middle.
        // (The footer glass's rounded top starts above 512.)
        BladeArea { left: x(body_l + 26.0), right: x(near_r + 4.0), header: y(64.0), footer: y(498.0) }
    }

    /// Blades footer: glyphs stacked like the dashboard (Y over X on the
    /// left, B over A on the right), lit when they do something here.
    fn blades_footer(&self, ui: &Ui, fonts: &Fonts, area: &BladeArea, height_bottom: f32, list_open: bool, in_game_list: bool, keys_hint: &str) {
        let y = area.footer + (height_bottom - area.footer) * 0.16;
        let lx = area.left + 40.0;
        let rx = area.right - 40.0;
        let g = 24.0;
        let in_settings = self.settings_row.is_some();
        let a_label = match self.blade() {
            _ if in_settings => Some("Change"),
            Blade::Home => Some("Select"),
            Blade::Games if !in_game_list => Some("Open"),
            Blade::Requests => Some("Select"),
            _ => None,
        };
        let b_label = if in_game_list || in_settings { "Back" } else { "Close" };
        let home = self.blade() == Blade::Home && !in_settings;
        // Left: Y (design) above, X (filter) below and to the left.
        glyph(ui, fonts, [lx + g * 0.85, y + g / 2.0], "Y", true, Some("Design"), false);
        glyph(ui, fonts, [lx, y + g * 1.75], "X", list_open || home, if home { Some("Settings") } else { list_open.then_some("Filter") }, false);
        // Right: B above, A below and to the right; labels on the left.
        glyph(ui, fonts, [rx - g * 0.85, y + g / 2.0], "B", true, Some(b_label), true);
        glyph(ui, fonts, [rx, y + g * 1.75], "A", a_label.is_some(), a_label, true);
        // Middle: switching, and the keyboard keys.
        let _f = ui.push_font(fonts.body);
        let mid = (area.left + area.right) / 2.0;
        let label = "LB / RB or left / right  switch blades";
        let t = ui.calc_text_size(label);
        ui.set_cursor_screen_pos([mid - t[0] / 2.0, y + 4.0]);
        ui.text_colored(with_alpha(pal().hint_text, 0.85), label);
        let t = ui.calc_text_size(keys_hint);
        ui.set_cursor_screen_pos([mid - t[0] / 2.0, y + 28.0]);
        ui.text_colored(with_alpha(pal().hint_text, 0.6), keys_hint);
    }

    /// Metro: light grey backdrop, lowercase tabs across the top, gamer card
    /// at the top right.
    fn metro_frame(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, profile: &Profile, origin: [f32; 2], width: f32, height: f32) {
        let max = [origin[0] + width, origin[1] + height];
        {
            let dl = ui.get_window_draw_list();
            gradient(&dl, origin, max, METRO_BG_TOP, METRO_BG_BOTTOM);
            // A faint floor glow, like the dashboard's stage.
            let floor = origin[1] + height * 0.70;
            gradient(&dl, [origin[0], floor], max, [1.0, 1.0, 1.0, 0.0], [1.0, 1.0, 1.0, 0.28]);
        }
        let active = self.blade_index();
        let y = origin[1] + 30.0;
        let mut x = origin[0] + 48.0;
        let _f = ui.push_font(fonts.nav);
        for (i, (b, label, _)) in BLADES.iter().enumerate() {
            let size = ui.calc_text_size(label);
            ui.set_cursor_screen_pos([x, y]);
            if ui.invisible_button(format!("##mtab{i}"), size) {
                self.blade = Some(*b);
            }
            let hv = self.anim.to(("mtab_hover", i), ui.is_item_hovered() as u8 as f32, 14.0, 0.0);
            let on = self.anim.to(("mtab_on", i), (i == active) as u8 as f32, 12.0, (i == active) as u8 as f32);
            let c = lerp_color(lerp_color([0.58, 0.58, 0.58, 1.0], [0.38, 0.38, 0.38, 1.0], hv), [0.10, 0.10, 0.10, 1.0], on);
            ui.get_window_draw_list().add_text([x, y - 2.0 * on], col(c), label);
            x += size[0] + 30.0;
        }
        drop(_f);
        self.gamer_card(ui, fonts, media, profile, [origin[0] + width - 48.0, origin[1] + 30.0], 44.0, false);
    }

    /// Classic: dark blue panel, gamer card header, slanted green tabs.
    fn classic_frame(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, profile: &Profile, origin: [f32; 2], width: f32, height: f32) {
        let max = [origin[0] + width, origin[1] + height];
        {
            let dl = ui.get_window_draw_list();
            gradient(&dl, origin, max, CLASSIC_BG_TOP, CLASSIC_BG_BOTTOM);
            dl.add_rect(origin, max, with_alpha(GREEN, 0.4)).build();
            let _f = ui.push_font(fonts.small_bold);
            let label = "ACHIEVEMENT HUB";
            let t = ui.calc_text_size(label);
            dl.add_text([max[0] - 24.0 - t[0], origin[1] + 24.0], col(with_alpha(GREEN, 0.9)), label);
        }
        self.gamer_card(ui, fonts, media, profile, [origin[0] + 24.0, origin[1] + 18.0], 52.0, true);

        let y = origin[1] + 86.0;
        let h = 32.0;
        let skew = 12.0;
        let _f = ui.push_font(fonts.small_bold);
        let labels: Vec<String> = BLADES.iter().map(|(_, l, _)| l.to_uppercase()).collect();
        let mut rects = Vec::new();
        let mut x = origin[0] + 18.0;
        for (i, label) in labels.iter().enumerate() {
            let bw = ui.calc_text_size(label)[0] + 46.0;
            ui.set_cursor_screen_pos([x, y]);
            if ui.invisible_button(format!("##ctab{i}"), [bw, h]) {
                self.blade = Some(BLADES[i].0);
            }
            rects.push(([x, y], [x + bw, y + h], ui.is_item_hovered()));
            x += bw - skew + 6.0;
        }
        let dl = ui.get_window_draw_list();
        dl.add_rect([origin[0], y + h], [max[0], y + h + 3.0], with_alpha(GREEN, 0.8)).filled(true).build();
        for (i, (min, max, hovered)) in rects.iter().enumerate() {
            let hv = self.anim.to(("ctab_hover", i), *hovered as u8 as f32, 14.0, 0.0);
            let lift = hv * 2.0;
            blade_shape(&dl, [min[0], min[1] - lift], [max[0], max[1] - lift], skew, lerp_color(CLASSIC_TAB, CLASSIC_TAB_HOVER, hv), 0.06 + 0.06 * hv);
        }
        let active = self.blade_index();
        let (amin, amax, _) = rects[active];
        let ax = self.anim.to("ctab_x", amin[0], 16.0, amin[0]);
        let aw = self.anim.to("ctab_w", amax[0] - amin[0], 16.0, amax[0] - amin[0]);
        blade_shape(&dl, [ax, y - 2.0], [ax + aw, y + h], skew, GREEN, 0.22);
        for (i, (min, max, _)) in rects.iter().enumerate() {
            let on = self.anim.to(("ctab_on", i), (i == active) as u8 as f32, 16.0, (i == active) as u8 as f32);
            centered_text(ui, &dl, [min[0], min[1] - on], [max[0], max[1] - on], lerp_color([0.780, 0.835, 0.878, 1.0], INK, on), &labels[i]);
        }
    }

    /// The active blade's contents: heading, gamer card, the blade's view
    /// and the button hints.
    #[allow(clippy::too_many_arguments)]
    fn inside(
        &mut self,
        ui: &Ui,
        fonts: &Fonts,
        media: &Media,
        game: Option<&Game>,
        profile: &Profile,
        keys: u64,
        pad: u32,
        origin: [f32; 2],
        width: f32,
        height: f32,
        layout: Layout,
    ) {
        let in_game_list = self.blade() == Blade::Games && self.open_game.is_some();
        let pad_x = layout.pad_x;
        let bt = ease_out_cubic(self.anim.timer("blade_t", 0.22));
        let slide = (1.0 - bt) * 40.0 * self.blade_dir;

        // Heading, big and light, with the game (or a subtitle) under it.
        let heading = match self.blade() {
            Blade::Games if in_game_list => "Achievements",
            b => BLADES.iter().find(|(x, ..)| *x == b).map(|(_, _, h)| *h).unwrap_or(""),
        };
        if let Some(area) = &layout.blades {
            // Title in the header band, light on the frame, like "System".
            let band = area.header - origin[1];
            fade(bt, || {
                let _f = ui.push_font(fonts.display);
                let h = ui.text_line_height();
                let pos = [area.left + 40.0 + slide, origin[1] + (band - h) / 2.0 + 2.0];
                ui.get_window_draw_list().add_text([pos[0] + 1.0, pos[1] + 2.0], col([0.0, 0.08, 0.0, 0.40]), heading);
                ui.get_window_draw_list().add_text(pos, col(pal().hint_text), heading);
            });
            self.gamer_card(ui, fonts, media, profile, [area.right - 30.0, origin[1] + (band - 40.0) / 2.0 + 2.0], 40.0, false);
        }

        // While the blades fan out the open one is still narrow: show its
        // contents only once there's room (imgui also rejects zero-width
        // widgets, which would hang the game).
        if width < 420.0 {
            return;
        }
        let top = layout.top;
        let footer = layout.footer;
        let cmin = [origin[0] + pad_x, origin[1] + top];
        let content = [(width - pad_x * 2.0).max(1.0), height - top - footer];
        ui.set_cursor_screen_pos([cmin[0] + slide, cmin[1]]);
        let in_settings = self.settings_row.is_some();
        ui.child_window("##ah_content").size(content).build(|| {
            if in_settings {
                return; // (Settings draws over this; its rows take the clicks.)
            }
            child(|| {
                fade(bt, || match self.blade() {
                    Blade::Home if skin() == Skin::Blades => self.home_blades(ui, fonts, media, game, profile, keys, pad),
                    Blade::Home => self.home(ui, fonts, media, game, profile),
                    Blade::NowPlaying => match game.and_then(|g| profile.games.get(g.id()).map(|s| (g, s))) {
                        Some((g, summary)) => self.now.draw(ui, fonts, media, summary, profile, &g.full_title(), Some(&g.progress)),
                        None => empty(ui, fonts, "No achievements for this game yet."),
                    },
                    Blade::Games => self.games(ui, fonts, media, game, profile),
                    Blade::Requests => self.requests(ui, fonts, media, game, profile, keys, pad),
                    Blade::Friends => self.friends(ui, fonts, media),
                })
            })
        });

        let list_open = self.blade() == Blade::NowPlaying || in_game_list;
        if let Some(area) = &layout.blades {
            let keys_hint = if in_game_list { "Q/E · F · D · arrows · Esc back" } else { "Q/E · F · D · arrows · Esc close" };
            self.blades_footer(ui, fonts, area, origin[1] + height, list_open, in_game_list, keys_hint);
            return;
        }
        // Footer: what you can do right here. Left: switching and options;
        // right: B and A, like the dashboard.
        let fy = origin[1] + height - footer + 14.0;
        ui.set_cursor_screen_pos([origin[0] + pad_x, fy]);
        button_hint(ui, fonts, "LB", "");
        button_hint(ui, fonts, "RB", "Switch");
        if list_open {
            button_hint(ui, fonts, "X", "Filter");
        } else if self.blade() == Blade::Home && self.settings_row.is_none() {
            button_hint(ui, fonts, "X", "Settings");
        }
        button_hint(ui, fonts, "Y", "Design");
        let keys_hint = match (list_open, self.blade() == Blade::Games && !in_game_list) {
            (true, _) if in_game_list => "Q/E · F · D · arrows · Esc back",
            (true, _) => "Q/E · F · D · arrows · Esc close",
            (_, true) => "Q/E · D · arrows · Enter · Esc close",
            _ => "Q/E · D design · Esc close",
        };
        {
            let _f = ui.push_font(fonts.body);
            let p = ui.cursor_screen_pos();
            ui.set_cursor_screen_pos([p[0], p[1] + 3.0]);
            ui.text_colored(pal().faint, keys_hint);
        }
        let mut right: Vec<(&str, &str)> = Vec::new();
        match self.blade() {
            _ if self.settings_row.is_some() => right.push(("A", "Change")),
            Blade::Games if !in_game_list => right.push(("A", "Open")),
            Blade::Requests => right.push(("A", "Select")),
            _ => {}
        }
        right.insert(0, ("B", if in_game_list || self.settings_row.is_some() { "Back" } else { "Close" }));
        let total: f32 = right.iter().map(|(b, l)| button_hint_width(ui, fonts, b, l)).sum();
        ui.set_cursor_screen_pos([origin[0] + width - pad_x - total + 22.0, fy]);
        for (b, l) in right {
            button_hint(ui, fonts, b, l);
        }
    }

    /// Gamer picture, gamertag and gamerscore at `anchor` (its top-left
    /// corner when `from_left`, else its top-right).
    fn gamer_card(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, profile: &Profile, anchor: [f32; 2], pic: f32, from_left: bool) {
        let score = self.anim.to("score", profile.gamerscore() as f32, 4.0, 0.0);
        let score_text = group_digits(score.round() as u32);
        let tag_w = {
            let _f = ui.push_font(fonts.title);
            ui.calc_text_size(&profile.gamertag)[0]
        };
        let score_w = {
            let _f = ui.push_font(fonts.body);
            ui.calc_text_size(group_digits(profile.gamerscore()))[0] + 26.0
        };
        let text_w = tag_w.max(score_w);
        let right_top = anchor;
        let x = if from_left { anchor[0] } else { anchor[0] - text_w - 12.0 - pic };
        let pop = ease_out_back(self.anim.timer("pic", 0.45));
        let s = pic * (0.6 + 0.4 * pop);
        let c = [x + pic / 2.0, right_top[1] + pic / 2.0];
        let (pmin, pmax) = ([c[0] - s / 2.0, c[1] - s / 2.0], [c[0] + s / 2.0, c[1] + s / 2.0]);
        {
            let dl = ui.get_window_draw_list();
            dl.add_rect([pmin[0] + 1.0, pmin[1] + 2.0], [pmax[0] + 1.0, pmax[1] + 2.0], [0.0, 0.0, 0.0, 0.25]).filled(true).rounding(4.0).build();
            gamer_pic(ui, &dl, fonts, media, &profile.avatar, &profile.gamertag, pmin, pmax);
            if pal().frame[3] > 0.0 {
                dl.add_rect(pmin, pmax, WHITE).rounding(4.0).thickness(2.0).build();
            }
        }
        ui.set_cursor_screen_pos([x + pic + 12.0, right_top[1] + 1.0]);
        {
            let _f = ui.push_font(fonts.title);
            // Blades: the card sits on the frame, so light text.
            let c = if skin() == Skin::Blades { pal().hint_text } else { pal().text };
            shadow_text(ui, c, &profile.gamertag);
        }
        ui.set_cursor_screen_pos([x + pic + 12.0, right_top[1] + pic * 0.52]);
        if skin() == Skin::Blades {
            let p = ui.cursor_screen_pos();
            let _f = ui.push_font(fonts.body);
            let h = ui.text_line_height();
            let r = h * 0.40;
            let c = [p[0] + r, p[1] + h / 2.0];
            {
                let dl = ui.get_window_draw_list();
                dl.add_circle(c, r, pal().hint_text).filled(true).build();
                let _g = ui.push_font(fonts.small_bold);
                centered_text(ui, &dl, [c[0] - r, c[1] - r], [c[0] + r, c[1] + r], [0.27, 0.50, 0.18, 1.0], "G");
            }
            ui.set_cursor_screen_pos([p[0] + r * 2.0 + 6.0, p[1]]);
            ui.text_colored(pal().hint_text, &score_text);
        } else {
            gamerscore(ui, fonts, &score_text, false, true, false);
        }
    }

    fn close_game(&mut self) {
        crate::sound::play(crate::sound::Sfx::Back);
        self.open_game = None;
        self.games_scroll_pending = true;
        self.anim.set("games_in", 0.0);
    }

    #[allow(clippy::too_many_arguments)]
    /// Friends' profiles: Gamerscore, what they're playing, latest unlocks.
    fn friends(&mut self, ui: &Ui, fonts: &Fonts, media: &Media) {
        let (list, status) = crate::social::friends();
        if list.is_empty() {
            empty(ui, fonts, if status.is_empty() { "No friends yet." } else { &status });
            return;
        }
        let now = crate::social::shared::now();
        let width = ui.content_region_avail()[0].clamp(1.0, 820.0);
        let mut list = list;
        // Online first, then by Gamerscore.
        list.sort_by(|a, b| (b.online(now), b.gamerscore).cmp(&(a.online(now), a.gamerscore)));
        for f in &list {
            let p = ui.cursor_screen_pos();
            let h = 92.0;
            {
                let dl = ui.get_window_draw_list();
                card_box(&dl, p, [p[0] + width, p[1] + h]);
                let (pmin, pmax) = ([p[0] + 12.0, p[1] + 12.0], [p[0] + h - 12.0, p[1] + h - 12.0]);
                gamer_pic(ui, &dl, fonts, media, &f.avatar, &f.gamertag, pmin, pmax);
                dl.add_rect(pmin, pmax, [1.0, 1.0, 1.0, 0.6]).thickness(1.5).build();
                let dot = if f.online(now) { [0.35, 0.85, 0.3, 1.0] } else { [0.5, 0.5, 0.5, 0.9] };
                dl.add_circle([pmax[0] - 2.0, pmax[1] - 2.0], 7.5, [0.0, 0.0, 0.0, 0.6]).filled(true).build();
                dl.add_circle([pmax[0] - 2.0, pmax[1] - 2.0], 6.0, dot).filled(true).build();
            }
            ui.set_cursor_screen_pos([p[0] + h + 6.0, p[1] + 10.0]);
            ui.group(|| {
                {
                    let _f = ui.push_font(fonts.title);
                    shadow_text(ui, pal().text, &f.gamertag);
                }
                let _f = ui.push_font(fonts.body);
                let line = match (&f.playing, f.online(now)) {
                    (Some(game), true) => format!("Playing {game}"),
                    (_, true) => "Online".to_string(),
                    _ => "Offline".to_string(),
                };
                ui.text_colored(if f.online(now) { pal().accent } else { pal().faint }, line);
                if let Some(r) = f.recent.first() {
                    ui.text_colored(pal().soft, format!("Latest: {} - {}", r.name, r.title));
                }
            });
            {
                let _f = ui.push_font(fonts.title);
                let score = format!("{} G", f.gamerscore);
                let t = ui.calc_text_size(&score);
                ui.set_cursor_screen_pos([p[0] + width - t[0] - 16.0, p[1] + 12.0]);
                shadow_text(ui, pal().text, &score);
            }
            ui.set_cursor_screen_pos([p[0], p[1] + h + 10.0]);
        }
    }

    fn requests(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: Option<&Game>, profile: &Profile, keys: u64, pad: u32) {
        if let Some(rx) = &self.req_rx {
            if let Some(result) = rx.lock().ok().and_then(|r| r.try_recv().ok()) {
                self.req_rx = None;
                self.req_status = Some(match result {
                    Ok(()) => ("Request sent. Thanks!".into(), true),
                    Err(e) => (format!("Couldn't send: {e}"), false),
                });
                if self.req_status.as_ref().is_some_and(|s| s.1) {
                    self.req_note.clear();
                }
            }
        }
        let Some(g) = game else {
            empty(ui, fonts, "Requests are available while playing a supported game.");
            return;
        };
        let width = ui.content_region_avail()[0].clamp(1.0, 760.0);
        {
            let _f = ui.push_font(fonts.body);
            shadow_text(ui, pal().soft, "Tell us what you'd like for the game you're playing. It goes straight to the Achievement Hub wishlist.");
        }
        ui.dummy([0.0, 6.0]);

        // The game being requested.
        let p = ui.cursor_screen_pos();
        let bar_h = if themed_cards() { 58.0 * theme_scale() } else { 0.0 };
        {
            let dl = ui.get_window_draw_list();
            card_box(&dl, p, [p[0] + width, p[1] + 68.0 + bar_h]);
            game_icon(&dl, media, g.id(), [p[0] + 12.0, p[1] + bar_h + 12.0], 44.0);
        }
        card_title(ui, fonts, p, width, "Requesting For");
        ui.set_cursor_screen_pos([p[0] + 70.0, p[1] + bar_h + 11.0]);
        ui.group(|| {
            if bar_h == 0.0 {
                let _f = ui.push_font(fonts.small_bold);
                ui.text_colored(pal().faint, "REQUESTING FOR");
            }
            let _f = ui.push_font(fonts.title);
            shadow_text(ui, pal().text, &g.full_title());
        });
        ui.set_cursor_screen_pos([p[0], p[1] + 80.0 + bar_h]);

        // Reasons, then Send: D-pad/arrows move, A/Enter picks.
        let activate = keys & KEY_ENTER != 0 || pad & PAD_A != 0;
        for (i, reason) in REASONS.iter().enumerate() {
            let rp = ui.cursor_screen_pos();
            let rmax = [rp[0] + width, rp[1] + 38.0];
            if ui.invisible_button(format!("##reason{i}"), [width, 38.0]) {
                self.req_sel = i;
                self.req_reason = i;
            }
            let hv = self.anim.to(("req_hover", i), ui.is_item_hovered() as u8 as f32, 16.0, 0.0);
            let sel = self.anim.to(("req_sel", i), (self.req_sel == i) as u8 as f32, 16.0, 0.0);
            let on = self.anim.to(("req_on", i), (self.req_reason == i) as u8 as f32, 16.0, (i == 0) as u8 as f32);
            let dl = ui.get_window_draw_list();
            row_bg(&dl, rp, rmax, 1.0 - sel);
            hover_wash(&dl, rp, rmax, hv * (1.0 - sel));
            highlight(&dl, rp, rmax, sel);
            // Radio dot.
            let ring = lerp_color(pal().text, pal().on_sel, sel);
            let c = [rp[0] + 20.0, rp[1] + 19.0];
            dl.add_circle(c, 8.0, ring).thickness(1.6).build();
            if on > 0.01 {
                dl.add_circle(c, 5.0 * on, ring).filled(true).build();
            }
            let _f = ui.push_font(fonts.body);
            let t = ui.calc_text_size(reason);
            dl.add_text([rp[0] + 38.0, rp[1] + (38.0 - t[1]) / 2.0], col(lerp_color(pal().text, pal().on_sel, sel)), reason);
            drop(dl);
            ui.set_cursor_screen_pos([rp[0], rp[1] + 42.0]);
        }
        if activate && self.req_sel < REASONS.len() {
            self.req_reason = self.req_sel;
            crate::sound::play(crate::sound::Sfx::Select);
        }

        ui.dummy([0.0, 2.0]);
        {
            let _f = ui.push_font(fonts.body);
            ui.text_colored(pal().soft, "Note (optional, keyboard)");
            ui.set_next_item_width(width);
            ui.input_text("##req_note", &mut self.req_note).hint("Anything specific you'd like?").build();
        }
        ui.dummy([0.0, 6.0]);

        // Send: a glossy white button that goes green when selected.
        let sending = self.req_rx.is_some();
        let bp = ui.cursor_screen_pos();
        let bmax = [bp[0] + 230.0, bp[1] + 40.0];
        let clicked = ui.invisible_button("##send", [230.0, 40.0]);
        let selected = self.req_sel == REASONS.len();
        let hv = self.anim.to("send_hover", (ui.is_item_hovered() || selected) as u8 as f32, 16.0, 0.0);
        {
            let dl = ui.get_window_draw_list();
            if sending {
                card_box(&dl, bp, bmax);
            } else {
                // The selection colour, turning green when pointed at.
                highlight(&dl, bp, bmax, 1.0);
                let r = pal().radius;
                dl.add_rect_filled_multicolor(
                    [bp[0] + r * 0.3, bp[1]],
                    [bmax[0] - r * 0.3, bmax[1]],
                    col(with_alpha(GREEN_LIGHT, hv)),
                    col(with_alpha(GREEN_LIGHT, hv)),
                    col(with_alpha(GREEN_DARK, hv)),
                    col(with_alpha(GREEN_DARK, hv)),
                );
            }
            let _f = ui.push_font(fonts.title);
            let text = if sending { "Sending..." } else { "Send Request" };
            centered_text(ui, &dl, bp, bmax, if sending { pal().faint } else { lerp_color(pal().on_sel, WHITE, hv) }, text);
        }
        if !sending && (clicked || (activate && selected)) {
            self.req_status = None;
            let build = crate::games::exe_build().map(|(t, s)| format!("{t:08x}/{s:x}")).unwrap_or_default();
            let exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            let note = if self.req_note.trim().is_empty() {
                REASONS[self.req_reason].to_string()
            } else {
                format!("{}: {}", REASONS[self.req_reason], self.req_note.trim())
            };
            crate::sound::play(crate::sound::Sfx::Select);
            self.req_rx = Some(std::sync::Mutex::new(crate::requests::send(crate::requests::Request {
                game: g.full_title(),
                store: "in-game overlay".into(),
                exe: format!("{exe} {build}"),
                gamertag: profile.gamertag.clone(),
                note,
            }, Some(profile.dir.clone()))));
        }
        if let Some((msg, good)) = &self.req_status {
            let _f = ui.push_font(fonts.body);
            ui.set_cursor_screen_pos([bmax[0] + 16.0, bp[1] + 10.0]);
            shadow_text(ui, if *good { pal().text } else { pal().warn }, msg);
        }
    }

    /// Home in the Blades design, laid out like the dashboard's Home blade:
    /// gamer card, a stats box and the latest unlock on the left; a menu
    /// with icons and a big tile for the current game on the right.
    #[allow(clippy::too_many_arguments)]
    fn home_blades(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: Option<&Game>, profile: &Profile, keys: u64, pad: u32) {
        let avail = ui.content_region_avail();
        let o = ui.cursor_screen_pos();
        let gap = 14.0;
        let left_w = (avail[0] * 0.47).floor();
        let right_x = o[0] + left_w + gap;
        let right_w = avail[0] - left_w - gap;
        let bar = if themed_cards() { 58.0 * theme_scale() } else { 0.0 };
        let (got, total) = profile.games.values().map(|g| profile.game_progress(g)).fold((0, 0), |(a, b), (g, t, _)| (a + g, b + t));
        let completion = if total == 0 { 0.0 } else { got as f32 / total as f32 };

        // ── Gamer card ──
        let card_h = bar + 128.0;
        {
            let dl = ui.get_window_draw_list();
            card_box(&dl, o, [o[0] + left_w, o[1] + card_h]);
        }
        let title_off = card_title(ui, fonts, o, left_w, &profile.gamertag);
        let body = [o[0] + 14.0, o[1] + title_off + 12.0];
        {
            // Gamer picture.
            let dl = ui.get_window_draw_list();
            let (pmin, pmax) = (body, [body[0] + 76.0, body[1] + 76.0]);
            gamer_pic(ui, &dl, fonts, media, &profile.avatar, &profile.gamertag, pmin, pmax);
            dl.add_rect(pmin, pmax, WHITE).thickness(2.0).build();
            // Five stars for overall completion (where the 360 showed rep).
            let filled = (completion * 5.0).round() as usize;
            for k in 0..5 {
                let c = [pmin[0] + 8.0 + k as f32 * 15.0, pmax[1] + 14.0];
                star(&dl, c, 6.5, if k < filled { [1.0, 0.85, 0.15, 1.0] } else { [0.0, 0.0, 0.0, 0.25] });
            }
        }
        ui.set_cursor_screen_pos([body[0] + 92.0, body[1] - 2.0]);
        ui.group(|| {
            let score = self.anim.to("home_score", profile.gamerscore() as f32, 4.0, 0.0);
            gamerscore(ui, fonts, &group_digits(score.round() as u32), false, true, false);
            let _f = ui.push_font(fonts.body);
            let n = profile.total_unlocked();
            ui.text_colored(pal().text, format!("{n} {}", if n == 1 { "achievement" } else { "achievements" }));
            ui.text_colored(pal().text, format!("{} {} played", profile.games.len(), if profile.games.len() == 1 { "game" } else { "games" }));
            ui.text_colored(pal().soft, format!("{}% complete", (completion * 100.0).round()));
        });

        // ── Stats box (the dashboard's "Community" box) ──
        let sy = o[1] + card_h + gap;
        let stats_h = bar + 70.0;
        {
            let dl = ui.get_window_draw_list();
            card_box(&dl, [o[0], sy], [o[0] + left_w, sy + stats_h]);
        }
        let t2 = card_title(ui, fonts, [o[0], sy], left_w, "Progress");
        let pill_y = sy + t2 + 14.0;
        let pills = [
            ("Unlocked", profile.total_unlocked().to_string()),
            ("Games", profile.games.len().to_string()),
            ("Complete", format!("{}%", (completion * 100.0).round())),
        ];
        let pw = (left_w - 28.0 - 2.0 * 10.0) / 3.0;
        for (k, (label, value)) in pills.iter().enumerate() {
            let pmin = [o[0] + 14.0 + k as f32 * (pw + 10.0), pill_y];
            let pmax = [pmin[0] + pw, pill_y + 40.0];
            let dl = ui.get_window_draw_list();
            dl.add_rect(pmin, pmax, [0.0, 0.0, 0.0, 0.16]).filled(true).rounding(20.0).build();
            dl.add_rect(pmin, pmax, [1.0, 1.0, 1.0, 0.45]).rounding(20.0).build();
            let _f = ui.push_font(fonts.small_bold);
            dl.add_text([pmin[0] + 14.0, pmin[1] + 12.0], col(pal().soft), label);
            drop(_f);
            let _f = ui.push_font(fonts.title);
            let t = ui.calc_text_size(value);
            dl.add_text([pmax[0] - 14.0 - t[0], pmin[1] + (40.0 - t[1]) / 2.0], col(pal().text), value);
        }

        // ── Latest unlock (the dashboard's news box) ──
        let ly = sy + stats_h + gap;
        let lh = (o[1] + avail[1] - ly).max(bar + 60.0);
        {
            let dl = ui.get_window_draw_list();
            card_box(&dl, [o[0], ly], [o[0] + left_w, ly + lh]);
        }
        let recent = profile.recent(1);
        let latest = recent.first();
        let t3 = card_title(ui, fonts, [o[0], ly], left_w, &match latest {
            Some(r) => format!("Latest: {}", r.achievement.name),
            None => "Latest achievement".to_string(),
        });
        ui.set_cursor_screen_pos([o[0] + 14.0, ly + t3 + 10.0]);
        ui.group(|| {
            let _f = ui.push_font(fonts.body);
            let _w = ui.push_text_wrap_pos_with_pos(o[0] + left_w - 14.0 - ui.window_pos()[0]);
            match latest {
                Some(r) => {
                    ui.text_colored(pal().text, &r.achievement.description);
                    ui.text_colored(pal().soft, format!("{}  ·  {}  ·  {} G", r.game.title, format_ago(r.when), r.achievement.points));
                }
                None => ui.text_colored(pal().soft, "Nothing unlocked yet. Go get 'em."),
            }
        });

        // ── Right: menu with icons, then the game tile ──
        let activate = keys & KEY_ENTER != 0 || pad & PAD_A != 0;
        let design = skin().name();
        let items: [(&str, String); 5] = [
            ("now", match game {
                Some(g) => format!("Now Playing: {}", g.title()),
                None => "Now Playing".to_string(),
            }),
            ("games", "My Games".to_string()),
            ("request", "Request a Game".to_string()),
            ("design", format!("Dashboard Design: {design}")),
            ("settings", "Settings".to_string()),
        ];
        let row_h = 56.0;
        let mut y = o[1];
        let mut chosen = None;
        for (k, (kind, label)) in items.iter().enumerate() {
            let min = [right_x, y];
            let max = [right_x + right_w, y + row_h];
            ui.set_cursor_screen_pos(min);
            if ui.invisible_button(format!("##home{k}"), [right_w, row_h]) {
                self.home_sel = k;
                chosen = Some(k);
            }
            if ui.is_item_hovered() {
                self.home_sel = k;
            }
            let sel = self.anim.to(("home_item", k), (self.home_sel == k) as u8 as f32, 14.0, 0.0);
            {
                let dl = ui.get_window_draw_list();
                dl.add_rect(min, max, [1.0, 1.0, 1.0, 0.10]).filled(true).rounding(10.0).build();
                dl.add_rect(min, max, [1.0, 1.0, 1.0, 0.40]).rounding(10.0).build();
                highlight(&dl, min, max, sel);
                // Icon: the game's for game items, a simple mark otherwise.
                let ic = [min[0] + 8.0, min[1] + 8.0];
                match *kind {
                    "now" | "games" => match game.map(|g| g.id().to_string()).or_else(|| profile.games_by_recent().first().map(|g| g.id.clone())) {
                        Some(id) => game_icon(&dl, media, &id, ic, 48.0),
                        None => emblem(&dl, ic, [ic[0] + 48.0, ic[1] + 48.0], [0.3, 0.38, 0.48, 1.0], true),
                    },
                    "request" => emblem(&dl, ic, [ic[0] + 48.0, ic[1] + 48.0], [0.25, 0.45, 0.80, 1.0], true),
                    "settings" => emblem(&dl, ic, [ic[0] + 48.0, ic[1] + 48.0], [0.45, 0.48, 0.52, 1.0], true),
                    _ => {
                        let (a, b) = (ic, [ic[0] + 48.0, ic[1] + 48.0]);
                        gradient(&dl, a, b, BLADE_TOP, BLADE_BOTTOM);
                        dl.add_rect(a, b, WHITE).thickness(1.5).build();
                    }
                }
                let _f = ui.push_font(fonts.title);
                let t = ui.calc_text_size(label);
                let c = lerp_color(pal().text, pal().on_sel, sel);
                dl.add_text([min[0] + 70.0, min[1] + (row_h - t[1]) / 2.0], col(c), fit(ui, label, right_w - 84.0));
            }
            y += row_h + 8.0;
        }

        // The current game's tile (the dashboard's big promo tile).
        let tmin = [right_x, y + 4.0];
        let tmax = [right_x + right_w, o[1] + avail[1]];
        ui.set_cursor_screen_pos(tmin);
        if ui.invisible_button("##home_tile", [right_w, (tmax[1] - tmin[1]).max(1.0)]) {
            self.home_sel = 5;
            chosen = Some(5);
        }
        if ui.is_item_hovered() {
            self.home_sel = 5;
        }
        let tsel = self.anim.to(("home_item", 5usize), (self.home_sel == 5) as u8 as f32, 14.0, 0.0);
        {
            let dl = ui.get_window_draw_list();
            dl.add_rect(tmin, tmax, [0.0, 0.0, 0.0, 0.22]).filled(true).rounding(10.0).build();
            dl.add_rect(tmin, tmax, lerp_color([1.0, 1.0, 1.0, 0.45], [1.0, 1.0, 1.0, 1.0], tsel)).rounding(10.0).thickness(1.0 + 2.0 * tsel).build();
        }
        if let Some(g) = game {
            let (got, total, pts) = profile.games.get(g.id()).map(|s| profile.game_progress(s)).unwrap_or((0, 0, 0));
            let side = (tmax[1] - tmin[1] - 24.0).clamp(40.0, 170.0);
            {
                let dl = ui.get_window_draw_list();
                game_icon(&dl, media, g.id(), [tmin[0] + 12.0, tmin[1] + 12.0], side);
            }
            ui.set_cursor_screen_pos([tmin[0] + side + 28.0, tmin[1] + 14.0]);
            ui.group(|| {
                let _f = ui.push_font(fonts.large);
                ui.text_colored(WHITE, g.title());
                drop(_f);
                let _f = ui.push_font(fonts.body);
                ui.text_colored([1.0, 1.0, 1.0, 0.85], format!("{got} of {total} achievements"));
                ui.text_colored([1.0, 1.0, 1.0, 0.85], format!("{pts} / {} G", g.total_points()));
                ui.text_colored([1.0, 1.0, 1.0, 0.7], "Keep going: see what's left");
            });
        }

        // Act on A / Enter / click.
        let pick = chosen.or_else(|| activate.then_some(self.home_sel));
        match pick {
            Some(0) | Some(5) => self.blade = Some(Blade::NowPlaying),
            Some(4) => {
                self.open_settings();
                return;
            }
            Some(1) => self.blade = Some(Blade::Games),
            Some(2) => self.blade = Some(Blade::Requests),
            Some(3) => self.skin_request = Some(skin().next()),
            _ => {}
        }
        if pick.is_some() {
            crate::sound::play(crate::sound::Sfx::Select);
        }
    }

    fn home(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: Option<&Game>, profile: &Profile) {
        let avail = ui.content_region_avail();
        let left_w = 290.0;
        let p = ui.cursor_screen_pos();

        // Left: profile card.
        {
            let dl = ui.get_window_draw_list();
            card_box(&dl, p, [p[0] + left_w, p[1] + avail[1]]);
        }
        // Themed: the gamertag goes in the card's gold bar.
        let bar = card_title(ui, fonts, p, left_w, &profile.gamertag);
        ui.set_cursor_screen_pos([p[0] + 20.0, p[1] + 18.0 + bar]);
        let score = self.anim.to("home_score", profile.gamerscore() as f32, 4.0, 0.0);
        let total_unlocked = self.anim.to("home_ach", profile.total_unlocked() as f32, 4.0, 0.0);
        ui.group(|| {
            if bar == 0.0 {
                let _f = ui.push_font(fonts.small_bold);
                ui.text_colored(pal().faint, "PROFILE");
                drop(_f);
                let _f = ui.push_font(fonts.large);
                shadow_text(ui, pal().text, &profile.gamertag);
            }
            gamerscore(ui, fonts, &group_digits(score.round() as u32), true, true, false);
            ui.dummy([0.0, 12.0]);
            let _f = ui.push_font(fonts.body);
            let stat = |label: &str, value: String| {
                ui.text_colored(pal().soft, label);
                ui.same_line_with_pos(160.0);
                ui.text_colored(pal().text, value);
            };
            stat("Achievements", (total_unlocked.round() as u32).to_string());
            stat("Games played", profile.games.len().to_string());
            let (got, total) = profile.games.values().map(|g| profile.game_progress(g)).fold((0, 0), |(a, b), (g, t, _)| (a + g, b + t));
            stat("Completion", format!("{}%", if total == 0 { 0 } else { got * 100 / total }));
        });

        // Bottom of the profile card: pick the dashboard design.
        let current = skin();
        let row_h = 34.0;
        let mut y = p[1] + avail[1] - 16.0 - Skin::ALL.len() as f32 * (row_h + 4.0) - 46.0;
        ui.set_cursor_screen_pos([p[0] + 16.0, y]);
        {
            let _f = ui.push_font(fonts.small_bold);
            ui.text_colored(pal().faint, "DASHBOARD DESIGN");
        }
        {
            let _f = ui.push_font(fonts.body);
            ui.set_cursor_screen_pos([p[0] + 16.0, y + 18.0]);
            ui.text_colored(pal().faint, "Y or D to switch");
        }
        y += 46.0;
        let w = left_w - 32.0;
        for (i, s) in Skin::ALL.into_iter().enumerate() {
            let min = [p[0] + 16.0, y];
            let max = [min[0] + w, y + row_h];
            ui.set_cursor_screen_pos(min);
            if ui.invisible_button(format!("##skin{i}"), [w, row_h]) && s != current {
                self.skin_request = Some(s);
            }
            let hv = self.anim.to(("skin_hover", i), ui.is_item_hovered() as u8 as f32, 16.0, 0.0);
            let sel = self.anim.to(("skin_sel", i), (s == current) as u8 as f32, 14.0, (s == current) as u8 as f32);
            let dl = ui.get_window_draw_list();
            row_bg(&dl, min, max, 1.0 - sel);
            hover_wash(&dl, min, max, hv * (1.0 - sel));
            highlight(&dl, min, max, sel);
            // A tiny swatch of each design.
            let sw = [min[0] + 10.0, min[1] + 8.0];
            let swm = [sw[0] + 28.0, max[1] - 8.0];
            match s {
                Skin::Blades => gradient(&dl, sw, swm, BLADE_TOP, BLADE_BOTTOM),
                Skin::Metro => {
                    gradient(&dl, sw, swm, METRO_BG_TOP, METRO_BG_BOTTOM);
                    dl.add_rect([sw[0] + 4.0, sw[1] + 5.0], [sw[0] + 14.0, swm[1] - 3.0], METRO_GREEN).filled(true).build();
                }
                Skin::Classic => {
                    gradient(&dl, sw, swm, CLASSIC_BG_TOP, CLASSIC_BG_BOTTOM);
                    dl.add_rect([sw[0], sw[1] + 6.0], [swm[0], sw[1] + 8.0], GREEN).filled(true).build();
                }
            }
            dl.add_rect(sw, swm, with_alpha(pal().text, 0.35)).build();
            let _f = ui.push_font(fonts.body);
            let t = ui.calc_text_size(s.name());
            dl.add_text([swm[0] + 12.0, min[1] + (row_h - t[1]) / 2.0], col(lerp_color(pal().text, pal().on_sel, sel)), s.name());
            drop(dl);
            y += row_h + 4.0;
        }

        // Right: now playing + recent.
        let rx = p[0] + left_w + 16.0;
        let right_w = avail[0] - left_w - 16.0;
        ui.set_cursor_screen_pos([rx, p[1]]);
        let now_t = self.anim.timer("home_in", 1.0);
        ui.group(|| {
            if let Some(g) = game {
                let (got, total, pts) = profile.games.get(g.id()).map(|s| profile.game_progress(s)).unwrap_or((0, 0, 0));
                // The whole card opens Now Playing.
                let cp = ui.cursor_screen_pos();
                let bar_h = if themed_cards() { 58.0 * theme_scale() } else { 0.0 };
                let card_h = 104.0 + bar_h;
                let cmax = [cp[0] + right_w, cp[1] + card_h];
                if ui.invisible_button("##now_playing", [right_w, card_h]) {
                    self.blade = Some(Blade::NowPlaying);
                }
                let hv = self.anim.to("now_card_hover", ui.is_item_hovered() as u8 as f32, 16.0, 0.0);
                {
                    let dl = ui.get_window_draw_list();
                    card_box(&dl, cp, cmax);
                    hover_wash(&dl, cp, cmax, hv);
                    if bar_h == 0.0 {
                        dl.add_rect(cp, cmax, with_alpha(pal().accent, 0.6 * hv)).rounding(pal().radius).build();
                    }
                    let s = 76.0 * (1.0 + 0.05 * hv);
                    game_icon(&dl, media, g.id(), [cp[0] + 14.0 - (s - 76.0) / 2.0, cp[1] + bar_h + 14.0 - (s - 76.0) / 2.0], s);
                }
                // Themed: the game's name goes in the gold bar.
                card_title(ui, fonts, cp, right_w, &format!("Now Playing: {}", g.full_title()));
                ui.set_cursor_screen_pos([cp[0] + 106.0, cp[1] + bar_h + 12.0]);
                let frac = if total == 0 { 0.0 } else { got as f32 / total as f32 };
                let shown = self.anim.to("home_bar", frac, 3.0, 0.0);
                ui.group(|| {
                    if bar_h == 0.0 {
                        let _f = ui.push_font(fonts.small_bold);
                        ui.text_colored(pal().faint, "NOW PLAYING");
                        drop(_f);
                        let _f = ui.push_font(fonts.title);
                        shadow_text(ui, pal().text, &g.full_title());
                    }
                    progress(ui, shown, right_w - 126.0, 8.0);
                    let _f = ui.push_font(fonts.body);
                    ui.text_colored(pal().soft, format!("{got} of {total} achievements"));
                    ui.same_line_with_pos(right_w - 250.0);
                    drop(_f);
                    gamerscore(ui, fonts, &format!("{pts} / {}", g.total_points()), false, true, false);
                });
                ui.set_cursor_screen_pos([cp[0], cp[1] + card_h + 14.0]);
            }

            let _f = ui.push_font(fonts.title);
            shadow_text(ui, pal().text, "Recent achievements");
            drop(_f);
            let recent = profile.recent(6);
            if recent.is_empty() {
                let _f = ui.push_font(fonts.body);
                ui.text_colored(pal().soft, "Nothing unlocked yet. Go get 'em.");
            }
            for (i, r) in recent.into_iter().enumerate() {
                let rp = ui.cursor_screen_pos();
                // Staggered entrance, then hover glow.
                let enter = ease_out_cubic(((now_t - 0.08 * i as f32) / 0.3).clamp(0.0, 1.0));
                ui.invisible_button(format!("##recent{i}"), [right_w, 58.0]);
                let hv = self.anim.to(("recent_hover", i), ui.is_item_hovered() as u8 as f32, 16.0, 0.0);
                let x = rp[0] + (1.0 - enter) * 30.0;
                let (min, max) = ([x, rp[1]], [rp[0] + right_w, rp[1] + 58.0]);
                fade(enter, || {
                    {
                        let dl = ui.get_window_draw_list();
                        row_bg(&dl, min, max, 1.0);
                        hover_wash(&dl, min, max, hv);
                    }
                    ui.set_cursor_screen_pos([x + 8.0, rp[1] + 7.0]);
                    let a = r.achievement;
                    icon(ui, media, &r.game.id, &a.id, &a.category, true, false, a.platinum, 44.0, 1.0 + 0.08 * hv);
                    ui.set_cursor_screen_pos([x + 64.0 + 3.0 * hv, rp[1] + 6.0]);
                    {
                        let _f = ui.push_font(fonts.title);
                        ui.text_colored(pal().text, &a.name);
                    }
                    ui.set_cursor_screen_pos([x + 64.0 + 3.0 * hv, rp[1] + 32.0]);
                    {
                        let _f = ui.push_font(fonts.body);
                        ui.text_colored(pal().soft, format!("{}  ·  {}", r.game.title, format_ago(r.when)));
                    }
                    points_pill(ui, fonts, max[0] - 10.0, rp[1] + 29.0, a.points, true, false);
                });
                ui.set_cursor_screen_pos([rp[0], rp[1] + 64.0]);
            }
        });
    }

    /// The game list in the Blades design, like the dashboard's "Select a
    /// Game": art, title and developer with a store badge on each row, and
    /// a picture, description and info boxes for the highlighted game.
    fn games_blades(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: Option<&Game>, profile: &Profile) {
        let list = profile.games_by_recent();
        self.games_order = list.iter().map(|g| g.id.clone()).collect();
        if list.is_empty() {
            empty(ui, fonts, "No games played yet.");
            return;
        }
        self.games_sel = self.games_sel.min(list.len() - 1);
        let avail = ui.content_region_avail();
        let o = ui.cursor_screen_pos();
        let list_w = (avail[0] * 0.52).floor();
        let row_h = 70.0;
        // What we know about a game: the running game's definition wins
        // (its summary may predate the newer fields).
        let info = |g: &crate::profile::GameSummary| -> (String, String, Option<u16>, String) {
            // The summary carries the store's details when fetched; the
            // running game's definition fills any gaps.
            let current = game.filter(|cur| cur.id() == g.id);
            let dev = g.developer.clone().or_else(|| current.and_then(|c| c.developer.clone())).unwrap_or_default();
            let publisher = g.publisher.clone().or_else(|| current.and_then(|c| c.publisher.clone())).unwrap_or_default();
            let year = g.year.or_else(|| current.and_then(|c| c.year));
            let store = current.map(|c| c.store.to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| if g.store.is_empty() { "PC".into() } else { g.store.clone() });
            let maker = match (dev.is_empty(), publisher.is_empty(), dev == publisher) {
                (false, _, true) | (false, true, _) => dev,
                (false, false, false) => format!("{dev} / {publisher}"),
                (true, false, _) => publisher,
                _ => String::new(),
            };
            (maker, store, year, g.description.clone().or_else(|| current.and_then(|c| c.description.clone())).unwrap_or_default())
        };

        // ── The list ──
        ui.child_window("##games_b").size([list_w, avail[1]]).build(|| {
            child(|| {
                let width = (ui.content_region_avail()[0] - 4.0).max(1.0);
                for (i, g) in list.iter().enumerate() {
                    let p = ui.cursor_screen_pos();
                    if ui.invisible_button(format!("##gb{i}"), [width, row_h]) {
                        if self.games_sel == i {
                            self.open(g.id.clone());
                        }
                        self.games_sel = i;
                    }
                    let sel = self.anim.to(("gb_sel", i), (self.games_sel == i) as u8 as f32, 14.0, 0.0);
                    let hv = self.anim.to(("gb_hover", i), ui.is_item_hovered() as u8 as f32, 16.0, 0.0);
                    let (min, max) = (p, [p[0] + width, p[1] + row_h]);
                    let (maker, store, _, _) = info(g);
                    {
                        let dl = ui.get_window_draw_list();
                        row_bg(&dl, min, max, 1.0 - sel);
                        hover_wash(&dl, min, max, hv * (1.0 - sel));
                        highlight(&dl, min, max, sel);
                        // The game's own icon (crisp at this size).
                        let side = row_h - 12.0;
                        game_icon(&dl, media, &g.id, [min[0] + 6.0, min[1] + 6.0], side);
                    }
                    let badge_w = store_badge(ui, fonts, max[0] - 24.0, min[1] + row_h / 2.0, &store);
                    let tx = min[0] + row_h + 8.0;
                    // (The badge starts 24 in from the right; keep a gap before it.)
                    let text_w = width - row_h - 8.0 - badge_w - 24.0 - 16.0;
                    ui.set_cursor_screen_pos([tx, min[1] + 8.0]);
                    {
                        let _f = ui.push_font(fonts.title);
                        ui.text_colored(lerp_color(pal().text, pal().on_sel, sel), fit(ui, &g.full_title(), text_w));
                    }
                    ui.set_cursor_screen_pos([tx, min[1] + 38.0]);
                    {
                        let _f = ui.push_font(fonts.body);
                        let line = if maker.is_empty() { format!("Last played {}", format_ago(g.last_played)) } else { maker };
                        ui.text_colored(lerp_color(pal().soft, pal().on_sel_soft, sel), fit(ui, &line, text_w));
                    }
                    ui.set_cursor_screen_pos([p[0], p[1] + row_h + 2.0]);
                }
            })
        });

        // ── The highlighted game ──
        let Some(g) = list.get(self.games_sel) else { return };
        let (maker, store, year, description) = info(g);
        let unlocked: Vec<String> = g.achievement.iter().filter(|a| profile.unlocked_at(&g.id, &a.id).is_some()).map(|a| a.id.clone()).collect();
        media.load_game(&g.id, profile.screenshots_dir(&g.id), profile.cached_icons(&g.id), unlocked);
        let latest_shot = g
            .achievement
            .iter()
            .filter_map(|a| profile.unlocked_at(&g.id, &a.id).map(|t| (t, a.id.as_str())))
            .max()
            .and_then(|(_, id)| media.get(Kind::Shot, &g.id, id));
        let px = o[0] + list_w + 18.0;
        let pw = avail[0] - list_w - 18.0;
        // Picture: the latest unlock's screenshot, else the game's icon.
        let pic_h = (pw * 9.0 / 16.0).min(avail[1] * 0.48);
        {
            let dl = ui.get_window_draw_list();
            let (a, b) = ([px, o[1]], [px + pw, o[1] + pic_h]);
            dl.add_rect([a[0] - 2.0, a[1] - 2.0], [b[0] + 2.0, b[1] + 2.0], [1.0, 1.0, 1.0, 0.9]).filled(true).build();
            // Store screenshots, cycling with a crossfade like a store page.
            let mut shots: Vec<_> = (0..crate::storeinfo::MAX_SHOTS).map_while(|n| media.get(Kind::Art, &g.id, &n.to_string())).collect();
            // No store screenshots: the player's own, captured at unlocks
            // (newest first).
            if shots.is_empty() {
                let mut mine: Vec<(u64, &str)> = g.achievement.iter().filter_map(|a| profile.unlocked_at(&g.id, &a.id).map(|t| (t, a.id.as_str()))).collect();
                mine.sort_by(|x, y| y.cmp(x));
                shots = mine.iter().filter_map(|(_, id)| media.get(Kind::Shot, &g.id, id)).take(crate::storeinfo::MAX_SHOTS).collect();
            }
            let cover = |tex, [w, h]: [f32; 2], alpha: f32| {
                // Cover the frame, cropping the overflow.
                let (fw, fh) = (b[0] - a[0], b[1] - a[1]);
                let s = (fw / w).max(fh / h);
                let (cw, ch) = (fw / (w * s), fh / (h * s));
                dl.add_image(tex, a, b)
                    .uv_min([(1.0 - cw) / 2.0, (1.0 - ch) / 2.0])
                    .uv_max([(1.0 + cw) / 2.0, (1.0 + ch) / 2.0])
                    .col([1.0, 1.0, 1.0, alpha])
                    .build();
            };
            match shots.first().copied().or(latest_shot) {
                Some(first) if shots.len() > 1 => {
                    const HOLD: f64 = 6.0;
                    const FADE: f64 = 0.8;
                    let t = ui.time();
                    let i = (t / HOLD) as usize % shots.len();
                    let next = (i + 1) % shots.len();
                    let phase = t % HOLD;
                    let _ = first;
                    cover(shots[i].0, shots[i].1, 1.0);
                    if phase > HOLD - FADE {
                        let f = ((phase - (HOLD - FADE)) / FADE) as f32;
                        cover(shots[next].0, shots[next].1, f * f * (3.0 - 2.0 * f));
                    }
                    // Which one is showing.
                    let n = shots.len() as f32;
                    let (r, gap) = (3.0, 10.0);
                    let x0 = b[0] - 12.0 - (n - 1.0) * gap;
                    for k in 0..shots.len() {
                        let on = k == i;
                        dl.add_circle([x0 + k as f32 * gap, b[1] - 10.0], r, [1.0, 1.0, 1.0, if on { 0.95 } else { 0.4 }]).filled(true).build();
                    }
                }
                Some((tex, size)) => cover(tex, size, 1.0),
                None => {
                    gradient(&dl, a, b, [0.12, 0.12, 0.14, 1.0], [0.03, 0.03, 0.04, 1.0]);
                    let side = pic_h * 0.8;
                    game_icon(&dl, media, &g.id, [a[0] + (pw - side) / 2.0, a[1] + (pic_h - side) / 2.0], side);
                }
            }
        }
        ui.set_cursor_screen_pos([px, o[1] + pic_h + 12.0]);
        ui.group(|| {
            let _w = ui.push_text_wrap_pos_with_pos(px + pw - ui.window_pos()[0]);
            let _f = ui.push_font(fonts.body);
            if description.is_empty() {
                ui.text_colored(pal().soft, format!("{} - achievements by Achievement Hub.", g.full_title()));
            } else {
                ui.text_colored(pal().text, &description);
                if let Some(src) = g.description_source.as_deref().filter(|s| !s.is_empty()) {
                    ui.text_colored(pal().faint, format!("From {src}"));
                }
            }
        });

        // Bottom: who made it and when (like the rating box), and the
        // achievements (like "Available Content").
        let (got, total, pts) = profile.game_progress(g);
        let box_h = 64.0;
        let by = o[1] + avail[1] - box_h;
        let small_w = (pw * 0.38).floor();
        let boxes = [
            ([px, by], [px + small_w, by + box_h], store.clone(), match year {
                // (Developer only here; the row shows the publisher too.)
                Some(y) if !maker.is_empty() => format!("{}, {y}", maker.split(" / ").next().unwrap_or(&maker)),
                Some(y) => y.to_string(),
                None => maker.clone(),
            }),
            ([px + small_w + 12.0, by], [px + pw, by + box_h], "Achievements".to_string(), format!("{got} of {total}  ·  {pts} / {} G", g.total_points)),
        ];
        for (a, b, title, line) in boxes {
            {
                let dl = ui.get_window_draw_list();
                dl.add_rect(a, b, [0.0, 0.0, 0.0, 0.14]).filled(true).rounding(8.0).build();
                dl.add_rect(a, b, [1.0, 1.0, 1.0, 0.45]).rounding(8.0).build();
            }
            ui.set_cursor_screen_pos([a[0] + 12.0, a[1] + 8.0]);
            ui.group(|| {
                let _f = ui.push_font(fonts.body);
                ui.text_colored(pal().soft, &title);
                ui.text_colored(pal().text, fit(ui, &line, b[0] - a[0] - 24.0));
            });
        }
    }

    fn games(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: Option<&Game>, profile: &Profile) {
        // A game is open: its achievement list, with a way back.
        if let Some(id) = self.open_game.clone() {
            let Some(summary) = profile.games.get(&id) else {
                self.open_game = None;
                return;
            };
            let unlocked = summary.achievement.iter().filter(|a| profile.unlocked_at(&id, &a.id).is_some()).map(|a| a.id.clone()).collect();
            media.load_game(&id, profile.screenshots_dir(&id), profile.cached_icons(&id), unlocked);

            let t = ease_out_cubic(self.anim.timer("open_game_t", 0.3));
            let start = ui.cursor_screen_pos();
            let bmax = [start[0] + 110.0, start[1] + 28.0];
            let back = ui.invisible_button("##back", [110.0, 28.0]);
            let hv = self.anim.to("back_hover", ui.is_item_hovered() as u8 as f32, 16.0, 0.0);
            {
                let dl = ui.get_window_draw_list();
                dl.add_rect(start, bmax, lerp_color(pal().card, pal().hover, hv)).filled(true).rounding(pill_round(28.0)).build();
                dl.add_rect(start, bmax, with_alpha(pal().text, 0.4)).rounding(pill_round(28.0)).build();
                let _f = ui.push_font(fonts.small_bold);
                centered_text(ui, &dl, start, bmax, pal().text, "<  games");
            }
            if back {
                self.close_game();
                return;
            }
            ui.set_cursor_screen_pos([start[0] + (1.0 - t) * 40.0, start[1] + 38.0]);
            let title = summary.full_title();
            fade(t, || ui.group(|| self.opened.draw(ui, fonts, media, summary, profile, &title, game.filter(|g| g.id() == summary.id).map(|g| &g.progress))));
            return;
        }

        if skin() == Skin::Blades {
            self.games_blades(ui, fonts, media, game, profile);
            return;
        }
        let list = profile.games_by_recent();
        self.games_order = list.iter().map(|g| g.id.clone()).collect();
        if list.is_empty() {
            empty(ui, fonts, "No games played yet.");
            return;
        }
        self.games_sel = self.games_sel.min(list.len() - 1);
        let now_t = self.anim.timer("games_in", 1.0);
        let avail = ui.content_region_avail();
        let list_w = avail[0] * 0.56;

        // Left: the list, like the dashboard's game picker.
        ui.child_window("##games").size([list_w, avail[1]]).build(|| {
            child(|| {
                if let Some(target) = self.games_scroll_target {
                    let cur = ui.scroll_y();
                    let next = lerp(cur, target, 1.0 - (-14.0 * ui.io().delta_time.clamp(0.0, 0.1)).exp());
                    if (next - target).abs() < 0.5 {
                        ui.set_scroll_y(target);
                        self.games_scroll_target = None;
                    } else {
                        ui.set_scroll_y(next);
                    }
                }
                let width = (ui.content_region_avail()[0] - 4.0).max(1.0);
                let row_h = 66.0;
                for (i, g) in list.iter().enumerate() {
                    let (got, total, _) = profile.game_progress(g);
                    let p = ui.cursor_screen_pos();
                    if ui.invisible_button(format!("##game{i}"), [width, row_h]) {
                        if self.games_sel == i {
                            self.open(g.id.clone());
                        }
                        self.games_sel = i;
                    }
                    if self.games_scroll_pending && self.games_sel == i {
                        let want = ui.scroll_y() + (p[1] + row_h / 2.0) - (ui.window_pos()[1] + ui.window_size()[1] / 2.0);
                        self.games_scroll_target = Some(want.clamp(0.0, ui.scroll_max_y().max(0.0)));
                        self.games_scroll_pending = false;
                    }
                    let hv = self.anim.to(("game_hover", i), ui.is_item_hovered() as u8 as f32, 16.0, 0.0);
                    let sel = self.anim.to(("game_sel", i), (self.games_sel == i) as u8 as f32, 14.0, 0.0);
                    let enter = ease_out_cubic(((now_t - 0.06 * i as f32) / 0.3).clamp(0.0, 1.0));
                    let min = [p[0] + (1.0 - enter) * 30.0, p[1]];
                    let max = [p[0] + width, p[1] + row_h];
                    fade(enter, || {
                        {
                            let dl = ui.get_window_draw_list();
                            row_bg(&dl, min, max, 1.0 - sel);
                            hover_wash(&dl, min, max, hv * (1.0 - sel));
                            highlight(&dl, min, max, sel);
                            game_icon(&dl, media, &g.id, [min[0] + 9.0, min[1] + 9.0], 48.0);
                        }
                        let text_w = width - 70.0 - 20.0;
                        ui.set_cursor_screen_pos([min[0] + 70.0 + 3.0 * hv, min[1] + 8.0]);
                        {
                            let _f = ui.push_font(fonts.title);
                            ui.text_colored(lerp_color(pal().text, pal().on_sel, sel), fit(ui, &g.full_title(), text_w));
                        }
                        ui.set_cursor_screen_pos([min[0] + 70.0 + 3.0 * hv, min[1] + 35.0]);
                        let _f = ui.push_font(fonts.body);
                        let playing = game.is_some_and(|cur| cur.id() == g.id);
                        let line = if playing {
                            format!("Playing now  ·  {got} of {total}")
                        } else {
                            format!("{}  ·  {got} of {total}", format_ago(g.last_played))
                        };
                        ui.text_colored(lerp_color(pal().soft, pal().on_sel_soft, sel), line);
                    });
                    ui.set_cursor_screen_pos([p[0], p[1] + row_h + 4.0]);
                }
            })
        });

        // Right: the selected game's info pane.
        ui.same_line_with_spacing(0.0, 16.0);
        let Some(g) = list.get(self.games_sel) else { return };
        let (got, total, pts) = profile.game_progress(g);
        if self.anim.peek("games_info_for", -1.0) != self.games_sel as f32 {
            self.anim.set("games_info_for", self.games_sel as f32);
            self.anim.set("games_info_t", 0.0);
        }
        let it = ease_out_cubic(self.anim.timer("games_info_t", 0.25));
        let p = ui.cursor_screen_pos();
        ui.child_window("##game_info").size([0.0, avail[1]]).build(|| {
            child(|| {
                let full = ui.content_region_avail();
                {
                    let dl = ui.get_window_draw_list();
                    card_box(&dl, p, [p[0] + full[0], p[1] + full[1]]);
                }
                let playing = game.is_some_and(|cur| cur.id() == g.id);
                let bar = card_title(ui, fonts, p, full[0], if playing { "Playing Now" } else { "Game Details" });
                let pane_bottom = p[1] + full[1];
                let p = [p[0], p[1] + bar];
                fade(it, || {
                    let pop = ease_out_back(it);
                    let s = 128.0 * (0.85 + 0.15 * pop);
                    {
                        let dl = ui.get_window_draw_list();
                        game_icon(&dl, media, &g.id, [p[0] + 18.0 + (128.0 - s) / 2.0, p[1] + 18.0 + (128.0 - s) / 2.0], s);
                    }
                    ui.set_cursor_screen_pos([p[0] + 162.0, p[1] + 20.0]);
                    ui.group(|| {
                        let _w = ui.push_text_wrap_pos_with_pos(full[0] - 16.0);
                        if playing && bar == 0.0 {
                            let _f = ui.push_font(fonts.small_bold);
                            let pulse = 0.7 + 0.3 * ((ui.time() * 3.0).sin() as f32 * 0.5 + 0.5);
                            ui.text_colored(with_alpha(pal().text, pulse), "PLAYING NOW");
                        }
                        {
                            let _f = ui.push_font(fonts.large);
                            ui.text_colored(pal().text, &g.title);
                        }
                        if let Some(sub) = &g.subtitle {
                            let _f = ui.push_font(fonts.body);
                            ui.text_colored(pal().soft, sub);
                        }
                    });
                    let y = (p[1] + 166.0).max(ui.cursor_screen_pos()[1] + 12.0);
                    ui.set_cursor_screen_pos([p[0] + 18.0, y]);
                    let frac = if total == 0 { 0.0 } else { got as f32 / total as f32 };
                    progress(ui, self.anim.to(("games_bar", self.games_sel), frac, 3.0, 0.0), full[0] - 36.0, 10.0);
                    ui.set_cursor_screen_pos([p[0] + 18.0, ui.cursor_screen_pos()[1] + 4.0]);
                    {
                        let _f = ui.push_font(fonts.body);
                        let pct = if total == 0 { 0 } else { got * 100 / total };
                        ui.text_colored(pal().text, format!("{got} of {total} achievements  ·  {pct}%"));
                    }
                    ui.set_cursor_screen_pos([p[0] + 18.0, ui.cursor_screen_pos()[1] + 2.0]);
                    gamerscore(ui, fonts, &format!("{pts} / {}", g.total_points), false, true, false);
                    ui.set_cursor_screen_pos([p[0] + 18.0, ui.cursor_screen_pos()[1] + 2.0]);
                    {
                        let _f = ui.push_font(fonts.body);
                        ui.text_colored(pal().soft, format!("Last played {}", format_ago(g.last_played)));
                    }
                    let hy = pane_bottom - 30.0;
                    glyph(ui, fonts, [p[0] + 30.0, hy], "A", true, None, false);
                    ui.set_cursor_screen_pos([p[0] + 50.0, hy - 10.0]);
                    let _f = ui.push_font(fonts.body);
                    ui.text_colored(pal().text, "View achievements");
                });
            })
        });
    }
}

/// A gamer picture: the player's uploaded one (once downloaded), else the
/// letter or emoji (or the gamertag's initial) on a green tile.
#[allow(clippy::too_many_arguments)]
fn gamer_pic(ui: &Ui, dl: &DrawListMut, fonts: &Fonts, media: &Media, avatar: &str, gamertag: &str, pmin: [f32; 2], pmax: [f32; 2]) {
    if let Some((tex, _)) = avatar.starts_with("img:").then(|| media.get(Kind::Gamerpic, "", gamertag)).flatten() {
        dl.add_image(tex, pmin, pmax).build();
        return;
    }
    gradient(dl, pmin, pmax, GREEN_LIGHT, GREEN_DARK);
    let letter = if avatar.is_empty() || avatar.starts_with("img:") { gamertag.chars().next().unwrap_or('?').to_uppercase().to_string() } else { avatar.to_string() };
    let _f = ui.push_font(fonts.large);
    centered_text(ui, dl, pmin, pmax, WHITE, &letter);
}

fn group_digits(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn empty(ui: &Ui, fonts: &Fonts, message: &str) {
    let avail = ui.content_region_avail();
    let _f = ui.push_font(fonts.title);
    let t = ui.calc_text_size(message);
    ui.set_cursor_pos([(avail[0] - t[0]) / 2.0, avail[1] / 2.0 - t[1]]);
    shadow_text(ui, pal().soft, message);
}
