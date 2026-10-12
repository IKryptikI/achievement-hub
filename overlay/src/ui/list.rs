//! A game's achievement list, 360 style: filter pills, a list of rows with
//! a white highlight bar on the selection, and an info pane on the right.
//! Used for the game being played and for any game opened from the Games
//! blade; each use keeps its own state.

use super::anim::{ease_out_back, ease_out_cubic, lerp, lerp_color, Anim};
use super::empty;
use super::theme::*;
use super::theme::p as pal;
use crate::media::{Kind, Media};
use crate::profile::{format_ago, format_date, GameSummary, Profile};
use crate::Fonts;
use std::collections::HashMap;
use hudhook::imgui::Ui;

/// Achievement list filter.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Filter {
    #[default]
    All,
    Unlocked,
    Locked,
}

const FILTERS: [(Filter, &str); 3] = [(Filter::All, "all"), (Filter::Unlocked, "unlocked"), (Filter::Locked, "locked")];

/// A click ripple expanding inside a row.
struct Ripple {
    center: [f32; 2],
    row: usize,
    born: f64,
}

const RIPPLE_SECS: f64 = 0.55;

#[derive(Default)]
pub struct AchList {
    pub selected: usize,
    anim: Anim,
    filter: Filter,
    /// Bumped on every filter change so the rows' entrance replays.
    filter_gen: u32,
    /// Achievement indexes shown by the current filter (last frame).
    visible: Vec<usize>,
    /// Selection the detail pane last showed (to animate changes).
    detail_for: Option<usize>,
    ripples: Vec<Ripple>,
    /// Keyboard/controller navigation asked to bring the selection into view.
    scroll_pending: bool,
    scroll_target: Option<f32>,
}

impl AchList {
    /// Fresh state: everything animates in again.
    pub fn reset(&mut self) {
        *self = AchList { selected: self.selected, filter: self.filter, scroll_pending: true, ..AchList::default() };
    }

    /// Moves the selection within the filtered list.
    pub fn navigate(&mut self, delta: i32) {
        if delta == 0 || self.visible.is_empty() {
            return;
        }
        let pos = self.visible.iter().position(|&i| i == self.selected).unwrap_or(0) as i64;
        let next = (pos + delta as i64).clamp(0, self.visible.len() as i64 - 1) as usize;
        self.selected = self.visible[next];
        self.scroll_pending = true;
    }

    pub fn cycle_filter(&mut self) {
        let i = FILTERS.iter().position(|(f, _)| *f == self.filter).unwrap_or(0);
        self.set_filter(FILTERS[(i + 1) % FILTERS.len()].0);
    }

    fn set_filter(&mut self, f: Filter) {
        if self.filter != f {
            self.filter = f;
            self.filter_gen += 1;
            self.scroll_pending = true;
        }
    }

