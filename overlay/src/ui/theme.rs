//! Look and feel: Xbox 360 "blades" style. Glossy green active blade, silver
//! blades stacked at the edges, white text with soft shadows. Shared drawing
//! helpers live here, including vertex tricks (fading, sideways text) that
//! imgui doesn't do by itself.

use crate::media::{Kind, Media};
use crate::Fonts;
use hudhook::imgui::{sys, DrawListMut, ImColor32, TextureId, Ui};
use std::cell::Cell;

// Toasts and shared accents.
pub const GREEN: [f32; 4] = [0.486, 0.749, 0.118, 1.0];
pub const GREEN_LIGHT: [f32; 4] = [0.639, 0.871, 0.271, 1.0];
pub const GREEN_DARK: [f32; 4] = [0.267, 0.471, 0.059, 1.0];
pub const WHITE: [f32; 4] = [0.98, 0.99, 0.98, 1.0];
pub const PLATINUM: [f32; 4] = [0.82, 0.88, 0.95, 1.0];
pub const INK: [f32; 4] = [0.04, 0.06, 0.08, 1.0];

// Blades.
pub const BLADE_TOP: [f32; 4] = [0.557, 0.878, 0.353, 1.0];
pub const BLADE_BOTTOM: [f32; 4] = [0.161, 0.553, 0.094, 1.0];

// On the green blade.

// Metro.
pub const METRO_GREEN: [f32; 4] = [0.357, 0.725, 0.161, 1.0];
pub const METRO_BG_TOP: [f32; 4] = [0.965, 0.965, 0.965, 0.99];
pub const METRO_BG_BOTTOM: [f32; 4] = [0.780, 0.785, 0.790, 0.99];

// Classic.
pub const CLASSIC_BG_TOP: [f32; 4] = [0.105, 0.157, 0.220, 0.98];
pub const CLASSIC_BG_BOTTOM: [f32; 4] = [0.063, 0.082, 0.118, 0.98];
pub const CLASSIC_TAB: [f32; 4] = [0.149, 0.200, 0.267, 1.0];
pub const CLASSIC_TAB_HOVER: [f32; 4] = [0.200, 0.267, 0.349, 1.0];

/// Dashboard designs the player can pick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub enum Skin {
    #[default]
    Blades,
    Metro,
    Classic,
}

impl Skin {
    pub const ALL: [Skin; 3] = [Skin::Blades, Skin::Metro, Skin::Classic];

    /// Name stored in profile.toml.
    pub fn id(self) -> &'static str {
        match self {
            Skin::Blades => "blades",
            Skin::Metro => "metro",
            Skin::Classic => "classic",
        }
    }

    pub fn from_id(id: &str) -> Skin {
        Skin::ALL.into_iter().find(|s| s.id() == id).unwrap_or_default()
    }

    pub fn name(self) -> &'static str {
        match self {
            Skin::Blades => "Blades",
            Skin::Metro => "Metro",
            Skin::Classic => "Classic",
        }
    }

    pub fn next(self) -> Skin {
        let i = Skin::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Skin::ALL[(i + 1) % Skin::ALL.len()]
    }
}

/// Colours for content drawn on the dashboard's background, per design.
pub struct Palette {
    pub text: [f32; 4],
    pub soft: [f32; 4],
    pub faint: [f32; 4],
    /// Row and box backgrounds.
    pub row: [f32; 4],
    pub card: [f32; 4],
    /// Hover wash over rows and buttons.
    pub hover: [f32; 4],
    /// The selection: fill (top/bottom), outline, and text on it.
    pub sel_top: [f32; 4],
    pub sel_bottom: [f32; 4],
    pub sel_edge: [f32; 4],
    /// Accent bar at the left of the selection (Classic).
    pub sel_bar: bool,
    pub on_sel: [f32; 4],
    pub on_sel_soft: [f32; 4],
    pub accent: [f32; 4],
    /// Progress bar groove, fill and shine.
    pub track: [f32; 4],
    pub fill_top: [f32; 4],
    pub fill_bottom: [f32; 4],
    pub shine: [f32; 4],
    /// Drop shadow strength under text and boxes (0 for flat designs).
    pub shadow: f32,
    pub radius: f32,
    /// Gamerscore "G" disc and letter, normally and on the selection.
    pub g_disc: [f32; 4],
    pub g_letter: [f32; 4],
    pub g_disc_sel: [f32; 4],
    pub g_letter_sel: [f32; 4],
    /// Points pills, normally and on the selection.
    pub pill: [f32; 4],
    pub pill_sel: [f32; 4],
    pub pill_sel_text: [f32; 4],
    /// Outline of unlocked icons and game tiles (alpha 0 for none).
    pub icon_edge: [f32; 4],
    pub frame: [f32; 4],
    pub warn: [f32; 4],
    /// Rows are engraved bars split by lines, not separate boxes (Blades).
    pub row_lines: bool,
    /// Outline around cards (alpha 0 for none).
    pub card_edge: [f32; 4],
    /// Drop shadow under the selection.
    pub box_shadow: f32,
    /// The selection glows white on the left and fades out to the right.
    pub sel_fade: bool,
    /// Button hint labels (they sit on the frame, not the content).
    pub hint_text: [f32; 4],
    pub hint_shadow: f32,
}

const BLADES: Palette = Palette {
    text: [0.05, 0.09, 0.04, 1.0],
    soft: [0.09, 0.17, 0.05, 0.88],
    faint: [0.10, 0.22, 0.06, 0.70],
    row: [1.0, 1.0, 1.0, 0.13],
    card: [1.0, 1.0, 1.0, 0.16],
    hover: [1.0, 1.0, 1.0, 0.18],
    sel_top: [1.0, 1.0, 1.0, 0.95],
    sel_bottom: [0.90, 0.98, 0.86, 0.85],
    sel_edge: [1.0, 1.0, 1.0, 0.85],
    sel_bar: false,
    on_sel: [0.04, 0.12, 0.02, 1.0],
    on_sel_soft: [0.10, 0.30, 0.04, 0.92],
    accent: [1.0, 1.0, 1.0, 1.0],
    track: [0.0, 0.18, 0.0, 0.30],
    fill_top: [1.0, 1.0, 1.0, 1.0],
    fill_bottom: [0.82, 0.96, 0.74, 1.0],
    shine: [0.6, 1.0, 0.4, 0.55],
    shadow: 0.0,
    radius: 7.0,
    g_disc: [0.06, 0.10, 0.05, 1.0],
    g_letter: WHITE,
    g_disc_sel: [0.16, 0.50, 0.04, 1.0],
    g_letter_sel: WHITE,
    pill: [1.0, 1.0, 1.0, 0.30],
    pill_sel: [0.16, 0.50, 0.04, 1.0],
    pill_sel_text: WHITE,
    icon_edge: [1.0, 1.0, 1.0, 0.9],
    frame: [1.0, 1.0, 1.0, 0.9],
    warn: [0.55, 0.08, 0.02, 1.0],
    row_lines: true,
    card_edge: [1.0, 1.0, 1.0, 0.65],
    box_shadow: 0.0,
    sel_fade: true,
    hint_text: [1.0, 1.0, 1.0, 1.0],
    hint_shadow: 0.50,
};

