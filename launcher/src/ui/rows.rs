//! The rows of the Settings page: which option key, how it reads for a player, its choices and the game's default.
//! Same options as the game's own Options menu (they are the same file), in the game's groups: every row of the in-game menu that is
//! kept in pc_options.txt. Not here: the key and button binding (Options > Controls, `keys=` / `pad=` lines the launcher keeps as they are)
//! and the camera's Left / Right and Up / Down reversing (kept in the game's memory card, not in the options file).
use crate::pcopts::{self, Options};
use crate::pcopts_extra as extra;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Graphics,
    /// Screen and HUD: window, aspect ratio, render scale, upscaler, FPS limit, frame smoothing, HUD.
    Display,
    Enhanced,
    /// Camera and aiming.
    Camera,
    Gameplay,
    /// Controls: stick dead zone, Quick Select mouse, button glyphs.
    Controls,
    Language,
}

/// A percent slider's range and step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pct {
    pub min: u32,
    pub max: u32,
    pub step: u32,
}

pub const PCT_CAMSPEED: Pct = Pct { min: pcopts::CAMSPEED_MIN, max: pcopts::CAMSPEED_MAX, step: 10 };

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    /// (file value, label) pairs.
    Choice(&'static [(&'static str, &'static str)]),
    /// The Camera row: Classic / Static / Smooth, stored as camera= and camfeel=.
    Camera,
    /// The Strafe aiming row: Raised / Shoulder / Steady / Off, stored as `strafe=` plus `aimcam=` (`Options::strafe_byte`).
    Strafe,
    /// A percent slider: Camera speed (no key = the game's own Slow / Medium / Fast row), Mouse speed, Lock-on strength, Metal reflections
    /// and HUD scale (no key = the row's `default`).
    Percent(Pct),
    /// Culling: On / Off. Off is `culling=off` plus the game's marker line `culling_off=kept` (games from 0.4.1 read an Off without it as On).
    Culling,
}

#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: Kind,
    /// The file value the game uses when the key is missing ("" = no default: the game decides at the title screen).
    pub default: &'static str,
    pub group: Group,
}

const ON_OFF: &[(&str, &str)] = &[("on", "On"), ("off", "Off")];
const OFF_ON: &[(&str, &str)] = &[("off", "Off"), ("on", "On")];
const LEVELS: &[(&str, &str)] = &[("off", "Off"), ("low", "Low"), ("medium", "Medium"), ("high", "High")];
const LEVELS_CLASSIC: &[(&str, &str)] = &[("classic", "Classic"), ("low", "Low"), ("medium", "Medium"), ("high", "High")];

