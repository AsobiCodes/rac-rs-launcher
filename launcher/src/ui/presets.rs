//! Graphics presets of the Settings page: Low, Medium, High and Custom.
//!
//! A preset is a fixed set of values for the picture options (one table, `TABLE`). High is the game's own default (the picture the game
//! shows with no pc_options.txt); Low and Medium turn the costly options down. Only picture quality options are in the table: camera,
//! controls, language, window mode and the graphics system (Vulkan / DirectX 12) are never touched by a preset.
//!
//! The game version decides what a preset can set (`gating`): options the version does not know are skipped, and a level the version does
//! not know (Water / Particles Low and Medium before 0.3.0) becomes the next lower level it knows. Custom is not a set of values: it is what
//! the selector shows when the options match none of the presets.
use super::rows;
use crate::gating;
use crate::pcopts::Options;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Low,
    Medium,
    High,
    Custom,
}

impl Preset {
    /// Selector order.
    pub const ALL: [Preset; 4] = [Preset::Low, Preset::Medium, Preset::High, Preset::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Low => "Low",
            Preset::Medium => "Medium",
            Preset::High => "High",
            Preset::Custom => "Custom",
        }
    }

    /// Index into a `TABLE` row (Custom has no values).
    fn column(self) -> Option<usize> {
        match self {
            Preset::Low => Some(0),
            Preset::Medium => Some(1),
            Preset::High => Some(2),
            Preset::Custom => None,
        }
    }
}

/// THE PRESET TABLE: (pc_options.txt key, [Low, Medium, High]). High must stay equal to the game's defaults (a test checks it against the
/// rows of the Settings page). Lighting stays Classic everywhere; the Enhanced effects below it only show once Lighting is Enhanced, and a
/// preset sets them too, so "Low + Lighting Enhanced" is a light Enhanced picture.
pub const TABLE: &[(&str, [&str; 3])] = &[
    //  key            Low         Medium      High (= default)
    ("lighting", ["classic", "classic", "classic"]),
    ("draw", ["low", "low", "medium"]),
    ("smoothing", ["off", "on", "on"]),
    ("hud", ["original", "sharp", "sharp"]),
    ("ao", ["low", "medium", "high"]),
    ("shadows", ["low", "medium", "high"]),
    ("water", ["low", "medium", "high"]),
    ("particles", ["low", "medium", "high"]),
    ("sunrays", ["low", "medium", "high"]),
    ("vegetation", ["off", "moderate", "moderate"]),
    ("gi", ["off", "off", "off"]),
    ("waves", ["off", "on", "on"]),
    ("fog", ["classic", "low", "high"]),
    ("clouds", ["classic", "low", "high"]),
    ("texfilter", ["smooth", "smooth", "smooth"]),
    ("fur", ["off", "low", "high"]),
    ("fire", ["off", "low", "high"]),
    ("smoke", ["off", "low", "high"]),
];

/// Culling (the game skips drawing what is out of view) is On in every preset. It has no row on the Settings page and no column in `TABLE`:
/// the game reads a missing key as On, so a preset removes the key (also a hand-written `culling=off`), like any other value equal to the default.
const CULLING: &str = "culling";
/// MARKER KEY of a deliberate Culling Off (v0.4.1 game: `culling=off` without this line is read as On). TO MATCH THE GAME'S FINAL KEY: change
/// only this constant. The launcher has no Culling control, so it never writes the marker; it keeps the line untouched on save (an unknown
/// line) and a preset, which sets Culling On, removes it.
pub const CULLING_MARKER_KEY: &str = "culling_off";
/// The first game version that reads a `culling=off` WITHOUT the marker as On (the one-time v0.4.0 reset). An install's VERSION is
/// always a plain x.y.z (`games::local_version` turns anything else into 0.1.0, so an "-rc" name never gets here). Older games (0.3.0 .. 0.4.0) take `culling=off` as a real Off.
pub(super) const CULLING_RESET_VERSION: &str = "0.4.1";

