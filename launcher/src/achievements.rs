//! The game's `achievements.json` (version 1): parser, model and a cheap file watcher. No egui here (the page is `ui/page_achievements.rs`).
//!
//! The game writes the file atomically into its player folder (next to `pc_options.txt`). The parser is lenient on purpose: unknown keys are
//! ignored, a broken single entry is skipped, a missing optional field is "not set"; only a file that is not JSON, not an object, or has a version
//! other than 1 is refused (with a plain message). Nothing in here panics on bad input.
use crate::games::GameDef;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

pub const FILE_NAME: &str = "achievements.json";
pub const SUPPORTED_VERSION: i64 = 1;
/// How often the open page looks at the file's modification time.
pub const POLL: Duration = Duration::from_secs(1);

pub const MSG_MISSING: &str = "No achievements yet: this game version has none, or you have not started playing.";
pub const MSG_BROKEN: &str = "achievements.json could not be read.";
/// The line under MSG_BROKEN: what the game does with a damaged file (REVIEW_B2 B18; the game keeps the copy and rebuilds from the save data).
pub const MSG_BROKEN_NOTE: &str = "The game keeps the damaged file as achievements.json.broken, so your old data is still there. It starts again from what your save data holds and writes a new achievements.json. Nothing else is affected.";
pub const HIDDEN_NAME: &str = "Hidden trophy";
pub const HIDDEN_DESC: &str = "Keep playing to find out";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grade {
    Bronze,
    Silver,
    Gold,
    Platinum,
}

impl Grade {
    pub const ALL: [Grade; 4] = [Grade::Bronze, Grade::Silver, Grade::Gold, Grade::Platinum];

