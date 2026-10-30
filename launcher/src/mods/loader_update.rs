//! The OmniLoader library updated on its own, without a game release (releases of github.com/AsobiCodes/omniloader; the file layout and the
//! ABI rule are in that repo's docs/RELEASES.md). The game keeps its own copy in `bundled/` as the fallback.
//!
//! `fetch` reads the latest release and its `omniloader-release.json`; `evaluate` decides (offer / up to date / blocked with a reason) from
//! that manifest, the library installed next to the game exe and the ABI the game's host serves (`host_abi`: `rac-rs omni-host-abi`);
//! `install` downloads, checks the SHA-256 and the file header, and swaps the file in only while the game is not running, keeping the old
//! one as `<lib>.prev`; `rollback` swaps the two back; `restore_bundled` puts the game's own copy back. A failure at any step leaves the
//! installed library exactly as it was.
use super::{loader, semver};
use crate::releases::{self, Asset, Release, ReleaseError, ReleaseSource, Transfer};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub const REPO: (&str, &str) = ("AsobiCodes", "omniloader");
pub const MANIFEST_ASSET: &str = "omniloader-release.json";
/// The first public loader: a library with no version information next to the game is taken for it (every earlier file is that one).
pub const FIRST_VERSION: &str = "0.1.0";
/// Games from this version answer `rac-rs omni-host-abi`; older ones are not asked (an old exe would start the game for an unknown word).
pub const HOST_ABI_FROM_GAME: &str = "0.4.2";
/// The manifest is a few lines of JSON; the library is a small DLL. Anything bigger than this is not what the release says it is.
pub const MANIFEST_MAX: u64 = 64 * 1024;
pub const LIB_MAX: u64 = 64 * 1024 * 1024;
pub const OLD_GAME: &str = "Loader updates need game version 0.4.2 or newer.";
pub const COULD_NOT_ASK: &str = "Could not ask the game which loader interface it supports, so no update is offered.";
pub const GAME_RUNNING: &str = "The game is running. Close it first: the mod loader cannot be changed while it is open.";

/// `major.minor` of the OmniLoader C ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Abi {
    pub major: u32,
    pub minor: u32,
}

