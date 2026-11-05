//! The Achievements tab of the game page: totals, a filter and the list. The data comes from `crate::achievements` (the game's `achievements.json`,
//! re-read when its modification time changes). The row icons are the bundled pictures (ach_icons.rs); the grade cups of the header card are drawn with the painter.
use super::state::App;
use super::theme;
use crate::achievements::{self, Achievement, Achievements, Filter, Grade, Load};
use egui::{pos2, Color32, CornerRadius, Painter, Rect, RichText, ScrollArea, Shape, Stroke, Ui, Vec2};
use std::time::Instant;

pub fn grade_color(g: Grade) -> Color32 {
    match g {
        Grade::Bronze => Color32::from_rgb(0xCD, 0x7F, 0x32),
        Grade::Silver => Color32::from_rgb(0xC3, 0xCB, 0xD6),
        Grade::Gold => Color32::from_rgb(0xFF, 0xC8, 0x3D),
        Grade::Platinum => Color32::from_rgb(0x9F, 0xE3, 0xF2),
    }
}

/// A simple cup (bowl, two handles, stem, base) tinted `color`, filling `rect`.
pub fn paint_cup(p: &Painter, rect: Rect, color: Color32) {
    let s = rect.width().min(rect.height());
    let at = |x: f32, y: f32| pos2(rect.min.x + x * s, rect.min.y + y * s);
    let dark = Color32::from_rgba_unmultiplied(0, 0, 0, 70);
    let hi = Color32::from_rgba_unmultiplied(255, 255, 255, 70);
    let handle = Stroke::new((s * 0.07).max(1.5), color);
    p.circle_stroke(at(0.26, 0.28), s * 0.12, handle);
    p.circle_stroke(at(0.74, 0.28), s * 0.12, handle);
    let bowl = vec![at(0.26, 0.10), at(0.74, 0.10), at(0.69, 0.46), at(0.58, 0.60), at(0.42, 0.60), at(0.31, 0.46)];
    p.add(Shape::convex_polygon(bowl, color, Stroke::new(1.0, dark)));
    p.add(Shape::convex_polygon(vec![at(0.31, 0.14), at(0.40, 0.14), at(0.40, 0.46), at(0.36, 0.46)], hi, Stroke::NONE));
    p.rect_filled(Rect::from_min_max(at(0.45, 0.58), at(0.55, 0.76)), 1, color);
    p.rect_filled(Rect::from_min_max(at(0.30, 0.76), at(0.70, 0.88)), CornerRadius::same(2), color);
    p.rect_stroke(Rect::from_min_max(at(0.30, 0.76), at(0.70, 0.88)), CornerRadius::same(2), Stroke::new(1.0, dark), egui::StrokeKind::Inside);
}

/// The entry's icon in a ring of its grade colour (dimmed while locked); the painted cup when there is no picture for its id.
fn badge(ui: &mut Ui, e: &Achievement, size: f32, ring: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    if !super::ach_icons::paint(ui, rect, e, ring) {
        paint_cup(ui.painter(), rect, ring);
    }
}

fn cup_icon(ui: &mut Ui, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint_cup(ui.painter(), rect, color);
}

impl App {
    /// The tab: finds the file of the selected install, re-reads it when it changed, shows it.
    pub fn show_achievements(&mut self, ui: &mut Ui) {
        let path = achievements::file_path(&self.root, self.rac1());
        self.ach.poll(&path, Instant::now());
        ui.ctx().request_repaint_after(achievements::POLL);
        show_view(ui, &self.ach.load, &mut self.ach_filter);
    }
}

/// The page for one load result (a free function so the tests can draw it without an `App`).
pub fn show_view(ui: &mut Ui, load: &Load, filter: &mut Filter) {
    ui.label(theme::bold("Achievements", 22.0));
    ui.add_space(4.0);
    match load {
        Load::Unread | Load::Missing => {
            theme::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(theme::muted(achievements::MSG_MISSING));
            });
        }
        Load::Broken(msg) => {
            theme::card_colored(ui, theme::RED, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(msg.clone()).color(theme::RED).size(14.0));
                ui.label(theme::muted(achievements::MSG_BROKEN_NOTE));
            });
        }
        Load::Ok(a) => show_list(ui, a, filter),
    }
}