pub const ROWS: &[Row] = &[
    Row {
        key: "renderer",
        label: "Graphics system",
        help: "What the game draws with. Vulkan is best for most PCs. If the game does not start or the picture is broken, try DirectX 12.",
        kind: Kind::Choice(&[("vulkan", "Vulkan"), ("dx12", "DirectX 12")]),
        default: "vulkan",
        group: Group::Graphics,
    },
    Row {
        key: "display",
        label: "Window",
        help: "Play in a window or fill the screen (without borders).",
        kind: Kind::Choice(&[("windowed", "Windowed"), ("fullscreen", "Fullscreen")]),
        default: "windowed",
        group: Group::Display,
    },
    Row { key: "aspect", label: "Aspect ratio", help: "Auto fills the window. Another shape draws the game in the largest box of that shape, with black bars around it. 4:3 is the shape of the original PS2 picture.", kind: Kind::Choice(&[("auto", "Auto"), ("4:3", "4:3"), ("16:9", "16:9"), ("16:10", "16:10"), ("21:9", "21:9"), ("32:9", "32:9")]), default: "auto", group: Group::Display },
    Row { key: "renderscale", label: "Render scale", help: "Draws the picture at this size of the window, then fits it to the window. Below 100% runs faster, above 100% looks sharper.", kind: Kind::Choice(&[("50", "50%"), ("75", "75%"), ("100", "100%"), ("125", "125%"), ("150", "150%"), ("200", "200%")]), default: "100", group: Group::Display },
    Row { key: "upscaler", label: "Upscaler", help: "Draws the game smaller, then AMD FSR or NVIDIA DLSS scales it up to the window: faster and nearly as sharp. DLSS needs an NVIDIA RTX graphics card. If your PC cannot run the one you pick, the game uses Off.", kind: Kind::Choice(&[("off", "Off"), ("fsr", "FSR"), ("dlss", "DLSS")]), default: "off", group: Group::Display },
    Row { key: "upscale_q", label: "Upscale quality", help: "The size the game is drawn at before the upscaler, in percent of the window. Lower is faster and softer.", kind: Kind::Choice(&[("native", "Native AA (100%)"), ("quality", "Quality (67%)"), ("balanced", "Balanced (59%)"), ("performance", "Performance (50%)"), ("ultra", "Ultra performance (33%)")]), default: "quality", group: Group::Display },
    Row { key: "fps", label: "FPS limit", help: "The most pictures a second the game draws. Display matches your monitor, Off removes the limit.", kind: Kind::Choice(&[("display", "Display"), ("30", "30"), ("60", "60"), ("120", "120"), ("144", "144"), ("off", "Off")]), default: "display", group: Group::Display },
    Row {
        key: "lighting",
        label: "Lighting",
        help: "Classic is the picture of the PlayStation 2 game. Enhanced adds modern lighting; the effects below need it.",
        kind: Kind::Choice(&[("classic", "Classic"), ("enhanced", "Enhanced")]),
        default: "classic",
        group: Group::Graphics,
    },
    Row {
        key: "draw",
        label: "Draw distance",
        help: "How far away you can see. Low is the original distance.",
        kind: Kind::Choice(&[("low", "Low"), ("medium", "Medium"), ("high", "High")]),
        default: "medium",
        group: Group::Graphics,
    },
    Row {
        key: "smoothing",
        label: "Frame smoothing",
        help: "Adds in-between pictures so movement looks smooth on a fast screen. The game itself still runs at its original speed.",
        kind: Kind::Choice(ON_OFF),
        default: "on",
        group: Group::Display,
    },
    Row {
        key: "hud",
        label: "Menus and HUD",
        help: "Sharp draws the menus and the on-screen display again at your screen's resolution. Original keeps the old low-resolution look.",
        kind: Kind::Choice(&[("sharp", "Sharp"), ("original", "Original")]),
        default: "sharp",
        group: Group::Display,
    },
    Row { key: "hudpos", label: "HUD position", help: "4:3 keeps the HUD where the original game has it. Corners moves the top boxes (nanotech, bolts and ammo) to the screen corners on wide screens.", kind: Kind::Choice(&[("4:3", "4:3"), ("corners", "Corners")]), default: "4:3", group: Group::Display },
    Row { key: "hudscale", label: "HUD scale", help: "Size of the top HUD boxes. 100% is the original size.", kind: Kind::Percent(Pct { min: extra::HUDSCALE_MIN, max: extra::HUDSCALE_MAX, step: extra::HUDSCALE_STEP }), default: "100", group: Group::Display },
    Row {
        key: "texfilter",
        label: "Texture filtering",
        help: "Smooth blends texture detail as you move. Classic keeps the original detail steps. Requires Lighting Enhanced.",
        kind: Kind::Choice(&[("classic", "Classic"), ("smooth", "Smooth")]),
        default: "smooth",
        group: Group::Enhanced,
    },
    Row {
        key: "ao",
        label: "Ambient occlusion",
        help: "Soft shading in corners and creases.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "shadows",
        label: "Dynamic shadows",
        help: "Shadows that move with the characters and objects.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "water",
        label: "Water",
        help: "Reflections and movement of water. Classic is the original look.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "waves",
        label: "Waves",
        help: "Real moving waves on the sea (with Water on Medium or High).",
        kind: Kind::Choice(ON_OFF),
        default: "on",
        group: Group::Enhanced,
    },
    Row { key: "underwater", label: "Underwater look", help: "Fog while the camera is under water, light patterns from Water Medium, light shafts at High. Off is the original look under water; the water surface stays improved.", kind: Kind::Choice(ON_OFF), default: "on", group: Group::Enhanced },
    Row {
        key: "fog",
        label: "Volumetric fog",
        help: "Volumetric fog that drifts and catches the sun, in each planet's own fog colour. Classic is the original look.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "clouds",
        label: "Volumetric clouds",
        help: "Real 3D clouds that drift and change shape, lit by the sun. Classic is the original sky.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "fur",
        label: "Fur",
        help: "Ratchet's fur: a shader that gives his furry parts a soft, fluffy edge. Off keeps the flat original look.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "furstyle",
        label: "Fur style",
        help: "Soft is a short, smooth coat. Fluffy is a longer, shaggier one (needs Fur on).",
        kind: Kind::Choice(&[("soft", "Soft"), ("fluffy", "Fluffy")]),
        default: "soft",
        group: Group::Enhanced,
    },
    Row {
        key: "metal",
        label: "Metal",
        help: "Shiny metal: reflections of the sky and bright highlights on metal surfaces. Off keeps the original look.",
        kind: Kind::Choice(ON_OFF),
        default: "off",
        group: Group::Enhanced,
    },
    Row { key: "metal_reflect", label: "Metal reflections", help: "How strongly metal mirrors the sky and the scene. 100% is the full shiny look, lower is more matte, 0% has no reflections (highlights from lights stay).", kind: Kind::Percent(Pct { min: 0, max: 100, step: 5 }), default: "75", group: Group::Enhanced },
    Row {
        key: "fire",
        label: "Fire",
        help: "Realistic flames from weapons and effects, and on burning buildings.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "smoke",
        label: "Smoke",
        help: "Volumetric smoke that rises and drifts, lit by the fire. Off keeps the original smoke.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "particles",
        label: "Particles",
        help: "Smoke, sparks, fire and other effects. Classic is the original look.",
        kind: Kind::Choice(LEVELS_CLASSIC),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "sunrays",
        label: "Sun rays",
        help: "Beams of light through the air.",
        kind: Kind::Choice(LEVELS),
        default: "high",
        group: Group::Enhanced,
    },
    Row {
        key: "vegetation",
        label: "Vegetation",
        help: "How many plants and how lush the greenery is.",
        kind: Kind::Choice(&[("off", "Off"), ("moderate", "Moderate"), ("lush", "Lush")]),
        default: "moderate",
        group: Group::Enhanced,
    },
    Row {
        key: "gi",
        label: "Global illumination",
        help: "Light that bounces off surfaces. Looks great but needs a strong graphics card.",
        kind: Kind::Choice(LEVELS),
        default: "off",
        group: Group::Enhanced,
    },
    Row { key: "culling", label: "Culling", help: "Skips drawing what the camera cannot see. On is faster and looks the same. Off gains nothing and puts a heavy load on the PC.", kind: Kind::Culling, default: "on", group: Group::Graphics },
    Row { key: "supporters", label: "Supporter drawings", help: "The children's drawings made for the Ko-fi supporters, on the walls of Ratchet's garage on Veldin. Works with any Lighting.", kind: Kind::Choice(OFF_ON), default: "off", group: Group::Graphics },
    Row { key: "camera", label: "Camera", help: "Classic is the original camera. Static is a plain modern camera. Smooth is the modern camera that follows Ratchet smoothly.", kind: Kind::Camera, default: "smooth", group: Group::Camera },
    Row { key: "camspeed", label: "Camera speed", help: "How fast the right stick turns the camera, in percent. Default uses the game's own setting.", kind: Kind::Percent(PCT_CAMSPEED), default: "", group: Group::Camera },
    Row { key: "mousespeed", label: "Mouse speed", help: "How fast the mouse turns the camera (Static and Smooth camera), in percent.", kind: Kind::Percent(PCT_CAMSPEED), default: "50", group: Group::Camera },
    Row {
        key: "strafe",
        label: "Strafe aiming",
        help: "Hold the aim button to move sideways while aiming. Raised lifts the camera over Ratchet and shows the aim circle, Shoulder aims over his shoulder, Steady keeps the camera as it is (like Ratchet & Clank 2), Off turns strafing off.",
        kind: Kind::Strafe,
        default: "on",
        group: Group::Camera,
    },
    Row {
        key: "lockon",
        label: "Lock-on circle",
        help: "The red circle that locks onto a target while strafing. Auto shows it with a controller (and with Steady aiming) and hides it with keyboard and mouse.",
        kind: Kind::Choice(&[("auto", "Auto"), ("on", "On"), ("off", "Off")]),
        default: "auto",
        group: Group::Camera,
    },
    Row { key: "lockon_strength", label: "Lock-on strength", help: "How strongly the aim snaps to enemies while strafing. 100% is full lock-on; lower means you must aim closer to an enemy before it locks; 0% is no lock-on.", kind: Kind::Percent(Pct { min: 0, max: 100, step: 10 }), default: "100", group: Group::Camera },
    Row {
        key: "buyall",
        label: "Buy all ammo",
        help: "A shortcut at the Gadgetron vendor that fills up your ammo in one press.",
        kind: Kind::Choice(ON_OFF),
        default: "on",
        group: Group::Gameplay,
    },
    Row { key: "wrenchthrow", label: "Wrench throw", help: "Classic: Ratchet stands still until the wrench is back. Moving: he can move once it leaves his hand, and it flies where he aims.", kind: Kind::Choice(&[("classic", "Classic"), ("moving", "Moving")]), default: "classic", group: Group::Gameplay },
    Row { key: "qspause", label: "Quick Select pauses the game", help: "While the Quick Select ring is open, enemies, shots and the world stand still. Off: the world keeps moving, as in the original.", kind: Kind::Choice(OFF_ON), default: "off", group: Group::Gameplay },
    Row { key: "autogadget", label: "Auto-equip gadgets", help: "Fire at a Versa-Target, a Hydrodisplacer pad or a Trespasser lock to use that gadget; then your weapon comes back. Off: as in the original.", kind: Kind::Choice(OFF_ON), default: "off", group: Group::Gameplay },
    Row {
        key: "intro",
        label: "Skip intro (go straight to the title screen)",
        help: "Starts the game without the opening logos and movie. Off plays them as on the console.",
        kind: Kind::Choice(&[("skip", "On"), ("play", "Off")]),
        default: "play",
        group: Group::Gameplay,
    },
    Row { key: "skipcut", label: "Start skips cutscenes", help: "Pressing Start during a cutscene skips it. Off plays every cutscene to the end (the original rule).", kind: Kind::Choice(ON_OFF), default: "on", group: Group::Gameplay },
    Row { key: "menustyle", label: "Menu style", help: "Modern is the game's new options menu. Classic brings back the original-style option pages.", kind: Kind::Choice(&[("modern", "Modern"), ("classic", "Classic")]), default: "modern", group: Group::Gameplay },
    Row { key: "stickdz", label: "Stick dead zone", help: "How far a stick must move before the game notices it. High (the original) ignores small drifts of a worn stick.", kind: Kind::Choice(&[("retail", "High"), ("small", "Medium"), ("minimal", "Low")]), default: "retail", group: Group::Controls },
    Row { key: "wheelmouse", label: "Quick Select with the mouse", help: "Opens the Quick Select ring with the mouse and picks a weapon by moving the mouse.", kind: Kind::Choice(OFF_ON), default: "off", group: Group::Controls },
    Row { key: "glyphs", label: "Button glyphs", help: "The button pictures in prompts and menus. Letters shows A B X Y; the Switch set swaps the letters like a Switch controller.", kind: Kind::Choice(&[("playstation", "PlayStation"), ("letters", "Letters"), ("switch", "Letters (Switch)")]), default: "playstation", group: Group::Controls },
    Row {
        key: "lang",
        label: "Language",
        help: "Leave it unmarked and the game asks at the title screen, as on the console.",
        kind: Kind::Choice(&[("en", "English"), ("fr", "Français"), ("de", "Deutsch"), ("es", "Español"), ("it", "Italiano")]),
        default: "",
        group: Group::Language,
    },
];