/// True when the game at `version` runs the options with Culling Off: `culling=off`, and, from the reset version on, the marker too.
/// The game itself rewrites a marker-less Off as On at its next start; until then the launcher reads it the same way.
pub fn culling_off(o: &Options, version: &str) -> bool {
    if !gating::available(version, CULLING) || o.get(CULLING) != Some("off") {
        return false;
    }
    crate::update::compare_version(version, CULLING_RESET_VERSION) == std::cmp::Ordering::Less
        || o.extra(CULLING_MARKER_KEY).is_some_and(|v| v.eq_ignore_ascii_case("kept"))
}

fn default_of(key: &str) -> Option<&'static str> {
    rows::ROWS.iter().find(|r| r.key == key).map(|r| r.default)
}

/// The keys whose missing entry reads as the level of the preset the other rows match, not as the row default. The game has the same
/// rule for the same three keys (rac-rs retail/pcext/furopt.rs `MISSING_KEY_FOLLOWS_PRESET = true`, `rows`, and presets.rs
/// `effects_of_matching`), so a pc_options.txt from before the rows keeps its Preset and the game shows no surprise.
const FOLLOWS_PRESET: [&str; 3] = ["fur", "fire", "smoke"];

/// What the game reads for `key`: the file's value; else, for fur / fire / smoke, the TABLE value of the first of High, Medium, Low whose
/// other TABLE keys (these three left out) match the file, Off when none does; else the row's default. ONE reading for the Settings
/// rows (`rows::current_index`) and the Preset selector (`matches`), so both show what the game uses. No game-version gating here: the
/// three keys exist from 0.5.0, which knows every other key (an older version never asks, `matches` skips them).
pub fn value_of<'a>(o: &'a Options, key: &str) -> Option<&'a str> {
    if let Some(v) = o.get(key) {
        return Some(v);
    }
    if FOLLOWS_PRESET.contains(&key) {
        // the game's preset match also needs Culling On; the three keys exist from 0.5.0, which reads Culling as every game from the
        // reset version on does (`culling=off` counts only with the marker)
        let col = (!culling_off(o, CULLING_RESET_VERSION)).then(|| [2usize, 1, 0].into_iter().find(|&c| TABLE.iter().all(|(k, v)| FOLLOWS_PRESET.contains(k) || value_of(o, k) == Some(v[c])))).flatten();
        let own = col.and_then(|c| TABLE.iter().find(|(k, _)| *k == key).map(|(_, v)| v[c]));
        return Some(own.unwrap_or("off"));
    }
    default_of(key)
}

/// The levels of Water and Particles, lowest first (the only values the gating table knows per version).
const SCALE: [&str; 4] = ["classic", "low", "medium", "high"];

