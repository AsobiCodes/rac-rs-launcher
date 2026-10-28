//! Script mods that update themselves from their own GitHub repo.
//! A mod opts in with `"update": {"github": "owner/repo"}` in its omni-mod.json (the game's loader ignores the key). The launcher then looks
//! at the repo's latest release (`releases.rs`, one request per repo per launcher start): when the tag is a higher version than the installed
//! one and the release has `<mod id>-<version>.zip` (with a checksum), the Mods page offers "Update to <version>".
//! Installing is all-or-nothing and never touches the player's data: the zip is downloaded and checked, unpacked and validated in
//! `<install>\mod_update_tmp\` (NEXT TO `mods\`, never inside it: the game's loader would list a second copy of the mod), then the old folder is
//! renamed to `<install>\mod_backups\<id>-<old version>` and the new one takes its name. `mods\active.txt`, `settings.json` and `player\omni\`
//! (progress) are not part of a mod folder and are not touched. Only script mods are offered updates; a package that is not a script mod,
//! or that brings a program file, is refused.
use super::import::{open_zip, safe_rel};
use super::script::{self, Parsed, ScriptMod};
use super::semver;
use crate::releases::{self, Asset, Release, ReleaseError, ReleaseSource, Transfer};
use serde_json::Value as Json;
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Next to `mods\`: the old versions after an update.
pub const BACKUPS_DIR: &str = "mod_backups";
/// Next to `mods\`: the download and the unpacked package while an update runs (removed at the end).
pub const STAGING_DIR: &str = "mod_update_tmp";
/// Backups kept per mod; older ones are deleted after a successful update.
pub const KEEP_BACKUPS: usize = 3;
/// Limits against a package that unpacks into something huge.
pub const MAX_UNPACKED: u64 = 256 * 1024 * 1024;
pub const MAX_FILES: usize = 20_000;
pub const CANCELLED: &str = "Cancelled.";

/// A mod mentioned by the launcher itself ("Get Arsenal Mastery" when it is not installed).
pub struct Featured {
    pub id: &'static str,
    pub name: &'static str,
    /// "owner/repo"
    pub repo: &'static str,
}

pub const FEATURED: &[Featured] = &[Featured { id: "arsenal-mastery", name: "Arsenal Mastery", repo: "AsobiCodes/arsenal-mastery" }];

// ---------------------------------------------------------------------------------------------------------------- the manifest key

/// Where a mod's releases live (`"update": {"github": "owner/repo"}`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct UpdateSource {
    pub owner: String,
    pub repo: String,
}

fn owner_ok(s: &str) -> bool {
    (1..=39).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') && !s.starts_with('-') && !s.ends_with('-')
}

fn repo_ok(s: &str) -> bool {
    (1..=100).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) && !s.bytes().all(|b| b == b'.')
}

impl UpdateSource {
    /// "owner/repo" -> a source (GitHub's naming rules; anything else is None).
    pub fn parse(text: &str) -> Option<UpdateSource> {
        let (o, r) = text.trim().split_once('/')?;
        (owner_ok(o) && repo_ok(r)).then(|| UpdateSource { owner: o.to_string(), repo: r.to_string() })
    }

    /// The `update` key of a manifest. A missing key, another shape or an invalid name is simply "no update source", never an error.
    pub fn from_manifest(obj: &serde_json::Map<String, Json>) -> Option<UpdateSource> {
        Self::parse(obj.get("update")?.as_object()?.get("github")?.as_str()?)
    }

    /// "owner/repo" (also the key of the check cache).
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    pub fn repo_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }

    /// The release's page; a tag with unusual characters falls back to the list of releases.
    pub fn release_url(&self, tag: &str) -> String {
        let tag = tag.trim();
        if !tag.is_empty() && tag.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+')) {
            format!("{}/releases/tag/{tag}", self.repo_url())
        } else {
            format!("{}/releases", self.repo_url())
        }
    }

    /// GitHub, or (tests and `--mod-releases`) the folder `<dir>/<owner>/<repo>/` with a `latest.json`.
    pub fn release_source(&self, test_dir: Option<&Path>) -> ReleaseSource {
        match test_dir {
            Some(d) => ReleaseSource::Folder(d.join(&self.owner).join(&self.repo)),
            None => ReleaseSource::GitHub { owner: self.owner.clone(), repo: self.repo.clone() },
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------- what a release offers

/// The release asset of mod `id` at `version`: `<id>-<version>.zip`.
pub fn asset_name(id: &str, version: &str) -> String {
    format!("{id}-{version}.zip")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub id: String,
    pub name: String,
    /// Installed version ("" = a fresh install).
    pub from: String,
    pub to: String,
    pub tag: String,
    pub notes: String,
    pub asset: Asset,
    pub page_url: String,
    pub source: UpdateSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The release is the installed version or older (its version).
    UpToDate(String),
    Offer(Box<Offer>),
    /// A release exists but the launcher cannot use it (one sentence for the Details).
    Skipped(String),
}

/// What the latest release means for the installed mod. `installed` = None: the mod is not installed (any usable release is offered).
pub fn evaluate(rel: &Release, src: &UpdateSource, id: &str, name: &str, installed: Option<&str>) -> Verdict {
    let tag = rel.tag_name.trim();
    if rel.draft || rel.prerelease {
        return Verdict::Skipped(format!("the latest release \"{tag}\" is a draft or pre-release"));
    }
    let version = rel.version();
    let Some(remote) = semver::parse(&version) else { return Verdict::Skipped(format!("the latest release is called \"{tag}\", which is not a version number")) };
    if let Some(have) = installed {
        let Some(local) = semver::parse(have) else { return Verdict::Skipped(format!("the installed version \"{have}\" is not a version number")) };
        if remote <= local {
            return Verdict::UpToDate(remote.to_string());
        }
    }
    let want = asset_name(id, &version);
    let Some(asset) = rel.asset(&want) else { return Verdict::Skipped(format!("version {version} is out, but the release has no file called {want}")) };
    if releases::digest_hex(asset).is_none() {
        return Verdict::Skipped(format!("version {version} is out, but {want} has no checksum, so the launcher will not use it"));
    }
    Verdict::Offer(Box::new(Offer {
        id: id.to_string(),
        name: name.to_string(),
        from: installed.unwrap_or_default().to_string(),
        to: version,
        tag: tag.to_string(),
        notes: rel.notes(),
        asset: asset.clone(),
        page_url: src.release_url(tag),
        source: src.clone(),
    }))
}

// ---------------------------------------------------------------------------------------------------------------- the package

/// Where the mod's files start inside the zip: "" (`omni-mod.json` at the top) or "folder/" (everything inside one folder, as GitHub's own
/// "source code" zips and many hand-made zips are). `names` = the file entries (folders and `__MACOSX/` junk are ignored).
pub fn detect_layout(names: &[String]) -> Result<String, String> {
    let kept: Vec<String> = names.iter().map(|n| n.replace('\\', "/")).filter(|n| !n.ends_with('/') && !n.starts_with("__MACOSX/")).collect();
    if kept.is_empty() {
        return Err("The package is empty.".to_string());
    }
    let is_manifest = |n: &str| n == script::FILE || n == script::FILE_COMPAT;
    if kept.iter().any(|n| is_manifest(n)) {
        return Ok(String::new());
    }
    let tops: BTreeSet<&str> = kept.iter().map(|n| n.split('/').next().unwrap_or("")).collect();
    if tops.len() == 1 {
        let t = tops.iter().next().copied().unwrap_or("");
        if kept.iter().all(|n| n.len() > t.len() + 1 && n.starts_with(t) && n.as_bytes()[t.len()] == b'/') && kept.iter().any(|n| is_manifest(&n[t.len() + 1..])) {
            return Ok(format!("{t}/"));
        }
    }
    Err(format!("The package has no {} at its top level (or inside one single folder).", script::FILE))
}

/// Unpacks the mod's files into `dest` (created). Entries with unusable names are left out; the declared and the real size are capped.
fn unpack_zip(zip_path: &Path, dest: &Path, t: &Transfer) -> Result<(), String> {
    let mut z = open_zip(zip_path)?;
    let mut files: Vec<(usize, String)> = Vec::new();
    let mut declared = 0u64;
    for i in 0..z.len() {
        let f = z.by_index_raw(i).map_err(|e| e.to_string())?;
        if f.is_dir() {
            continue;
        }
        declared += f.size();
        files.push((i, f.name().map_err(|e| e.to_string())?.to_string()));
        if files.len() > MAX_FILES {
            return Err(format!("The package holds more than {MAX_FILES} files, so it was not used."));
        }
    }
    if declared > MAX_UNPACKED {
        return Err(format!("The package unpacks to more than {} MB, so it was not used.", MAX_UNPACKED >> 20));
    }
    let names: Vec<String> = files.iter().map(|f| f.1.clone()).collect();
    let prefix = detect_layout(&names)?;
    std::fs::create_dir_all(dest).map_err(|e| format!("Could not create the staging folder: {e}"))?;
    let mut taken = BTreeSet::new();
    let mut written = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    for (i, name) in files {
        if t.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.to_string());
        }
        let name = name.replace('\\', "/");
        let Some(rest) = name.strip_prefix(prefix.as_str()).filter(|_| !name.starts_with("__MACOSX/")) else { continue };
        let Some(rel) = safe_rel(rest).filter(|p| taken.insert(p.to_string_lossy().to_lowercase())) else { continue };
        let out_path = dest.join(rel);
        if let Some(p) = out_path.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut src = z.by_index(i).map_err(|e| e.to_string())?;
        let mut out = std::fs::File::create(&out_path).map_err(|e| format!("Could not write {}: {e}", out_path.display()))?;
        loop {
            let n = src.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > MAX_UNPACKED {
                return Err(format!("The package unpacks to more than {} MB, so it was not used.", MAX_UNPACKED >> 20));
            }
            out.write_all(&buf[..n]).map_err(|e| releases::io_error(&e).text())?;
        }
    }
    Ok(())
}

fn read_manifest(dir: &Path) -> Option<Box<ScriptMod>> {
    let file = script::manifest_in(dir)?;
    let text = String::from_utf8_lossy(&std::fs::read(file).ok()?).into_owned();
    match script::parse(&text) {
        Parsed::Mod(m) => Some(m),
        _ => None,
    }
}

/// The unpacked package must be exactly the mod the offer says: same id, a script mod, the offered version, no program file, and every
/// script its manifest lists.
fn validate_staged(dir: &Path, offer: &Offer) -> Result<(), String> {
    let file = script::manifest_in(dir).ok_or_else(|| format!("The package has no {}.", script::FILE))?;
    let text = String::from_utf8_lossy(&std::fs::read(&file).map_err(|e| e.to_string())?).into_owned();
    let m = match script::parse(&text) {
        Parsed::Mod(m) => m,
        Parsed::DataPack => return Err("The package is not a script mod.".to_string()),
        Parsed::Bad(why) => return Err(format!("The package's {} has a problem: {why}.", script::FILE)),
    };
    if m.id != offer.id {
        return Err(format!("The package is for the mod \"{}\", not \"{}\".", m.id, offer.id));
    }
    if m.kind != "script" {
        return Err("The package is not a script mod. The launcher only updates script mods.".to_string());
    }
    let (got, want) = (semver::parse(&m.version), semver::parse(&offer.to));
    if got.is_none() || got != want {
        return Err(format!("The package says it is version \"{}\", but the release is {}.", m.version, offer.to));
    }
    let json: Json = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    if json.get("plugin").is_some() {
        return Err("The package brings a program file (plugin), which a script mod update may not do.".to_string());
    }
    for s in json.get("scripts").and_then(Json::as_array).into_iter().flatten().filter_map(Json::as_str) {
        if !safe_rel(s).is_some_and(|p| dir.join(p).is_file()) {
            return Err(format!("The package lacks {s}, which its {} lists.", script::FILE));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------- putting it in place

/// "<base>" if free, else "<base> (2)", "<base> (3)"...
fn unique_path(dir: &Path, base: &str) -> PathBuf {
    let p = dir.join(base);
    if !p.exists() {
        return p;
    }
    (2..).map(|n| dir.join(format!("{base} ({n})"))).find(|p| !p.exists()).unwrap_or(p)
}

/// "<id>-<version>" or "<id>-<version> (n)" -> (version, n).
fn backup_version(name: &str, id: &str) -> Option<(semver::Ver, u32)> {
    let rest = name.strip_prefix(id)?.strip_prefix('-')?;
    let (ver, n) = match rest.rsplit_once(" (").and_then(|(v, tail)| Some((v, tail.strip_suffix(')')?.parse::<u32>().ok()?))) {
        Some((v, n)) => (v, n),
        None => (rest, 1),
    };
    Some((semver::parse(ver)?, n))
}

/// Keeps the `keep` newest backups of mod `id` (by the version in the name); folders that do not look like our backups are never touched.
pub fn prune_backups(backups: &Path, id: &str, keep: usize) {
    let Ok(rd) = std::fs::read_dir(backups) else { return };
    let mut found: Vec<((semver::Ver, u32), PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if let (true, Some(k)) = (e.path().is_dir(), backup_version(&name, id)) {
            found.push((k, e.path()));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in found.into_iter().skip(keep) {
        let _ = std::fs::remove_dir_all(p);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Done {
    pub id: String,
    pub name: String,
    /// "" = a fresh install.
    pub from: String,
    pub to: String,
    /// Where the old version went.
    pub backup: Option<PathBuf>,
}

type Mover<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

/// Replaces `existing` by the validated `unpacked` folder (or, with None, adds it as `mods\<id>`). The old folder is renamed to the backups
/// folder first; when the second rename fails the first one is undone, so the mod is either the old or the new version, never half of each.
fn swap(base: &Path, mods_dir: &Path, unpacked: &Path, offer: &Offer, existing: Option<&Path>, mv: Mover) -> Result<Done, String> {
    let done = |from: &str, backup: Option<PathBuf>| Done { id: offer.id.clone(), name: offer.name.clone(), from: from.to_string(), to: offer.to.clone(), backup };
    let Some(dir) = existing else {
        let target = mods_dir.join(&offer.id);
        if target.exists() {
            return Err(format!("A folder called \"{}\" is already in the mods folder, so nothing was added.", offer.id));
        }
        std::fs::create_dir_all(mods_dir).map_err(|e| e.to_string())?;
        mv(unpacked, &target).map_err(|e| format!("The mod could not be put in the mods folder: {e}"))?;
        return Ok(done("", None));
    };
    // the installed copy may have changed while the download ran
    let Some(now) = read_manifest(dir).filter(|m| m.id == offer.id).map(|m| m.version) else {
        return Err("The installed copy changed or can no longer be read, so it was not replaced.".to_string());
    };
    if !semver::is_newer(&offer.to, &now) {
        return Err(format!("The installed copy is now version \"{now}\", so it was not replaced."));
    }
    let backups = base.join(BACKUPS_DIR);
    std::fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
    let backup = unique_path(&backups, &format!("{}-{}", offer.id, now));
    mv(dir, &backup).map_err(|e| format!("The old version could not be moved away (is the game or another program using its files?): {e}"))?;
    if let Err(e) = mv(unpacked, dir) {
        return Err(match mv(&backup, dir) {
            Ok(()) => format!("The new version could not be put in place ({e}). The old version is back; nothing was changed."),
            Err(e2) => format!("The new version could not be put in place ({e}) and the old one could not be restored ({e2}). The old version is in {}.", backup.display()),
        });
    }
    prune_backups(&backups, &offer.id, KEEP_BACKUPS);
    Ok(done(&now, Some(backup)))
}

/// The whole update, on the calling thread: download + checksum, unpack, validate, swap. `phase` is told what is going on.
/// `mods_dir` = `<install>\mods`; the staging and backup folders are its siblings. `existing` = the installed mod's folder (None = add it).
pub fn install_blocking(mods_dir: &Path, src: &ReleaseSource, offer: &Offer, existing: Option<&Path>, t: &Transfer, phase: &dyn Fn(&'static str)) -> Result<Done, String> {
    install_with(mods_dir, src, offer, existing, t, phase, &|a, b| std::fs::rename(a, b))
}

fn install_with(mods_dir: &Path, src: &ReleaseSource, offer: &Offer, existing: Option<&Path>, t: &Transfer, phase: &dyn Fn(&'static str), mv: Mover) -> Result<Done, String> {
    let base = mods_dir.parent().filter(|p| !p.as_os_str().is_empty()).ok_or("The mods folder has no folder around it.")?;
    let stage = base.join(STAGING_DIR);
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| releases::io_error(&e).text())?;
    let r = (|| {
        if let Some(free) = crate::platform::free_bytes(&stage) {
            if free < offer.asset.size.saturating_mul(3).saturating_add(16 << 20) {
                return Err(ReleaseError::DiskFull.text());
            }
        }
        phase("Downloading");
        let zip_path = stage.join("download.zip");
        releases::download(src, &offer.asset, &zip_path, t).map_err(|e| e.text())?;
        phase("Unpacking");
        let unpacked = stage.join("unpacked");
        unpack_zip(&zip_path, &unpacked, t)?;
        phase("Checking");
        validate_staged(&unpacked, offer)?;
        if t.cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.to_string());
        }
        phase("Installing");
        swap(base, mods_dir, &unpacked, offer, existing, mv)
    })();
    let _ = std::fs::remove_dir_all(&stage);
    r
}

// ---------------------------------------------------------------------------------------------------------------- the background job

/// An update running on its own thread (the window polls it).
pub struct UpdateJob {
    pub offer: Offer,
    pub transfer: Arc<Transfer>,
    pub started: Instant,
    phase: Arc<Mutex<&'static str>>,
    result: Arc<Mutex<Option<Result<Done, String>>>>,
}

impl UpdateJob {
    pub fn start(mods_dir: PathBuf, src: ReleaseSource, offer: Offer, existing: Option<PathBuf>) -> UpdateJob {
        let (transfer, phase, result) = (Transfer::new(), Arc::new(Mutex::new("Starting")), Arc::new(Mutex::new(None)));
        let (t2, p2, r2, o2) = (transfer.clone(), phase.clone(), result.clone(), offer.clone());
        std::thread::spawn(move || {
            let set = |s: &'static str| {
                if let Ok(mut g) = p2.lock() {
                    *g = s;
                }
            };
            let res = install_blocking(&mods_dir, &src, &o2, existing.as_deref(), &t2, &set);
            if let Ok(mut g) = r2.lock() {
                *g = Some(res);
            }
        });
        UpdateJob { offer, transfer, started: Instant::now(), phase, result }
    }

    pub fn phase(&self) -> &'static str {
        self.phase.lock().map(|g| *g).unwrap_or("")
    }

    pub fn take_result(&self) -> Option<Result<Done, String>> {
        self.result.lock().ok().and_then(|mut g| g.take())
    }

    pub fn cancel(&self) {
        self.transfer.cancel.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::releases::write_test_release;

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_modup_{n}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const ID: &str = "demo-mod";

    fn manifest(version: &str) -> String {
        format!(
            r#"{{"format":2,"id":"{ID}","name":"Demo Mod","version":"{version}","author":"me","type":"script","api":"1.3","description":"d","games":{{"rac1":["pal"]}},"scripts":["main.luau"],"update":{{"github":"someone/demo-mod"}}}}"#
        )
    }

    fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (n, b) in files {
            w.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn pkg(version: &str) -> Vec<u8> {
        zip_bytes(&[("omni-mod.json", manifest(version).as_bytes()), ("main.luau", b"-- new"), ("perks/a.luau", b"-- perk")])
    }

    fn write(p: &Path, t: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }

    fn good(_: &str, hex: &str) -> Option<String> {
        Some(format!("sha256:{hex}"))
    }

    fn source() -> UpdateSource {
        UpdateSource::parse("someone/demo-mod").unwrap()
    }

    /// An install root with `mods\demo-mod` at `installed`, the player's files next to it, and a release folder with `zip` as tag `tag`.
    struct Fix {
        root: PathBuf,
        rel: PathBuf,
    }

    /// Publishes `zip` as the latest release `tag` of the fake repo (asset `demo-mod-<version>.zip`).
    fn publish(rel: &Path, tag: &str, zip: &[u8], digest: &dyn Fn(&str, &str) -> Option<String>) {
        let name = asset_name(ID, tag.trim_start_matches('v'));
        write_test_release(rel, tag, "Fixed things", &[(name.as_str(), zip)], digest);
    }

    fn fixture(name: &str, installed: &str, tag: &str, zip: &[u8]) -> Fix {
        let root = scratch(name);
        write(&root.join("mods/demo-mod/omni-mod.json"), &manifest(installed));
        write(&root.join("mods/demo-mod/main.luau"), "-- old");
        write(&root.join("mods/active.txt"), "# Enabled mods\ndemo-mod\n");
        write(&root.join("mods/settings.json"), r#"{"format":1,"mods":{"demo-mod":{"x":7}}}"#);
        write(&root.join("player/omni/slot1.json"), "progress");
        let rel = root.join("releases");
        publish(&rel, tag, zip, &good);
        Fix { root, rel }
    }

    fn run(f: &Fix) -> Result<Done, String> {
        let src = ReleaseSource::Folder(f.rel.clone());
        let rel = src.latest().unwrap();
        let installed = read_manifest(&f.root.join("mods/demo-mod")).map(|m| m.version);
        let Verdict::Offer(o) = evaluate(&rel, &source(), ID, "Demo Mod", installed.as_deref()) else { panic!("no offer") };
        install_blocking(&f.root.join("mods"), &src, &o, Some(&f.root.join("mods/demo-mod")), &Transfer::default(), &|_| {})
    }

    #[test]
    fn the_update_key_is_read_strictly_and_never_an_error() {
        let ok = |t: &str| UpdateSource::from_manifest(serde_json::from_str::<Json>(t).unwrap().as_object().unwrap());
        assert_eq!(ok(r#"{"update":{"github":"AsobiCodes/arsenal-mastery"}}"#), UpdateSource::parse("AsobiCodes/arsenal-mastery"));
        assert_eq!(ok(r#"{"update":{"github":"a/b","other":1},"x":2}"#).unwrap().slug(), "a/b");
        for bad in [
            r#"{}"#,
            r#"{"update":"a/b"}"#,
            r#"{"update":{"github":5}}"#,
            r#"{"update":{"gitlab":"a/b"}}"#,
            r#"{"update":{"github":"nobar"}}"#,
            r#"{"update":{"github":"a/b/c"}}"#,
            r#"{"update":{"github":"-a/b"}}"#,
            r#"{"update":{"github":"a_b/c"}}"#,
            r#"{"update":{"github":"a/.."}}"#,
            r#"{"update":{"github":"a/b c"}}"#,
            r#"{"update":{"github":"../x"}}"#,
            r#"{"update":[]}"#,
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
        let s = source();
        assert_eq!(s.repo_url(), "https://github.com/someone/demo-mod");
        assert_eq!(s.release_url("v0.3.3"), "https://github.com/someone/demo-mod/releases/tag/v0.3.3");
        assert_eq!(s.release_url("a b?x"), "https://github.com/someone/demo-mod/releases");
        assert!(matches!(s.release_source(Some(Path::new("T"))), ReleaseSource::Folder(p) if p == Path::new("T").join("someone").join("demo-mod")));
        assert!(matches!(s.release_source(None), ReleaseSource::GitHub { .. }));
    }

    fn rel_json(tag: &str, assets: &[(&str, bool)], extra: &str) -> Release {
        let a: Vec<String> = assets
            .iter()
            .map(|(n, digest)| format!(r#"{{"name":"{n}","size":5,"browser_download_url":"{n}","digest":{}}}"#, if *digest { format!("\"sha256:{}\"", "a".repeat(64)) } else { "null".into() }))
            .collect();
        releases::parse_release(&format!(r#"{{"tag_name":"{tag}","body":"n\r\nm","assets":[{}]{extra}}}"#, a.join(","))).unwrap()
    }

    #[test]
    fn a_newer_release_with_the_right_asset_is_offered_and_everything_else_is_explained() {
        let s = source();
        let r = rel_json("v0.3.3", &[("demo-mod-0.3.3.zip", true)], "");
        let Verdict::Offer(o) = evaluate(&r, &s, ID, "Demo Mod", Some("0.3.2")) else { panic!() };
        assert_eq!((o.from.as_str(), o.to.as_str(), o.tag.as_str(), o.notes.as_str()), ("0.3.2", "0.3.3", "v0.3.3", "n\nm"));
        assert_eq!(o.asset.name, "demo-mod-0.3.3.zip");
        assert_eq!(o.page_url, "https://github.com/someone/demo-mod/releases/tag/v0.3.3");
        let Verdict::Offer(fresh) = evaluate(&r, &s, ID, "Demo Mod", None) else { panic!() };
        assert_eq!(fresh.from, "");
        assert_eq!(evaluate(&r, &s, ID, "Demo Mod", Some("0.3.3")), Verdict::UpToDate("0.3.3".into()));
        assert_eq!(evaluate(&r, &s, ID, "Demo Mod", Some("0.4.0")), Verdict::UpToDate("0.3.3".into()), "never a downgrade");
        assert!(matches!(evaluate(&r, &s, ID, "Demo Mod", Some("0.3.3-rc.1")), Verdict::Offer(_)), "the final release replaces a pre-release install");
        for (rel, installed, word) in [
            (rel_json("v0.3.3", &[("other.zip", true)], ""), "0.3.2", "no file called demo-mod-0.3.3.zip"),
            (rel_json("v0.3.3", &[("demo-mod-0.3.3.zip", false)], ""), "0.3.2", "no checksum"),
            (rel_json("v0.3.3", &[("demo-mod-0.3.3.zip", true)], r#","prerelease":true"#), "0.3.2", "pre-release"),
            (rel_json("v0.3.3", &[("demo-mod-0.3.3.zip", true)], r#","draft":true"#), "0.3.2", "draft"),
            (rel_json("nightly", &[("demo-mod-nightly.zip", true)], ""), "0.3.2", "not a version number"),
            (rel_json("v0.3.3", &[("demo-mod-0.3.3.zip", true)], ""), "dev", "installed version"),
        ] {
            let Verdict::Skipped(why) = evaluate(&rel, &s, ID, "Demo Mod", Some(installed)) else { panic!("{word}") };
            assert!(why.contains(word), "{why}");
        }
        // the asset must be named for the tag as written: tag without "v" works, too
        let Verdict::Offer(o2) = evaluate(&rel_json("0.3.3", &[("demo-mod-0.3.3.zip", true)], ""), &s, ID, "Demo Mod", Some("0.3.2")) else { panic!() };
        assert_eq!(o2.to, "0.3.3");
    }

    #[test]
    fn layout_is_found_at_the_top_or_in_one_folder() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(detect_layout(&v(&["omni-mod.json", "main.luau"])), Ok(String::new()));
        assert_eq!(detect_layout(&v(&["openrac-mod.json", "x/y.luau"])), Ok(String::new()));
        assert_eq!(detect_layout(&v(&["demo-main/omni-mod.json", "demo-main/perks/a.luau", "demo-main/"])), Ok("demo-main/".to_string()));
        assert_eq!(detect_layout(&v(&["omni-mod.json", "__MACOSX/._omni-mod.json", "dir/"])), Ok(String::new()));
        assert_eq!(detect_layout(&v(&["demo/omni-mod.json", "__MACOSX/demo/._x"])), Ok("demo/".to_string()));
        assert_eq!(detect_layout(&v(&["demo\\omni-mod.json", "demo\\main.luau"])), Ok("demo/".to_string()));
        for bad in [
            v(&[]),
            v(&["a/omni-mod.json", "b/omni-mod.json"]),
            v(&["a/omni-mod.json", "README.md"]),
            v(&["a/b/omni-mod.json"]),
            v(&["README.md", "main.luau"]),
            v(&["a/readme.txt"]),
        ] {
            assert!(detect_layout(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_update_replaces_the_mod_keeps_its_folder_and_the_players_files_and_backs_up_the_old_one() {
        let f = fixture("ok", "0.3.2", "v0.3.3", &pkg("0.3.3"));
        let before = (std::fs::read(f.root.join("mods/active.txt")).unwrap(), std::fs::read(f.root.join("mods/settings.json")).unwrap(), std::fs::read(f.root.join("player/omni/slot1.json")).unwrap());
        let done = run(&f).unwrap();
        assert_eq!((done.from.as_str(), done.to.as_str(), done.id.as_str()), ("0.3.2", "0.3.3", ID));
        let m = f.root.join("mods/demo-mod");
        assert_eq!(std::fs::read_to_string(m.join("main.luau")).unwrap(), "-- new");
        assert_eq!(std::fs::read_to_string(m.join("perks/a.luau")).unwrap(), "-- perk");
        assert!(read_manifest(&m).unwrap().version == "0.3.3");
        let b = done.backup.unwrap();
        assert_eq!(b, f.root.join("mod_backups/demo-mod-0.3.2"));
        assert_eq!(std::fs::read_to_string(b.join("main.luau")).unwrap(), "-- old");
        assert_eq!(read_manifest(&b).unwrap().version, "0.3.2");
        let after = (std::fs::read(f.root.join("mods/active.txt")).unwrap(), std::fs::read(f.root.join("mods/settings.json")).unwrap(), std::fs::read(f.root.join("player/omni/slot1.json")).unwrap());
        assert_eq!(before, after, "active.txt, settings.json and the progress are not touched");
        assert!(!f.root.join(STAGING_DIR).exists(), "staging is removed");
        let in_mods: Vec<String> = std::fs::read_dir(f.root.join("mods")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(in_mods.iter().all(|n| n == "demo-mod" || n == "active.txt" || n == "settings.json"), "no backup or staging folder inside mods: {in_mods:?}");
        // the same offer a second time is refused at the swap (the installed copy is already that version)
        let again = run_with_forced_offer(&f).unwrap_err();
        assert!(again.contains("now version \"0.3.3\""), "{again}");
        assert_eq!(std::fs::read_dir(f.root.join("mod_backups")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn a_zip_with_one_folder_around_the_mod_installs_too() {
        let z = zip_bytes(&[("demo-mod-main/omni-mod.json", manifest("0.3.3").as_bytes()), ("demo-mod-main/main.luau", b"-- new"), ("__MACOSX/demo-mod-main/._x", b"junk")]);
        let f = fixture("folder", "0.3.2", "v0.3.3", &z);
        run(&f).unwrap();
        assert_eq!(std::fs::read_to_string(f.root.join("mods/demo-mod/main.luau")).unwrap(), "-- new");
        assert!(!f.root.join("mods/demo-mod/demo-main").exists() && !f.root.join("mods/demo-mod/__MACOSX").exists());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn bad_packages_are_refused_and_the_installed_mod_stays_as_it_was() {
        let m3 = manifest("0.3.3");
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            ("wrong id", zip_bytes(&[("omni-mod.json", m3.replace(ID, "other-mod").as_bytes()), ("main.luau", b"x")]), "not \"demo-mod\""),
            ("wrong version", zip_bytes(&[("omni-mod.json", manifest("0.3.4").as_bytes()), ("main.luau", b"x")]), "version \"0.3.4\""),
            ("no manifest", zip_bytes(&[("main.luau", b"x")]), "no omni-mod.json"),
            ("script missing", zip_bytes(&[("omni-mod.json", m3.as_bytes())]), "lacks main.luau"),
            ("broken manifest", zip_bytes(&[("omni-mod.json", b"{ nope"), ("main.luau", b"x")]), "has a problem"),
            ("native mod", zip_bytes(&[("omni-mod.json", m3.replace("\"script\"", "\"native\"").as_bytes()), ("main.luau", b"x")]), "not a script mod"),
            ("plugin", zip_bytes(&[("omni-mod.json", m3.replace("\"scripts\":[\"main.luau\"]", "\"scripts\":[\"main.luau\"],\"plugin\":{\"windows-x64\":\"a.dll\"}").as_bytes()), ("main.luau", b"x"), ("a.dll", b"x")]), "plugin"),
            ("not a zip", b"this is not a zip".to_vec(), "zip"),
        ];
        for (name, z, word) in cases {
            let f = fixture("bad", "0.3.2", "v0.3.3", &z);
            let err = run(&f).expect_err(name);
            assert!(err.contains(word), "{name}: {err}");
            assert_eq!(read_manifest(&f.root.join("mods/demo-mod")).unwrap().version, "0.3.2", "{name}: untouched");
            assert_eq!(std::fs::read_to_string(f.root.join("mods/demo-mod/main.luau")).unwrap(), "-- old", "{name}");
            assert!(!f.root.join("mod_backups").exists() && !f.root.join(STAGING_DIR).exists(), "{name}: no backup, no staging");
            let _ = std::fs::remove_dir_all(&f.root);
        }
    }

    #[test]
    fn a_wrong_or_missing_checksum_is_refused_before_anything_is_unpacked() {
        let f = fixture("digest", "0.3.2", "v0.3.3", &pkg("0.3.3"));
        publish(&f.rel, "v0.3.3", &pkg("0.3.3"), &|_, _| Some(format!("sha256:{}", "0".repeat(64))));
        let err = run_with_forced_offer(&f).unwrap_err();
        assert!(err.contains("did not match its checksum"), "{err}");
        publish(&f.rel, "v0.3.3", &pkg("0.3.3"), &|_, _| None);
        let err = run_with_forced_offer(&f).unwrap_err();
        assert!(err.contains("no checksum"), "{err}");
        assert_eq!(read_manifest(&f.root.join("mods/demo-mod")).unwrap().version, "0.3.2");
        assert!(!f.root.join(STAGING_DIR).exists());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    /// An offer built by hand (evaluate would not offer a release without a checksum), to test what install does with it.
    fn run_with_forced_offer(f: &Fix) -> Result<Done, String> {
        let src = ReleaseSource::Folder(f.rel.clone());
        let rel = src.latest().unwrap();
        let a = rel.asset(&asset_name(ID, "0.3.3")).unwrap().clone();
        let o = Offer { id: ID.into(), name: "Demo Mod".into(), from: "0.3.2".into(), to: "0.3.3".into(), tag: "v0.3.3".into(), notes: String::new(), asset: a, page_url: String::new(), source: source() };
        install_blocking(&f.root.join("mods"), &src, &o, Some(&f.root.join("mods/demo-mod")), &Transfer::default(), &|_| {})
    }

    #[test]
    fn a_copy_that_changed_during_the_download_is_not_replaced() {
        let f = fixture("changed", "0.3.2", "v0.3.3", &pkg("0.3.3"));
        // the player (or a bundled-mod copy) already put 0.3.3 in place before the swap
        write(&f.root.join("mods/demo-mod/omni-mod.json"), &manifest("0.3.3"));
        let err = run_with_forced_offer(&f).unwrap_err();
        assert!(err.contains("now version \"0.3.3\""), "{err}");
        assert!(!f.root.join("mod_backups").exists());
        // and a folder whose manifest is for another mod is never replaced
        write(&f.root.join("mods/demo-mod/omni-mod.json"), &manifest("0.3.2").replace(ID, "someone-else"));
        assert!(run_with_forced_offer(&f).is_err());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn when_the_second_rename_fails_the_old_version_comes_back() {
        let f = fixture("rollback", "0.3.2", "v0.3.3", &pkg("0.3.3"));
        let src = ReleaseSource::Folder(f.rel.clone());
        let rel = src.latest().unwrap();
        let Verdict::Offer(o) = evaluate(&rel, &source(), ID, "Demo Mod", Some("0.3.2")) else { panic!() };
        let mods = f.root.join("mods");
        let target = mods.join("demo-mod");
        let fail_into_target = |a: &Path, b: &Path| -> std::io::Result<()> {
            if b == target && a.file_name().is_some_and(|n| n == "unpacked") {
                Err(std::io::Error::other("locked"))
            } else {
                std::fs::rename(a, b)
            }
        };
        let err = install_with(&mods, &src, &o, Some(&target), &Transfer::default(), &|_| {}, &fail_into_target).unwrap_err();
        assert!(err.contains("old version is back"), "{err}");
        assert_eq!(read_manifest(&target).unwrap().version, "0.3.2");
        assert_eq!(std::fs::read_to_string(target.join("main.luau")).unwrap(), "-- old");
        assert!(!f.root.join(STAGING_DIR).exists());
        // the very first rename failing: nothing moved
        let fail_all = |_: &Path, _: &Path| -> std::io::Result<()> { Err(std::io::Error::other("busy")) };
        let err = install_with(&mods, &src, &o, Some(&target), &Transfer::default(), &|_| {}, &fail_all).unwrap_err();
        assert!(err.contains("could not be moved away"), "{err}");
        assert_eq!(read_manifest(&target).unwrap().version, "0.3.2");
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn unsafe_names_in_the_zip_are_left_out_and_the_size_caps_hold() {
        let z = zip_bytes(&[("omni-mod.json", manifest("0.3.3").as_bytes()), ("main.luau", b"-- new"), ("../evil.txt", b"x"), ("..\\evil2.txt", b"x"), ("a/../../evil3.txt", b"x"), ("C:/evil4.txt", b"x")]);
        let f = fixture("slip", "0.3.2", "v0.3.3", &z);
        run(&f).unwrap();
        for n in ["evil.txt", "evil2.txt", "evil3.txt", "evil4.txt"] {
            assert!(!f.root.join(n).exists() && !f.root.join("mods").join(n).exists() && !f.root.join("mods/demo-mod").join(n).exists(), "{n}");
        }
        assert_eq!(std::fs::read_to_string(f.root.join("mods/demo-mod/main.luau")).unwrap(), "-- new");
        let _ = std::fs::remove_dir_all(&f.root);
        // too many files
        let many: Vec<(String, Vec<u8>)> = (0..MAX_FILES + 1).map(|i| (format!("f{i}.txt"), vec![])).collect();
        let mut files: Vec<(&str, &[u8])> = many.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        files.push(("omni-mod.json", b"{}"));
        let dir = scratch("caps");
        std::fs::write(dir.join("a.zip"), zip_bytes(&files)).unwrap();
        let err = unpack_zip(&dir.join("a.zip"), &dir.join("out"), &Transfer::default()).unwrap_err();
        assert!(err.contains("more than"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_stops_the_update_and_leaves_nothing() {
        let f = fixture("cancel", "0.3.2", "v0.3.3", &pkg("0.3.3"));
        let src = ReleaseSource::Folder(f.rel.clone());
        let Verdict::Offer(o) = evaluate(&src.latest().unwrap(), &source(), ID, "Demo Mod", Some("0.3.2")) else { panic!() };
        let t = Transfer::default();
        t.cancel.store(true, Ordering::Relaxed);
        let err = install_blocking(&f.root.join("mods"), &src, &o, Some(&f.root.join("mods/demo-mod")), &t, &|_| {}).unwrap_err();
        assert_eq!(err, CANCELLED);
        assert_eq!(read_manifest(&f.root.join("mods/demo-mod")).unwrap().version, "0.3.2");
        assert!(!f.root.join(STAGING_DIR).exists() && !f.root.join("mod_backups").exists());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn a_fresh_install_adds_the_mod_and_refuses_an_existing_folder() {
        let f = fixture("fresh", "0.3.2", "v0.3.3", &pkg("0.3.3"));
        let mods = f.root.join("mods");
        let src = ReleaseSource::Folder(f.rel.clone());
        let Verdict::Offer(o) = evaluate(&src.latest().unwrap(), &source(), ID, "Demo Mod", None) else { panic!() };
        let err = install_blocking(&mods, &src, &o, None, &Transfer::default(), &|_| {}).unwrap_err();
        assert!(err.contains("already in the mods folder"), "{err}");
        std::fs::remove_dir_all(mods.join("demo-mod")).unwrap();
        let done = install_blocking(&mods, &src, &o, None, &Transfer::default(), &|_| {}).unwrap();
        assert_eq!((done.from.as_str(), done.to.as_str(), done.backup), ("", "0.3.3", None));
        assert_eq!(read_manifest(&mods.join("demo-mod")).unwrap().version, "0.3.3");
        assert!(!f.root.join("mod_backups").exists());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn only_the_newest_three_backups_of_a_mod_are_kept() {
        let d = scratch("prune");
        for n in ["demo-mod-0.1.0", "demo-mod-0.2.0", "demo-mod-0.3.0", "demo-mod-0.3.1", "demo-mod-0.3.1 (2)", "demo-mod-0.10.0", "demo-mod-2.0.0-rc.1", "other-mod-0.0.1", "demo-mod-notes", "demo-mod-extra-0.0.1", "demo"] {
            std::fs::create_dir_all(d.join(n)).unwrap();
        }
        std::fs::write(d.join("demo-mod-0.0.1"), "a file, not a backup").unwrap();
        prune_backups(&d, "demo-mod", KEEP_BACKUPS);
        let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        // kept: the three highest versions (2.0.0-rc.1, 0.10.0, 0.3.1 (2)) and everything that is not one of this mod's backups
        assert_eq!(left, ["demo", "demo-mod-0.0.1", "demo-mod-0.10.0", "demo-mod-0.3.1 (2)", "demo-mod-2.0.0-rc.1", "demo-mod-extra-0.0.1", "demo-mod-notes", "other-mod-0.0.1"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn four_updates_in_a_row_leave_three_backups_and_a_name_clash_gets_a_number() {
        let f = fixture("chain", "0.1.0", "v0.2.0", &pkg("0.2.0"));
        for v in ["0.2.0", "0.3.0", "0.4.0", "0.5.0"] {
            publish(&f.rel, &format!("v{v}"), &pkg(v), &good);
            run(&f).unwrap();
        }
        let mut left: Vec<String> = std::fs::read_dir(f.root.join("mod_backups")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        left.sort();
        assert_eq!(left, ["demo-mod-0.2.0", "demo-mod-0.3.0", "demo-mod-0.4.0"]);
        assert_eq!(read_manifest(&f.root.join("mods/demo-mod")).unwrap().version, "0.5.0");
        std::fs::create_dir_all(f.root.join("mod_backups/demo-mod-0.5.0")).unwrap();
        assert_eq!(unique_path(&f.root.join("mod_backups"), "demo-mod-0.5.0"), f.root.join("mod_backups/demo-mod-0.5.0 (2)"));
        let _ = std::fs::remove_dir_all(&f.root);
    }
}