pub fn rows_of(group: Group) -> impl Iterator<Item = &'static Row> {
    ROWS.iter().filter(move |r| r.group == group)
}

pub const CAMERA_LABELS: [&str; 3] = ["Classic", "Static", "Smooth"];
/// The Strafe aiming row's choices in order (`Options::strafe_byte`): (name for the gating table, label).
pub const STRAFE_VALUES: [(&str, &str); 4] = [("raised", "Raised"), ("shoulder", "Shoulder"), ("steady", "Steady"), ("off", "Off")];
/// The Culling row's choices: (file value, label).
pub const CULLING_VALUES: [(&str, &str); 2] = [("on", "On"), ("off", "Off")];

/// Index of the current choice (the value the game uses: the one the player set, else the game's default); None = nothing to mark.
/// Culling reads as a game from 0.4.1 on does; the Settings page passes its install's version (`culling_index`).
pub fn current_index(o: &Options, row: &Row) -> Option<usize> {
    match row.kind {
        Kind::Choice(list) => {
            let v = match extra::extra_key(row.key) {
                Some(e) => extra::value(o, e),
                None => super::presets::value_of(o, row.key).unwrap_or(row.default),
            };
            list.iter().position(|(val, _)| *val == v)
        }
        Kind::Camera => Some(o.camera_byte() as usize),
        Kind::Strafe => Some(o.strafe_byte() as usize),
        Kind::Percent(_) => None,
        Kind::Culling => Some(culling_index(o, super::presets::CULLING_RESET_VERSION)),
    }
}