const METRO: Palette = Palette {
    text: [0.10, 0.10, 0.10, 1.0],
    soft: [0.32, 0.32, 0.32, 1.0],
    faint: [0.50, 0.50, 0.50, 1.0],
    row: [1.0, 1.0, 1.0, 0.55],
    card: [1.0, 1.0, 1.0, 0.82],
    hover: [0.357, 0.725, 0.161, 0.14],
    sel_top: METRO_GREEN,
    sel_bottom: METRO_GREEN,
    sel_edge: [0.0, 0.0, 0.0, 0.0],
    sel_bar: false,
    on_sel: WHITE,
    on_sel_soft: [0.93, 1.0, 0.88, 0.92],
    accent: METRO_GREEN,
    track: [0.0, 0.0, 0.0, 0.12],
    fill_top: METRO_GREEN,
    fill_bottom: METRO_GREEN,
    shine: [1.0, 1.0, 1.0, 0.45],
    shadow: 0.0,
    radius: 0.0,
    g_disc: METRO_GREEN,
    g_letter: WHITE,
    g_disc_sel: WHITE,
    g_letter_sel: METRO_GREEN,
    pill: [0.0, 0.0, 0.0, 0.08],
    pill_sel: [1.0, 1.0, 1.0, 0.25],
    pill_sel_text: WHITE,
    icon_edge: [0.0, 0.0, 0.0, 0.0],
    frame: [0.0, 0.0, 0.0, 0.0],
    warn: [0.80, 0.30, 0.10, 1.0],
    row_lines: false,
    card_edge: [0.0, 0.0, 0.0, 0.0],
    box_shadow: 0.0,
    sel_fade: false,
    hint_text: [0.10, 0.10, 0.10, 1.0],
    hint_shadow: 0.0,
};

const CLASSIC: Palette = Palette {
    text: [0.96, 0.97, 0.98, 1.0],
    soft: [0.780, 0.835, 0.878, 1.0],
    faint: [0.561, 0.596, 0.627, 1.0],
    row: [0.118, 0.165, 0.227, 1.0],
    card: [0.082, 0.118, 0.165, 1.0],
    hover: [1.0, 1.0, 1.0, 0.05],
    sel_top: [0.188, 0.278, 0.380, 1.0],
    sel_bottom: [0.188, 0.278, 0.380, 1.0],
    sel_edge: [0.486, 0.749, 0.118, 0.45],
    sel_bar: true,
    on_sel: [0.96, 0.97, 0.98, 1.0],
    on_sel_soft: [0.780, 0.835, 0.878, 1.0],
    accent: GREEN,
    track: [0.0, 0.0, 0.0, 0.45],
    fill_top: GREEN_LIGHT,
    fill_bottom: GREEN_DARK,
    shine: [1.0, 1.0, 1.0, 0.35],
    shadow: 0.0,
    radius: 2.0,
    g_disc: GREEN,
    g_letter: INK,
    g_disc_sel: GREEN,
    g_letter_sel: INK,
    pill: [0.0, 0.0, 0.0, 0.30],
    pill_sel: GREEN_DARK,
    pill_sel_text: WHITE,
    icon_edge: [0.486, 0.749, 0.118, 0.9],
    frame: [0.0, 0.0, 0.0, 0.0],
    warn: [0.95, 0.55, 0.35, 1.0],
    row_lines: false,
    card_edge: [0.0, 0.0, 0.0, 0.0],
    box_shadow: 0.0,
    sel_fade: false,
    hint_text: [0.561, 0.596, 0.627, 1.0],
    hint_shadow: 0.0,
};

thread_local! {
    static SKIN: Cell<Skin> = const { Cell::new(Skin::Blades) };
    /// An installed theme pack's pieces for this frame (None: draw ours).
    static THEMED: Cell<Option<ThemeTextures>> = const { Cell::new(None) };
}

/// Pieces of an installed theme pack the shared helpers draw with.
#[derive(Clone, Copy)]
pub struct ThemeTextures {
    /// Selection bar.
    pub focus: Option<TextureId>,
    /// Gamer-card frame and its pixel size.
    pub card: Option<(TextureId, [f32; 2])>,
    /// A, B, X, Y.
    pub glyphs: [Option<TextureId>; 4],
    /// Screen pixels per texture pixel for frames (scales with the panel).
    pub scale: f32,
}

pub fn set_theme_textures(t: Option<ThemeTextures>) {
    THEMED.set(t);
}

fn themed() -> Option<ThemeTextures> {
    THEMED.get().filter(|_| skin() == Skin::Blades)
}

/// Cards are drawn with a theme pack's gamer-card frame.
pub fn themed_cards() -> bool {
    themed().is_some_and(|t| t.card.is_some())
}

pub fn theme_scale() -> f32 {
    themed().map_or(1.0, |t| t.scale)
}

/// Draws `tex` into `min..max` keeping the borders' size: the texture's
/// `inset` (left, top, right, bottom, in texture pixels, after cropping to
/// `crop`) is drawn at `scale`, the middle stretches.
fn nine_slice(dl: &DrawListMut, tex: TextureId, size: [f32; 2], crop: [f32; 4], inset: [f32; 4], scale: f32, min: [f32; 2], max: [f32; 2], alpha: f32) {
    let (cx0, cy0, cx1, cy1) = (crop[0], crop[1], size[0] - crop[2], size[1] - crop[3]);
    let tx = [cx0, cx0 + inset[0], cx1 - inset[2], cx1];
    let ty = [cy0, cy0 + inset[1], cy1 - inset[3], cy1];
    let w = max[0] - min[0];
    let h = max[1] - min[1];
    let (l, r) = ((inset[0] * scale).min(w / 2.0), (inset[2] * scale).min(w / 2.0));
    let (t, b) = ((inset[1] * scale).min(h * 0.6), (inset[3] * scale).min(h * 0.4));
    let sx = [min[0], min[0] + l, max[0] - r, max[0]];
    let sy = [min[1], min[1] + t, max[1] - b, max[1]];
    for j in 0..3 {
        for i in 0..3 {
            if sx[i + 1] - sx[i] < 0.5 || sy[j + 1] - sy[j] < 0.5 {
                continue;
            }
            dl.add_image(tex, [sx[i], sy[j]], [sx[i + 1], sy[j + 1]])
                .uv_min([tx[i] / size[0], ty[j] / size[1]])
                .uv_max([tx[i + 1] / size[0], ty[j + 1] / size[1]])
                .col([1.0, 1.0, 1.0, alpha])
                .build();
        }
    }
}

