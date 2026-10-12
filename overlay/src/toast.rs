//! Xbox 360 style pop-ups at the bottom centre, shown one at a time.

use crate::games::Game;
use crate::media::Media;
use crate::ui::anim::ease_out_back;
use crate::ui::theme::*;
use hudhook::imgui::{Condition, WindowFlags};
use crate::Fonts;
use hudhook::imgui::Ui;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Pop-ups at the top of the screen instead of the bottom (Settings).
pub static POPUPS_TOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const SHOW_FOR: Duration = Duration::from_secs(6);
const FADE: f32 = 0.35; // seconds of fade in / out
const HEIGHT: f32 = 76.0;
const MARGIN: f32 = 56.0;

enum Content {
    /// Index into the game's achievements.
    Unlock(usize),
    /// `icon`: a game id whose icon to show.
    Info { title: String, body: String, icon: Option<String> },
}

struct Toast {
    content: Content,
    shown_at: Option<Instant>,
}

#[derive(Default)]
pub struct Toasts {
    queue: VecDeque<Toast>,
}

impl Toasts {
    pub fn push_unlock(&mut self, index: usize) {
        self.queue.push_back(Toast { content: Content::Unlock(index), shown_at: None });
    }

    pub fn push_info(&mut self, title: &str, body: &str, icon: Option<&str>) {
        self.queue.push_back(Toast {
            content: Content::Info { title: title.into(), body: body.into(), icon: icon.map(String::from) },
            shown_at: None,
        });
    }