fn show_list(ui: &mut Ui, a: &Achievements, filter: &mut Filter) {
    let t = a.totals();
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::bold(format!("{} of {} unlocked", t.unlocked, t.total), 18.0));
            ui.label(theme::muted(format!("{}%", t.percent())));
        });
        theme::progress(ui, t.percent() as f32 / 100.0, 8.0, theme::ORANGE);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for g in Grade::ALL {
                let (u, n) = t.grade(g);
                cup_icon(ui, 20.0, grade_color(g));
                ui.label(RichText::new(format!("{u}/{n}")).color(theme::TEXT).size(13.5));
                ui.add_space(10.0);
            }
        });
    });
    ui.add_space(8.0);
    let cur = Some(filter.index());
    if let Some(i) = theme::segmented(ui, "ach_filter", "Show", &Filter::LABELS, cur) {
        *filter = Filter::from_index(i);
    }
    ui.add_space(6.0);
    if a.list.is_empty() {
        ui.label(theme::muted(achievements::MSG_MISSING));
        return;
    }
    let mut shown = 0;
    ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for e in a.filtered(*filter) {
            row(ui, e);
            ui.add_space(4.0);
            shown += 1;
        }
        if shown == 0 {
            ui.label(theme::muted(if *filter == Filter::Unlocked { "Nothing unlocked yet." } else { "Everything is unlocked." }));
        }
    });
}

fn row(ui: &mut Ui, e: &Achievement) {
    let dim = |c: Color32| if e.unlocked { c } else { c.gamma_multiply(0.55) };
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            badge(ui, e, 52.0, dim(grade_color(e.grade)));
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::bold(e.shown_name(), 15.5).color(dim(theme::TEXT)));
                    ui.label(RichText::new(e.grade.label()).color(dim(grade_color(e.grade))).size(12.5));
                });
                ui.label(RichText::new(e.shown_description()).color(dim(theme::MUTED)).size(13.5));
                if let Some((done, max, pct)) = e.progress_view() {
                    ui.add_space(2.0);
                    theme::progress(ui, pct as f32 / 100.0, 6.0, theme::BLUE);
                    ui.label(theme::muted(format!("{done} / {max} ({pct}%)")));
                }
                if e.unlocked {
                    let when = e.unlocked_at.as_deref().map(achievements::format_stamp);
                    ui.label(RichText::new(match when {
                        Some(w) => format!("Unlocked {w}"),
                        None => "Unlocked".to_string(),
                    })
                    .color(theme::GREEN)
                    .size(12.5));
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(load: &Load, filter: &mut Filter) {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(900.0, 600.0))), ..Default::default() };
        for _ in 0..2 {
            ctx.run_ui(raw.clone(), |ui| show_view(ui, load, filter)).drop_without_applying_deltas();
        }
    }

    #[test]
    fn every_state_draws_without_panicking() {
        let a = achievements::parse(
            r#"{"version":1,"achievements":[
            {"id":"a","name":"A","grade":"gold","unlocked":true,"unlocked_at":"2026-11-02T12:00:00Z"},
            {"id":"b","name":"B","grade":"silver","unlocked":false,"progress":5,"progress_max":0},
            {"id":"c","name":"C","grade":"platinum","hidden":true,"unlocked":false,"progress":1,"progress_max":4}]}"#,
        )
        .unwrap();
        for f in [Filter::All, Filter::Unlocked, Filter::Locked] {
            let mut f = f;
            draw(&Load::Ok(a.clone()), &mut f);
        }
        let mut f = Filter::All;
        draw(&Load::Ok(Achievements::default()), &mut f);
        draw(&Load::Missing, &mut f);
        draw(&Load::Unread, &mut f);
        draw(&Load::Broken(achievements::MSG_BROKEN.to_string()), &mut f);
    }

    /// REVIEW_B2 B18: the broken-file text says what the game really does (keeps the copy as .broken, starts again from the save data).
    #[test]
    fn the_broken_note_names_the_kept_copy() {
        let n = achievements::MSG_BROKEN_NOTE;
        assert!(n.contains("achievements.json.broken") && n.contains("old data") && n.contains("save data"), "{n}");
        assert!(!n.contains("writes it again"), "the old text promised that nothing is lost");
    }

    #[test]
    fn grade_colors_differ() {
        let c: Vec<_> = Grade::ALL.iter().map(|g| grade_color(*g)).collect();
        for i in 0..c.len() {
            for j in i + 1..c.len() {
                assert_ne!(c[i], c[j]);
            }
        }
    }
}
