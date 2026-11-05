//! The player's options file `player\pc_options.txt`, the same file the game's own Pause -> Options menu edits.
//! Mirrors `PcOptions::parse` / `to_text` of the game (rac-rs/src/retail/pcopts.rs): `version=1` first, only the keys the player set,
//! the same value names, unknown keys kept verbatim, UTF-8 without BOM, written through a temporary file and a rename.
//! Plus one key of the launcher's: `renderer=vulkan|dx12` (a missing key = vulkan).
//!
//! Values are kept as the canonical text of the file (for example `ao` = "high"); `get` gives the value the player set, `None` = never
//! set (the game's default applies).
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Keys in the order the game writes them (the launcher's `renderer` after `display`).
pub const KEYS: &[&str] = &[
    "lang", "camera", "camfeel", "strafe", "buyall", "lockon", "lockon_strength", "metal_reflect", "camspeed", "mousespeed", "lighting", "ao", "shadows", "display", "renderer", "water", "particles", "sunrays",
    "smoothing", "vegetation", "culling", "gi", "waves", "draw", "fog", "clouds", "fur", "furstyle", "fire", "smoke", "metal", "texfilter", "hud", "intro", "skipcut", "debug",
];

pub const LEVELS: [&str; 4] = ["off", "low", "medium", "high"];
pub const LEVELS_CLASSIC: [&str; 4] = ["classic", "low", "medium", "high"];
pub const DRAW: [&str; 3] = ["low", "medium", "high"];
pub const LOCKON: [&str; 3] = ["auto", "on", "off"];
pub const VEGETATION: [&str; 3] = ["off", "moderate", "lush"];
/// `intro=play` (the retail logos and movie, the default) or `intro=skip` (straight to the title screen).
pub const INTRO: [&str; 2] = ["play", "skip"];
pub const LANGS: [&str; 5] = ["en", "fr", "de", "es", "it"];
/// The Strafe aiming row's values (`Options::strafe_byte`) and the game's second key for it (a line the launcher keeps as an extra).
pub const STRAFE_RAISED: u8 = 0;
pub const STRAFE_SHOULDER: u8 = 1;
/// Steady (game 0.5.0): `strafe=on` + `aimcam=steady`, the camera stays as it is while strafing.
pub const STRAFE_STEADY: u8 = 2;
pub const STRAFE_OFF: u8 = 3;
pub const AIMCAM: &str = "aimcam";
pub const CAMSPEED_MIN: u32 = 10;
pub const CAMSPEED_MAX: u32 = 200;
/// Lock-on strength (`lockon_strength=0..100` in steps of 10, no key = 100) and Metal reflections (`metal_reflect=0..100` in steps of 5,
/// no key = 75), game 0.5.0.
pub const LOCKSTR_DEFAULT: u32 = 100;
pub const METAL_REFLECT_DEFAULT: u32 = 75;

/// The game's `flag()`: the two names plus true/1/yes and false/0/no.
fn flag(v: &str, yes: &str, no: &str) -> Option<bool> {
    let v = v.trim().to_ascii_lowercase();
    if v == yes || matches!(v.as_str(), "true" | "1" | "yes") {
        Some(true)
    } else if v == no || matches!(v.as_str(), "false" | "0" | "no") {
        Some(false)
    } else {
        None
    }
}

fn two(v: &str, yes: &'static str, no: &'static str) -> Option<String> {
    flag(v, yes, no).map(|b| if b { yes } else { no }.to_string())
}

fn level(v: &str, names: &[&'static str; 4]) -> Option<String> {
    let idx = match v.trim().to_ascii_lowercase().as_str() {
        "off" | "classic" | "false" | "0" | "no" => 0,
        "low" => 1,
        "medium" => 2,
        "high" | "on" | "enhanced" | "true" | "1" | "yes" => 3,
        _ => return None,
    };
    Some(names[idx].to_string())
}

fn one_of(v: &str, names: &[&str]) -> Option<String> {
    let v = v.trim().to_ascii_lowercase();
    names.iter().find(|n| **n == v).map(|n| n.to_string())
}

/// camera speed and mouse speed in percent: snapped to the grid of 10 inside 10..=200.
pub fn snap_camspeed(p: u32) -> u32 {
    ((p.min(255) + 5) / 10 * 10).clamp(CAMSPEED_MIN, CAMSPEED_MAX)
}

/// The game's snaps (pcopts.rs `snap_lockstr`, `snap_metal_refl`): nearest step, at most 100.
pub fn snap_lockstr(p: u32) -> u32 {
    ((p.min(255) + 5) / 10 * 10).min(100)
}

pub fn snap_metal_reflect(p: u32) -> u32 {
    ((p.min(255) + 2) / 5 * 5).min(100)
}

fn pct(v: &str) -> Option<u32> {
    v.trim().trim_end_matches('%').trim().parse::<u32>().ok()
}

/// The canonical value text of a known key, or None when the game could not read it (the key is then dropped, as the game does).
pub fn canon(key: &str, v: &str) -> Option<String> {
    match key {
        "lang" => one_of(v, &LANGS),
        "camera" => two(v, "modern", "classic"),
        "camfeel" => two(v, "smooth", "static"),
        "strafe" | "buyall" | "smoothing" | "culling" | "waves" | "debug" | "metal" | "skipcut" => two(v, "on", "off"),
        "lockon" => one_of(v, &LOCKON),
        "camspeed" | "mousespeed" => pct(v).map(|n| snap_camspeed(n).to_string()),
        "lockon_strength" => pct(v).map(|n| snap_lockstr(n).to_string()),
        "metal_reflect" => pct(v).map(|n| snap_metal_reflect(n).to_string()),
        "lighting" => match v.trim().to_ascii_lowercase().as_str() {
            "classic" | "original" => Some("classic".into()),
            "enhanced" | "modern" | "experimental" => Some("enhanced".into()),
            _ => None,
        },
        "ao" | "shadows" | "sunrays" | "gi" | "fur" | "fire" | "smoke" => level(v, &LEVELS),
        "furstyle" => {
            if v.trim().eq_ignore_ascii_case("long") { Some("fluffy".to_string()) } else { two(v, "fluffy", "soft") }
        }
        "water" | "particles" | "fog" | "clouds" => level(v, &LEVELS_CLASSIC),
        "display" => two(v, "fullscreen", "windowed"),
        "renderer" => one_of(v, &["vulkan", "dx12"]),
        "vegetation" => one_of(v, &VEGETATION),
        "draw" => one_of(v, &DRAW),
        "hud" => two(v, "sharp", "original"),
        "intro" => one_of(v, &INTRO),
        "texfilter" => two(v, "smooth", "classic"),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    known: HashMap<&'static str, String>,
    /// Lines this build does not know, kept verbatim (lower-case key, trimmed value), the last one of a key wins.
    extra: Vec<(String, String)>,
}

impl Options {
    pub fn new() -> Options {
        Options::default()
    }

    /// Tolerant: never fails; a missing or unreadable key gives that key's default.
    pub fn parse(text: &str) -> Options {
        let mut o = Options::default();
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}').trim();
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "version" {
                continue;
            }
            if let Some(key) = KEYS.iter().find(|n| **n == k) {
                match canon(key, v) {
                    Some(c) => {
                        o.known.insert(key, c);
                    }
                    None => {
                        o.known.remove(key);
                    }
                }
            } else if !k.is_empty() && !k.starts_with('#') {
                o.extra.retain(|(e, _)| *e != k);
                o.extra.push((k, v.to_string()));
            }
        }
        o
    }

    /// The value the player set (canonical text), None = never set.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.known.get(key).map(|s| s.as_str())
    }

    pub fn set(&mut self, key: &str, value: Option<&str>) {
        let Some(k) = KEYS.iter().find(|n| **n == key) else { return };
        match value.and_then(|v| canon(k, v)) {
            Some(c) => {
                self.known.insert(k, c);
            }
            None => {
                self.known.remove(k);
            }
        }
    }

    /// Removes every key the launcher shows (`aimcam` of the Strafe aiming row, the Culling Off marker and the extra lines of the
    /// Settings rows included); `debug` and unknown keys (controls, ...) stay.
    pub fn reset_all(&mut self) {
        let keep = self.known.get("debug").cloned();
        self.known.clear();
        if let Some(d) = keep {
            self.known.insert("debug", d);
        }
        self.remove_extra(AIMCAM);
        self.remove_extra(crate::ui::presets::CULLING_MARKER_KEY);
        for k in crate::pcopts_extra::row_keys() {
            self.remove_extra(k);
        }
    }

    /// Drops one line the launcher does not know (no-op when it is not there).
    pub fn remove_extra(&mut self, key: &str) {
        self.extra.retain(|(e, _)| e != key);
    }

    /// The value of a line the launcher does not know (kept untouched on save).
    pub fn extra(&self, key: &str) -> Option<&str> {
        self.extra.iter().find(|(e, _)| e == key).map(|(_, v)| v.as_str())
    }

    pub fn extra_keys(&self) -> impl Iterator<Item = &str> {
        self.extra.iter().map(|(k, _)| k.as_str())
    }

    /// Sets (Some) or removes (None) a line the launcher has no table row for; an existing line keeps its place, a new one goes last
    /// (the game writes such lines after its own keys, in the order they were set).
    pub fn set_extra(&mut self, key: &str, value: Option<&str>) {
        match (self.extra.iter().position(|(e, _)| e == key), value) {
            (Some(i), Some(v)) => self.extra[i].1 = v.to_string(),
            (Some(i), None) => {
                self.extra.remove(i);
            }
            (None, Some(v)) => self.extra.push((key.to_string(), v.to_string())),
            (None, None) => {}
        }
    }

    /// The Strafe aiming row: 0 Raised, 1 Shoulder (game 0.4.2), 2 Steady (game 0.5.0), 3 Off. The game's two keys: `strafe=on|off`
    /// (as in every version; Off = off) plus `aimcam=shoulder` / `aimcam=steady`. Nothing set = Raised, the game's default. As the game
    /// reads them (`pcinput::strafe_row`, `modcam::steady::from_opts`): strafe off wins, then `aimcam` (any case), anything else is Raised.
    pub fn strafe_byte(&self) -> u8 {
        let aimcam = |v: &str| self.extra(AIMCAM).is_some_and(|a| a.trim().eq_ignore_ascii_case(v));
        if self.get("strafe") == Some("off") {
            STRAFE_OFF
        } else if aimcam("shoulder") {
            STRAFE_SHOULDER
        } else if aimcam("steady") {
            STRAFE_STEADY
        } else {
            STRAFE_RAISED
        }
    }

    /// As the game's menu does (`pcinput::set_strafe_row`, `modcam::steady::row_set`): Raised = strafe on, no `aimcam`; Shoulder / Steady
    /// = strafe on + `aimcam=shoulder` / `aimcam=steady`; Off = strafe off and a remembered `aimcam` stays (switching strafe back on gives
    /// Shoulder or Steady again).
    pub fn set_strafe_byte(&mut self, b: u8) {
        match b {
            STRAFE_SHOULDER | STRAFE_STEADY => {
                self.set("strafe", Some("on"));
                self.set_extra(AIMCAM, Some(if b == STRAFE_STEADY { "steady" } else { "shoulder" }));
            }
            STRAFE_OFF => self.set("strafe", Some("off")),
            _ => {
                self.set("strafe", Some("on"));
                self.set_extra(AIMCAM, None);
            }
        }
    }

    /// The Camera row: 0 Classic, 1 Static, 2 Smooth (camera=modern + camfeel=smooth is the default = 2).
    pub fn camera_byte(&self) -> u8 {
        let modern = self.get("camera").map(|c| c == "modern").unwrap_or(true);
        let smooth = self.get("camfeel").map(|c| c == "smooth").unwrap_or(true);
        match (modern, smooth) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        }
    }

    /// As the game's menu does: Classic keeps the remembered feel; only Static is remembered as a feel (Smooth is the default, no key).
    pub fn set_camera_byte(&mut self, b: u8) {
        self.set("camera", Some(if b != 0 { "modern" } else { "classic" }));
        if b != 0 {
            self.set("camfeel", if b < 2 { Some("static") } else { None });
        }
    }

    /// The file text: only the keys the player set, then the unknown ones.
    pub fn to_text(&self) -> String {
        let mut s = String::from("version=1\n");
        for key in KEYS {
            let Some(v) = self.known.get(key) else { continue };
            // the game writes camfeel only as `static` and debug only as `on` (the other value is the default)
            if (*key == "camfeel" && v != "static") || (*key == "debug" && v != "on") {
                continue;
            }
            s += &format!("{key}={v}\n");
        }
        for (k, v) in &self.extra {
            s += &format!("{k}={v}\n");
        }
        s
    }

    pub fn load(path: &Path) -> Options {
        std::fs::read(path).map(|b| Options::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
    }

    /// Writes the file atomically (temporary file + rename), UTF-8 without BOM.
    pub fn store(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_writes_only_the_version_line() {
        assert_eq!(Options::new().to_text(), "version=1\n");
        assert_eq!(Options::parse("").to_text(), "version=1\n");
    }

    #[test]
    fn the_v05_keys_are_known_and_written_as_the_game_writes_them() {
        let o = Options::parse("version=1
fur=Medium
furstyle=long
fire=low
smoke=off
metal=on
future=1
");
        assert_eq!((o.get("fur"), o.get("furstyle"), o.get("fire"), o.get("smoke"), o.get("metal")), (Some("medium"), Some("fluffy"), Some("low"), Some("off"), Some("on")));
        assert_eq!(o.to_text(), "version=1
fur=medium
furstyle=fluffy
fire=low
smoke=off
metal=on
future=1
", "known keys in the game's order, unknown last");
        assert_eq!(Options::parse("fur=classic
metal=maybe
").get("metal"), None);
    }

    #[test]
    fn round_trip_keeps_unknown_keys_and_order() {
        let text = "version=1\nlang=de\ncamera=modern\ncamfeel=static\nstrafe=off\nlockon=off\ncamspeed=70\nmousespeed=120\nlighting=enhanced\nao=low\nshadows=medium\ndisplay=fullscreen\nwater=classic\nparticles=high\nsunrays=off\nsmoothing=off\nvegetation=lush\ngi=medium\nwaves=off\ndraw=high\nhud=original\nfuture_thing=7\nkeys=cross:KeyK\n";
        let o = Options::parse(text);
        assert_eq!(o.to_text(), text);
        assert_eq!(o.extra_keys().collect::<Vec<_>>(), vec!["future_thing", "keys"]);
    }

    #[test]
    fn old_names_are_rewritten_like_the_game_does() {
        let o = Options::parse("lighting=Modern\nao=ON\nshadows=off\nwater=enhanced\n");
        assert_eq!(o.to_text(), "version=1\nlighting=enhanced\nao=high\nshadows=off\nwater=high\n");
        assert_eq!(Options::parse("lighting=original\n").to_text(), "version=1\nlighting=classic\n");
    }

    #[test]
    fn unreadable_values_give_the_default_and_the_key_is_dropped() {
        let o = Options::parse("version=1\nao=banana\ncamspeed=abc\nlockon=maybe\n");
        assert_eq!(o.to_text(), "version=1\n");
    }

    #[test]
    fn camera_row_matches_the_game_menu() {
        let mut o = Options::new();
        assert_eq!(o.camera_byte(), 2);
        o.set_camera_byte(1);
        assert_eq!(o.to_text(), "version=1\ncamera=modern\ncamfeel=static\n");
        o.set_camera_byte(0);
        assert_eq!(o.to_text(), "version=1\ncamera=classic\ncamfeel=static\n");
        assert_eq!(o.camera_byte(), 0);
        o.set_camera_byte(2);
        assert_eq!(o.to_text(), "version=1\ncamera=modern\n", "smooth = default: no camfeel key");
    }

    /// The Strafe aiming row (Raised / Shoulder / Off) and the two keys of the game: `strafe=on|off` and `aimcam=shoulder`
    /// (rac-rs/src/retail/pcinput.rs `strafe_row` / `set_strafe_row`, same table).
    #[test]
    fn strafe_row_maps_to_the_games_two_keys() {
        let row = |t: &str| Options::parse(t).strafe_byte();
        // reading: nothing set / strafe=on = Raised (the default); aimcam=shoulder = Shoulder; strafe=off wins over aimcam; other aimcam = Raised
        assert_eq!([row(""), row("strafe=on"), row("strafe=off"), row("aimcam=level"), row("strafe=on\naimcam=shoulder"), row("aimcam=SHOULDER")], [0, 0, 3, 0, 1, 1]);
        assert_eq!(row("strafe=off\naimcam=shoulder"), STRAFE_OFF);
        // Steady (game 0.5.0): aimcam=steady; Off keeps it, Raised removes it
        assert_eq!([row("aimcam=Steady"), row("strafe=off\naimcam=steady")], [STRAFE_STEADY, STRAFE_OFF]);
        let mut s = Options::new();
        s.set_strafe_byte(STRAFE_STEADY);
        assert_eq!(s.to_text(), "version=1\nstrafe=on\naimcam=steady\n");
        s.set_strafe_byte(STRAFE_OFF);
        s.set_strafe_byte(STRAFE_STEADY);
        assert_eq!(s.to_text(), "version=1\nstrafe=on\naimcam=steady\n");
        s.set_strafe_byte(STRAFE_RAISED);
        assert_eq!(s.to_text(), "version=1\nstrafe=on\n");
        // writing: Shoulder, then Off keeps aimcam (the game does the same), then Raised removes it
        let mut o = Options::parse("version=1\nlang=de\nfuture=1\n");
        o.set_strafe_byte(STRAFE_SHOULDER);
        assert_eq!(o.to_text(), "version=1\nlang=de\nstrafe=on\nfuture=1\naimcam=shoulder\n");
        assert_eq!(o.strafe_byte(), STRAFE_SHOULDER);
        o.set_strafe_byte(STRAFE_OFF);
        assert_eq!(o.to_text(), "version=1\nlang=de\nstrafe=off\nfuture=1\naimcam=shoulder\n");
        assert_eq!(o.strafe_byte(), STRAFE_OFF);
        o.set_strafe_byte(STRAFE_RAISED);
        assert_eq!(o.to_text(), "version=1\nlang=de\nstrafe=on\nfuture=1\n");
        assert_eq!(o.strafe_byte(), STRAFE_RAISED);
        // an aimcam line the file already has keeps its place; Reset to defaults takes the row's keys away and keeps unknown lines
        let mut o = Options::parse("version=1\naimcam=level\nzzz=1\nhudscale=120\nglyphs=letters\nkeys=r2:Period\nculling=off\nculling_off=kept\n");
        o.set_strafe_byte(STRAFE_SHOULDER);
        assert_eq!(o.to_text(), "version=1\nstrafe=on\nculling=off\naimcam=shoulder\nzzz=1\nhudscale=120\nglyphs=letters\nkeys=r2:Period\nculling_off=kept\n");
        o.reset_all();
        assert_eq!(o.to_text(), "version=1\nzzz=1\nkeys=r2:Period\n", "the Settings rows' lines and the Culling marker go, the controls stay");
        assert_eq!(o.strafe_byte(), STRAFE_RAISED);
    }

    #[test]
    fn camspeed_snaps_and_debug_only_written_when_on() {
        assert_eq!(Options::parse("camspeed=73%\n").to_text(), "version=1\ncamspeed=70\n");
        assert_eq!(Options::parse("camspeed=3\n").to_text(), "version=1\ncamspeed=10\n");
        assert_eq!(Options::parse("camspeed=999\n").to_text(), "version=1\ncamspeed=200\n");
        assert_eq!(Options::parse("mousespeed=86%\ncamspeed=150\n").to_text(), "version=1\ncamspeed=150\nmousespeed=90\n");
        assert_eq!(Options::parse("debug=on\n").to_text(), "version=1\ndebug=on\n");
        assert_eq!(Options::parse("debug=off\n").to_text(), "version=1\n");
    }

    /// Lock-on strength, Metal reflections (0.5.0) and Start skips cutscenes (0.4.1): the game's own keys, snapped as the game does,
    /// written in its order (after `lockon`, after `intro`).
    #[test]
    fn lockon_strength_metal_reflect_and_skipcut() {
        let o = Options::parse("skipcut=OFF\nmetal_reflect=63%\nlockon_strength=44\nlockon=on\n");
        assert_eq!((o.get("lockon_strength"), o.get("metal_reflect"), o.get("skipcut")), (Some("40"), Some("65"), Some("off")));
        assert_eq!(o.to_text(), "version=1\nlockon=on\nlockon_strength=40\nmetal_reflect=65\nskipcut=off\n");
        assert_eq!(Options::parse("lockon_strength=250\nmetal_reflect=999\n").to_text(), "version=1\nlockon_strength=100\nmetal_reflect=100\n");
        assert_eq!(Options::parse("lockon_strength=x\nskipcut=maybe\n").to_text(), "version=1\n");
        assert_eq!([snap_lockstr(4), snap_lockstr(5), snap_metal_reflect(2), snap_metal_reflect(3)], [0, 10, 0, 5]);
    }

    #[test]
    fn renderer_key_and_set_clear() {
        let mut o = Options::parse("version=1\nzzz=1\n");
        o.set("renderer", Some("dx12"));
        assert_eq!(o.get("renderer"), Some("dx12"));
        assert_eq!(o.to_text(), "version=1\nrenderer=dx12\nzzz=1\n");
        o.set("renderer", None);
        assert_eq!(o.to_text(), "version=1\nzzz=1\n");
        o.set("renderer", Some("opengl"));
        assert_eq!(o.get("renderer"), None, "unknown value: not stored");
    }

    #[test]
    fn intro_key_is_play_or_skip_and_unknown_lines_survive() {
        let o = Options::parse("version=1\nintro=SKIP\nkeys=r2:Period\nfoo=1\n");
        assert_eq!(o.get("intro"), Some("skip"));
        assert_eq!(o.to_text(), "version=1\nintro=skip\nkeys=r2:Period\nfoo=1\n");
        let mut o = Options::parse("version=1\nhud=original\ndebug=on\nkeys=r2:Period\n");
        o.set("intro", Some("play"));
        assert_eq!(o.to_text(), "version=1\nhud=original\nintro=play\ndebug=on\nkeys=r2:Period\n");
        o.set("intro", Some("maybe"));
        assert_eq!(o.get("intro"), None, "unknown value: dropped");
        assert_eq!(Options::parse("intro=banana\n").to_text(), "version=1\n");
    }

    #[test]
    fn bom_and_crlf_are_tolerated_and_output_has_no_bom() {
        let o = Options::parse("\u{feff}version=1\r\nlang=fr\r\n");
        assert_eq!(o.get("lang"), Some("fr"));
        assert!(!o.to_text().starts_with('\u{feff}'));
    }

    #[test]
    fn store_and_load_through_a_file() {
        let dir = std::env::temp_dir().join(format!("racl_opts_{}", std::process::id()));
        let p = dir.join("player").join("pc_options.txt");
        let mut o = Options::parse("version=1\nfoo=bar\n");
        o.set("lang", Some("it"));
        o.store(&p).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_ne!(b[0], 0xEF);
        assert_eq!(Options::load(&p), o);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn real_game_file_round_trips() {
        // a pc_options.txt written by the game itself (RAC_TEST_PC_OPTIONS), skipped when not set
        let Some(p) = std::env::var_os("RAC_TEST_PC_OPTIONS") else { return };
        if let Ok(t) = std::fs::read_to_string(p) {
            let o = Options::parse(&t);
            assert_eq!(Options::parse(&o.to_text()), o);
        }
    }
}