/// With a themed gamer-card frame, writes `title` in its gold bar (as the
/// dashboard puts the gamertag there) and returns how far down the card's
/// own contents should start; 0 otherwise, so callers keep their layout.
pub fn card_title(ui: &Ui, fonts: &Fonts, min: [f32; 2], width: f32, title: &str) -> f32 {
    let Some(t) = themed().filter(|t| t.card.is_some()) else { return 0.0 };
    let s = t.scale;
    let (top, bottom) = (min[1] + 6.0 * s, min[1] + 54.0 * s);
    let _f = ui.push_font(fonts.title);
    let text = fit(ui, title, width - 28.0);
    let h = ui.calc_text_size(&text)[1];
    ui.get_window_draw_list().add_text([min[0] + 14.0, (top + bottom - h) / 2.0], col([0.12, 0.09, 0.0, 1.0]), &text);
    58.0 * s
}

/// Sets the design used for drawing this frame.
pub fn set_skin(skin: Skin) {
    SKIN.set(skin);
}

pub fn skin() -> Skin {
    SKIN.get()
}

/// The current design's colours.
pub fn p() -> &'static Palette {
    match SKIN.get() {
        Skin::Blades => &BLADES,
        Skin::Metro => &METRO,
        Skin::Classic => &CLASSIC,
    }
}

pub fn col(c: [f32; 4]) -> ImColor32 {
    ImColor32::from_rgba_f32s(c[0], c[1], c[2], c[3])
}

pub fn with_alpha(c: [f32; 4], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * a]
}

/// Badge colour per category (used for icons without artwork).
pub fn category_color(category: &str) -> [f32; 4] {
    match category {
        "Leon's Story" => [0.20, 0.42, 0.78, 1.0],
        "Claire's Story" => [0.78, 0.22, 0.30, 1.0],
        "Bosses & Creatures" => [0.45, 0.62, 0.22, 1.0],
        "Familiar Faces" => [0.55, 0.40, 0.70, 1.0],
        "Weapons & Gear" => [0.70, 0.45, 0.18, 1.0],
        "Odds & Ends" => [0.25, 0.60, 0.62, 1.0],
        "Challenges" => [0.80, 0.30, 0.15, 1.0],
        "Bonus Modes" => [0.35, 0.35, 0.40, 1.0],
        "Platinum" => [0.70, 0.76, 0.84, 1.0],
        _ => [0.30, 0.38, 0.48, 1.0],
    }
}

// ── Vertex tricks ───────────────────────────────────────────────────────
//
// Everything drawn into a window lands in its draw list's vertex buffer, so
// we can note where the buffer ends, draw, then adjust the new vertices:
// fade them (imgui has no per-draw alpha) or rotate them (sideways labels).

