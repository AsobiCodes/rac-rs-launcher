//! The options file keys of RAC-RS 2 (R&C 2): every `pc_options.txt` key the R&C 2 game reads, or is getting in this release.
//! `pcopts::KEYS` is R&C 1's table; both games share one file format (`pcopts.rs`) and mostly the same key names, so this table
//! only says which keys R&C 2 knows and in what state they are. The Settings page of an R&C 2 install shows exactly the rows whose
//! key is `Status::InGame` or `Status::Coming` here (`ui/rows_rc2.rs`).
//!
//! Sources: the R&C 2 game's options code (rac2/options.rs, modern_rows*.rs, modcam_hand.rs, pc_input.rs, pc_prompts.rs, host/movie.rs),
//! the engine's `pcopts.rs` / `extra_get` reads, and the parity table rows 5, 6, 7, 10, 12, 16, 18, 20, 21, 23, 25, 27, 45, 46, 58, 63, 70
//! (lanes that were still adding their keys when this table was made: `Coming`, with R&C 1's key name).
//! Re-checked against the game source at build 52 (2026-11-13: every `row("key"...)` of rac2/modern_rows*.rs, every `extra_get` / `extra_set` key and the
//! engine's pcopts.rs reads are in this table; no key was added or removed since build 48). Checked at build 48 (2026-11-12): `video60` (the hidden 60 Hz mode) is neither read nor written, and the
//! Snow & sand footprints are not in the game yet, so neither key is listed.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Read by the R&C 2 game (build 48, the base of v0.1).
    InGame,
    /// Added by a v0.1 lane (parity row in the note) that was not in build 42 yet; the game ignores the key until that lane is merged.
    Coming,
    /// Kept in the file and read by the game, but it has no Settings row (binds are set in the game's Controls page).
    NoRow,
}

#[derive(Clone, Copy, Debug)]
pub struct Rc2Key {
    pub key: &'static str,
    pub status: Status,
    pub note: &'static str,
}

const fn k(key: &'static str, status: Status, note: &'static str) -> Rc2Key {
    Rc2Key { key, status, note }
}

use Status::{Coming, InGame, NoRow};

/// Every key R&C 2 reads, in the order of the Settings page.
pub const RC2_KEYS: &[Rc2Key] = &[
    // graphics system, window, picture (parity 7, 25, 53, 62, 63)
    k("renderer", InGame, "vulkan | dx12, read by the engine"),
    k("display", InGame, "windowed | fullscreen"),
    k("aspect", InGame, "engine aspect_box (row 25: the ultrawide fill is a later step, the box works)"),
    k("renderscale", InGame, "engine render scale, row 7"),
    k("upscaler", InGame, "off | fsr | dlss (modern_rows_upscale.rs)"),
    k("upscale_q", InGame, "native | quality | balanced | performance | ultra"),
    k("framegen", InGame, "off | fsr | dlss2 | dlss3 | dlss4 (engine fgen/choice.rs; follows the Upscaler, applies after a restart)"),
    k("resolution", NoRow, "Desktop or WxH from the monitor's own size list; set in the game's Graphics page (list differs per PC)"),
    k("brightness", InGame, "0..100, 50 = the original picture (engine picture_adjust.rs, no line = 50)"),
    k("contrast", InGame, "0..100, 50 = the original picture (engine picture_adjust.rs, no line = 50)"),
    k("dlss_preset", InGame, "engine upscale/dlss_preset.rs, launcher-only row"),
    k("scene_msaa", InGame, "engine scene_msaa.rs, launcher-only row"),
    k("fps", InGame, "FPS limit, row 63"),
    k("smoothing", InGame, "frame smoothing"),
    k("hud", InGame, "sharp | original"),
    k("hudpos", InGame, "row 27: 4:3 | corners"),
    k("hudscale", InGame, "row 27: 75..150"),
    // picture
    k("lighting", InGame, "classic | enhanced"),
    k("draw", InGame, "row 5: the row is read; per-class draw distance may still be a lane's"),
    k("culling", InGame, "row 23, with the marker line culling_off"),
    k("culling_off", InGame, "marker line written with culling=off"),
    k("texfilter", InGame, "classic | smooth"),
    k("ao", InGame, "Ambient occlusion"),
    k("shadows", InGame, "Sun shadows, row 10"),
    k("gi", InGame, "Global illumination"),
    k("water", InGame, "classic | low | medium | high"),
    k("waves", InGame, "on | off"),
    k("underwater", InGame, "launcher-only row"),
    k("fog", InGame, "row 18: volumetric fog"),
    k("clouds", InGame, "row 19: volumetric clouds"),
    k("fur", InGame, "row 28"),
    k("furstyle", InGame, "soft | fluffy"),
    k("metal", InGame, "row 29"),
    k("metal_reflect", InGame, "0..100"),
    k("metal_detail", InGame, "low | medium | high (engine metal_detail.rs; R&C 2 default high)"),
    k("fire", InGame, "row 30"),
    k("smoke", InGame, "row 31"),
    k("particles", InGame, "row 16"),
    k("sunrays", InGame, "row 12"),
    k("vegetation", InGame, "row 17: off | moderate | lush"),
    // camera and aiming
    k("camera", InGame, "classic | modern"),
    k("camfeel", InGame, "smooth | static (part of the Camera row)"),
    k("camspeed", InGame, "10..200"),
    k("mousespeed", InGame, "10..200"),
    k("mouse_ud", InGame, "like camera (no line) | normal | inverted (rac2/modcam_mouseinv.rs)"),
    k("mouse_lr", InGame, "like camera (no line) | normal | inverted (rac2/modcam_mouseinv.rs)"),
    k("strafe", InGame, "on | off (raised strafe aim); no Shoulder / Steady / lock-on rows: R&C 2 has its own"),
    // gameplay
    k("buyall", NoRow, "parsed by the engine's pcopts.rs, but nothing in rac2 reads it (the vendor's Buy all ammo row is always there): no Settings row (audit at build 53)"),
    k("intro", InGame, "row 58: skip the boot logo movie (rac2 options.rs intro_skip; the movie side may still be a lane's)"),
    k("skipcut", InGame, "Start skips cutscenes"),
    k("menustyle", InGame, "modern | classic"),
    k("subtitles", InGame, "movie subtitles on | off (R&C 2 only)"),
    // controls
    k("wheelmouse", InGame, "Quick Select with the mouse"),
    k("keyprompts", InGame, "prompts name the bound key (R&C 2 only)"),
    k("stickdz", InGame, "row 45: retail | small | minimal"),
    k("glyphs", InGame, "row 46: playstation | letters | switch"),
    k("kbm", NoRow, "keyboard and mouse bindings, set in the game (V051 kbm system)"),
    k("keys", NoRow, "key remaps, set in the game"),
    k("pad", NoRow, "button remaps, set in the game"),
    k("gamepad", NoRow, "gamepad choice"),
    // language
    k("lang", InGame, "en | fr | de | es | it, remembered by the game"),
    // debug (kept as it is; R&C 1's launcher keeps it too)
    k("debug", InGame, "debug mode, row 70"),
];