    pub fn draw(&mut self, ui: &Ui, fonts: &Fonts, media: &Media, game: &GameSummary, profile: &Profile, title: &str, counts: Option<&HashMap<String, (i64, i64)>>) {
        let progress_of = |id: &str| counts.and_then(|p| p.get(id)).copied().filter(|(n, t)| *t > 0 && n < t);
        self.anim.begin(ui.io().delta_time);
        let all = &game.achievement;
        let unlocked_at = |id: &str| profile.unlocked_at(&game.id, id);
        let got = all.iter().filter(|a| unlocked_at(&a.id).is_some()).count();
        let pts: u32 = all.iter().filter(|a| unlocked_at(&a.id).is_some()).map(|a| a.points).sum();

        // Header: game title and filter pills; points count up, bar fills.
        let row_y = ui.cursor_screen_pos()[1];
        {
            let _f = ui.push_font(fonts.title);
            shadow_text(ui, pal().text, title);
        }
        self.filter_pills(ui, fonts, [all.len(), got, all.len() - got], row_y);
        {
            let _f = ui.push_font(fonts.body);
            shadow_text(ui, pal().soft, &format!("{got} of {} unlocked", all.len()));
            ui.same_line_with_spacing(0.0, 18.0);
        }
        let shown_pts = self.anim.to("pts", pts as f32, 4.0, 0.0).round() as u32;
        gamerscore(ui, fonts, &format!("{shown_pts} / {}", game.total_points), false, true, false);
        let frac = if all.is_empty() { 0.0 } else { got as f32 / all.len() as f32 };
        progress(ui, self.anim.to("bar", frac, 3.0, 0.0), ui.content_region_avail()[0], 8.0);
        ui.dummy([0.0, 6.0]);

        // Apply the filter; keep the selection inside it.
        let visible: Vec<usize> = (0..all.len())
            .filter(|&i| {
                let u = unlocked_at(&all[i].id).is_some();
                match self.filter {
                    Filter::All => true,
                    Filter::Unlocked => u,
                    Filter::Locked => !u,
                }
            })
            .collect();
        if !visible.contains(&self.selected) {
            if let Some(&first) = visible.first() {
                self.selected = first;
            }
        }
        self.visible = visible.clone();

        let avail = ui.content_region_avail();
        let list_w = avail[0] * 0.56;
        let now = ui.time();
        let generation = self.filter_gen;
        ui.child_window("##ah_list").size([list_w, avail[1]]).build(|| {
            child(|| {
                // Smooth scrolling toward the selection.
                if let Some(target) = self.scroll_target {
                    let cur = ui.scroll_y();
                    let next = lerp(cur, target, 1.0 - (-14.0 * ui.io().delta_time.clamp(0.0, 0.1)).exp());
                    if (next - target).abs() < 0.5 {
                        ui.set_scroll_y(target);
                        self.scroll_target = None;
                    } else {
                        ui.set_scroll_y(next);
                    }
                }

                let row_h = 62.0;
                let win_top = ui.window_pos()[1];
                let win_h = ui.window_size()[1];
                if visible.is_empty() {
                    let msg = if self.filter == Filter::Unlocked { "Nothing unlocked yet." } else { "Everything's unlocked. Nice." };
                    empty(ui, fonts, msg);
                }
                self.ripples.retain(|r| now - r.born < RIPPLE_SECS);
                for (n, &i) in visible.iter().enumerate() {
                    let a = &all[i];
                    let unlocked = unlocked_at(&a.id);
                    let hidden = a.secret && unlocked.is_none();

                    let top = ui.cursor_screen_pos();
                    let width = (ui.content_region_avail()[0] - 4.0).max(1.0);
                    let clicked = ui.invisible_button(format!("##row{i}"), [width, row_h]);
                    let hovered = ui.is_item_hovered();
                    if clicked {
                        self.selected = i;
                        self.ripples.push(Ripple { center: ui.io().mouse_pos, row: i, born: now });
                    }
                    if self.scroll_pending && self.selected == i {
                        let want = ui.scroll_y() + (top[1] + row_h / 2.0) - (win_top + win_h / 2.0);
                        self.scroll_target = Some(want.clamp(0.0, ui.scroll_max_y().max(0.0)));
                        self.scroll_pending = false;
                    }
                    let after = ui.cursor_screen_pos();
                    // Rows outside the view: nothing to draw.
                    if top[1] > win_top + win_h || top[1] + row_h < win_top {
                        ui.set_cursor_screen_pos([after[0], after[1] + 4.0]);
                        continue;
                    }

                    // Entrance: rows slide and fade in, staggered.
                    let enter =
                        ease_out_cubic(((self.anim.timer(("row_in", generation, i), 1.0) - (n.min(12) as f32) * 0.03) / 0.3).clamp(0.0, 1.0));
                    let dx = (1.0 - enter) * 28.0;
                    let hv = self.anim.to(("row_hover", i), hovered as u8 as f32, 16.0, 0.0);
                    let sel = self.anim.to(("row_sel", i), (self.selected == i) as u8 as f32, 14.0, 0.0);
                    let min = [top[0] + dx, top[1]];
                    let max = [top[0] + width, top[1] + row_h];

                    fade(enter, || {
                        {
                            let dl = ui.get_window_draw_list();
                            row_bg(&dl, min, max, 0.6 + 0.4 * (1.0 - sel));
                            hover_wash(&dl, min, max, hv * (1.0 - sel));
                            highlight(&dl, min, max, sel);
                            for r in self.ripples.iter().filter(|r| r.row == i) {
                                let t = ((now - r.born) / RIPPLE_SECS) as f32;
                                let radius = ease_out_cubic(t) * width;
                                dl.with_clip_rect_intersect(min, max, || {
                                    dl.add_circle(r.center, radius, [1.0, 1.0, 1.0, (1.0 - t) * 0.25]).filled(true).build();
                                });
                            }
                        }

                        ui.set_cursor_screen_pos([min[0] + 8.0, top[1] + 7.0]);
                        icon(ui, media, &game.id, &a.id, &a.category, unlocked.is_some(), hidden, a.platinum, 48.0, 1.0 + 0.08 * hv.max(sel));
                        let tx = min[0] + 68.0 + 4.0 * hv;
                        let text_w = width - 68.0 - 84.0;
                        let name = if hidden { "Secret achievement" } else { a.name.as_str() };
                        let line = if hidden { "Keep playing to reveal this one." } else { a.description.as_str() };
                        let counting = if unlocked.is_none() { progress_of(&a.id) } else { None };
                        let (c1, c2) = match unlocked {
                            Some(_) => (lerp_color(pal().text, pal().on_sel, sel), lerp_color(pal().soft, pal().on_sel_soft, sel)),
                            None => (lerp_color(pal().soft, pal().on_sel, sel), lerp_color(pal().faint, pal().on_sel_soft, sel)),
                        };
                        {
                            let _f = ui.push_font(fonts.title);
                            ui.set_cursor_screen_pos([tx, top[1] + 7.0]);
                            ui.text_colored(c1, fit(ui, name, text_w));
                        }
                        {
                            let _f = ui.push_font(fonts.body);
                            // Counting: "54 / 300" at the end of the line and a thin bar under it.
                            let count = counting.map(|(n, t)| format!("{n} / {t}"));
                            let count_w = count.as_ref().map_or(0.0, |c| ui.calc_text_size(c)[0] + 12.0);
                            ui.set_cursor_screen_pos([tx, top[1] + 33.0]);
                            ui.text_colored(c2, fit(ui, line, text_w - count_w));
                            if let (Some(c), Some((n, t))) = (count, counting) {
                                let dl = ui.get_window_draw_list();
                                dl.add_text([tx + text_w - count_w + 12.0, top[1] + 33.0], col(c1), &c);
                                let (b0, b1) = ([tx, top[1] + row_h - 7.0], [tx + text_w, top[1] + row_h - 4.0]);
                                dl.add_rect(b0, b1, with_alpha(pal().text, 0.15)).filled(true).rounding(1.5).build();
                                dl.add_rect(b0, [b0[0] + (b1[0] - b0[0]) * n as f32 / t as f32, b1[1]], pal().accent).filled(true).rounding(1.5).build();
                            }
                        }
                        points_pill(ui, fonts, max[0] - 10.0, top[1] + row_h / 2.0, a.points, unlocked.is_some(), sel > 0.5);
                    });
                    ui.set_cursor_screen_pos([after[0], after[1] + 4.0]);
                }
            })
        });

        ui.same_line_with_spacing(0.0, 16.0);
        // Info pane: slides and fades in when the selection changes; the big
        // icon pops.
        if self.detail_for != Some(self.selected) {
            self.detail_for = Some(self.selected);
            self.anim.set("detail_t", 0.0);
            self.anim.set("detail_pop", 0.0);
        }
        let dt = ease_out_cubic(self.anim.timer("detail_t", 0.25));
        let pop = ease_out_back(self.anim.timer("detail_pop", 0.4));
        let dpos = ui.cursor_screen_pos();
        ui.child_window("##ah_detail").size([0.0, avail[1]]).build(|| {
            child(|| {
                if visible.is_empty() {
                    return;
                }
                let Some(a) = all.get(self.selected) else { return };
                let when = unlocked_at(&a.id);
                let hidden = a.secret && when.is_none();
                let full = ui.content_region_avail();
                let p = [dpos[0], dpos[1]];
                {
                    let dl = ui.get_window_draw_list();
                    card_box(&dl, p, [p[0] + full[0], p[1] + full[1]]);
                }
                // Themed: whether it's unlocked goes in the gold bar.
                let status = match when {
                    Some(t) => format!("Unlocked {}", format_date(t)),
                    None => "Locked".to_string(),
                };
                let bar = card_title(ui, fonts, p, full[0], &status);
                let p = [p[0], p[1] + bar];
                let y_off = (1.0 - dt) * 14.0;
                fade(dt, || {
                    ui.set_cursor_screen_pos([p[0] + 16.0, p[1] + 16.0 + y_off]);
                    icon(ui, media, &game.id, &a.id, &a.category, when.is_some(), hidden, a.platinum, 104.0, 0.85 + 0.15 * pop);
                    ui.same_line_with_spacing(0.0, 16.0);
                    ui.group(|| {
                        if bar == 0.0 {
                            let _f = ui.push_font(fonts.small_bold);
                            match when {
                                Some(_) => shadow_text(ui, pal().text, "UNLOCKED"),
                                None => shadow_text(ui, pal().faint, "LOCKED"),
                            }
                        }
                        {
                            let _f = ui.push_font(fonts.large);
                            // Wrap inside the card, not the window.
                            let _w = ui.push_text_wrap_pos_with_pos(p[0] + full[0] - 16.0 - ui.window_pos()[0]);
                            ui.text_colored(pal().text, if hidden { "Secret achievement" } else { a.name.as_str() });
                        }
                        gamerscore(ui, fonts, &a.points.to_string(), false, when.is_some(), false);
                    });

                    ui.set_cursor_screen_pos([p[0] + 16.0, ui.cursor_screen_pos()[1] + 10.0]);
                    let _f = ui.push_font(fonts.body);
                    let _w = ui.push_text_wrap_pos_with_pos(full[0] - 16.0);
                    ui.text_colored(pal().text, if hidden { "This is a secret achievement. Keep playing to reveal it." } else { a.description.as_str() });
                    ui.dummy([0.0, 4.0]);
                    ui.set_cursor_screen_pos([p[0] + 16.0, ui.cursor_screen_pos()[1]]);
                    match when {
                        Some(t) => ui.text_colored(pal().soft, format!("Unlocked {}  ·  {}", format_date(t), format_ago(t))),
                        None => ui.text_colored(pal().faint, "Locked"),
                    }
                    if let Some((n, t)) = when.is_none().then(|| progress_of(&a.id)).flatten() {
                        // Counting: how far along, with a bar.
                        ui.set_cursor_screen_pos([p[0] + 16.0, ui.cursor_screen_pos()[1] + 4.0]);
                        ui.text_colored(pal().text, format!("Progress: {n} / {t}  ({}%)", n * 100 / t));
                        let y = ui.cursor_screen_pos()[1] + 2.0;
                        let (b0, b1) = ([p[0] + 16.0, y], [p[0] + full[0] - 16.0, y + 6.0]);
                        let dl = ui.get_window_draw_list();
                        dl.add_rect(b0, b1, with_alpha(pal().text, 0.15)).filled(true).rounding(3.0).build();
                        dl.add_rect(b0, [b0[0] + (b1[0] - b0[0]) * n as f32 / t as f32, b1[1]], pal().accent).filled(true).rounding(3.0).build();
                        ui.dummy([0.0, 12.0]);
                    }
                    let learned = crate::recorder::status(&game.id, &a.id);
                    let recording = crate::recorder::watching(&game.id);
                    let note = match learned {
                        Some((r, 1)) => Some(format!("Mapped with record mode ({r} recording{}).", if r == 1 { "" } else { "s" })),
                        Some((r, left)) => Some(format!(
                            "Learned from {r} recording{}, {left} possibilities left.{}",
                            if r == 1 { "" } else { "s" },
                            if recording { " Earn it again and press A to narrow it down." } else { "" }
                        )),
                        None if !a.trackable && recording && when.is_none() => Some("Not mapped yet. Press A right after you earn this to record it.".into()),
                        None if !a.trackable => Some("Not tracked yet. This one is still being mapped out.".into()),
                        None => None,
                    };
                    if let Some(note) = note {
                        ui.set_cursor_screen_pos([p[0] + 16.0, ui.cursor_screen_pos()[1]]);
                        ui.text_colored(pal().faint, note);
                    }

                    if let (Some(_), Some((tex, [iw, ih]))) = (when, media.get(Kind::Shot, &game.id, &a.id)) {
                        ui.dummy([0.0, 8.0]);
                        let w = full[0] - 32.0;
                        let h = (w * ih / iw.max(1.0)).min(p[1] + full[1] - ui.cursor_screen_pos()[1] - 56.0).max(0.0);
                        let w = if iw > 0.0 { w.min(h * iw / ih.max(1.0)) } else { w };
                        let pos = [p[0] + 16.0, ui.cursor_screen_pos()[1]];
                        {
                            let dl = ui.get_window_draw_list();
                            dl.add_rect([pos[0] - 2.0, pos[1] - 2.0], [pos[0] + w + 2.0, pos[1] + h + 2.0], WHITE).filled(true).build();
                            dl.add_image(tex, pos, [pos[0] + w, pos[1] + h]).build();
                        }
                        ui.set_cursor_screen_pos([pos[0], pos[1] + h + 6.0]);
                        ui.text_colored(pal().faint, "Captured at unlock");
                    }
                });
            })
        });
    }