/// What `value` of `key` becomes on this game version: None = the version does not know the option (a preset skips it); a level it does
/// not know is replaced by the next lower one it knows.
fn effective(version: &str, key: &str, value: &'static str) -> Option<&'static str> {
    if !gating::available(version, key) {
        return None;
    }
    let Some(mut i) = SCALE.iter().position(|s| *s == value) else { return Some(value) };
    while i > 0 && !gating::value_available(version, key, SCALE[i]) {
        i -= 1;
    }
    Some(SCALE[i])
}

/// Sets the options of `preset` (Custom changes nothing), and Culling On (the key is removed). A value equal to the game's default removes the key, so High leaves the file as
/// it would be with no options at all for these keys.
pub fn apply(o: &mut Options, version: &str, preset: Preset) {
    let Some(col) = preset.column() else { return };
    for (key, values) in TABLE {
        let Some(v) = effective(version, key, values[col]) else { continue };
        o.set(key, if default_of(key) == Some(v) { None } else { Some(v) });
    }
    if gating::available(version, CULLING) {
        o.set(CULLING, None);
        o.remove_extra(CULLING_MARKER_KEY);
    }
}

/// Call BEFORE the player changes a picture row (a key of `TABLE`): writes the Fur / Fire / Smoke values the page shows now as explicit
/// keys, for the keys the file lacks. Without them the change could shift what is shown (a missing key follows the preset the other rows
/// match, so changing one row turns Fur / Fire / Smoke Off), while the game writes the three keys itself when it saves. Only for a game
/// version that knows the keys: an older game ignores them, and a pinned `fur=off` would outlive a later High preset on that install.
pub fn pin_before_change(o: &mut Options, version: &str, changed_key: &str) {
    if !TABLE.iter().any(|(k, _)| *k == changed_key) {
        return;
    }
    for key in FOLLOWS_PRESET {
        if o.get(key).is_none() && gating::available(version, key) {
            let shown = value_of(o, key).map(str::to_string);
            o.set(key, shown.as_deref());
        }
    }
}

/// True when every option of the preset that this game version knows has the preset's value (a key never set counts as the default; a
/// file that runs Culling Off, see `culling_off`, matches no preset). Fur / Fire / Smoke read as the game reads them (`value_of`).
pub fn matches(o: &Options, version: &str, preset: Preset) -> bool {
    let Some(col) = preset.column() else { return false };
    !culling_off(o, version)
        && TABLE.iter().all(|(key, values)| match effective(version, key, values[col]) {
            Some(v) => value_of(o, key) == Some(v),
            None => true,
        })
}

/// The preset the options are: High first (the game's default), then Medium, then Low; Custom when none matches.
pub fn detect(o: &Options, version: &str) -> Preset {
    [Preset::High, Preset::Medium, Preset::Low].into_iter().find(|p| matches(o, version, *p)).unwrap_or(Preset::Custom)
}

/// The table as a plain data block, one `<preset>.<key>=<value>` line each (low, medium, high). The game can ship the same block as a file;
/// `TABLE` stays the one place the preset code reads its values from.
pub fn table_text() -> String {
    let mut out = String::new();
    for (col, name) in ["low", "medium", "high"].into_iter().enumerate() {
        for (key, values) in TABLE {
            out += &format!("{name}.{key}={}
", values[col]);
        }
    }
    out
}

/// The line under the selector when this game version knows only some of the options (None = it knows them all).
pub fn version_note(version: &str) -> Option<String> {
    let all_known = TABLE.iter().all(|(key, values)| values.iter().all(|v| effective(version, key, v) == Some(*v)));
    (!all_known).then(|| format!("Game version {version} knows only some of these options, so a preset sets just those."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::rows::{Group, Kind};

    fn keys() -> Vec<&'static str> {
        TABLE.iter().map(|(k, _)| *k).collect()
    }

    #[test]
    fn high_is_the_game_default_and_every_value_is_one_the_rows_offer() {
        for (key, values) in TABLE {
            let row = rows::ROWS.iter().find(|r| r.key == *key).unwrap_or_else(|| panic!("{key} has no row"));
            assert_eq!(values[2], row.default, "High of {key} must be the game's default");
            let Kind::Choice(list) = row.kind else { panic!("{key} is not a choice row") };
            for v in values {
                assert!(list.iter().any(|(lv, _)| lv == v), "{key}={v} is not a choice of its row");
                assert_eq!(crate::pcopts::canon(key, v).as_deref(), Some(*v), "{key}={v} is not written the way the game reads it");
            }
        }
    }

    #[test]
    fn only_picture_quality_options_are_in_the_table() {
        for k in keys() {
            let row = rows::ROWS.iter().find(|r| r.key == k).unwrap();
            assert!(matches!(row.group, Group::Graphics | Group::Display | Group::Enhanced), "{k}");
        }
        // never part of a preset (assets/gfx_presets.txt): the game's preset row leaves these as they are
        let never_v05 = ["aspect", "renderscale", "upscaler", "upscale_q", "fps", "hudpos", "hudscale", "underwater", "metal", "metal_reflect", "supporters", "culling", "lockon_strength", "furstyle"];
        for never in ["camera", "camfeel", "camspeed", "mousespeed", "strafe", "lockon", "buyall", "lang", "renderer", "display", "debug", "keys", "pad"].into_iter().chain(never_v05) {
            assert!(!keys().contains(&never), "{never} must not be part of a preset");
        }
        let mut seen = keys();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), TABLE.len(), "a key twice in the table");
    }

    #[test]
    fn no_options_file_is_high_and_high_writes_nothing() {
        let o = Options::new();
        assert_eq!(detect(&o, "0.4.0"), Preset::High);
        let mut p = Options::parse("version=1\nao=low\ndraw=low\nwater=classic\n");
        apply(&mut p, "0.4.0", Preset::High);
        assert_eq!(p.to_text(), "version=1\n", "High removes the keys: the game's own defaults");
        assert_eq!(detect(&p, "0.4.0"), Preset::High);
        // the same defaults written out are still High
        assert_eq!(detect(&Options::parse("ao=high\nshadows=high\nwater=high\nhud=sharp\nwaves=on\n"), "0.4.0"), Preset::High);
    }

    #[test]
    fn low_and_medium_lower_the_costly_options_and_are_found_again() {
        let mut o = Options::new();
        apply(&mut o, "0.4.0", Preset::Low);
        assert_eq!(o.get("draw"), Some("low"));
        assert_eq!((o.get("smoothing"), o.get("hud"), o.get("waves"), o.get("vegetation")), (Some("off"), Some("original"), Some("off"), Some("off")));
        assert_eq!((o.get("ao"), o.get("water"), o.get("particles")), (Some("low"), Some("low"), Some("low")));
        assert_eq!(detect(&o, "0.4.0"), Preset::Low);
        apply(&mut o, "0.4.0", Preset::Medium);
        assert_eq!((o.get("draw"), o.get("ao"), o.get("water")), (Some("low"), Some("medium"), Some("medium")));
        assert_eq!(o.get("smoothing"), None, "default On: no key");
        assert_eq!(detect(&o, "0.4.0"), Preset::Medium);
        apply(&mut o, "0.4.0", Preset::High);
        assert_eq!(detect(&o, "0.4.0"), Preset::High);
        // each step is a real change
        let (mut l, mut m) = (Options::new(), Options::new());
        apply(&mut l, "0.4.0", Preset::Low);
        apply(&mut m, "0.4.0", Preset::Medium);
        assert_ne!(l.to_text(), m.to_text());
        assert_ne!(m.to_text(), Options::new().to_text());
    }

    #[test]
    fn changing_one_value_afterwards_makes_it_custom_and_a_preset_never_touches_other_options() {
        let mut o = Options::parse("version=1\nlang=de\ncamera=classic\nstrafe=off\nlockon=on\ncamspeed=70\nrenderer=dx12\ndisplay=fullscreen\ndebug=on\nkeys=cross:KeyK\nfuture=1\n");
        let before = o.to_text();
        apply(&mut o, "0.4.0", Preset::High);
        assert_eq!(o.to_text(), before, "High on an options file with only other options changes nothing");
        apply(&mut o, "0.4.0", Preset::Low);
        for line in before.lines() {
            assert!(o.to_text().lines().any(|l| l == line), "{line} lost");
        }
        assert_eq!(detect(&o, "0.4.0"), Preset::Low);
        // other options do not make it Custom
        o.set("renderer", Some("vulkan"));
        o.set("display", Some("windowed"));
        assert_eq!(detect(&o, "0.4.0"), Preset::Low);
        // one picture option does
        let ao = rows::ROWS.iter().find(|r| r.key == "ao").unwrap();
        rows::choose(&mut o, ao, 3);
        assert_eq!(detect(&o, "0.4.0"), Preset::Custom);
        assert!(!matches(&o, "0.4.0", Preset::Custom), "Custom is not a set of values");
        let mut c = o.clone();
        apply(&mut c, "0.4.0", Preset::Custom);
        assert_eq!(c, o, "applying Custom does nothing");
        // Lighting is part of the presets
        let mut e = Options::new();
        e.set("lighting", Some("enhanced"));
        assert_eq!(detect(&e, "0.4.0"), Preset::Custom);
    }

    #[test]
    fn older_game_versions_get_only_what_they_know() {
        // 0.2.2 knows lighting ao shadows water particles sunrays, and Water / Particles only as classic or high
        let mut o = Options::new();
        apply(&mut o, "0.2.2", Preset::Low);
        let t = o.to_text();
        assert_eq!((o.get("water"), o.get("particles")), (Some("classic"), Some("classic")), "{t}");
        assert_eq!((o.get("ao"), o.get("shadows"), o.get("sunrays")), (Some("low"), Some("low"), Some("low")));
        for key in ["draw", "smoothing", "hud", "vegetation", "gi", "waves"] {
            assert_eq!(o.get(key), None, "{key} is unknown to 0.2.2: {t}");
        }
        assert_eq!(detect(&o, "0.2.2"), Preset::Low);
        apply(&mut o, "0.2.2", Preset::Medium);
        assert_eq!((o.get("water"), o.get("particles"), o.get("ao")), (Some("classic"), Some("classic"), Some("medium")));
        assert_eq!(detect(&o, "0.2.2"), Preset::Medium);
        // 0.3.0 knows every level and every key except fog (0.4.0)
        let mut p = Options::new();
        apply(&mut p, "0.3.0", Preset::Medium);
        assert_eq!((p.get("water"), p.get("draw")), (Some("medium"), Some("low")));
        // the note
        assert!(version_note("0.2.2").unwrap().contains("0.2.2"));
        assert_eq!(p.get("fog"), None, "fog is unknown to 0.3.0");
        assert!(version_note("0.5.0").is_none() && version_note("0.4.0").is_some() && version_note("0.3.0").is_some());
    }

    #[test]
    fn fur_fire_and_smoke_follow_the_preset_from_version_050_and_stay_out_before() {
        let mut o = Options::new();
        apply(&mut o, "0.5.0", Preset::Low);
        assert_eq!((o.get("fur"), o.get("fire"), o.get("smoke")), (Some("off"), Some("off"), Some("off")));
        assert_eq!(detect(&o, "0.5.0"), Preset::Low);
        apply(&mut o, "0.5.0", Preset::Medium);
        assert_eq!((o.get("fur"), o.get("fire"), o.get("smoke")), (Some("low"), Some("low"), Some("low")));
        assert_eq!(detect(&o, "0.5.0"), Preset::Medium);
        apply(&mut o, "0.5.0", Preset::High);
        assert_eq!((o.get("fur"), o.get("fire"), o.get("smoke")), (None, None, None), "High is the default: no keys");
        assert_eq!(detect(&o, "0.5.0"), Preset::High);
        // a preset leaves the Fur style alone, and an older game version never gets the three keys
        let mut s = Options::parse("furstyle=fluffy
");
        apply(&mut s, "0.5.0", Preset::Low);
        assert_eq!(s.get("furstyle"), Some("fluffy"));
        let mut old = Options::new();
        apply(&mut old, "0.4.0", Preset::Low);
        assert_eq!((old.get("fur"), old.get("fire"), old.get("smoke")), (None, None, None));
        // fur=off with the other rows at High is Custom
        let c = Options::parse("fur=off
");
        assert_eq!(detect(&c, "0.5.0"), Preset::Custom);
    }

    /// A pc_options.txt body of a game before the fur / fire / smoke rows: every table line of one preset except those three keys (the
    /// same file the game's test builds from assets/gfx_presets.txt, furopt.rs `a_file_without_the_key_follows_its_preset`).
    fn v04_file(preset: &str) -> String {
        let skip = ["fur=", "fire=", "smoke="];
        let pre = format!("{preset}.");
        table_text().lines().filter_map(|l| l.strip_prefix(&pre)).filter(|l| !skip.iter().any(|s| l.starts_with(s))).map(|l| format!("{l}\n")).collect()
    }

    /// What the Settings rows mark for Fur, Fire and Smoke, as the choice's file value.
    fn shown(o: &Options) -> [&'static str; 3] {
        ["fur", "fire", "smoke"].map(|k| {
            let row = rows::ROWS.iter().find(|r| r.key == k).unwrap();
            let Kind::Choice(list) = row.kind else { panic!("{k} is a choice row") };
            list[rows::current_index(o, row).unwrap()].0
        })
    }

    /// REVIEW_B2 B12: a file without the fur / fire / smoke keys shows what the game uses (same cases and levels as the game's test:
    /// 0 Off, 1 Low, 2 Medium, 3 High). Level words here: off / low / medium / high.
    #[test]
    fn missing_fur_fire_smoke_keys_read_as_the_game_reads_them() {
        let v = "0.5.0";
        // No file at all: the High preset, so High / High / High.
        let none = Options::new();
        assert_eq!((shown(&none), detect(&none, v)), (["high"; 3], Preset::High));
        // A v0.4 Low file (every Low value written, none of the three): Preset Low, Fur / Fire / Smoke Off.
        let low = Options::parse(&v04_file("low"));
        assert_eq!((shown(&low), detect(&low, v)), (["off"; 3], Preset::Low));
        // A v0.4 Medium file: Preset Medium, the three Low.
        let med = Options::parse(&v04_file("medium"));
        assert_eq!((shown(&med), detect(&med, v)), (["low"; 3], Preset::Medium));
        // A v0.4 High file with every value written: Preset High, the three High.
        let high = Options::parse(&v04_file("high"));
        assert_eq!((shown(&high), detect(&high, v)), (["high"; 3], Preset::High));
        // A custom file (rows that match no preset): the three Off, and the Preset is Custom.
        let custom = Options::parse("water=low\nshadows=high\nsunrays=medium\n");
        assert_eq!((shown(&custom), detect(&custom, v)), (["off"; 3], Preset::Custom));
        // The file the launcher itself writes for a v0.4 game (the three keys are gated off): the same as a v0.4 file.
        for (p, want) in [(Preset::Low, "off"), (Preset::Medium, "low"), (Preset::High, "high")] {
            let mut o = Options::new();
            apply(&mut o, "0.4.0", p);
            assert_eq!((shown(&o), detect(&o, v)), ([want; 3], p), "{p:?} written for 0.4.0");
        }
    }

    #[test]
    fn a_key_in_the_file_wins_and_a_missing_one_still_follows_the_preset() {
        let v = "0.5.0";
        for (preset, p, level) in [("low", Preset::Low, "off"), ("medium", Preset::Medium, "low"), ("high", Preset::High, "high")] {
            let text = v04_file(preset);
            assert_eq!(shown(&Options::parse(&format!("{text}fur=low\n"))), ["low", level, level], "{preset}: fur only");
            assert_eq!(shown(&Options::parse(&format!("{text}fire=off\nsmoke=high\n"))), [level, "off", "high"], "{preset}: fire and smoke");
            assert_eq!(shown(&Options::parse(&format!("{text}fur=off\nfire=low\nsmoke=medium\n"))), ["off", "low", "medium"], "{preset}: all three");
            // the Preset selector reads the same values: the three keys equal to the preset's keep it, any other makes it Custom
            let same = format!("{text}fur={level}\nfire={level}\nsmoke={level}\n");
            assert_eq!(detect(&Options::parse(&same), v), p, "{preset}: the preset's own values written out");
            assert_eq!(detect(&Options::parse(&format!("{text}fur=low\n")), v) == p, level == "low", "{preset}: fur=low");
        }
        // an unreadable value is dropped by the parser, which makes the key missing again
        assert_eq!(shown(&Options::parse(&format!("{}fur=bogus\n", v04_file("low")))), ["off"; 3]);
        // other rows keep their default when missing
        assert_eq!(value_of(&Options::new(), "water"), Some("high"));
        assert_eq!(value_of(&Options::new(), "lang"), Some(""));
    }

    /// Culling the way the game's preset match reads it: Off (with the game's marker line) matches no preset, so the selector is Custom and
    /// Fur / Fire / Smoke read Off; `culling=off` alone is a v0.4.0 leftover and reads as On (games before 0.4.1:
    /// `every_preset_sets_culling_on`).
    #[test]
    fn culling_off_reads_like_the_game_and_a_preset_switches_it_on() {
        let v = "0.5.0";
        let off = "culling=off\nculling_off=kept\n";
        for preset in ["low", "medium", "high"] {
            let o = Options::parse(&format!("{}{off}", v04_file(preset)));
            assert_eq!((detect(&o, v), shown(&o)), (Preset::Custom, ["off"; 3]), "{preset} rows with Culling Off");
        }
        let alone = Options::parse(off);
        assert_eq!((detect(&alone, v), shown(&alone)), (Preset::Custom, ["off"; 3]), "no other row set, Culling Off");
        // the marker's value is not case sensitive and its line may come first
        assert_eq!(detect(&Options::parse("culling_off=Kept\nculling=Off\n"), v), Preset::Custom);
        // a leftover Off (no marker), another marker value, a marker alone, an On with a marker: all read as On
        for text in ["culling=off\n", "culling=off\nculling_off=nope\n", "culling_off=kept\n", "culling=on\nculling_off=kept\n"] {
            let o = Options::parse(text);
            assert_eq!((detect(&o, v), shown(&o)), (Preset::High, ["high"; 3]), "{text:?}");
            assert!(!culling_off(&o, v), "{text:?}");
        }
        let low_leftover = Options::parse(&format!("{}culling=off\n", v04_file("low")));
        assert_eq!((detect(&low_leftover, v), shown(&low_leftover)), (Preset::Low, ["off"; 3]));
        assert_eq!(detect(&Options::parse(off), "0.4.1"), Preset::Custom);
        // a preset switches Culling On: line and marker go, other lines stay
        let mut o = Options::parse(&format!("lang=de\nfuture=1\n{off}"));
        apply(&mut o, v, Preset::Low);
        assert_eq!(detect(&o, v), Preset::Low);
        assert!(!o.to_text().contains("culling"), "{}", o.to_text());
        assert!(o.to_text().contains("lang=de\n") && o.to_text().contains("future=1\n"));
        // High on a Culling Off file: the file ends as it would with no options at all
        let mut h = Options::parse(off);
        apply(&mut h, v, Preset::High);
        assert_eq!(h.to_text(), "version=1\n");
    }

    /// Pinning: the Fur / Fire / Smoke values shown before a picture row changes are written, so the change cannot shift them.
    #[test]
    fn pinning_keeps_what_the_page_shows_when_a_picture_row_changes() {
        let v = "0.5.0";
        let ao = rows::ROWS.iter().find(|r| r.key == "ao").unwrap();
        for (preset, level) in [("low", "off"), ("medium", "low"), ("high", "high")] {
            let before = Options::parse(&v04_file(preset));
            assert_eq!(shown(&before), [level; 3]);
            // without the pin the change shifts the three (the other rows no longer match the preset), except Low, which is Off anyway
            let mut unpinned = before.clone();
            rows::choose(&mut unpinned, ao, 0);
            assert_eq!(shown(&unpinned) == [level; 3], preset == "low", "{preset}: without the pin");
            // with it they stay
            let mut o = before.clone();
            pin_before_change(&mut o, v, "ao");
            assert_eq!(shown(&o), [level; 3], "pinning changes nothing that is shown");
            assert_eq!(["fur", "fire", "smoke"].map(|k| o.get(k)), [Some(level); 3], "{preset}: the keys are written");
            rows::choose(&mut o, ao, 0);
            assert_eq!((shown(&o), detect(&o, v)), ([level; 3], Preset::Custom), "{preset}: after the change");
        }
        // keys already in the file stay; the missing ones are pinned to what is shown (High rows -> high)
        let mut o = Options::parse("fur=low\n");
        pin_before_change(&mut o, v, "shadows");
        assert_eq!(["fur", "fire", "smoke"].map(|k| o.get(k)), [Some("low"), Some("high"), Some("high")]);
        // only a picture row pins; a game that does not know the keys never gets them
        let mut lang = Options::new();
        pin_before_change(&mut lang, v, "lang");
        assert_eq!(lang.to_text(), "version=1\n");
        let mut old = Options::new();
        pin_before_change(&mut old, "0.4.3", "ao");
        assert_eq!(old.to_text(), "version=1\n");
        // a Culling Off file shows Off, and Off is what is pinned
        let mut c = Options::parse("culling=off\nculling_off=kept\n");
        pin_before_change(&mut c, v, "ao");
        assert_eq!(["fur", "fire", "smoke"].map(|k| c.get(k)), [Some("off"); 3]);
    }

    #[test]
    fn every_preset_sets_culling_on() {
        for preset in [Preset::Low, Preset::Medium, Preset::High] {
            let mut o = Options::parse("version=1\nculling=off\n");
            assert_eq!(o.get("culling"), Some("off"));
            assert!(!matches(&o, "0.4.0", preset), "culling=off matches no preset");
            apply(&mut o, "0.4.0", preset);
            assert_eq!(o.get("culling"), None, "{preset:?}: the key is gone, the game reads a missing culling as On");
            assert!(!o.to_text().contains("culling"));
            assert_eq!(detect(&o, "0.4.0"), preset);
        }
        // a version that does not know Culling keeps its line untouched
        let mut old = Options::parse("culling=off\n");
        apply(&mut old, "0.2.2", Preset::Low);
        assert_eq!(old.get("culling"), Some("off"), "0.2.2 does not know culling: the line is left alone");
        assert!(old.to_text().contains("culling=off"));
        assert_eq!(detect(&old, "0.2.2"), Preset::Low);
        // the marker of a deliberate Off is an unknown line: kept on save, removed by a preset
        let mut m = Options::parse("culling=off\nculling_off=kept\nfuture=1\n");
        assert_eq!(m.extra(CULLING_MARKER_KEY), Some("kept"));
        assert_eq!(Options::parse(&m.to_text()).to_text(), m.to_text(), "a launcher save keeps culling=off and the marker");
        assert!(m.to_text().contains("culling_off=kept") && m.to_text().contains("culling=off"));
        m.set("ao", Some("low"));
        assert!(m.to_text().contains("culling_off=kept"), "changing another option keeps the marker");
        apply(&mut m, "0.4.0", Preset::High);
        assert!(!m.to_text().contains("culling"), "a preset sets Culling On: both lines gone");
        assert!(m.to_text().contains("future=1"));
        // the table itself never lists it, and High with culling off in the file is Custom
        assert!(!keys().contains(&"culling"));
        assert_eq!(detect(&Options::parse("culling=off\n"), "0.4.0"), Preset::Custom);
    }

    /// From 0.4.1 on a `culling=off` without the marker is a v0.4.0 leftover that the game reads (and rewrites) as On, so it is no
    /// reason for Custom; with the marker (case-insensitive value) it is a deliberate Off. 0.4.0 itself has no marker: a real Off.
    #[test]
    fn a_markerless_culling_off_reads_as_on_from_the_reset_version() {
        let plain = Options::parse("culling=off\n");
        let marked = Options::parse("culling=off\nculling_off=kept\n");
        let shouting = Options::parse("culling=off\nculling_off=KEPT\n");
        let wrong = Options::parse("culling=off\nculling_off=nope\n");
        for v in ["0.4.1", "0.4.10", "0.5.0"] {
            assert!(!culling_off(&plain, v), "{v}: no marker, On");
            assert_eq!(detect(&plain, v), Preset::High, "{v}: the file is High");
            assert!(culling_off(&marked, v) && culling_off(&shouting, v), "{v}: marker, Off");
            assert_eq!(detect(&marked, v), Preset::Custom);
            assert_eq!(detect(&shouting, v), Preset::Custom);
            assert!(!culling_off(&wrong, v));
        }
        assert!(culling_off(&plain, "0.4.0") && culling_off(&plain, "0.3.0"), "older games take the line as a real Off");
        assert_eq!(detect(&plain, "0.4.0"), Preset::Custom);
        assert!(!culling_off(&plain, "0.2.2"), "a game that does not know Culling");
        assert!(!culling_off(&Options::parse("culling=on\nculling_off=kept\n"), "0.4.1"));
    }

    #[test]
    fn the_table_as_a_data_block() {
        let t = table_text();
        assert_eq!(t.lines().count(), TABLE.len() * 3);
        assert!(t.lines().any(|l| l == "low.draw=low") && t.lines().any(|l| l == "medium.water=medium") && t.lines().any(|l| l == "high.draw=medium"));
    }

    #[test]
    fn the_level_walk_stops_at_what_the_version_knows() {
        assert_eq!(effective("0.2.2", "water", "medium"), Some("classic"));
        assert_eq!(effective("0.2.2", "water", "high"), Some("high"));
        assert_eq!(effective("0.3.0", "particles", "low"), Some("low"));
        assert_eq!(effective("0.2.2", "waves", "off"), None, "unknown key");
        assert_eq!(effective("0.2.2", "ao", "low"), Some("low"), "rows without per-value gating are untouched");
        assert_eq!(effective("0.4.0", "draw", "medium"), Some("medium"));
    }
}