impl Abi {
    pub fn parse(s: &str) -> Option<Abi> {
        let (a, b) = s.trim().split_once('.')?;
        Some(Abi { major: a.parse().ok()?, minor: b.parse().ok()? })
    }
    pub fn text(&self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

/// The ABI out of the one JSON line `rac-rs omni-host-abi` prints (`{"omni_host":{"abi":"1.6",...}}`).
pub fn parse_host_output(text: &str) -> Option<Abi> {
    let line = text.lines().map(str::trim).find(|l| l.starts_with('{'))?;
    let j: Value = serde_json::from_str(line).ok()?;
    Abi::parse(j.get("omni_host")?.get("abi")?.as_str()?)
}

/// True for a game that answers `rac-rs omni-host-abi` (version `HOST_ABI_FROM_GAME` or newer; the same comparison the feature gates use).
pub fn supported_game(game_version: &str) -> bool {
    crate::update::compare_version(game_version, HOST_ABI_FROM_GAME) != std::cmp::Ordering::Less
}

/// The ABI the game's host serves. `ask` runs the exe (`ask_exe`; a closure in tests) and is not called for a game older than
/// `HOST_ABI_FROM_GAME`. Nothing is guessed: an older game and a game that does not answer (or prints junk) are both an `Err` with the
/// plain reason, and no update is offered for them.
pub fn host_abi_with(exe: &Path, game_version: &str, ask: &dyn Fn(&Path) -> Option<String>) -> Result<Abi, String> {
    if !supported_game(game_version) {
        return Err(OLD_GAME.to_string());
    }
    ask(exe).as_deref().and_then(parse_host_output).ok_or_else(|| COULD_NOT_ASK.to_string())
}

pub fn host_abi(exe: &Path, game_version: &str) -> Result<Abi, String> {
    host_abi_with(exe, game_version, &|e| ask_exe(e, Duration::from_millis(ASK_LIMIT_MS)))
}

/// The longest the launcher lets the game take to answer `omni-host-abi`, and how long `probing()` stays up after a run ended.
const ASK_LIMIT_MS: u64 = 5_000;
const PROBE_TAIL_MS: u64 = 1_500;
/// How long an "is the game running?" check waits for the launcher's own question to finish: the question's limit, the tail and a little room.
pub const PROBE_WAIT: Duration = Duration::from_millis(ASK_LIMIT_MS + PROBE_TAIL_MS + 500);

/// How many `omni-host-abi` runs are going on, and when the last one ended (milliseconds on `EPOCH`).
static PROBES: AtomicUsize = AtomicUsize::new(0);
static LAST_PROBE_END_MS: AtomicU64 = AtomicU64::new(0);
static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

fn now_ms() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 10_000
}

/// Held while the launcher itself runs the game exe to ask it a question.
struct Probe;

impl Probe {
    fn start() -> Probe {
        PROBES.fetch_add(1, Ordering::SeqCst);
        Probe
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        LAST_PROBE_END_MS.store(now_ms(), Ordering::SeqCst);
        PROBES.fetch_sub(1, Ordering::SeqCst);
    }
}

/// True while the launcher runs the game exe for its own question (and for a moment after: the system needs a beat to let go of the
/// program file). The "is the game running?" watch has to ignore that, or it would take the launcher's own question for the player's game.
pub fn probing() -> bool {
    PROBES.load(Ordering::SeqCst) > 0 || now_ms().saturating_sub(LAST_PROBE_END_MS.load(Ordering::SeqCst)) < PROBE_TAIL_MS
}

/// The decision behind `exe_in_use_settled`: while the launcher's own question runs (`probing`), wait in steps of `step` (at most `limit` in
/// all), then ask `in_use`. A question that has not ended by `limit` gets the plain answer of `in_use` (the safe side: refuse).
fn in_use_after_probe(probing: &dyn Fn() -> bool, in_use: &dyn Fn() -> bool, limit: Duration, step: Duration, sleep: &dyn Fn(Duration)) -> bool {
    let mut waited = Duration::ZERO;
    while waited < limit && probing() {
        sleep(step);
        waited += step;
    }
    in_use()
}

/// `platform::exe_in_use` for the buttons that change files next to the game. The launcher's own `omni-host-abi` run of the same program
/// would read as "the game is running", so a run in progress is waited out first (at most `PROBE_WAIT`). Call it from a worker thread
/// where there is one.
pub fn exe_in_use_settled(exe: &Path) -> bool {
    in_use_after_probe(&probing, &|| crate::platform::exe_in_use(exe), PROBE_WAIT, Duration::from_millis(50), &std::thread::sleep)
}

/// Runs `<exe> omni-host-abi` and returns its output; the program is killed (its own process tree) when it takes longer than `limit`.
pub fn ask_exe(exe: &Path, limit: Duration) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    let _probe = Probe::start();
    let child = std::process::Command::new(exe)
        .arg("omni-host-abi")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(limit) {
        Ok(Ok(out)) if out.status.success() => String::from_utf8(out.stdout).ok(),
        Ok(_) => None,
        Err(_) => {
            crate::platform::kill_tree(pid);
            None
        }
    }
}

/// One library file of a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub install_as: String,
    pub size: u64,
    pub sha256: String,
}

/// `omniloader-release.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    pub abi: Abi,
    pub min_host: Abi,
    pub files: HashMap<String, FileEntry>,
}