/// The player picked choice `idx` of a row: written into the options (the caller stores the file).
pub fn choose(o: &mut Options, row: &Row, idx: usize) {
    match row.kind {
        Kind::Choice(list) => {
            if let Some((v, _)) = list.get(idx) {
                match extra::extra_key(row.key) {
                    Some(e) => extra::write(o, e, v),
                    None => o.set(row.key, Some(v)),
                }
            }
        }
        Kind::Camera => o.set_camera_byte(idx.min(2) as u8),
        Kind::Strafe => o.set_strafe_byte(idx.min(STRAFE_VALUES.len() - 1) as u8),
        Kind::Percent(_) => {}
        Kind::Culling => choose_culling(o, idx),
    }
}

/// The Culling row on a game at `version`: 0 On, 1 Off (`presets::culling_off`: from 0.4.1 an Off needs the marker line).
pub fn culling_index(o: &Options, version: &str) -> usize {
    super::presets::culling_off(o, version) as usize
}

/// As the game's menu does: Off writes `culling=off` and the marker `culling_off=kept` (a game before 0.4.1 ignores the marker line);
/// On removes both lines, as a preset does (no key = On).
pub fn choose_culling(o: &mut Options, idx: usize) {
    if idx == 1 {
        o.set("culling", Some("off"));
        o.set_extra(super::presets::CULLING_MARKER_KEY, Some("kept"));
    } else {
        o.set("culling", None);
        o.set_extra(super::presets::CULLING_MARKER_KEY, None);
    }
}