    pub fn draw(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: Option<&Game>) {
        let Some(toast) = self.queue.front_mut() else { return };
        let shown_at = *toast.shown_at.get_or_insert_with(|| {
            if matches!(toast.content, Content::Unlock(_)) {
                crate::sound::play(crate::sound::Sfx::Unlock);
            }
            Instant::now()
        });
        let t = shown_at.elapsed();
        if t >= SHOW_FOR {
            self.queue.pop_front();
            return;
        }
        let secs = t.as_secs_f32();
        let left = (SHOW_FOR - t).as_secs_f32();
        let alpha = (secs / FADE).min(left / FADE).min(1.0);
        let rise = (1.0 - (secs / FADE).min(1.0)) * 18.0;

        let achievement = match toast.content {
            Content::Unlock(i) => game.and_then(|g| g.achievements().get(i)),
            Content::Info { .. } => None,
        };
        let platinum = achievement.is_some_and(|a| a.platinum);
        let accent = if platinum { PLATINUM } else { GREEN };

        // Text first, to size the bar.
        let (line1, line2) = match (&toast.content, achievement) {
            (Content::Unlock(_), Some(a)) => (
                if platinum { "Platinum unlocked".to_string() } else { "Achievement unlocked".to_string() },
                format!("{} - {}", a.points, a.name),
            ),
            (Content::Info { title, body, .. }, _) => (title.clone(), body.clone()),
            _ => return,
        };
        let w2 = {
            let _f = ui.push_font(fonts.title);
            ui.calc_text_size(&line2)[0]
        };
        let info_icon = match &toast.content {
            Content::Info { icon: Some(id), .. } => media.get(crate::media::Kind::GameIcon, id, ""),
            _ => None,
        };
        let icon_w = if achievement.is_some() || info_icon.is_some() { HEIGHT } else { 18.0 };
        let gs = if achievement.is_some() { 26.0 } else { 0.0 };
        let width = (icon_w + 18.0 + gs + w2 + 28.0).max(340.0);

        // Laid out at the panel's layout size in its own click-through
        // window, then scaled to the screen like the panel.
        let scale = font_scale();
        let [dw, dh] = ui.io().display_size;
        let (dw, dh) = (dw / scale, dh / scale);
        ui.window("##ah_toast")
            .position([0.0, 0.0], Condition::Always)
            .size([dw, dh], Condition::Always)
            .flags(
                WindowFlags::NO_DECORATION
                    | WindowFlags::NO_INPUTS
                    | WindowFlags::NO_BACKGROUND
                    | WindowFlags::NO_SAVED_SETTINGS
                    | WindowFlags::NO_FOCUS_ON_APPEARING
                    | WindowFlags::NO_NAV,
            )
            .build(|| {
                panel_window(ui);
            let min = if POPUPS_TOP.load(std::sync::atomic::Ordering::Relaxed) {
                [(dw - width) / 2.0, MARGIN - rise]
            } else {
                [(dw - width) / 2.0, dh - MARGIN - HEIGHT + rise]
            };
            let max = [min[0] + width, min[1] + HEIGHT];
            let dl = ui.get_window_draw_list();
            let a = |c: [f32; 4]| with_alpha(c, alpha);

            // Pill body with a glossy top and accent edge.
            let r = HEIGHT / 2.0;
            dl.add_rect(min, max, a([0.05, 0.07, 0.09, 0.94])).filled(true).rounding(r).build();
            dl.add_rect(min, [max[0], min[1] + HEIGHT * 0.5], a([1.0, 1.0, 1.0, 0.05])).filled(true).rounding(r).build();
            dl.add_rect(min, max, a(accent)).rounding(r).thickness(2.0).build();

            // A shine sweeps across the bar shortly after it appears.
            let sweep = ((secs - 0.25) / 0.9).clamp(0.0, 1.0);
            if sweep > 0.0 && sweep < 1.0 {
                let band = 90.0;
                let sx = min[0] - band + sweep * (width + band * 2.0);
                let clear = col([1.0, 1.0, 1.0, 0.0]);
                let glow = col(a([1.0, 1.0, 1.0, 0.18]));
                dl.with_clip_rect_intersect([min[0] + r * 0.4, min[1]], [max[0] - r * 0.4, max[1]], || {
                    dl.add_rect_filled_multicolor([sx, min[1]], [sx + band / 2.0, max[1]], clear, glow, glow, clear);
                    dl.add_rect_filled_multicolor([sx + band / 2.0, min[1]], [sx + band, max[1]], glow, clear, clear, glow);
                });
            }

            let mut x = min[0] + 12.0;
            if let Some(ach) = achievement {
                // Round icon on the left, like the 360's achievement orb; it pops
                // in with a little overshoot.
                let full = HEIGHT - 20.0;
                let pop = ease_out_back((secs / 0.45).min(1.0)).max(0.0);
                let s = full * (0.55 + 0.45 * pop);
                let ip = [x + (full - s) / 2.0, min[1] + 10.0 + (full - s) / 2.0];
                let game_id = game.map_or("", |g| g.id());
                let art = media
                    .get(crate::media::Kind::Icon, game_id, &ach.id)
                    .or_else(|| media.get(crate::media::Kind::GameIcon, game_id, ""));
                match art {
                    Some((tex, _)) => {
                        dl.add_image_rounded(tex, ip, [ip[0] + s, ip[1] + s], s / 2.0).col(a(WHITE)).build();
                    }
                    None => {
                        let c = category_color(&ach.category);
                        dl.add_circle([ip[0] + s / 2.0, ip[1] + s / 2.0], s / 2.0, a(c)).filled(true).build();
                        emblem(&dl, [ip[0] + s * 0.15, ip[1] + s * 0.15], [ip[0] + s * 0.85, ip[1] + s * 0.85], c, true);
                    }
                }
                dl.add_circle([ip[0] + s / 2.0, ip[1] + s / 2.0], s / 2.0, a(accent)).thickness(2.0).build();
                x += full + 16.0;
            } else if let Some((tex, _)) = info_icon {
                // Game icon, popping in like the achievement orb.
                let full = HEIGHT - 20.0;
                let pop = ease_out_back((secs / 0.45).min(1.0)).max(0.0);
                let s = full * (0.55 + 0.45 * pop);
                let ip = [x + (full - s) / 2.0, min[1] + 10.0 + (full - s) / 2.0];
                dl.add_image_rounded(tex, ip, [ip[0] + s, ip[1] + s], s / 2.0).col(a(WHITE)).build();
                dl.add_circle([ip[0] + s / 2.0, ip[1] + s / 2.0], s / 2.0, a(accent)).thickness(2.0).build();
                x += full + 16.0;
            } else {
                x += 14.0;
            }

            {
                let _f = ui.push_font(fonts.body);
                dl.add_text([x, min[1] + 13.0], col(a(accent)), &line1);
            }
            let y2 = min[1] + 37.0;
            let mut tx = x;
            if achievement.is_some() {
                // Gamerscore disc before the points.
                let rr = 9.0;
                dl.add_circle([tx + rr, y2 + 12.0], rr, a(GREEN)).filled(true).build();
                let _f = ui.push_font(fonts.small_bold);
                centered_text(ui, &dl, [tx, y2 + 3.0], [tx + rr * 2.0, y2 + 21.0], a(INK), "G");
                tx += gs;
            }
            let _f = ui.push_font(fonts.title);
            dl.add_text([tx, y2], col(a(WHITE)), &line2);

            });
        scale_panel([0.0, 0.0], scale);
    }
}