fn plain_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 100 && !s.contains(['/', '\\', ':', '\0']) && s != "." && s != ".."
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let j: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("not JSON: {e}"))?;
    if j.get("format").and_then(Value::as_u64) != Some(1) {
        return Err("unknown manifest format".into());
    }
    let version = j.get("version").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if semver::parse(&version).is_none() {
        return Err(format!("bad version {version:?}"));
    }
    let abi = j.get("abi").and_then(Value::as_str).and_then(Abi::parse).ok_or("bad abi")?;
    let min_host = match j.get("min_host_abi") {
        None => Abi { major: abi.major, minor: 0 },
        Some(v) => v.as_str().and_then(Abi::parse).ok_or("bad min_host_abi")?,
    };
    let mut files = HashMap::new();
    for (key, f) in j.get("files").and_then(Value::as_object).ok_or("no files")? {
        let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let e = FileEntry { name: s("name"), install_as: s("install_as"), size: f.get("size").and_then(Value::as_u64).unwrap_or(0), sha256: s("sha256").to_ascii_lowercase() };
        if !plain_name(&e.name) || !plain_name(&e.install_as) || e.sha256.len() != 64 || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("bad entry for {key}"));
        }
        files.insert(key.clone(), e);
    }
    Ok(Manifest { version, abi, min_host, files })
}

/// The key of this system's file in the manifest (`None` = no release file exists for this CPU).
pub fn platform_key(windows: bool, arch: &str) -> Option<&'static str> {
    match (windows, arch) {
        (true, "x86_64") => Some("windows-x86_64"),
        (false, "x86_64") => Some("linux-x86_64"),
        _ => None,
    }
}

fn with_suffix(p: &Path, s: &str) -> PathBuf {
    let mut o = p.as_os_str().to_owned();
    o.push(s);
    PathBuf::from(o)
}

/// What is installed next to the game exe. The version and ABI come from the `<lib>.json` the launcher or setup wrote, and only while that
/// file's `sha256` is the library's own AND its version is a real version number. A library replaced by hand (stale sidecar), a sidecar
/// without a `sha256`, or a version that cannot be read all leave the version unknown (`evaluate` takes unknown for `FIRST_VERSION`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Installed {
    pub present: bool,
    pub version: Option<String>,
    pub abi: Option<Abi>,
    pub sha256: Option<String>,
}

pub fn installed(exe: &Path) -> Installed {
    let info = loader::info(exe);
    if !info.present {
        return Installed::default();
    }
    let lib = loader::lib_path(exe);
    let sha256 = releases::sha256_file(&lib).ok();
    let known = sha256.as_deref().is_some_and(|h| sidecar_belongs(&lib, h));
    let version = info.version.filter(|v| known && semver::parse(v).is_some());
    if version.is_none() {
        return Installed { present: true, version: None, abi: None, sha256 };
    }
    Installed { present: true, version, abi: info.abi.as_deref().and_then(Abi::parse), sha256 }
}

/// True when `<lib>.json` names `lib_sha` as its `sha256` (any letter case). A sidecar with no `sha256`, or another file's, does not belong.
pub fn sidecar_belongs(lib: &Path, lib_sha: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(with_suffix(lib, ".json")) else { return false };
    let Ok(j) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return false };
    j.get("sha256").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case(lib_sha))
}

/// A release that can be installed here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub abi: Abi,
    /// The oldest host interface this loader works with (checked again right before the install).
    pub min_host: Abi,
    pub tag: String,
    pub file: FileEntry,
    pub asset: Asset,
    pub notes: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing newer (the version text is the release's).
    UpToDate(String),
    Offer(Offer),
    /// A release exists but cannot be installed here; the text says why, in plain words.
    Blocked(String),
}

/// What a release means for one game folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    Install,
    UpToDate,
}

/// The interface and version rules in one place (used when the offer is made and again right before it is installed): `Err` = the loader
/// cannot be used with this game (plain reason), `UpToDate` = nothing to do, `Install` = go ahead. An installed loader whose version is
/// unknown counts as `FIRST_VERSION`.
pub fn judge(version: &str, abi: Abi, min_host: Abi, sha256: &str, host: Abi, inst: &Installed) -> Result<Fit, String> {
    if abi.major != host.major {
        let which = if abi.major > host.major { "needs a newer game than this one" } else { "is for an older game than this one" };
        return Err(format!("Loader {version} {which} (its interface is {}, this game serves {}).", abi.text(), host.text()));
    }
    if host.minor < min_host.minor {
        return Err(format!("Loader {version} needs a game that serves interface {} or newer; this game serves {}.", min_host.text(), host.text()));
    }
    if inst.present && inst.sha256.as_deref() == Some(sha256) {
        return Ok(Fit::UpToDate);
    }
    if inst.present {
        let have = inst.version.as_deref().unwrap_or(FIRST_VERSION);
        if !semver::is_newer(version, have) {
            return Ok(Fit::UpToDate);
        }
        if matches!(inst.abi, Some(a) if a.major == abi.major && abi.minor < a.minor) {
            return Ok(Fit::UpToDate);
        }
    }
    Ok(Fit::Install)
}