/// The percent the player set in a `Kind::Percent` row (`camspeed`, `mousespeed`, `lockon_strength`, `metal_reflect`, `hudscale`);
/// None = the row's default.
pub fn percent(o: &Options, key: &str) -> Option<u32> {
    if key == extra::HUDSCALE {
        return extra::hudscale(o);
    }
    o.get(key).and_then(|v| v.parse().ok())
}

/// Sets a percent row (snapped as the game snaps it); None = back to the default (no key).
pub fn set_percent(o: &mut Options, key: &str, pct: Option<u32>) {
    if key == extra::HUDSCALE {
        return extra::set_hudscale(o, pct);
    }
    o.set(key, pct.map(|p| p.to_string()).as_deref());
}

/// Why a row does nothing with the other options as they are now (the in-game menu dims it the same way); None = it applies.
pub fn inactive_note(o: &Options, key: &str) -> Option<&'static str> {
    let ex = |k: &str| extra::extra_key(k).map(|e| extra::value(o, e));
    let val = |k: &str| super::presets::value_of(o, k);
    match key {
        "upscale_q" => (ex("upscaler") == Some("off")).then_some("Applies when the Upscaler is FSR or DLSS."),
        "renderscale" => (ex("upscaler") != Some("off")).then_some("Applies while the Upscaler is Off (FSR and DLSS pick their own size)."),
        "lockon_strength" => (val("lockon") == Some("off")).then_some("Applies while Lock-on circle is Auto or On."),
        "metal_reflect" => (val("metal") != Some("on")).then_some("Applies while Metal is On."),
        "underwater" => (val("water") == Some("classic")).then_some("Applies with Water on Low, Medium or High."),
        _ => None,
    }
}

