//! The Settings rows of an R&C 2 install: the rows of `rows::ROWS` whose key R&C 2 reads (`pcopts_rc2::RC2_KEYS`), in the same groups,
//! plus the rows only R&C 2 has and the Strafe row of R&C 2 (on / off: its aim camera is its own, no Shoulder / Steady / lock-on).
use super::rows::{self, Group, Kind, Row};
use crate::pcopts_rc2 as rc2;

const ON_OFF: &[(&str, &str)] = &[("on", "On"), ("off", "Off")];
const OFF_ON: &[(&str, &str)] = &[("off", "Off"), ("on", "On")];
const FRAMEGEN: &[(&str, &str)] = &[("off", "Off"), ("fsr", "FSR 2x"), ("dlss2", "DLSS 2x"), ("dlss3", "DLSS 3x"), ("dlss4", "DLSS 4x")];
const DETAIL3: &[(&str, &str)] = &[("low", "Low"), ("medium", "Medium"), ("high", "High")];
const MOUSE_DIR: &[(&str, &str)] = &[("like", "Like camera"), ("normal", "Normal"), ("inverted", "Inverted")];
const PICTURE: rows::Pct = rows::Pct { min: 0, max: 100, step: 5 };

/// R&C 2's Strafe aiming row (the key is the same `strafe=on|off`; the game's menu calls the values Raised and Steady, and the right
/// mouse button strafes either way, modern_rows_input.rs).
pub const STRAFE: Row = Row {
    key: "strafe",
    label: "Strafe aiming",
    help: "Hold the right mouse button to strafe. Raised lifts the camera while you strafe, Steady keeps the camera as it is, like Ratchet & Clank 2.",
    kind: Kind::Choice(&[("on", "Raised"), ("off", "Steady")]),
    default: "on",
    group: Group::Camera,
};

/// Rows R&C 2 has and R&C 1 does not.
pub const ONLY_RC2: &[Row] = &[
    Row { key: "subtitles", label: "Subtitles", help: "Shows the subtitles of the movies. Off shows only the cards the original always shows (such as \"Two weeks later\").", kind: Kind::Choice(OFF_ON), default: "off", group: Group::Display },
    Row { key: "framegen", label: "Frame Generation", help: "Adds generated frames for smoother motion, with a little more input lag. It follows the Upscaler (FSR or DLSS) and needs one of them on; DLSS 3x and 4x need a recent NVIDIA card. Applies after a restart.", kind: Kind::Choice(FRAMEGEN), default: "off", group: Group::Graphics },
    Row { key: "brightness", label: "Brightness", help: "Lifts or darkens the shadows and mid-tones of the game picture. The HUD and menus are not changed. 50 is the original picture.", kind: Kind::Percent(PICTURE), default: "50", group: Group::Display },
    Row { key: "contrast", label: "Contrast", help: "Spreads the picture's tones apart (above 50) or brings them closer to grey (below 50). The HUD and menus are not changed. 50 is the original picture.", kind: Kind::Percent(PICTURE), default: "50", group: Group::Display },
    Row { key: "metal_detail", label: "Metal reflection detail", help: "How sharp the reflections on metal are. Lower is faster. Only matters while Metal is on.", kind: Kind::Choice(DETAIL3), default: "high", group: Group::Enhanced },
    Row { key: "mouse_ud", label: "Mouse up/down", help: "Which way the mouse tilts the camera. Like camera follows the game's Up / Down option; Normal and Inverted set it for the mouse alone.", kind: Kind::Choice(MOUSE_DIR), default: "like", group: Group::Camera },
    Row { key: "mouse_lr", label: "Mouse left/right", help: "Which way the mouse turns the camera sideways. Like camera follows the game's Left / Right option; Normal and Inverted set it for the mouse alone.", kind: Kind::Choice(MOUSE_DIR), default: "like", group: Group::Camera },
    Row { key: "keyprompts", label: "Key names in prompts", help: "While you play with the keyboard, button prompts name the key bound to that button (Enter, Q, E ...). A controller brings back the button pictures. Off keeps the PlayStation buttons.", kind: Kind::Choice(ON_OFF), default: "on", group: Group::Controls },
];