/// Is the offer still right for this game and this installed file? Asked again right before the download (the verdict can be old, and
/// the game or the loader may have changed since).
pub fn still_offered(o: &Offer, host: Abi, inst: &Installed) -> Result<(), String> {
    match judge(&o.version, o.abi, o.min_host, &o.file.sha256, host, inst)? {
        Fit::Install => Ok(()),
        Fit::UpToDate => Err("The loader is already up to date for this game.".to_string()),
    }
}

/// A download address of a GitHub source has to be a release download of the pinned repo (a folder source has no addresses to check).
pub fn url_ok(src: &ReleaseSource, asset: &Asset) -> Result<(), String> {
    let ReleaseSource::GitHub { owner, repo } = src else { return Ok(()) };
    let prefix = format!("https://github.com/{owner}/{repo}/releases/download/");
    let url = asset.browser_download_url.as_str();
    if url.len() > prefix.len() && url.is_char_boundary(prefix.len()) && url[..prefix.len()].eq_ignore_ascii_case(&prefix) {
        return Ok(());
    }
    Err(format!("This loader release points to a download outside github.com/{owner}/{repo}, so it was not used."))
}

pub fn evaluate(rel: &Release, man: &Manifest, host: Abi, inst: &Installed, windows: bool, arch: &str) -> Verdict {
    let Some(file) = platform_key(windows, arch).and_then(|k| man.files.get(k)) else {
        return Verdict::Blocked("This loader release has no file for this system.".into());
    };
    if file.install_as != loader::lib_name_for(windows) {
        return Verdict::Blocked("This loader release names its file in an unexpected way, so it was not used.".into());
    }
    if semver::cmp_text(&rel.version(), &man.version) != Some(std::cmp::Ordering::Equal) {
        return Verdict::Blocked("This loader release is inconsistent (its tag and its version differ), so it was not used.".into());
    }
    let Some(asset) = rel.asset(&file.name) else {
        return Verdict::Blocked("This loader release lists a file it does not contain, so it was not used.".into());
    };
    if file.size == 0 || file.size > LIB_MAX || file.size != asset.size {
        return Verdict::Blocked("This loader release lists a file whose size does not add up, so it was not used.".into());
    }
    match judge(&man.version, man.abi, man.min_host, &file.sha256, host, inst) {
        Err(why) => Verdict::Blocked(why),
        Ok(Fit::UpToDate) => Verdict::UpToDate(man.version.clone()),
        Ok(Fit::Install) => Verdict::Offer(Offer { version: man.version.clone(), abi: man.abi, min_host: man.min_host, tag: rel.tag_name.clone(), file: file.clone(), asset: asset.clone(), notes: rel.notes() }),
    }
}

/// The latest release and its manifest (the manifest asset needs a checksum like every download).
pub fn fetch(src: &ReleaseSource, tmp: &Path) -> Result<(Release, Manifest), ReleaseError> {
    let rel = src.latest()?;
    let asset = rel.asset(MANIFEST_ASSET).ok_or_else(|| ReleaseError::Bad(format!("the release has no {MANIFEST_ASSET}")))?.clone();
    url_ok(src, &asset).map_err(ReleaseError::Bad)?;
    std::fs::create_dir_all(tmp).map_err(|e| releases::io_error(&e))?;
    let p = tmp.join(MANIFEST_ASSET);
    releases::download_capped(src, &asset, &p, &Transfer::default(), MANIFEST_MAX)?;
    let text = std::fs::read_to_string(&p).map_err(|e| releases::io_error(&e));
    let _ = std::fs::remove_file(&p);
    let man = parse_manifest(&text?).map_err(ReleaseError::Bad)?;
    Ok((rel, man))
}