/// True when the row is a graphics effect that only works with Lighting = Enhanced.
pub fn needs_enhanced(row: &Row) -> bool {
    row.group == Group::Enhanced
}

pub fn lighting_enhanced(o: &Options) -> bool {
    o.get("lighting").unwrap_or("classic") == "enhanced"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_value_is_accepted_by_the_options_file() {
        for r in ROWS {
            if let Kind::Choice(list) = r.kind {
                match extra::extra_key(r.key) {
                    // an extra line: the row offers exactly the game's values, in the game's order, each read back as itself
                    Some(e) => {
                        assert_eq!(list.iter().map(|(v, _)| *v).collect::<Vec<_>>(), e.values, "{}", r.key);
                        for (v, _) in list {
                            assert_eq!(extra::read(e, Some(v)), *v, "{} = {v}", r.key);
                        }
                        assert_eq!(r.default, e.default, "default of {}", r.key);
                    }
                    None => {
                        for (v, _) in list {
                            assert!(pcopts::canon(r.key, v).as_deref() == Some(*v), "{} = {v}", r.key);
                        }
                    }
                }
                if !r.default.is_empty() {
                    assert!(list.iter().any(|(v, _)| *v == r.default), "default of {}", r.key);
                }
            }
        }
    }

    /// Every row of the game's own Options menu (rac-rs v0.5 modern_menu/rows_*.rs, by its row id) is a row here, except the ones that are
    /// not kept in pc_options.txt. Add the row here and in ROWS when the game gets a new one.
    #[test]
    fn every_in_game_options_row_is_here() {
        // (in-game row id, launcher key): the ids that name their key differently
        let renamed = [("refill", "buyall"), ("displaymode", "display"), ("rscale", "renderscale"), ("wheel", "wheelmouse")];
        let in_game = [
            "camera", "camspeed", "mousespeed", "strafe", "lockon", "lockon_strength", "refill", "menustyle", "displaymode", "aspect", "rscale", "fps", "smoothing", "hud", "hudpos", "hudscale", "particles",
            "metal", "metal_reflect", "fire", "smoke", "fur", "furstyle", "supporters", "wrenchthrow", "qspause", "autogadget", "lighting", "ao", "shadows", "gi", "sunrays", "draw", "culling", "vegetation",
            "water", "waves", "underwater", "fog", "clouds", "renderer", "stickdz", "wheel", "glyphs", "intro", "skipcut", "upscaler", "upscale_q",
        ];
        // the Preset row is the Settings page's preset selector; Left / Right and Up / Down live in the game's memory card
        let elsewhere = ["preset", "leftright", "updown"];
        for id in in_game {
            let key = renamed.iter().find(|(i, _)| *i == id).map_or(id, |(_, k)| k);
            assert!(ROWS.iter().any(|r| r.key == key), "the game's {id} row has no launcher row");
        }
        assert!(elsewhere.iter().all(|id| !in_game.contains(id)));
    }

    /// A row the 0.2.x games do not read must name its first game version (the page greys it for older installs).
    #[test]
    fn every_newer_row_is_in_the_gating_table() {
        let known_by_0_2 = ["lang", "camera", "strafe", "buyall", "lighting", "ao", "shadows", "display", "water", "particles", "sunrays"];
        for r in ROWS {
            if !known_by_0_2.contains(&r.key) {
                assert!(crate::gating::first_version(r.key).is_some(), "{} has no first version", r.key);
            }
        }
        assert!(!crate::gating::available("0.4.3", "upscaler") && crate::gating::available("0.5.0", "upscaler"));
        assert!(!crate::gating::available("0.4.0", "hudscale") && crate::gating::available("0.4.1", "hudscale"));
        assert!(!crate::gating::value_available("0.4.3", "strafe", "steady") && crate::gating::value_available("0.4.3", "strafe", "shoulder"));
    }

    fn row(key: &str) -> &'static Row {
        ROWS.iter().find(|r| r.key == key).unwrap()
    }

    #[test]
    fn defaults_are_marked_and_choices_write_through() {
        let mut o = Options::new();
        let water = row("water");
        assert_eq!(current_index(&o, water), Some(3));
        choose(&mut o, water, 1);
        assert_eq!(o.get("water"), Some("low"));
        let cam = row("camera");
        assert_eq!(current_index(&o, cam), Some(2));
        choose(&mut o, cam, 1);
        assert_eq!((o.get("camera"), o.get("camfeel")), (Some("modern"), Some("static")));
        assert_eq!(o.camera_byte(), 1);
        choose(&mut o, cam, 0);
        assert_eq!(o.camera_byte(), 0);
        let strafe = row("strafe");
        assert_eq!(current_index(&o, strafe), Some(0), "default = Raised");
        choose(&mut o, strafe, 1);
        assert_eq!((o.get("strafe"), o.extra("aimcam"), current_index(&o, strafe)), (Some("on"), Some("shoulder"), Some(1)));
        choose(&mut o, strafe, 2);
        assert_eq!((o.get("strafe"), o.extra("aimcam"), current_index(&o, strafe)), (Some("on"), Some("steady"), Some(2)));
        choose(&mut o, strafe, 3);
        assert_eq!((o.get("strafe"), current_index(&o, strafe)), (Some("off"), Some(3)));
        choose(&mut o, strafe, 0);
        assert_eq!((o.get("strafe"), o.extra("aimcam"), current_index(&o, strafe)), (Some("on"), None, Some(0)));
        let lang = row("lang");
        assert_eq!(current_index(&o, lang), None);
        choose(&mut o, lang, 2);
        assert_eq!(o.get("lang"), Some("de"));
        set_percent(&mut o, "camspeed", Some(73));
        assert_eq!(percent(&o, "camspeed"), Some(70));
        set_percent(&mut o, "mousespeed", Some(140));
        assert_eq!((percent(&o, "camspeed"), percent(&o, "mousespeed")), (Some(70), Some(140)), "two keys");
        set_percent(&mut o, "camspeed", None);
        assert_eq!(percent(&o, "camspeed"), None);
        assert_eq!(o.get("mousespeed"), Some("140"));
    }

    /// The rows kept as extra lines show what the game reads and write what the game writes (no line for the values it never writes).
    #[test]
    fn extra_line_rows_read_and_write_like_the_game() {
        let mut o = Options::parse("version=1\naspect=21x9\nfps=75\nupscaler=fsr1\nkeys=r2:Period\n");
        assert_eq!([current_index(&o, row("aspect")), current_index(&o, row("fps")), current_index(&o, row("upscaler"))], [Some(4), Some(2), Some(1)]);
        assert_eq!([current_index(&o, row("renderscale")), current_index(&o, row("glyphs")), current_index(&o, row("underwater"))], [Some(2), Some(0), Some(0)], "no line = the game's default");
        choose(&mut o, row("glyphs"), 1);
        choose(&mut o, row("stickdz"), 2);
        choose(&mut o, row("wheelmouse"), 1);
        choose(&mut o, row("upscaler"), 2);
        choose(&mut o, row("upscale_q"), 3);
        choose(&mut o, row("supporters"), 1);
        assert_eq!(o.to_text(), "version=1\naspect=21x9\nfps=75\nupscaler=dlss\nkeys=r2:Period\nglyphs=letters\nstickdz=minimal\nwheelmouse=on\nupscale_q=performance\nsupporters=on\n");
        choose(&mut o, row("glyphs"), 0);
        choose(&mut o, row("wheelmouse"), 0);
        choose(&mut o, row("fps"), 0);
        choose(&mut o, row("aspect"), 0);
        assert_eq!(o.to_text(), "version=1\naspect=auto\nupscaler=dlss\nkeys=r2:Period\nstickdz=minimal\nupscale_q=performance\nsupporters=on\n");
        // the 0.5.0 percent rows and HUD scale
        set_percent(&mut o, "lockon_strength", Some(44));
        set_percent(&mut o, "metal_reflect", Some(63));
        set_percent(&mut o, "hudscale", Some(122));
        assert_eq!([percent(&o, "lockon_strength"), percent(&o, "metal_reflect"), percent(&o, "hudscale")], [Some(40), Some(65), Some(120)]);
        set_percent(&mut o, "hudscale", None);
        set_percent(&mut o, "lockon_strength", None);
        assert_eq!([percent(&o, "lockon_strength"), percent(&o, "hudscale")], [None, None]);
        o.reset_all();
        assert_eq!(o.to_text(), "version=1\nkeys=r2:Period\n", "Reset to defaults clears every row, the controls stay");
    }

    #[test]
    fn culling_row_writes_the_marker_like_the_game_menu() {
        let mut o = Options::parse("version=1\nculling=off\n");
        assert_eq!((culling_index(&o, "0.4.0"), culling_index(&o, "0.5.0")), (1, 0), "a v0.4.0 leftover Off reads On from 0.4.1");
        choose_culling(&mut o, 1);
        assert_eq!(o.to_text(), "version=1\nculling=off\nculling_off=kept\n");
        assert_eq!((culling_index(&o, "0.4.0"), culling_index(&o, "0.5.0")), (1, 1));
        assert!(!super::super::presets::matches(&o, "0.5.0", super::super::presets::Preset::High), "Culling Off is Custom");
        choose_culling(&mut o, 0);
        assert_eq!(o.to_text(), "version=1\n");
    }

    #[test]
    fn rows_that_other_rows_switch_off_say_why() {
        let o = Options::new();
        assert!(inactive_note(&o, "upscale_q").is_some() && inactive_note(&o, "renderscale").is_none(), "Upscaler Off: quality has no use");
        assert!(inactive_note(&o, "metal_reflect").is_some(), "Metal is Off by default");
        assert!(inactive_note(&o, "lockon_strength").is_none() && inactive_note(&o, "underwater").is_none());
        let o = Options::parse("upscaler=dlss\nmetal=on\nlockon=off\nwater=classic\n");
        assert!(inactive_note(&o, "upscale_q").is_none() && inactive_note(&o, "renderscale").is_some());
        assert!(inactive_note(&o, "metal_reflect").is_none() && inactive_note(&o, "lockon_strength").is_some() && inactive_note(&o, "underwater").is_some());
    }
}
