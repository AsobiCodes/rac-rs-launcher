//! The option lines the game reads outside its main table (rac-rs `PcOptions::extra_get`): the Screen and HUD rows (game 0.4.1), the
//! Controller rows (0.4.2) and most of the 0.5.0 rows. The launcher keeps them as extra lines (`Options::extra`); this table says how
//! the game reads and writes each one, so a Settings row shows the value the game uses and writes the line the game itself would.
//! Sources (rac-rs v0.5): viewer/aspect_box.rs, viewer/upscale/choice.rs, viewer/under_choice.rs, retail/pcdisp.rs, retail/pcinput.rs,
//! retail/glyphs.rs, retail/pcsupp.rs, retail/wrench_throw.rs, retail/qs_pause.rs, retail/auto_gadget.rs, modern_menu/style.rs.
use crate::pcopts::Options;

/// One extra line of a choice row.
pub struct ExtraKey {
    pub key: &'static str,
    /// The file values in the row's order.
    pub values: &'static [&'static str],
    /// The value a missing or unreadable line reads as.
    pub default: &'static str,
    /// Some(v): the game writes no line for `v` (it removes the line); None: the game always writes the value.
    pub unwritten: Option<&'static str>,
}

const ON_OFF: &[&str] = &["on", "off"];
const OFF_ON: &[&str] = &["off", "on"];

pub const EXTRA_KEYS: &[ExtraKey] = &[
    ExtraKey { key: "aspect", values: &["auto", "4:3", "16:9", "16:10", "21:9", "32:9"], default: "auto", unwritten: None },
    ExtraKey { key: "renderscale", values: &["50", "75", "100", "125", "150", "200"], default: "100", unwritten: Some("100") },
    ExtraKey { key: "upscaler", values: &["off", "fsr", "dlss"], default: "off", unwritten: None },
    ExtraKey { key: "upscale_q", values: &["native", "quality", "balanced", "performance", "ultra"], default: "quality", unwritten: None },
    ExtraKey { key: "fps", values: &["display", "30", "60", "120", "144", "off"], default: "display", unwritten: Some("display") },
    // "4:3" is never written: only `corners` is
    ExtraKey { key: "hudpos", values: &["4:3", "corners"], default: "4:3", unwritten: Some("4:3") },
    ExtraKey { key: "underwater", values: ON_OFF, default: "on", unwritten: None },
    ExtraKey { key: "supporters", values: OFF_ON, default: "off", unwritten: None },
    ExtraKey { key: "wrenchthrow", values: &["classic", "moving"], default: "classic", unwritten: None },
    ExtraKey { key: "qspause", values: OFF_ON, default: "off", unwritten: None },
    ExtraKey { key: "autogadget", values: OFF_ON, default: "off", unwritten: None },
    ExtraKey { key: "menustyle", values: &["modern", "classic"], default: "modern", unwritten: Some("modern") },
    ExtraKey { key: "stickdz", values: &["retail", "small", "minimal"], default: "retail", unwritten: Some("retail") },
    ExtraKey { key: "wheelmouse", values: OFF_ON, default: "off", unwritten: Some("off") },
    ExtraKey { key: "glyphs", values: &["playstation", "letters", "switch"], default: "playstation", unwritten: Some("playstation") },
];

/// The HUD scale slider (`hudscale=75..150` in steps of 5, no line = 100, the game removes the line at 100).
pub const HUDSCALE: &str = "hudscale";
pub const HUDSCALE_MIN: u32 = 75;
pub const HUDSCALE_MAX: u32 = 150;
pub const HUDSCALE_STEP: u32 = 5;
pub const HUDSCALE_DEFAULT: u32 = 100;

pub fn extra_key(key: &str) -> Option<&'static ExtraKey> {
    EXTRA_KEYS.iter().find(|e| e.key == key)
}

/// Every extra line a Settings row shows (Reset to defaults removes them).
pub fn row_keys() -> impl Iterator<Item = &'static str> {
    EXTRA_KEYS.iter().map(|e| e.key).chain(std::iter::once(HUDSCALE))
}

/// The game's HUD scale snap (pcdisp.rs `snap_hud`): clamped to 75..150, nearest step of 5.
pub fn snap_hudscale(v: u32) -> u32 {
    let v = v.clamp(HUDSCALE_MIN, HUDSCALE_MAX);
    ((v + HUDSCALE_STEP / 2) / HUDSCALE_STEP * HUDSCALE_STEP).clamp(HUDSCALE_MIN, HUDSCALE_MAX)
}

/// The nearest of `list` (numbers as text) to `n`.
fn nearest(list: &[&'static str], n: u32) -> Option<&'static str> {
    list.iter().copied().filter_map(|s| s.parse::<u32>().ok().map(|l| (s, l))).min_by_key(|&(_, l)| l.abs_diff(n)).map(|(s, _)| s)
}

/// The value the game reads for the line `raw` of `e` (None = no line), as one of `e.values`.
pub fn read(e: &ExtraKey, raw: Option<&str>) -> &'static str {
    let Some(raw) = raw else { return e.default };
    let v = raw.trim().to_ascii_lowercase();
    let found = |v: &str| e.values.iter().copied().find(|n| *n == v);
    let got = match e.key {
        // `4:3`, `21x9`, ` 16/9 `
        "aspect" => found(&v.replace(['x', '/'], ":")),
        // the old FSR 1 value loads as FSR
        "upscaler" if v == "fsr1" => Some("fsr"),
        // the nearest listed value, 100 when it is not a number (pcdisp.rs `parse_rscale`)
        "renderscale" => v.trim_end_matches('%').parse::<u32>().ok().and_then(|n| nearest(e.values, n)),
        // `off`, else a number > 0 = the nearest listed limit (30..144), anything else = the display's rate (pcdisp.rs `parse_fps`)
        "fps" if v == "off" => Some("off"),
        "fps" => v.parse::<u32>().ok().filter(|&n| n > 0).and_then(|n| nearest(&e.values[1..5], n)),
        "wheelmouse" if v == "1" || v == "true" => Some("on"),
        // these read On only for `on` (supporters, qspause, autogadget, wheelmouse) or Off only for `off` (underwater): the default covers the rest
        _ => found(&v),
    };
    got.unwrap_or(e.default)
}

/// The value the game uses for the extra row `key` of `o`.
pub fn value(o: &Options, e: &ExtraKey) -> &'static str {
    read(e, o.extra(e.key))
}