/// The first bytes of a downloaded library must be a library for this system: `MZ`/`PE` x86_64 on Windows, 64-bit little-endian ELF x86_64 on Linux.
pub fn check_head(windows: bool, head: &[u8]) -> Result<(), String> {
    let le16 = |at: usize| head.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    if windows {
        let pe = head.get(0x3C..0x40).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        let ok = head.starts_with(b"MZ") && matches!(pe, Some(p) if head.get(p..p + 4) == Some(b"PE\0\0".as_slice()) && le16(p + 4) == Some(0x8664));
        return if ok { Ok(()) } else { Err("The downloaded file is not a 64-bit Windows library.".into()) };
    }
    if head.len() < 20 || head[..4] != [0x7f, b'E', b'L', b'F'] || head[4] != 2 || head[5] != 1 || le16(18) != Some(0x3E) {
        return Err("The downloaded file is not a 64-bit Linux library for this processor.".into());
    }
    Ok(())
}

fn head_of(p: &Path) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; 4096];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
    buf.truncate(n);
    buf
}

/// Puts `new` in place of `lib`: the old file becomes `.prev` and its sidecar `.json.prev` (only when that sidecar belongs to the old file;
/// a stale or foreign one is dropped, and an older `.json.prev` is removed, so a `.prev` never sits beside another file's sidecar), `new`
/// is renamed to `lib`, the sidecar text is written. When a step fails the old file is back and `new` is deleted.
pub fn put_in(new: &Path, lib: &Path, sidecar: Option<&str>) -> Result<(), String> {
    let (prev, side, prev_side) = (with_suffix(lib, ".prev"), with_suffix(lib, ".json"), with_suffix(lib, ".json.prev"));
    let had = lib.is_file();
    let carry = had && releases::sha256_file(lib).is_ok_and(|h| sidecar_belongs(lib, &h));
    let fail = |m: String| {
        let _ = std::fs::remove_file(new);
        Err(m)
    };
    if had {
        let _ = std::fs::remove_file(&prev);
        let _ = std::fs::remove_file(&prev_side);
        if let Err(e) = std::fs::rename(lib, &prev) {
            return fail(format!("The old loader could not be set aside ({e}). Nothing was changed."));
        }
        if carry {
            if let Err(e) = std::fs::rename(&side, &prev_side) {
                let _ = std::fs::rename(&prev, lib);
                return fail(format!("The old loader could not be set aside ({e}). Nothing was changed."));
            }
        }
    }
    if let Err(e) = std::fs::rename(new, lib) {
        if had {
            let _ = std::fs::rename(&prev, lib);
            if carry && prev_side.is_file() {
                let _ = std::fs::rename(&prev_side, &side);
            }
        }
        return fail(format!("The new loader could not be put in place ({e}). The old one is still there."));
    }
    match sidecar {
        Some(t) => {
            let tmp = with_suffix(&side, ".tmp");
            if std::fs::write(&tmp, t).is_err() || std::fs::rename(&tmp, &side).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        None => {
            let _ = std::fs::remove_file(&side);
        }
    }
    Ok(())
}

fn sidecar_text(o: &Offer) -> String {
    serde_json::json!({
        "name": o.file.install_as,
        "version": o.version,
        "abi": o.abi.text(),
        "sha256": o.file.sha256,
        "source": format!("github.com/{}/{} release {}", REPO.0, REPO.1, o.tag),
        "installed_by": "RAC-RS Launcher",
    })
    .to_string()
}

/// Downloads the offered library and swaps it in. `running` says whether the game runs (asked before the download and again right before the swap).
pub fn install(src: &ReleaseSource, offer: &Offer, exe: &Path, t: &Transfer, running: &dyn Fn() -> bool, windows: bool) -> Result<(), String> {
    if running() {
        return Err(GAME_RUNNING.into());
    }
    if offer.file.install_as != loader::lib_name_for(windows) {
        return Err("This loader file has an unexpected name, so it was not installed.".into());
    }
    url_ok(src, &offer.asset)?;
    if offer.file.size == 0 || offer.file.size > LIB_MAX || offer.file.size != offer.asset.size {
        return Err("This loader file has a size that does not add up, so it was not installed.".into());
    }
    let lib = loader::lib_path(exe);
    let new = with_suffix(&lib, ".new");
    let mut asset = offer.asset.clone();
    // GitHub's own digest, when the asset has one, has to agree with the manifest's
    match releases::digest_hex(&asset) {
        Some(d) if d != offer.file.sha256 => return Err("The release's two checksums disagree, so the loader was not installed.".into()),
        _ => asset.digest = Some(format!("sha256:{}", offer.file.sha256)),
    }
    releases::download_capped(src, &asset, &new, t, LIB_MAX).map_err(|e| e.text())?;
    if let Err(m) = check_head(windows, &head_of(&new)) {
        let _ = std::fs::remove_file(&new);
        return Err(m);
    }
    if running() {
        let _ = std::fs::remove_file(&new);
        return Err(GAME_RUNNING.into());
    }
    if t.cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&new);
        return Err(ReleaseError::Cancelled.text());
    }
    put_in(&new, &lib, Some(&sidecar_text(offer)))
}