/// R&C 1 rows that R&C 2 has with another default or another help text (same key, same choices; audit against the R&C 2 game source,
/// build 53, 2026-11-13): Strafe (its own row); Volumetric clouds (no line = Classic: rac2/options.rs `clouds_q`); Waves and Underwater
/// look (R&C 1's help names game version 0.5.1 and a Water row rule R&C 2 does not have); Language (no line = English, R&C 2 never asks:
/// rac2/options.rs `cold_language`); Camera speed (no line = 50 %: rac2/modcam_pose.rs `scale`).
fn overrides() -> &'static [Row] {
    static O: std::sync::OnceLock<Vec<Row>> = std::sync::OnceLock::new();
    O.get_or_init(|| {
        let base = |key: &str| *rows::ROWS.iter().find(|r| r.key == key).expect("an R&C 1 row");
        vec![
            STRAFE,
            Row { default: "classic", help: "Real 3D clouds that drift and change shape, lit by the sun, on the planets with an open sky. Classic is the original sky. They cost speed on some planets.", ..base("clouds") },
            Row { help: "Real moving waves on the sea (with Water on Medium or High).", ..base("waves") },
            Row { help: "Fog while the camera is under water, light patterns from Water Medium, light shafts at High. Off is the original look under water; the water surface stays improved. Set only here: the game's menu has no row for it.", ..base("underwater") },
            Row { default: "en", help: "The language of the game. You can also pick it in the game's Options on the title screen; the game remembers it.", ..base("lang") },
            Row { default: "50", help: "How fast the right stick turns the camera (Static and Smooth camera), in percent.", ..base("camspeed") },
        ]
    })
}

/// The rows of `group` for the game `game_id` ("rac1": the table of `rows::ROWS`; "rac2": the R&C 2 rows).
pub fn rows_for(game_id: &str, group: Group) -> Vec<&'static Row> {
    if game_id != "rac2" {
        return rows::rows_of(group).collect();
    }
    let own = |r: &'static Row| overrides().iter().find(|o| o.key == r.key).unwrap_or(r);
    let mut v: Vec<&'static Row> = rows::rows_of(group).filter(|r| rc2::has_row(r.key)).map(own).collect();
    v.extend(ONLY_RC2.iter().filter(|r| r.group == group));
    v
}