/// The table entry of a key.
pub fn find(key: &str) -> Option<&'static Rc2Key> {
    RC2_KEYS.iter().find(|r| r.key == key)
}

/// True when R&C 2 reads `key` (any status).
pub fn reads(key: &str) -> bool {
    find(key).is_some()
}

/// True when the key is shown as a Settings row of an R&C 2 install.
pub fn has_row(key: &str) -> bool {
    find(key).is_some_and(|r| r.status != Status::NoRow) && !matches!(key, "culling_off" | "debug" | "camfeel")
}

/// Keys R&C 1 has that R&C 2 does not read (the Settings page of an R&C 2 install leaves them out).
pub const NOT_IN_RC2: &[&str] = &["lockon", "lockon_strength", "supporters", "wrenchthrow", "qspause", "autogadget", "aimcam"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_twice() {
        for (i, a) in RC2_KEYS.iter().enumerate() {
            assert!(RC2_KEYS[i + 1..].iter().all(|b| b.key != a.key), "{} twice", a.key);
        }
    }

    /// Every key is one the shared options file understands: a known key of `pcopts::KEYS`, an extra-line key, or a game-only line.
    #[test]
    fn every_key_is_known_to_the_file_format() {
        for r in RC2_KEYS {
            let known = crate::pcopts::KEYS.contains(&r.key) || crate::pcopts_extra::extra_key(r.key).is_some() || matches!(r.key, "culling_off" | "resolution" | "kbm" | "keys" | "pad" | "gamepad" | "hudscale");
            assert!(known, "{} is neither in pcopts::KEYS nor an extra line", r.key);
        }
    }

    #[test]
    fn r1_only_keys_are_not_in_the_table() {
        for key in NOT_IN_RC2 {
            assert!(!reads(key), "{key}");
        }
    }

    #[test]
    fn the_parity_rows_keys_are_there() {
        // PARITY.md rows 5, 6(no key), 7, 10, 12, 16, 18, 20(no key), 21(no key), 23, 25, 27, 45, 46, 58, 63, 70
        for key in ["draw", "renderscale", "shadows", "sunrays", "particles", "fog", "culling", "aspect", "hudpos", "hudscale", "stickdz", "glyphs", "intro", "fps", "debug"] {
            assert!(reads(key), "{key}");
        }
        assert_eq!(find("hudpos").unwrap().status, Status::InGame);
        assert_eq!(find("fog").unwrap().status, Status::InGame);
    }
}