/// The game's own copy of the loader (`bundled/` of the install folder).
pub fn bundled_lib(root: &Path, windows: bool) -> PathBuf {
    root.join("bundled").join(loader::lib_name_for(windows))
}

/// Puts the game's bundled loader back (the installed one becomes `.prev`). `Ok(false)` = it is the installed one already.
pub fn restore_bundled(root: &Path, exe: &Path, running: &dyn Fn() -> bool, windows: bool) -> Result<bool, String> {
    let b = bundled_lib(root, windows);
    if !b.is_file() {
        return Err("This game folder has no bundled mod loader to go back to.".into());
    }
    if running() {
        return Err(GAME_RUNNING.into());
    }
    let lib = loader::lib_path(exe);
    let b_sha = releases::sha256_file(&b).ok();
    if lib.is_file() && releases::sha256_file(&lib).ok() == b_sha {
        return Ok(false);
    }
    let new = with_suffix(&lib, ".new");
    std::fs::copy(&b, &new).map_err(|e| format!("The bundled loader could not be copied ({e})."))?;
    if let Err(m) = check_head(windows, &head_of(&new)) {
        let _ = std::fs::remove_file(&new);
        return Err(m);
    }
    let side = std::fs::read_to_string(with_suffix(&b, ".json")).ok().filter(|_| b_sha.as_deref().is_some_and(|h| sidecar_belongs(&b, h)));
    put_in(&new, &lib, side.as_deref()).map(|_| true)
}

fn swap(a: &Path, b: &Path) -> std::io::Result<()> {
    match (a.exists(), b.exists()) {
        (true, true) => {
            let t = with_suffix(a, ".swap");
            std::fs::rename(a, &t)?;
            if let Err(e) = std::fs::rename(b, a) {
                let _ = std::fs::rename(&t, a);
                return Err(e);
            }
            std::fs::rename(&t, b)
        }
        (false, true) => std::fs::rename(b, a),
        (true, false) => std::fs::rename(a, b),
        (false, false) => Ok(()),
    }
}

pub fn can_roll_back(exe: &Path) -> bool {
    with_suffix(&loader::lib_path(exe), ".prev").is_file()
}

/// Swaps the installed loader with its `.prev` (twice in a row = back where it was).
pub fn roll_back(exe: &Path, running: &dyn Fn() -> bool) -> Result<(), String> {
    if running() {
        return Err(GAME_RUNNING.into());
    }
    let lib = loader::lib_path(exe);
    if !can_roll_back(exe) {
        return Err("There is no previous loader to go back to.".into());
    }
    swap(&lib, &with_suffix(&lib, ".prev")).map_err(|e| format!("The previous loader could not be put back ({e}). Nothing was changed."))?;
    let _ = swap(&with_suffix(&lib, ".json"), &with_suffix(&lib, ".json.prev"));
    Ok(())
}

#[cfg(test)]
#[path = "loader_update_tests.rs"]
mod tests;