/// The choice an R&C 2 row marks: the file's value, else the row's default. R&C 2 has no "a missing Fur / Fire / Smoke line follows the
/// preset" rule (no line = High: engine pcopts.rs `graphics_rows`), so `presets::value_of` (R&C 1's reading) is not used here.
pub fn current_index(o: &crate::pcopts::Options, row: &Row) -> Option<usize> {
    match row.kind {
        Kind::Choice(list) if crate::pcopts_extra::extra_key(row.key).is_none() => {
            let v = o.get(row.key).unwrap_or(row.default);
            list.iter().position(|(val, _)| *val == v)
        }
        _ => rows::current_index(o, row),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUPS: [Group; 7] = [Group::Graphics, Group::Display, Group::Enhanced, Group::Camera, Group::Gameplay, Group::Controls, Group::Language];

    fn keys(game: &str) -> Vec<&'static str> {
        GROUPS.iter().flat_map(|g| rows_for(game, *g)).map(|r| r.key).collect()
    }

    #[test]
    fn r1_keeps_every_row() {
        assert_eq!(keys("rac1").len(), rows::ROWS.len());
    }

    #[test]
    fn rac2_shows_only_keys_it_reads_and_none_of_the_r1_only_ones() {
        let k = keys("rac2");
        for key in &k {
            assert!(rc2::reads(key), "{key} is not an R&C 2 key");
        }
        for key in rc2::NOT_IN_RC2 {
            assert!(!k.contains(key), "{key}");
        }
        assert!(!k.contains(&"buyall"), "R&C 2 parses buyall= but nothing reads it: no row");
        for must in ["renderer", "display", "lighting", "ao", "fog", "metal", "fur", "camera", "strafe", "intro", "lang", "hudpos", "glyphs", "stickdz", "wheelmouse", "subtitles", "keyprompts", "upscaler", "culling", "framegen", "brightness", "contrast", "metal_detail", "mouse_ud", "mouse_lr"] {
            assert!(k.contains(&must), "{must} has no row");
        }
        assert!(!k.contains(&"lockon_strength") && !k.contains(&"supporters") && !k.contains(&"snow") && !k.contains(&"video60"));
        assert_eq!(k.iter().filter(|x| **x == "strafe").count(), 1);
    }

    /// Every R&C 2 table key that is not a hidden one has a row, and every row's choices are accepted by the file format.
    #[test]
    fn every_key_with_a_row_has_one_and_its_values_are_valid() {
        let k = keys("rac2");
        for r in rc2::RC2_KEYS.iter().filter(|r| rc2::has_row(r.key)) {
            assert!(k.contains(&r.key), "{} has no row", r.key);
        }
        for g in GROUPS {
            for r in rows_for("rac2", g) {
                if let Kind::Choice(list) = r.kind {
                    for (v, _) in list {
                        match crate::pcopts_extra::extra_key(r.key) {
                            Some(e) => assert_eq!(crate::pcopts_extra::read(e, Some(v)), *v, "{} = {v}", r.key),
                            None => assert_eq!(crate::pcopts::canon(r.key, v).as_deref(), Some(*v), "{} = {v}", r.key),
                        }
                    }
                }
            }
        }
    }

    fn rc2_row(key: &str) -> &'static Row {
        GROUPS.iter().flat_map(|g| rows_for("rac2", *g)).find(|r| r.key == key).unwrap()
    }

    /// What the page marks with no line in the file is what the R&C 2 game uses (not R&C 1's default or its preset rule).
    #[test]
    fn rc2_defaults_are_the_rc2_games() {
        let label = |o: &crate::pcopts::Options, key: &str| {
            let r = rc2_row(key);
            let Kind::Choice(list) = r.kind else { panic!("{key}") };
            current_index(o, r).map(|i| list[i].0)
        };
        let none = crate::pcopts::Options::new();
        assert_eq!(label(&none, "clouds"), Some("classic"));
        assert_eq!(label(&none, "lang"), Some("en"));
        // a file that matches no R&C 1 preset: Fur / Fire / Smoke still read High, as the game reads them
        let custom = crate::pcopts::Options::parse("version=1\nlighting=enhanced\nwater=low\n");
        for key in ["fur", "fire", "smoke"] {
            assert_eq!(label(&custom, key), Some("high"), "{key}");
        }
        assert_eq!(label(&custom, "water"), Some("low"));
        assert_eq!(rc2_row("camspeed").default, "50");
        // no R&C 2 help text names an R&C 1 game version or R&C 1's vendor
        for g in GROUPS {
            for r in rows_for("rac2", g) {
                assert!(!r.help.contains("0.5.") && !r.help.contains("Gadgetron"), "{}: {}", r.key, r.help);
            }
        }
        // R&C 1 keeps its own rows
        assert_eq!(rows::ROWS.iter().find(|r| r.key == "clouds").unwrap().default, "high");
    }

    #[test]
    fn the_new_rows_write_the_games_lines() {
        let mut o = crate::pcopts::Options::new();
        for (key, idx) in [("subtitles", 1usize), ("keyprompts", 1)] {
            let row = ONLY_RC2.iter().find(|r| r.key == key).unwrap();
            rows::choose(&mut o, row, idx);
        }
        assert_eq!(o.to_text(), "version=1\nsubtitles=on\nkeyprompts=off\n");
        assert_eq!(rows::current_index(&o, ONLY_RC2.iter().find(|r| r.key == "subtitles").unwrap()), Some(1));
        let st = &STRAFE;
        rows::choose(&mut o, st, 1);
        assert_eq!(o.get("strafe"), Some("off"));
        o.reset_all();
        assert_eq!(o.to_text(), "version=1\n");
    }
}