    /// all / unlocked / locked pills, right-aligned on the header row, with
    /// a white pill sliding to the active one. Leaves the cursor on the next
    /// line.
    fn filter_pills(&mut self, ui: &Ui, fonts: &Fonts, counts: [usize; 3], row_y: f32) {
        let _f = ui.push_font(fonts.small_bold);
        let labels: Vec<String> = FILTERS.iter().zip(counts).map(|((_, l), c)| format!("{l}  {c}")).collect();
        let h = 26.0;
        let gap = 6.0;
        let widths: Vec<f32> = labels.iter().map(|l| ui.calc_text_size(l)[0] + 26.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + gap * (widths.len() as f32 - 1.0);
        let right = ui.window_pos()[0] + ui.content_region_max()[0];
        let next_line = ui.cursor_screen_pos();
        let mut x = right - total;
        let mut rects = Vec::new();
        for (i, w) in widths.iter().enumerate() {
            let min = [x, row_y];
            ui.set_cursor_screen_pos(min);
            if ui.invisible_button(format!("##filter{i}"), [*w, h]) {
                self.set_filter(FILTERS[i].0);
            }
            rects.push((min, [x + w, row_y + h], ui.is_item_hovered()));
            x += w + gap;
        }
        let dl = ui.get_window_draw_list();
        let active = FILTERS.iter().position(|(f, _)| *f == self.filter).unwrap_or(0);
        for (i, (min, max, hovered)) in rects.iter().enumerate() {
            let hv = self.anim.to(("filter_hover", i), *hovered as u8 as f32, 16.0, 0.0);
            dl.add_rect(*min, *max, lerp_color(pal().card, pal().hover, hv)).filled(true).rounding(pill_round(h)).build();
        }
        let (amin, amax, _) = rects[active];
        let ax = self.anim.to("filter_x", amin[0], 16.0, amin[0]);
        let aw = self.anim.to("filter_w", amax[0] - amin[0], 16.0, amax[0] - amin[0]);
        dl.add_rect([ax, row_y], [ax + aw, row_y + h], pal().sel_bottom).filled(true).rounding(pill_round(h)).build();
        dl.add_rect([ax + 1.0, row_y + 1.0], [ax + aw - 1.0, row_y + h / 2.0], pal().sel_top).filled(true).rounding(pill_round(h)).build();
        for (i, (min, max, _)) in rects.iter().enumerate() {
            let on = self.anim.to(("filter_on", i), (i == active) as u8 as f32, 16.0, (i == active) as u8 as f32);
            centered_text(ui, &dl, *min, *max, lerp_color(pal().text, pal().on_sel, on), &labels[i]);
        }
        drop(dl);
        ui.set_cursor_screen_pos(next_line);
    }
}
