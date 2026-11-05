//! The achievement icons of the Achievements tab: the 36 art pictures of the game's list (35 trophy ids + `hidden`), 128 px PNGs bundled
//! in the launcher (`launcher/assets/achievements/<id>.png`, a copy made by the game lane's resize script, not a path into the game
//! tree). A locked trophy shows its grey look, a locked hidden one the `hidden` icon; an id without a picture keeps the painted cup.
use crate::achievements::Achievement;
use egui::{Color32, ColorImage, Context, CornerRadius, Rect, Stroke, TextureHandle, Ui};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Edge of every bundled icon in pixels (the game's tests check the same number).
pub const SIZE: usize = 128;
/// A locked trophy's grey icon keeps this share of the brightness.
const GREY_LEVEL: f32 = 0.6;

macro_rules! icons {
    ($($id:literal),* $(,)?) => {
        /// Every bundled icon id.
        pub const IDS: &[&str] = &[$($id),*];
        /// The PNG file of an icon id.
        pub fn bytes(id: &str) -> Option<&'static [u8]> {
            match id {
                $($id => Some(include_bytes!(concat!("../../assets/achievements/", $id, ".png")) as &[u8]),)*
                _ => None,
            }
        }
    };
}

icons!(
    "hit_me_more", "lombax_magnetic", "not_the_droids", "very_sucky", "all_qwarkd_up", "bolt_collector", "gadgeteer", "nano_this",
    "ultimate_animal", "unlock_sketchbook", "bolt_commando", "drek_this", "king_midas", "skillful", "speedy", "gadget_master", "take_aim",
    "transported", "destroyed", "sniper", "buried_treasure", "magician", "pest_control", "hey_over_here", "sitting_ducks",
    "careful_cruise", "strike_a_pose", "going_commando", "vehicle_bombardier", "cluck_cluck", "qwarktastic", "girl_trouble",
    "twisted_hoverboard", "vandal", "eat_lead", "hidden",
);

/// Which picture an entry shows: (icon id, grey). A locked hidden trophy shows `hidden` (as is), another locked one its grey icon.
pub fn pick(e: &Achievement) -> (&str, bool) {
    if e.masked() {
        ("hidden", false)
    } else {
        (e.id.as_str(), !e.unlocked)
    }
}

/// The decoded picture; `grey` = the locked look.
pub fn image(id: &str, grey: bool) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes(id)?).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let mut px = img.into_raw();
    if grey {
        for p in px.chunks_exact_mut(4) {
            let l = ((p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114) * GREY_LEVEL).round() as u8;
            p[..3].fill(l);
        }
    }
    Some(ColorImage::from_rgba_unmultiplied(size, &px))
}

/// Textures made so far (kept in the egui context, one per (id, grey)); a missing id is remembered as None.
#[derive(Clone, Default)]
struct Cache(Arc<Mutex<HashMap<(String, bool), Option<TextureHandle>>>>);

fn texture(ctx: &Context, id: &str, grey: bool) -> Option<TextureHandle> {
    let cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Cache>(egui::Id::new("ach_icon_cache")).clone());
    let mut map = cache.0.lock().unwrap_or_else(|e| e.into_inner());
    map.entry((id.to_string(), grey))
        .or_insert_with(|| {
            let opts = egui::TextureOptions::LINEAR.with_mipmap_mode(Some(egui::TextureFilter::Linear));
            image(id, grey).map(|im| ctx.load_texture(format!("ach-{id}{}", if grey { "-grey" } else { "" }), im, opts))
        })
        .clone()
}

/// Paints the entry's icon (a circle) inside a ring of colour `ring` filling `rect`. False = no picture for this id: the caller paints the cup.
pub fn paint(ui: &Ui, rect: Rect, e: &Achievement, ring: Color32) -> bool {
    let (id, grey) = pick(e);
    let Some(tex) = texture(ui.ctx(), id, grey) else { return false };
    let size = rect.width().min(rect.height());
    let w = (size * 0.07).max(2.0);
    let inner = Rect::from_center_size(rect.center(), egui::Vec2::splat(size * 0.9));
    egui::Image::new(&tex).corner_radius(CornerRadius::same(255)).paint_at(ui, inner);
    ui.painter().circle_stroke(rect.center(), size * 0.5 - w * 0.5, Stroke::new(w, ring));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::achievements::{Achievement, Grade};

    /// The ids of the game's list (rac-rs `src/rac1/trophies.rs` DEFS): the contract of achievements.json `id`.
    const TROPHIES: [&str; 35] = [
        "hit_me_more", "lombax_magnetic", "not_the_droids", "very_sucky", "all_qwarkd_up", "bolt_collector", "gadgeteer", "nano_this",
        "ultimate_animal", "unlock_sketchbook", "bolt_commando", "drek_this", "king_midas", "skillful", "speedy", "gadget_master", "take_aim",
        "transported", "destroyed", "sniper", "buried_treasure", "magician", "pest_control", "hey_over_here", "sitting_ducks",
        "careful_cruise", "strike_a_pose", "going_commando", "vehicle_bombardier", "cluck_cluck", "qwarktastic", "girl_trouble",
        "twisted_hoverboard", "vandal", "eat_lead",
    ];

    fn entry(id: &str, hidden: bool, unlocked: bool) -> Achievement {
        Achievement { id: id.into(), name: id.into(), description: String::new(), grade: Grade::Bronze, hidden, unlocked, unlocked_at: None, progress: None, progress_max: None }
    }

    #[test]
    fn every_trophy_and_the_hidden_icon_is_bundled_and_decodes_to_the_size() {
        assert_eq!(IDS.len(), 36);
        for id in TROPHIES.iter().chain(&["hidden"]) {
            assert!(IDS.contains(id), "{id} has no icon");
            for grey in [false, true] {
                let im = image(id, grey).unwrap_or_else(|| panic!("{id} does not decode"));
                assert_eq!(im.size, [SIZE, SIZE], "{id}");
            }
        }
        assert!(bytes("not_a_trophy").is_none() && image("not_a_trophy", false).is_none());
    }

    #[test]
    fn locked_is_grey_hidden_shows_the_hidden_icon_unlocked_is_colour() {
        assert_eq!(pick(&entry("sniper", true, false)), ("hidden", false), "locked hidden");
        assert_eq!(pick(&entry("sniper", true, true)), ("sniper", false), "unlocked hidden");
        assert_eq!(pick(&entry("speedy", false, false)), ("speedy", true), "locked");
        assert_eq!(pick(&entry("speedy", false, true)), ("speedy", false), "unlocked");
        let (c, g) = (image("speedy", false).unwrap(), image("speedy", true).unwrap());
        assert!(g.pixels.iter().all(|p| p.r() == p.g() && p.g() == p.b()), "grey is grey");
        assert!(c.pixels.iter().any(|p| p.r() != p.b()), "colour is colour");
    }

    #[test]
    fn painting_makes_one_texture_per_look_and_unknown_ids_fall_back() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 200.0))), ..Default::default() };
        let (mut known, mut unknown) = (false, true);
        ctx.run_ui(raw, |ui| {
            let r = Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(52.0, 52.0));
            known = paint(ui, r, &entry("vandal", false, true), Color32::GOLD);
            unknown = paint(ui, r, &entry("some_future_trophy", false, true), Color32::GOLD);
        })
        .drop_without_applying_deltas();
        assert!(known && !unknown);
    }
}