/// Writes `value` the way the game does: no line for the value the game never writes.
pub fn write(o: &mut Options, e: &ExtraKey, value: &str) {
    if !e.values.contains(&value) {
        return;
    }
    if e.unwritten == Some(value) {
        o.set_extra(e.key, None);
    } else {
        o.set_extra(e.key, Some(value));
    }
}

/// The HUD scale the player set (None = no line: 100).
pub fn hudscale(o: &Options) -> Option<u32> {
    o.extra(HUDSCALE).map(|v| v.trim().parse::<u32>().map_or(HUDSCALE_DEFAULT, snap_hudscale))
}

pub fn set_hudscale(o: &mut Options, pct: Option<u32>) {
    let v = pct.map(snap_hudscale).filter(|&p| p != HUDSCALE_DEFAULT);
    o.set_extra(HUDSCALE, v.map(|p| p.to_string()).as_deref());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(key: &str, raw: Option<&str>) -> &'static str {
        read(extra_key(key).unwrap(), raw)
    }

    #[test]
    fn reads_like_the_game() {
        assert_eq!([r("aspect", None), r("aspect", Some("21x9")), r("aspect", Some(" 16/9 ")), r("aspect", Some("5:4"))], ["auto", "21:9", "16:9", "auto"]);
        assert_eq!([r("upscaler", Some("FSR1")), r("upscaler", Some("dlss")), r("upscaler", Some("xess"))], ["fsr", "dlss", "off"]);
        assert_eq!([r("renderscale", Some("140")), r("renderscale", Some("60%")), r("renderscale", Some("big")), r("renderscale", None)], ["150", "50", "100", "100"]);
        assert_eq!([r("fps", Some("Off")), r("fps", Some("75")), r("fps", Some("0")), r("fps", Some("display")), r("fps", Some("999"))], ["off", "60", "display", "display", "144"]);
        assert_eq!([r("hudpos", Some("CORNERS")), r("hudpos", Some("4:3")), r("hudpos", Some("x"))], ["corners", "4:3", "4:3"]);
        assert_eq!([r("underwater", None), r("underwater", Some("0")), r("underwater", Some("off"))], ["on", "on", "off"]);
        assert_eq!([r("supporters", Some("yes")), r("supporters", Some(" On "))], ["off", "on"]);
        assert_eq!([r("wheelmouse", Some("1")), r("wheelmouse", Some("true")), r("wheelmouse", Some("no"))], ["on", "on", "off"]);
        assert_eq!([r("stickdz", Some("Small")), r("glyphs", Some("switch")), r("menustyle", Some("CLASSIC")), r("wrenchthrow", Some("moving"))], ["small", "switch", "classic", "moving"]);
    }

    #[test]
    fn writes_like_the_game() {
        let mut o = Options::parse("version=1\nzzz=1\n");
        for (key, v) in [("aspect", "auto"), ("renderscale", "150"), ("fps", "60"), ("hudpos", "corners"), ("glyphs", "letters"), ("underwater", "off")] {
            write(&mut o, extra_key(key).unwrap(), v);
        }
        assert_eq!(o.to_text(), "version=1\nzzz=1\naspect=auto\nrenderscale=150\nfps=60\nhudpos=corners\nglyphs=letters\nunderwater=off\n");
        for (key, v) in [("renderscale", "100"), ("fps", "display"), ("hudpos", "4:3"), ("glyphs", "playstation"), ("underwater", "on")] {
            write(&mut o, extra_key(key).unwrap(), v);
        }
        assert_eq!(o.to_text(), "version=1\nzzz=1\naspect=auto\nunderwater=on\n", "the values the game writes no line for remove it");
        write(&mut o, extra_key("aspect").unwrap(), "7:5");
        assert_eq!(o.extra("aspect"), Some("auto"), "a value the row does not offer is ignored");
    }

    #[test]
    fn hud_scale_snaps_and_100_has_no_line() {
        let mut o = Options::new();
        assert_eq!(hudscale(&o), None);
        set_hudscale(&mut o, Some(123));
        assert_eq!((o.extra(HUDSCALE), hudscale(&o)), (Some("125"), Some(125)));
        set_hudscale(&mut o, Some(40));
        assert_eq!(o.extra(HUDSCALE), Some("75"));
        set_hudscale(&mut o, Some(100));
        assert_eq!(o.extra(HUDSCALE), None);
        assert_eq!(hudscale(&Options::parse("hudscale=abc\n")), Some(100));
        assert_eq!(hudscale(&Options::parse("hudscale=148\n")), Some(150));
    }
}