    pub fn label(self) -> &'static str {
        match self {
            Grade::Bronze => "Bronze",
            Grade::Silver => "Silver",
            Grade::Gold => "Gold",
            Grade::Platinum => "Platinum",
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// An unknown grade text is read as bronze (the entry stays visible).
    pub fn parse(s: &str) -> Grade {
        match s.trim().to_ascii_lowercase().as_str() {
            "silver" => Grade::Silver,
            "gold" => Grade::Gold,
            "platinum" => Grade::Platinum,
            _ => Grade::Bronze,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Achievement {
    pub id: String,
    pub name: String,
    pub description: String,
    pub grade: Grade,
    pub hidden: bool,
    pub unlocked: bool,
    /// RFC3339 UTC text as the game wrote it.
    pub unlocked_at: Option<String>,
    pub progress: Option<i64>,
    pub progress_max: Option<i64>,
}

impl Achievement {
    /// A hidden trophy that is still locked shows a placeholder (the file holds the real text; it must not leak into the UI).
    pub fn masked(&self) -> bool {
        self.hidden && !self.unlocked
    }

    pub fn shown_name(&self) -> &str {
        if self.masked() {
            HIDDEN_NAME
        } else {
            &self.name
        }
    }

    pub fn shown_description(&self) -> &str {
        if self.masked() {
            HIDDEN_DESC
        } else {
            &self.description
        }
    }

    /// `(done, max, percent)` for the progress line: only for a locked, not masked trophy with a positive `progress_max`.
    /// `done` is clamped to 0..=max (a missing progress counts as 0).
    pub fn progress_view(&self) -> Option<(i64, i64, u32)> {
        if self.unlocked || self.masked() {
            return None;
        }
        let max = self.progress_max.filter(|m| *m > 0)?;
        let done = self.progress.unwrap_or(0).clamp(0, max);
        Some((done, max, percent(done, max)))
    }
}

/// `done / max` as a whole percent, rounded down, clamped to 0..=100; 0 when `max <= 0`.
pub fn percent(done: i64, max: i64) -> u32 {
    if max <= 0 {
        return 0;
    }
    let d = done.clamp(0, max) as i128;
    ((d * 100) / max as i128) as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Unlocked,
    Locked,
}

impl Filter {
    pub const LABELS: [&'static str; 3] = ["All", "Unlocked", "Locked"];

    pub fn from_index(i: usize) -> Filter {
        match i {
            1 => Filter::Unlocked,
            2 => Filter::Locked,
            _ => Filter::All,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn accepts(self, a: &Achievement) -> bool {
        match self {
            Filter::All => true,
            Filter::Unlocked => a.unlocked,
            Filter::Locked => !a.unlocked,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Achievements {
    pub game: String,
    pub updated: String,
    pub list: Vec<Achievement>,
}

/// Counts for the page header.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Totals {
    pub unlocked: usize,
    pub total: usize,
    /// Per grade in `Grade::ALL` order: (unlocked, total).
    pub grades: [(usize, usize); 4],
}

impl Totals {
    pub fn percent(&self) -> u32 {
        percent(self.unlocked as i64, self.total as i64)
    }

    pub fn grade(&self, g: Grade) -> (usize, usize) {
        self.grades[g.index()]
    }
}

impl Achievements {
    pub fn totals(&self) -> Totals {
        let mut t = Totals { total: self.list.len(), ..Default::default() };
        for a in &self.list {
            let slot = &mut t.grades[a.grade.index()];
            slot.1 += 1;
            if a.unlocked {
                t.unlocked += 1;
                slot.0 += 1;
            }
        }
        t
    }

    pub fn filtered(&self, f: Filter) -> impl Iterator<Item = &Achievement> {
        self.list.iter().filter(move |a| f.accepts(a))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParseError {
    /// Empty, truncated or not JSON, or not an object.
    Broken,
    /// `version` is missing or not 1 (carries what the file says, 0 when absent).
    Version(i64),
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn int(v: &Value, key: &str) -> Option<i64> {
    let x = v.get(key)?;
    x.as_i64().or_else(|| x.as_f64().filter(|f| f.is_finite() && f.fract() == 0.0).map(|f| f as i64))
}

fn parse_entry(v: &Value) -> Option<Achievement> {
    if !v.is_object() {
        return None;
    }
    let id = text(v, "id");
    let name = text(v, "name");
    if id.is_empty() && name.is_empty() {
        return None;
    }
    Some(Achievement {
        name: if name.is_empty() { id.clone() } else { name },
        id,
        description: text(v, "description"),
        grade: Grade::parse(v.get("grade").and_then(Value::as_str).unwrap_or("")),
        hidden: v.get("hidden").and_then(Value::as_bool).unwrap_or(false),
        unlocked: v.get("unlocked").and_then(Value::as_bool).unwrap_or(false),
        unlocked_at: v.get("unlocked_at").and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty()),
        progress: int(v, "progress"),
        progress_max: int(v, "progress_max"),
    })
}

/// Parses the file's text. A UTF-8 BOM is tolerated.
pub fn parse(src: &str) -> Result<Achievements, ParseError> {
    let v: Value = serde_json::from_str(src.trim_start_matches('\u{feff}')).map_err(|_| ParseError::Broken)?;
    if !v.is_object() {
        return Err(ParseError::Broken);
    }
    let version = int(&v, "version").unwrap_or(0);
    if version != SUPPORTED_VERSION {
        return Err(ParseError::Version(version));
    }
    let list = v.get("achievements").and_then(Value::as_array).map(|a| a.iter().filter_map(parse_entry).collect()).unwrap_or_default();
    Ok(Achievements { game: text(&v, "game"), updated: text(&v, "updated"), list })
}

// ---------------------------------------------------------------------------------------------------------------- dates

/// "2026-11-02T12:00:00Z" -> "2026-11-02 12:00 UTC". Accepts fractional seconds and "+00:00"; any other form (or an impossible date) comes back unchanged.
pub fn format_stamp(s: &str) -> String {
    parse_stamp(s).map(|(y, mo, d, h, mi)| format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC")).unwrap_or_else(|| s.to_string())
}

fn parse_stamp(s: &str) -> Option<(u32, u32, u32, u32, u32)> {
    let b = s.trim().as_bytes();
    if b.len() < 20 || !s.trim().is_ascii() {
        return None;
    }
    let num = |from: usize, len: usize| -> Option<u32> {
        let part = &b[from..from + len];
        if part.iter().all(u8::is_ascii_digit) {
            std::str::from_utf8(part).ok()?.parse().ok()
        } else {
            None
        }
    };
    if b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b't') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?, num(17, 2)?);
    // after the seconds: optional ".fff", then "Z" or "+00:00" / "-00:00"
    let mut rest = &s.trim()[19..];
    if let Some(r) = rest.strip_prefix('.') {
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &r[digits..];
    }
    if !(rest.eq_ignore_ascii_case("z") || rest == "+00:00" || rest == "-00:00") {
        return None;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match mo {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if d == 0 || d > dim || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some((y, mo, d, h, mi))
}

// ---------------------------------------------------------------------------------------------------------------- the file on disk

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Load {
    /// Not looked at yet.
    #[default]
    Unread,
    /// No file (a game version without achievements, or nothing played yet).
    Missing,
    /// The file exists but cannot be used (text for the user).
    Broken(String),
    Ok(Achievements),
}

/// Where the game writes the file: its player folder (the same one `pc_options.txt` and the memory card live in).
/// R&C 1 follows Play.cmd's RAC_PLAYER_DIR like the saves row does; other games use the folder of their options file.
pub fn file_path(root: &Path, game: &GameDef) -> PathBuf {
    if game.id == "rac1" {
        return crate::play::saves_dir(root).join(FILE_NAME);
    }
    game.options_path(root).parent().map(|p| p.join(FILE_NAME)).unwrap_or_else(|| root.join("player").join(FILE_NAME))
}

pub fn read_file(path: &Path) -> Load {
    match std::fs::read(path) {
        Ok(bytes) => match parse(&String::from_utf8_lossy(&bytes)) {
            Ok(a) => Load::Ok(a),
            Err(ParseError::Broken) => Load::Broken(MSG_BROKEN.to_string()),
            Err(ParseError::Version(v)) => Load::Broken(format!("{MSG_BROKEN} It has version {v}; this launcher reads version {SUPPORTED_VERSION}. Update the launcher.")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Load::Missing,
        Err(_) => Load::Broken(MSG_BROKEN.to_string()),
    }
}

/// Re-reads the file when its modification time or size changed; `poll` is cheap (one `metadata` call, at most once per `POLL`).
#[derive(Default)]
pub struct Watch {
    path: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    checked: Option<Instant>,
    pub load: Load,
}

impl Watch {
    /// Returns true when `load` changed. `path` = where the file should be (a different path resets the watcher).
    pub fn poll(&mut self, path: &Path, now: Instant) -> bool {
        let same = self.path.as_deref() == Some(path);
        if same && self.checked.map(|c| now.saturating_duration_since(c) < POLL).unwrap_or(false) {
            return false;
        }
        if !same {
            self.path = Some(path.to_path_buf());
            self.stamp = None;
            self.load = Load::Unread;
        }
        self.checked = Some(now);
        let stamp = std::fs::metadata(path).ok().filter(|m| m.is_file()).map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
        if same && stamp == self.stamp && self.load != Load::Unread {
            return false;
        }
        self.stamp = stamp;
        let new = if stamp.is_none() { Load::Missing } else { read_file(path) };
        let changed = new != self.load;
        self.load = new;
        changed
    }

    pub fn reset(&mut self) {
        *self = Watch::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{
 "version": 1, "game": "rac1", "updated": "2026-11-02T12:00:00Z", "future_key": {"x": 1},
 "achievements": [
  {"id":"hit_me_more","name":"Hit Me More!","description":"Obtain the Premium Nanotech upgrade","grade":"bronze","hidden":false,"unlocked":true,"unlocked_at":"2026-11-02T12:00:00Z","progress":null,"progress_max":null,"extra":true},
  {"id":"skillful","name":"Skillful","description":"Master all 30 skill points","grade":"gold","hidden":false,"unlocked":false,"unlocked_at":null,"progress":12,"progress_max":30},
  {"id":"secret","name":"Real Secret","description":"Real secret text","grade":"platinum","hidden":true,"unlocked":false,"unlocked_at":null,"progress":1,"progress_max":2},
  {"id":"secret2","name":"Opened Secret","description":"Now known","grade":"silver","hidden":true,"unlocked":true,"unlocked_at":"2026-11-03T08:09:10.5Z","progress":null,"progress_max":null}
 ]}"#;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_ach_{}_{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn valid_file_with_all_fields() {
        let a = parse(FULL).unwrap();
        assert_eq!((a.game.as_str(), a.updated.as_str(), a.list.len()), ("rac1", "2026-11-02T12:00:00Z", 4));
        let h = &a.list[0];
        assert_eq!((h.id.as_str(), h.name.as_str(), h.grade, h.hidden, h.unlocked), ("hit_me_more", "Hit Me More!", Grade::Bronze, false, true));
        assert_eq!(h.description, "Obtain the Premium Nanotech upgrade");
        assert_eq!(h.unlocked_at.as_deref(), Some("2026-11-02T12:00:00Z"));
        assert_eq!((h.progress, h.progress_max), (None, None));
        let s = &a.list[1];
        assert_eq!((s.grade, s.unlocked, s.unlocked_at.clone(), s.progress, s.progress_max), (Grade::Gold, false, None, Some(12), Some(30)));
        assert_eq!(s.progress_view(), Some((12, 30, 40)));
        assert_eq!(h.progress_view(), None, "unlocked: no progress line");
    }

    #[test]
    fn hidden_locked_entries_are_masked_until_unlocked() {
        let a = parse(FULL).unwrap();
        let s = &a.list[2];
        assert!(s.masked());
        assert_eq!((s.shown_name(), s.shown_description()), ("Hidden trophy", "Keep playing to find out"));
        assert_eq!(s.progress_view(), None, "progress must not leak either");
        assert_eq!(s.grade, Grade::Platinum, "the grade stays visible");
        let o = &a.list[3];
        assert!(!o.masked());
        assert_eq!((o.shown_name(), o.shown_description()), ("Opened Secret", "Now known"));
        assert!(!a.list[0].masked() && !a.list[1].masked());
    }

    #[test]
    fn progress_percent_math() {
        assert_eq!(percent(0, 30), 0);
        assert_eq!(percent(12, 30), 40);
        assert_eq!(percent(29, 30), 96);
        assert_eq!(percent(30, 30), 100);
        assert_eq!(percent(5, 0), 0, "progress_max 0");
        assert_eq!(percent(5, -3), 0);
        assert_eq!(percent(99, 30), 100, "progress above max clamps");
        assert_eq!(percent(-4, 30), 0);
        assert_eq!(percent(i64::MAX, i64::MAX), 100);
        let mk = |p: Option<i64>, m: Option<i64>| Achievement { id: "x".into(), name: "X".into(), description: String::new(), grade: Grade::Gold, hidden: false, unlocked: false, unlocked_at: None, progress: p, progress_max: m };
        assert_eq!(mk(Some(99), Some(30)).progress_view(), Some((30, 30, 100)));
        assert_eq!(mk(Some(3), Some(0)).progress_view(), None);
        assert_eq!(mk(Some(3), None).progress_view(), None);
        assert_eq!(mk(None, Some(10)).progress_view(), Some((0, 10, 0)), "progress absent counts as 0");
        assert_eq!(mk(Some(-5), Some(10)).progress_view(), Some((0, 10, 0)));
    }

    #[test]
    fn broken_empty_and_unknown_version_are_graceful() {
        assert_eq!(parse(""), Err(ParseError::Broken));
        assert_eq!(parse("   \n"), Err(ParseError::Broken));
        assert_eq!(parse("not json"), Err(ParseError::Broken));
        assert_eq!(parse(&FULL[..FULL.len() / 2]), Err(ParseError::Broken), "truncated");
        assert_eq!(parse("[1,2]"), Err(ParseError::Broken));
        assert_eq!(parse("null"), Err(ParseError::Broken));
        assert_eq!(parse(r#"{"version":2,"achievements":[]}"#), Err(ParseError::Version(2)));
        assert_eq!(parse(r#"{"achievements":[]}"#), Err(ParseError::Version(0)));
        assert_eq!(parse(r#"{"version":"1","achievements":[]}"#), Err(ParseError::Version(0)));
        // fine but empty
        assert_eq!(parse(r#"{"version":1}"#).unwrap().list.len(), 0);
        assert_eq!(parse(r#"{"version":1,"achievements":"oops"}"#).unwrap().list.len(), 0);
        assert!(parse(&format!("\u{feff}{}", FULL)).is_ok(), "BOM tolerated");
    }

    #[test]
    fn extra_keys_and_missing_optional_fields() {
        let a = parse(r#"{"version":1,"zzz":[1],"achievements":[
            {"id":"a","name":"A","unlocked":true,"mystery":{"k":1}},
            {"id":"b"},
            {"name":"only name","grade":"SILVER"},
            {"description":"no id and no name"},
            42,
            {"id":"c","name":"C","grade":"purple","progress":"x","progress_max":3.0,"unlocked_at":""}
        ]}"#)
        .unwrap();
        assert_eq!(a.list.len(), 4, "the entry without id and name and the non-object are skipped");
        let x = &a.list[0];
        assert_eq!((x.description.as_str(), x.grade, x.hidden, x.unlocked_at.clone(), x.progress, x.progress_max), ("", Grade::Bronze, false, None, None, None));
        assert_eq!(a.list[1].name, "b", "name falls back to the id");
        assert_eq!(a.list[2].grade, Grade::Silver);
        let c = &a.list[3];
        assert_eq!((c.grade, c.progress, c.progress_max, c.unlocked_at.clone()), (Grade::Bronze, None, Some(3), None));
        assert_eq!(a.game, "");
    }

    #[test]
    fn grades_are_counted() {
        let a = parse(FULL).unwrap();
        let t = a.totals();
        assert_eq!((t.unlocked, t.total, t.percent()), (2, 4, 50));
        assert_eq!(t.grade(Grade::Bronze), (1, 1));
        assert_eq!(t.grade(Grade::Silver), (1, 1));
        assert_eq!(t.grade(Grade::Gold), (0, 1));
        assert_eq!(t.grade(Grade::Platinum), (0, 1));
        let e = Achievements::default().totals();
        assert_eq!((e.unlocked, e.total, e.percent()), (0, 0, 0));
        let ids = |f: Filter| a.filtered(f).map(|x| x.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids(Filter::All).len(), 4);
        assert_eq!(ids(Filter::Unlocked), ["hit_me_more", "secret2"]);
        assert_eq!(ids(Filter::Locked), ["skillful", "secret"]);
        assert_eq!(Filter::from_index(Filter::Locked.index()), Filter::Locked);
        assert_eq!(Grade::parse("GOLD"), Grade::Gold);
        assert_eq!(Grade::parse(""), Grade::Bronze);
    }

    #[test]
    fn dates_are_formatted() {
        assert_eq!(format_stamp("2026-11-02T12:00:00Z"), "2026-11-02 12:00 UTC");
        assert_eq!(format_stamp("2026-11-03T08:09:10.5Z"), "2026-11-03 08:09 UTC");
        assert_eq!(format_stamp("2024-02-29T23:59:59+00:00"), "2024-02-29 23:59 UTC");
        assert_eq!(format_stamp("2025-02-29T00:00:00Z"), "2025-02-29T00:00:00Z", "impossible date stays as is");
        assert_eq!(format_stamp("2026-13-01T00:00:00Z"), "2026-13-01T00:00:00Z");
        assert_eq!(format_stamp("2026-11-02T12:00:00+02:00"), "2026-11-02T12:00:00+02:00", "other zones are not guessed");
        assert_eq!(format_stamp("yesterday"), "yesterday");
        assert_eq!(format_stamp(""), "");
        assert_eq!(format_stamp("2026-11-02T12:00:00Ü"), "2026-11-02T12:00:00Ü", "non-ascii does not panic");
    }

    #[test]
    fn file_lives_in_the_player_folder() {
        let d = tmp("path");
        let g = crate::games::game_by_id("rac1").unwrap();
        assert_eq!(file_path(&d, g), d.join("player").join("achievements.json"));
        std::fs::write(d.join("Play.cmd"), "set \"RAC_PLAYER_DIR=%~dp0saves\"\r\n").unwrap();
        assert_eq!(file_path(&d, g), d.join("saves").join("achievements.json"));
        let g2 = crate::games::game_by_id("rac2").unwrap();
        assert_eq!(file_path(&d, g2), d.join("player").join("achievements.json"), "R&C 2 has its own install folder; its achievements come in v0.2");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn watcher_follows_the_file() {
        let d = tmp("watch");
        let p = d.join("player").join(FILE_NAME);
        let mut w = Watch::default();
        let t0 = Instant::now();
        assert!(w.poll(&p, t0), "first poll: Unread -> Missing");
        assert_eq!(w.load, Load::Missing);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, FULL).unwrap();
        assert!(!w.poll(&p, t0 + Duration::from_millis(200)), "rate limited: not looked at again within a second");
        assert_eq!(w.load, Load::Missing);
        assert!(w.poll(&p, t0 + Duration::from_millis(1300)));
        assert!(matches!(&w.load, Load::Ok(a) if a.list.len() == 4));
        assert!(!w.poll(&p, t0 + Duration::from_millis(2600)), "unchanged file: no new read");
        // a longer file: size changes even when the clock is coarse
        std::fs::write(&p, r#"{"version":1,"achievements":[{"id":"a","name":"A","unlocked":true}]}"#).unwrap();
        assert!(w.poll(&p, t0 + Duration::from_millis(4000)));
        assert!(matches!(&w.load, Load::Ok(a) if a.list.len() == 1));
        std::fs::write(&p, "{\"version\": 1, \"achiev").unwrap();
        assert!(w.poll(&p, t0 + Duration::from_millis(5500)));
        assert!(matches!(&w.load, Load::Broken(m) if m == MSG_BROKEN));
        std::fs::write(&p, "").unwrap();
        w.poll(&p, t0 + Duration::from_millis(7000));
        assert!(matches!(&w.load, Load::Broken(_)), "empty file");
        std::fs::write(&p, r#"{"version":7}"#).unwrap();
        w.poll(&p, t0 + Duration::from_millis(8500));
        assert!(matches!(&w.load, Load::Broken(m) if m.contains("version 7")));
        std::fs::remove_file(&p).unwrap();
        assert!(w.poll(&p, t0 + Duration::from_millis(10000)));
        assert_eq!(w.load, Load::Missing);
        // another install: the watcher starts over
        let p2 = d.join("other.json");
        std::fs::write(&p2, FULL).unwrap();
        assert!(w.poll(&p2, t0 + Duration::from_millis(10100)), "a new path is read at once");
        assert!(matches!(w.load, Load::Ok(_)));
        let _ = std::fs::remove_dir_all(&d);
    }
}