thread_local! {
    /// Fade applied to child windows opened inside `fade` (their vertices
    /// live in their own draw lists).
    static AMBIENT: Cell<f32> = const { Cell::new(1.0) };
    /// How deep in child windows we are (for their font scale).
    static CHILD_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Draw lists of the panel's windows this frame (to scale afterwards).
    static PANEL_LISTS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

// ── Resolution scaling ──────────────────────────────────────────────────
//
// The panel is laid out on a virtual canvas 700 units tall (what its sizes
// were designed for) and its finished geometry is then scaled to the real
// screen. Fonts are rasterised at the final size and drawn at 1/scale in
// the layout, so text comes out sharp.

/// Scale the fonts were rasterised at (bits of an f32).
static FONT_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

pub fn set_font_scale(s: f32) {
    FONT_SCALE.store(s.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

pub fn font_scale() -> f32 {
    f32::from_bits(FONT_SCALE.load(std::sync::atomic::Ordering::Relaxed))
}

/// Call first thing in each of the panel's windows: draws text at layout
/// size and remembers the window's draw list for `scale_panel`.
pub fn panel_window(ui: &Ui) {
    ui.set_window_font_scale(1.0 / font_scale());
    // SAFETY: called while the window is being built.
    let dl = unsafe { sys::igGetWindowDrawList() } as usize;
    PANEL_LISTS.with_borrow_mut(|v| {
        if !v.contains(&dl) {
            v.push(dl);
        }
    });
}

/// After the panel is built: scales every vertex and clip rect of its
/// windows by `f` around `origin`.
pub fn scale_panel(origin: [f32; 2], f: f32) {
    let lists = PANEL_LISTS.with_borrow_mut(std::mem::take);
    for dl in lists {
        let dl = dl as *mut sys::ImDrawList;
        // SAFETY: these draw lists belong to windows built this frame and
        // stay alive until rendering; nothing else touches them now.
        unsafe {
            let vb = &(*dl).VtxBuffer;
            if !vb.Data.is_null() {
                for v in std::slice::from_raw_parts_mut(vb.Data, vb.Size as usize) {
                    v.pos.x = origin[0] + (v.pos.x - origin[0]) * f;
                    v.pos.y = origin[1] + (v.pos.y - origin[1]) * f;
                }
            }
            let cb = &(*dl).CmdBuffer;
            if !cb.Data.is_null() {
                for c in std::slice::from_raw_parts_mut(cb.Data, cb.Size as usize) {
                    c.ClipRect.x = origin[0] + (c.ClipRect.x - origin[0]) * f;
                    c.ClipRect.y = origin[1] + (c.ClipRect.y - origin[1]) * f;
                    c.ClipRect.z = origin[0] + (c.ClipRect.z - origin[0]) * f;
                    c.ClipRect.w = origin[1] + (c.ClipRect.w - origin[1]) * f;
                }
            }
        }
    }
}

/// The current window's vertex count.
pub fn vtx_mark() -> i32 {
    // SAFETY: called while a window is being built, so the list exists.
    unsafe { (*sys::igGetWindowDrawList()).VtxBuffer.Size }
}

fn with_verts_since(mark: i32, f: impl FnMut(&mut sys::ImDrawVert)) {
    // SAFETY: as above; `mark` came from `vtx_mark` on the same window this
    // frame, and the buffer only grows within a frame.
    unsafe {
        let buf = &(*sys::igGetWindowDrawList()).VtxBuffer;
        if buf.Data.is_null() || mark >= buf.Size {
            return;
        }
        std::slice::from_raw_parts_mut(buf.Data.add(mark as usize), (buf.Size - mark) as usize).iter_mut().for_each(f);
    }
}

/// Fades the vertices drawn since `mark` from clear at `left` to unchanged
/// at `right` (a horizontal ramp across a quad's corners).
pub fn fade_verts_x(mark: i32, left: f32, right: f32) {
    let span = (right - left).max(1.0);
    with_verts_since(mark, |v| {
        let a = ((v.pos.x - left) / span).clamp(0.0, 1.0);
        let old = (v.col >> 24) as f32;
        v.col = (v.col & 0x00ff_ffff) | (((old * a).round() as u32) << 24);
    });
}

fn scale_alpha(mark: i32, alpha: f32) {
    if alpha >= 0.999 {
        return;
    }
    let a = alpha.clamp(0.0, 1.0);
    with_verts_since(mark, |v| {
        let old = (v.col >> 24) as f32;
        v.col = (v.col & 0x00ff_ffff) | (((old * a).round() as u32) << 24);
    });
}

/// Runs `draw`, fading everything it adds (including child windows wrapped
/// in `child`) by `alpha`.
pub fn fade<R>(alpha: f32, draw: impl FnOnce() -> R) -> R {
    let outer = AMBIENT.get();
    AMBIENT.set(outer * alpha);
    let mark = vtx_mark();
    let r = draw();
    scale_alpha(mark, alpha);
    AMBIENT.set(outer);
    r
}

/// Child window contents: call first thing inside a child window's build
/// closure so fades from the parent apply in here too.
pub fn child<R>(draw: impl FnOnce() -> R) -> R {
    // Text at layout size; scaled with the rest of the panel afterwards.
    // imgui multiplies a window's font scale by its parent's (one level),
    // so with the panel at 1/scale its children use 1, theirs 1/scale...
    let depth = CHILD_DEPTH.get() + 1;
    CHILD_DEPTH.set(depth);
    // SAFETY: called inside a child window's build.
    let dl = unsafe {
        sys::igSetWindowFontScale(if depth % 2 == 1 { 1.0 } else { 1.0 / font_scale() });
        sys::igGetWindowDrawList()
    } as usize;
    PANEL_LISTS.with_borrow_mut(|v| {
        if !v.contains(&dl) {
            v.push(dl);
        }
    });
    let ambient = AMBIENT.get();
    let mark = vtx_mark();
    let r = draw();
    scale_alpha(mark, ambient);
    CHILD_DEPTH.set(depth - 1);
    r
}

/// Like `sideways_text`, with `spacing` extra pixels between letters.
pub fn sideways_text_spaced(ui: &Ui, center: [f32; 2], color: [f32; 4], text: &str, spacing: f32) {
    let chars: Vec<char> = text.chars().collect();
    let widths: Vec<f32> = chars.iter().map(|c| ui.calc_text_size(c.to_string())[0]).collect();
    let total = widths.iter().sum::<f32>() + spacing * chars.len().saturating_sub(1) as f32;
    let h = ui.calc_text_size(text)[1];
    let wp = ui.window_pos();
    let ws = ui.window_size();
    let pivot = [wp[0] + ws[0] / 2.0, wp[1] + ws[1] / 2.0];
    let mark = vtx_mark();
    let mut x = pivot[0] - total / 2.0;
    for (c, w) in chars.iter().zip(&widths) {
        ui.get_window_draw_list().add_text([x, pivot[1] - h / 2.0], col(color), c.to_string());
        x += w + spacing;
    }
    with_verts_since(mark, |v| {
        let (dx, dy) = (v.pos.x - pivot[0], v.pos.y - pivot[1]);
        v.pos.x = center[0] - dy;
        v.pos.y = center[1] + dx;
    });
}

/// Text turned 90° clockwise (reading top to bottom), centred on `center`.
pub fn sideways_text(ui: &Ui, center: [f32; 2], color: [f32; 4], text: &str) {
    let t = ui.calc_text_size(text);
    // Draw upright somewhere surely inside the clip rect, then move it.
    let wp = ui.window_pos();
    let ws = ui.window_size();
    let pivot = [wp[0] + ws[0] / 2.0, wp[1] + ws[1] / 2.0];
    let mark = vtx_mark();
    ui.get_window_draw_list().add_text([pivot[0] - t[0] / 2.0, pivot[1] - t[1] / 2.0], col(color), text);
    with_verts_since(mark, |v| {
        let (dx, dy) = (v.pos.x - pivot[0], v.pos.y - pivot[1]);
        v.pos.x = center[0] - dy;
        v.pos.y = center[1] + dx;
    });
}

// ── Shapes ──────────────────────────────────────────────────────────────

/// Vertical gradient rectangle.
pub fn gradient(dl: &DrawListMut, a: [f32; 2], b: [f32; 2], top: [f32; 4], bottom: [f32; 4]) {
    dl.add_rect_filled_multicolor(a, b, col(top), col(top), col(bottom), col(bottom));
}

/// Horizontal gradient rectangle.
pub fn hgradient(dl: &DrawListMut, a: [f32; 2], b: [f32; 2], left: [f32; 4], right: [f32; 4]) {
    dl.add_rect_filled_multicolor(a, b, col(left), col(right), col(right), col(left));
}

/// A rounded box, a little darker than the blade.
pub fn shade_box(dl: &DrawListMut, min: [f32; 2], max: [f32; 2], color: [f32; 4]) {
    dl.add_rect(min, max, color).filled(true).rounding(p().radius).build();
}

/// A row's background: a box, or (Blades) an engraved bar with a light
/// line on top and a dark one beneath.
pub fn row_bg(dl: &DrawListMut, min: [f32; 2], max: [f32; 2], amount: f32) {
    let pal = p();
    if themed().is_some() {
        // The dashboard's unselected rows are bare, split by thin lines.
        hgradient(dl, [min[0], max[1] - 1.0], [max[0], max[1]], [0.0, 0.0, 0.0, 0.28 * amount], [0.0, 0.0, 0.0, 0.08 * amount]);
        hgradient(dl, [min[0], max[1]], [max[0], max[1] + 1.0], [1.0, 1.0, 1.0, 0.30 * amount], [1.0, 1.0, 1.0, 0.06 * amount]);
        return;
    }
    if pal.row_lines {
        hgradient(dl, min, max, with_alpha(pal.row, amount), with_alpha(pal.row, 0.3 * amount));
        hgradient(dl, [min[0], max[1] - 1.0], [max[0], max[1]], [0.0, 0.05, 0.0, 0.30 * amount], [0.0, 0.05, 0.0, 0.10 * amount]);
        hgradient(dl, [min[0], max[1]], [max[0], max[1] + 1.0], [1.0, 1.0, 1.0, 0.25 * amount], [1.0, 1.0, 1.0, 0.05 * amount]);
    } else {
        shade_box(dl, min, max, with_alpha(pal.row, amount));
    }
}

/// A card (info pane, profile box) with its outline, if the design has one.
pub fn card_box(dl: &DrawListMut, min: [f32; 2], max: [f32; 2]) {
    if let Some(t) = themed() {
        if let Some((tex, size)) = t.card {
            // Cropped to the frame (the art has a transparent margin); the
            // gold title bar is in the top border.
            nine_slice(dl, tex, size, [6.0, 16.0, 6.0, 28.0], [20.0, 58.0, 20.0, 24.0], t.scale, min, max, 1.0);
            return;
        }
    }
    let pal = p();
    shade_box(dl, min, max, pal.card);
    if pal.card_edge[3] > 0.0 {
        dl.add_rect(min, max, pal.card_edge).rounding(pal.radius).thickness(1.2).build();
    }
}

/// The hover wash over a row or button.
pub fn hover_wash(dl: &DrawListMut, min: [f32; 2], max: [f32; 2], amount: f32) {
    if amount > 0.01 {
        dl.add_rect(min, max, with_alpha(p().hover, amount)).filled(true).rounding(p().radius).build();
    }
}

/// The white highlight bar behind the selected row.
pub fn highlight(dl: &DrawListMut, min: [f32; 2], max: [f32; 2], amount: f32) {
    if amount <= 0.01 {
        return;
    }
    let pal = p();
    let r = pal.radius;
    if pal.box_shadow > 0.0 {
        dl.add_rect([min[0] + 1.0, min[1] + 3.0], [max[0] + 1.0, max[1] + 3.0], [0.0, 0.0, 0.0, pal.box_shadow * amount]).filled(true).rounding(r).build();
    }
    let mid = (min[1] + max[1]) / 2.0;
    if let Some(t) = themed().and_then(|t| t.focus) {
        dl.add_image(t, min, max).col([1.0, 1.0, 1.0, amount]).build();
        return;
    }
    if pal.sel_fade {
        // Blades: a white glow, brightest on the left, with a gloss on top.
        let w = max[0] - min[0];
        let knee = min[0] + w * 0.55;
        let (a, b) = (with_alpha(pal.sel_top, amount), with_alpha(pal.sel_bottom, 0.35 * amount));
        dl.add_rect([min[0], min[1]], [min[0] + r, max[1]], a).filled(true).rounding(r).round_top_right(false).round_bot_right(false).build();
        hgradient(dl, [min[0] + r, min[1]], [knee, max[1]], a, with_alpha(pal.sel_bottom, 0.7 * amount));
        hgradient(dl, [knee, min[1]], [max[0], max[1]], with_alpha(pal.sel_bottom, 0.7 * amount), b);
        hgradient(dl, [min[0] + 2.0, min[1] + 1.0], [max[0], mid], [1.0, 1.0, 1.0, 0.35 * amount], [1.0, 1.0, 1.0, 0.0]);
        hgradient(dl, [min[0] + r, min[1]], [max[0], min[1] + 1.0], with_alpha(pal.sel_edge, amount), [1.0, 1.0, 1.0, 0.0]);
        hgradient(dl, [min[0] + r, max[1] - 1.0], [max[0], max[1]], [0.0, 0.0, 0.0, 0.25 * amount], [0.0, 0.0, 0.0, 0.0]);
        return;
    }
    dl.add_rect(min, max, with_alpha(pal.sel_bottom, amount)).filled(true).rounding(r).build();
    if pal.sel_top != pal.sel_bottom {
        dl.add_rect([min[0] + 1.0, min[1] + 1.0], [max[0] - 1.0, mid], with_alpha(pal.sel_top, amount)).filled(true).rounding(r).build();
    }
    if pal.sel_edge[3] > 0.0 {
        dl.add_rect(min, max, with_alpha(pal.sel_edge, amount)).rounding(r).build();
    }
    if pal.sel_bar {
        let h = (max[1] - min[1]) * amount;
        dl.add_rect([min[0], mid - h / 2.0], [min[0] + 4.0, mid + h / 2.0], pal.accent).filled(true).build();
    }
}

/// Text with a soft drop shadow, at the cursor. Advances the cursor.
pub fn shadow_text(ui: &Ui, color: [f32; 4], text: &str) {
    let shadow = p().shadow;
    if shadow > 0.0 {
        let pos = ui.cursor_screen_pos();
        ui.get_window_draw_list().add_text([pos[0] + 1.0, pos[1] + 2.0], col([0.0, 0.1, 0.0, shadow * color[3]]), text);
    }
    ui.text_colored(color, text);
}

/// A padlock centred in a rectangle (for secret/locked art).
pub fn padlock(dl: &DrawListMut, min: [f32; 2], max: [f32; 2], color: [f32; 4]) {
    let s = (max[0] - min[0]).min(max[1] - min[1]);
    let c = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let bw = s * 0.36;
    let bh = s * 0.27;
    let body_top = c[1] - bh * 0.15;
    dl.add_rect([c[0] - bw / 2.0, body_top], [c[0] + bw / 2.0, body_top + bh], color).filled(true).rounding(s * 0.04).build();
    // Shackle: an arch above the body.
    let r = bw * 0.32;
    let t = s * 0.055;
    let base = body_top + t * 0.3;
    dl.add_bezier_curve([c[0] - r, base], [c[0] - r, base - r * 2.2], [c[0] + r, base - r * 2.2], [c[0] + r, base], color)
        .thickness(t)
        .build();
    // Keyhole.
    dl.add_circle([c[0], body_top + bh * 0.42], s * 0.035, [0.0, 0.0, 0.0, 0.55]).filled(true).build();
}

/// A medal with a five-pointed star: stand-in art when nothing else exists.
pub fn emblem(dl: &DrawListMut, min: [f32; 2], max: [f32; 2], base: [f32; 4], lit: bool) {
    let top = if lit { base } else { [base[0] * 0.35, base[1] * 0.35, base[2] * 0.35, 1.0] };
    gradient(dl, min, max, top, [top[0] * 0.6, top[1] * 0.6, top[2] * 0.6, 1.0]);
    let s = (max[0] - min[0]).min(max[1] - min[1]);
    let c = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];
    let ink = if lit { [1.0, 1.0, 1.0, 0.9] } else { [1.0, 1.0, 1.0, 0.3] };
    dl.add_circle(c, s * 0.34, ink).thickness(s * 0.04).build();
    // Star: centre pentagon plus five point triangles (all convex pieces).
    let (outer, inner) = (s * 0.24, s * 0.095);
    let pt = |k: f32, r: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + k * std::f32::consts::PI / 5.0;
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    };
    let pentagon: Vec<[f32; 2]> = (0..5).map(|i| pt(i as f32 * 2.0 + 1.0, inner)).collect();
    dl.add_polyline(pentagon, ink).filled(true).build();
    for i in 0..5 {
        let k = i as f32 * 2.0;
        dl.add_triangle(pt(k, outer), pt(k - 1.0, inner), pt(k + 1.0, inner), ink).filled(true).build();
    }
}

/// A store badge (GOG, Steam, ...) ending at `right`, centred on `mid_y`,
/// silver and glossy like the dashboard's "XBOX 360" tag. Returns its width.
pub fn store_badge(ui: &Ui, fonts: &Fonts, right: f32, mid_y: f32, text: &str) -> f32 {
    // Plain lettering where the dashboard printed "XBOX 360".
    let label = text.to_uppercase();
    let _f = ui.push_font(fonts.title);
    let t = ui.calc_text_size(&label);
    // Centred on the capitals (they have no descenders, so the line box's
    // middle sits a little high).
    let pos = [right - t[0], mid_y - t[1] / 2.0 - t[1] * 0.12];
    let dl = ui.get_window_draw_list();
    dl.add_text([pos[0] + 1.0, pos[1] + 1.0], col([1.0, 1.0, 1.0, 0.35]), &label);
    dl.add_text(pos, col(with_alpha(p().soft, 0.85)), &label);
    t[0]
}

/// A five-pointed star (convex pieces), centred on `c`.
pub fn star(dl: &DrawListMut, c: [f32; 2], r: f32, color: [f32; 4]) {
    let inner = r * 0.42;
    let pt = |k: f32, rr: f32| {
        let a = -std::f32::consts::FRAC_PI_2 + k * std::f32::consts::PI / 5.0;
        [c[0] + rr * a.cos(), c[1] + rr * a.sin()]
    };
    let pentagon: Vec<[f32; 2]> = (0..5).map(|i| pt(i as f32 * 2.0 + 1.0, inner)).collect();
    dl.add_polyline(pentagon, color).filled(true).build();
    for i in 0..5 {
        let k = i as f32 * 2.0;
        dl.add_triangle(pt(k, r), pt(k - 1.0, inner), pt(k + 1.0, inner), color).filled(true).build();
    }
}

/// Text centred in a rectangle.
pub fn centered_text(ui: &Ui, dl: &DrawListMut, min: [f32; 2], max: [f32; 2], color: [f32; 4], text: &str) {
    let t = ui.calc_text_size(text);
    dl.add_text([min[0] + (max[0] - min[0] - t[0]) / 2.0, min[1] + (max[1] - min[1] - t[1]) / 2.0], col(color), text);
}

/// Gamerscore: a "G" disc followed by the number. `dark` is for use on the
/// white highlight. Advances the cursor.
pub fn gamerscore(ui: &Ui, fonts: &Fonts, value: &str, big: bool, active: bool, dark: bool) {
    let dl = ui.get_window_draw_list();
    let font = if big { fonts.large } else { fonts.body };
    let _f = ui.push_font(font);
    let h = ui.text_line_height();
    let r = h * 0.40;
    let p = ui.cursor_screen_pos();
    let center = [p[0] + r, p[1] + h / 2.0];
    let pal = self::p();
    let (disc, letter, text) = match (dark, active) {
        (true, true) => (pal.g_disc_sel, pal.g_letter_sel, pal.on_sel),
        (true, false) => (with_alpha(pal.g_disc_sel, 0.6), pal.g_letter_sel, pal.on_sel_soft),
        (false, true) => (pal.g_disc, pal.g_letter, pal.text),
        (false, false) => (with_alpha(pal.g_disc, 0.45), with_alpha(pal.g_letter, 0.8), pal.faint),
    };
    dl.add_circle(center, r, disc).filled(true).build();
    {
        let _g = ui.push_font(fonts.small_bold);
        let t = ui.calc_text_size("G");
        dl.add_text([center[0] - t[0] / 2.0, center[1] - t[1] / 2.0], col(letter), "G");
    }
    drop(dl);
    ui.set_cursor_screen_pos([p[0] + r * 2.0 + 6.0, p[1]]);
    if dark {
        ui.text_colored(text, value);
    } else {
        shadow_text(ui, text, value);
    }
}

/// A points pill ("G 20") ending at `right`, vertically centred on `mid_y`.
pub fn points_pill(ui: &Ui, fonts: &Fonts, right: f32, mid_y: f32, points: u32, unlocked: bool, on_white: bool) {
    let dl = ui.get_window_draw_list();
    let _f = ui.push_font(fonts.small_bold);
    let label = points.to_string();
    let t = ui.calc_text_size(&label);
    let h = 24.0;
    let r = 7.0;
    let w = t[0] + r * 2.0 + 24.0;
    let min = [right - w, mid_y - h / 2.0];
    let max = [right, mid_y + h / 2.0];
    let pal = p();
    let (fill, disc, letter, text) = match (on_white, unlocked) {
        (true, _) => (pal.pill_sel, pal.g_disc_sel, pal.g_letter_sel, pal.pill_sel_text),
        (false, true) => (pal.pill, pal.g_disc, pal.g_letter, pal.text),
        (false, false) => (with_alpha(pal.pill, 0.6), with_alpha(pal.g_disc, 0.5), pal.g_letter, pal.faint),
    };
    let round = if pal.radius > 0.0 { h / 2.0 } else { 0.0 };
    dl.add_rect(min, max, fill).filled(true).rounding(round).build();
    let c = [min[0] + 6.0 + r, mid_y];
    dl.add_circle(c, r, disc).filled(true).build();
    let g = ui.calc_text_size("G");
    dl.add_text([c[0] - g[0] / 2.0, c[1] - g[1] / 2.0], col(letter), "G");
    dl.add_text([c[0] + r + 5.0, mid_y - t[1] / 2.0], col(text), &label);
}

/// Controller button glyph (A/B/X/Y discs, LB/RB pills) and its label in
/// white. Advances the cursor on the same line.
pub fn button_hint(ui: &Ui, fonts: &Fonts, button: &str, label: &str) {
    let dl = ui.get_window_draw_list();
    let _f = ui.push_font(fonts.small_bold);
    let h = ui.text_line_height() + 6.0;
    let p = ui.cursor_screen_pos();
    let (color, w) = match button {
        "A" => ([0.38, 0.69, 0.20, 1.0], h),
        "B" => ([0.85, 0.24, 0.20, 1.0], h),
        "X" => ([0.20, 0.47, 0.85, 1.0], h),
        "Y" => ([0.92, 0.72, 0.12, 1.0], h),
        _ => ([0.22, 0.25, 0.28, 1.0], ui.calc_text_size(button)[0] + 14.0),
    };
    let max = [p[0] + w, p[1] + h];
    // Glossy glyph: shadow, colour, highlight, rim.
    if w == h {
        let c = [p[0] + h / 2.0, p[1] + h / 2.0];
        dl.add_circle([c[0], c[1] + 1.5], h / 2.0, [0.0, 0.0, 0.0, 0.3]).filled(true).build();
        dl.add_circle(c, h / 2.0, color).filled(true).build();
        dl.add_circle([c[0], c[1] - h * 0.18], h * 0.30, [1.0, 1.0, 1.0, 0.25]).filled(true).build();
        dl.add_circle(c, h / 2.0, [1.0, 1.0, 1.0, 0.7]).thickness(1.2).build();
    } else {
        dl.add_rect([p[0], p[1] + 1.5], [max[0], max[1] + 1.5], [0.0, 0.0, 0.0, 0.3]).filled(true).rounding(h / 2.0).build();
        dl.add_rect(p, max, color).filled(true).rounding(h / 2.0).build();
        dl.add_rect(p, max, [1.0, 1.0, 1.0, 0.6]).rounding(h / 2.0).thickness(1.2).build();
    }
    centered_text(ui, &dl, p, max, WHITE, button);
    drop(_f);
    drop(dl);
    let _f = ui.push_font(fonts.body);
    let t = ui.calc_text_size(label);
    ui.set_cursor_screen_pos([max[0] + 7.0, p[1] + (h - t[1]) / 2.0]);
    let pal = self::p();
    if pal.hint_shadow > 0.0 {
        let q = ui.cursor_screen_pos();
        ui.get_window_draw_list().add_text([q[0] + 1.0, q[1] + 2.0], col([0.0, 0.08, 0.0, pal.hint_shadow]), label);
    }
    ui.text_colored(pal.hint_text, label);
    ui.set_cursor_screen_pos([max[0] + 7.0 + t[0] + if label.is_empty() { 4.0 } else { 22.0 }, p[1]]);
}

/// Width `button_hint` will take (to right-align a group of hints).
pub fn button_hint_width(ui: &Ui, fonts: &Fonts, button: &str, label: &str) -> f32 {
    let w = {
        let _f = ui.push_font(fonts.small_bold);
        let h = ui.text_line_height() + 6.0;
        if button.len() == 1 { h } else { ui.calc_text_size(button)[0] + 14.0 }
    };
    let _f = ui.push_font(fonts.body);
    w + 7.0 + ui.calc_text_size(label)[0] + if label.is_empty() { 4.0 } else { 22.0 }
}

/// Achievement icon: captured art if available (greyed when locked),
/// otherwise the game's icon. Drawn at `size * scale` around the centre of
/// a `size` slot; advances the cursor past the slot.
#[allow(clippy::too_many_arguments)]
pub fn icon(ui: &Ui, media: &Media, game: &str, id: &str, category: &str, unlocked: bool, hidden: bool, platinum: bool, size: f32, scale: f32) {
    let slot = ui.cursor_screen_pos();
    let grow = size * (scale - 1.0) / 2.0;
    let pos = [slot[0] - grow, slot[1] - grow];
    let size_drawn = size * scale;
    let max = [pos[0] + size_drawn, pos[1] + size_drawn];
    let dl = ui.get_window_draw_list();
    let kind = if unlocked { Kind::Icon } else { Kind::IconLocked };
    let art = if hidden { None } else { media.get(kind, game, id) };
    // No picture yet: the game's icon stands in (greyed while locked).
    let placeholder = media.get(if unlocked { Kind::GameIcon } else { Kind::GameIconLocked }, game, "");

    if p().shadow > 0.0 {
        dl.add_rect([pos[0] + 1.0, pos[1] + 2.0], [max[0] + 1.0, max[1] + 2.0], [0.0, 0.0, 0.0, 0.25]).filled(true).build();
    }
    if let Some((tex, _)) = art {
        dl.add_image(tex, pos, max).build();
    } else if let Some((tex, _)) = placeholder {
        dl.add_image(tex, pos, max).build();
        if hidden {
            dl.add_rect(pos, max, [0.0, 0.0, 0.0, 0.45]).filled(true).build();
            padlock(&dl, pos, max, [0.85, 0.88, 0.92, 0.9]);
        }
    } else {
        let base = if hidden { [0.20, 0.23, 0.27, 1.0] } else { category_color(category) };
        emblem(&dl, pos, max, base, unlocked);
        if hidden {
            padlock(&dl, pos, max, [0.85, 0.88, 0.92, 0.9]);
        }
    }
    let edge = if platinum && unlocked {
        PLATINUM
    } else if unlocked {
        p().icon_edge
    } else {
        with_alpha(p().icon_edge, 0.35)
    };
    if edge[3] > 0.0 {
        dl.add_rect(pos, max, edge).thickness(if unlocked { 2.0 } else { 1.0 }).build();
    }
    drop(dl);
    ui.set_cursor_screen_pos(slot);
    ui.dummy([size, size]);
}

/// A game's icon (or a medal tile) at `min`, `size` square, with a white
/// frame like the 360's game tiles.
pub fn game_icon(dl: &DrawListMut, media: &Media, game_id: &str, min: [f32; 2], size: f32) {
    let max = [min[0] + size, min[1] + size];
    if p().shadow > 0.0 {
        dl.add_rect([min[0] + 1.0, min[1] + 2.0], [max[0] + 1.0, max[1] + 2.0], [0.0, 0.0, 0.0, 0.25]).filled(true).rounding(size * 0.08).build();
    }
    match media.get(Kind::GameIcon, game_id, "") {
        Some((tex, _)) => {
            dl.add_image_rounded(tex, min, max, size * 0.08).build();
        }
        None => emblem(dl, min, max, [0.30, 0.38, 0.48, 1.0], true),
    }
    if p().frame[3] > 0.0 {
        dl.add_rect(min, max, p().frame).rounding(size * 0.08).thickness(1.5).build();
    }
}

/// Progress bar drawn at the cursor: white fill in a dark groove, with a
/// light sweeping across the filled part; advances the cursor.
pub fn progress(ui: &Ui, frac: f32, width: f32, height: f32) {
    let dl = ui.get_window_draw_list();
    let p = ui.cursor_screen_pos();
    let pal = self::p();
    let r = if pal.radius > 0.0 { height / 2.0 } else { 0.0 };
    dl.add_rect(p, [p[0] + width, p[1] + height], pal.track).filled(true).rounding(r).build();
    let fill = width * frac.clamp(0.0, 1.0);
    if fill > 0.0 {
        let end = [p[0] + fill.max(height), p[1] + height];
        dl.add_rect(p, end, pal.fill_top).filled(true).rounding(r).build();
        dl.add_rect([p[0], p[1] + height / 2.0], end, pal.fill_bottom).filled(true).rounding(r).build();
        // Shine: a soft band travelling left to right every ~2.5 s.
        let band = (width * 0.18).max(30.0);
        let t = (ui.time() as f32 / 2.5).fract();
        let x = p[0] - band + t * (fill + band * 2.0);
        let clear = col(with_alpha(pal.shine, 0.0));
        let glow = col(pal.shine);
        dl.with_clip_rect_intersect(p, end, || {
            dl.add_rect_filled_multicolor([x, p[1]], [x + band / 2.0, end[1]], clear, glow, glow, clear);
            dl.add_rect_filled_multicolor([x + band / 2.0, p[1]], [x + band, end[1]], glow, clear, clear, glow);
        });
    }
    drop(dl);
    ui.dummy([width, height]);
}

/// Shortens `text` with "..." so it fits in `max_w` pixels.
pub fn fit(ui: &Ui, text: &str, max_w: f32) -> String {
    if ui.calc_text_size(text)[0] <= max_w {
        return text.to_string();
    }
    let mut s = text.to_string();
    while !s.is_empty() && ui.calc_text_size(format!("{s}..."))[0] > max_w {
        s.pop();
    }
    format!("{}...", s.trim_end())
}

/// Corner rounding for a pill of height `h` (square in flat designs).
pub fn pill_round(h: f32) -> f32 {
    if p().radius > 0.0 { h / 2.0 } else { 0.0 }
}

/// A slanted tab (parallelogram) with a glossy top half (Classic).
pub fn blade_shape(dl: &DrawListMut, min: [f32; 2], max: [f32; 2], skew: f32, fill: [f32; 4], gloss: f32) {
    let pts = vec![[min[0] + skew, min[1]], [max[0], min[1]], [max[0] - skew, max[1]], [min[0], max[1]]];
    dl.add_polyline(pts, fill).filled(true).build();
    if gloss > 0.0 {
        let mid = (min[1] + max[1]) / 2.0;
        let k = skew / 2.0;
        let pts = vec![[min[0] + skew, min[1]], [max[0], min[1]], [max[0] - k, mid], [min[0] + k, mid]];
        dl.add_polyline(pts, [1.0, 1.0, 1.0, gloss]).filled(true).build();
    }
}

/// One horizontal slice of a shape: at height `y` it spans `xl`..`xr`, with
/// colours `cl` and `cr` at the two ends.
#[derive(Clone, Copy)]
pub struct Slice {
    pub y: f32,
    pub xl: f32,
    pub xr: f32,
    pub cl: [f32; 4],
    pub cr: [f32; 4],
}

/// Fills the shape traced by `slices` (top to bottom) as one seamless mesh,
/// so it can be curved, notched and shaded both ways, which imgui's convex
/// fills can't do. Edges are smoothed with a thin outline.
pub fn fill_slices(ui: &Ui, slices: &[Slice]) {
    let n = slices.len();
    if n < 2 {
        return;
    }
    // SAFETY: the window draw list is valid while the window is built; we
    // reserve exactly the vertices and indices we then write.
    unsafe {
        let dl = sys::igGetWindowDrawList();
        let mut uv = sys::ImVec2 { x: 0.0, y: 0.0 };
        sys::igGetFontTexUvWhitePixel(&mut uv);
        sys::ImDrawList_PrimReserve(dl, ((n - 1) * 6) as i32, (n * 2) as i32);
        let base = (*dl)._VtxCurrentIdx;
        for s in slices {
            sys::ImDrawList_PrimWriteVtx(dl, sys::ImVec2 { x: s.xl, y: s.y }, uv, col(s.cl).to_bits());
            sys::ImDrawList_PrimWriteVtx(dl, sys::ImVec2 { x: s.xr, y: s.y }, uv, col(s.cr).to_bits());
        }
        for i in 0..(n as u32 - 1) {
            let a = base + i * 2;
            for k in [a, a + 1, a + 3, a, a + 3, a + 2] {
                sys::ImDrawList_PrimWriteIdx(dl, k as sys::ImDrawIdx);
            }
        }
    }
    let dl = ui.get_window_draw_list();
    let mid = &slices[n / 2];
    let left: Vec<[f32; 2]> = slices.iter().map(|s| [s.xl, s.y]).collect();
    let right: Vec<[f32; 2]> = slices.iter().map(|s| [s.xr, s.y]).collect();
    dl.add_polyline(left, with_alpha(mid.cl, 0.6)).thickness(1.0).build();
    dl.add_polyline(right, with_alpha(mid.cr, 0.6)).thickness(1.0).build();
}

/// A soft elliptical glow: `color` at the centre fading to nothing.
pub fn radial_glow(center: [f32; 2], radius: [f32; 2], color: [f32; 4]) {
    const SEGS: u32 = 40;
    // SAFETY: as in `fill_slices`.
    unsafe {
        let dl = sys::igGetWindowDrawList();
        let mut uv = sys::ImVec2 { x: 0.0, y: 0.0 };
        sys::igGetFontTexUvWhitePixel(&mut uv);
        sys::ImDrawList_PrimReserve(dl, (SEGS * 3) as i32, (SEGS + 1) as i32);
        let base = (*dl)._VtxCurrentIdx;
        sys::ImDrawList_PrimWriteVtx(dl, sys::ImVec2 { x: center[0], y: center[1] }, uv, col(color).to_bits());
        let clear = col(with_alpha(color, 0.0)).to_bits();
        for i in 0..SEGS {
            let a = i as f32 / SEGS as f32 * std::f32::consts::TAU;
            sys::ImDrawList_PrimWriteVtx(dl, sys::ImVec2 { x: center[0] + radius[0] * a.cos(), y: center[1] + radius[1] * a.sin() }, uv, clear);
        }
        for i in 0..SEGS {
            for k in [base, base + 1 + i, base + 1 + (i + 1) % SEGS] {
                sys::ImDrawList_PrimWriteIdx(dl, k as sys::ImDrawIdx);
            }
        }
    }
}

/// 0 at `a`, 1 at `b`, smooth in between.
pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Runs `draw` with the current window's clip rect narrowed to `min..max`
/// (without holding a draw list, so helpers that take one can be used).
pub fn with_clip<R>(min: [f32; 2], max: [f32; 2], draw: impl FnOnce() -> R) -> R {
    // SAFETY: push/pop on the current window's draw list, balanced.
    unsafe {
        let dl = sys::igGetWindowDrawList();
        sys::ImDrawList_PushClipRect(dl, sys::ImVec2 { x: min[0], y: min[1] }, sys::ImVec2 { x: max[0], y: max[1] }, true);
        let r = draw();
        sys::ImDrawList_PopClipRect(sys::igGetWindowDrawList());
        r
    }
}

/// A Blades-style controller glyph centred on `center`: coloured and glossy
/// when it does something, a faint grey ring otherwise. The label goes to
/// the right, or to the left when `label_left`.
pub fn glyph(ui: &Ui, fonts: &Fonts, center: [f32; 2], button: &str, lit: bool, label: Option<&str>, label_left: bool) {
    let r = 12.0;
    let color = match button {
        "A" => [0.38, 0.69, 0.20, 1.0],
        "B" => [0.85, 0.24, 0.20, 1.0],
        "X" => [0.20, 0.47, 0.85, 1.0],
        _ => [0.92, 0.72, 0.12, 1.0],
    };
    let art = themed().and_then(|t| t.glyphs[match button {
        "A" => 0,
        "B" => 1,
        "X" => 2,
        _ => 3,
    }]);
    if let Some(tex) = art {
        let g = r * 1.25;
        ui.get_window_draw_list()
            .add_image(tex, [center[0] - g, center[1] - g], [center[0] + g, center[1] + g])
            .col(if lit { [1.0, 1.0, 1.0, 1.0] } else { [0.75, 0.75, 0.75, 0.35] })
            .build();
    } else {
        let dl = ui.get_window_draw_list();
        if lit {
            dl.add_circle([center[0], center[1] + 1.5], r, [0.0, 0.0, 0.0, 0.3]).filled(true).build();
            dl.add_circle(center, r, color).filled(true).build();
            dl.add_circle([center[0], center[1] - r * 0.35], r * 0.6, [1.0, 1.0, 1.0, 0.22]).filled(true).build();
            dl.add_circle(center, r, [1.0, 1.0, 1.0, 0.75]).thickness(1.2).build();
        } else {
            dl.add_circle(center, r, [1.0, 1.0, 1.0, 0.18]).filled(true).build();
            dl.add_circle(center, r, [1.0, 1.0, 1.0, 0.35]).thickness(1.2).build();
        }
        let _f = ui.push_font(fonts.small_bold);
        let ink = if lit { WHITE } else { [1.0, 1.0, 1.0, 0.55] };
        centered_text(ui, &dl, [center[0] - r, center[1] - r], [center[0] + r, center[1] + r], ink, button);
    }
    if let Some(label) = label {
        let pal = p();
        let _f = ui.push_font(fonts.body);
        let t = ui.calc_text_size(label);
        let x = if label_left { center[0] - r - 8.0 - t[0] } else { center[0] + r + 8.0 };
        let pos = [x, center[1] - t[1] / 2.0];
        let dl = ui.get_window_draw_list();
        if pal.hint_shadow > 0.0 {
            dl.add_text([pos[0] + 1.0, pos[1] + 2.0], col([0.0, 0.08, 0.0, pal.hint_shadow]), label);
        }
        dl.add_text(pos, col(with_alpha(pal.hint_text, if lit { 1.0 } else { 0.6 })), label);
    }
}
