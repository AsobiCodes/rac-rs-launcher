//! "Back up your current version first": a playable copy of an install, made before an update touches anything.
//!
//! What a backup holds (found from what the game reads at run time, see `run_set`):
//! - the game exe and the DLLs next to it (`build\release\*.exe|dll`; no build caches, no `deps`, no generated sources),
//! - `Play.cmd`, `VERSION` and the small docs, `generated\play.cfg` (the disc path) + `generated\built_version.txt`,
//!   `oracle\stubs.txt` + `oracle\hand\hand_skip.txt` (read by the game and by Play.cmd),
//! - a COPY of the player folder (saves, `pc_options.txt`, log), of `profiles\` and of the small mods and `mods\active.txt`, so a newer
//!   version's saves can never break the old one,
//! - big texture packs (a mod folder of 32 MB or more) as HARD LINKS to the same files (no second copy on the disk) when the drive allows it;
//!   else they are left out and the dialog says so.
//!
//! `setup.ps1` and `Setup.cmd` are NOT copied: without them the old game exe cannot ask for an update on its own and nothing can update the backup.
//! The main install is only read. A failed or cancelled backup removes its own folder.
use crate::games::GameDef;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A mod folder this big or bigger is a "pack": linked, not copied.
pub const PACK_MIN_BYTES: u64 = 32 * 1024 * 1024;
/// Written last into a finished backup (a folder without it is a leftover of a cut run).
pub const MARKER: &str = "backup.txt";
pub const CANCELLED: &str = "The backup was cancelled.";
/// `Play.cmd` on Windows, `play.sh` on Linux (the start script of the backup).
fn top_files(windows: bool) -> [&'static str; 6] {
    ["VERSION", if windows { "Play.cmd" } else { "play.sh" }, "CHANGELOG.md", "README.md", "LICENSE", "NOTICE.md"]
}
const FIXED_FILES: [&str; 4] = ["generated/play.cfg", "generated/built_version.txt", "oracle/stubs.txt", "oracle/hand/hand_skip.txt"];
const MARGIN_BYTES: u64 = 64 * 1024 * 1024;

/// Tests only: while true the worker waits before each file (so a test can look at, or cancel, a running backup).
pub static TEST_HOLD: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Link,
}

/// One file of the backup (path relative to the install).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Op {
    pub rel: PathBuf,
    pub size: u64,
    pub mode: Mode,
    /// The mod folder (relative to the install) it belongs to, for a linked pack file.
    pub pack: Option<PathBuf>,
}

/// A big texture pack (mod folder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pack {
    pub name: String,
    pub bytes: u64,
    pub files: usize,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub root: PathBuf,
    /// The version being backed up.
    pub version: String,
    /// The backup folder (does not exist yet).
    pub dest: PathBuf,
    /// The library name of the backup.
    pub name: String,
    pub ops: Vec<Op>,
    /// Packs that are hard-linked (when `link_ok`) or left out.
    pub packs: Vec<Pack>,
    /// The drive of the backup folder can hard-link the packs from the install.
    pub link_ok: bool,
}

impl Plan {
    /// Bytes that are really written.
    pub fn copy_bytes(&self) -> u64 {
        self.ops.iter().filter(|o| o.mode == Mode::Copy).map(|o| o.size).sum()
    }

    /// Bytes shared through hard links (no extra space).
    pub fn shared_bytes(&self) -> u64 {
        if self.link_ok {
            self.packs.iter().map(|p| p.bytes).sum()
        } else {
            0
        }
    }

    /// Bytes of the packs that are left out of the backup.
    pub fn left_out_bytes(&self) -> u64 {
        if self.link_ok {
            0
        } else {
            self.packs.iter().map(|p| p.bytes).sum()
        }
    }

    /// Free space the backup needs (the copy plus a margin).
    pub fn needed_bytes(&self) -> u64 {
        let c = self.copy_bytes();
        c + c / 20 + MARGIN_BYTES
    }

    pub fn total_bytes(&self) -> u64 {
        self.ops.iter().map(|o| o.size).sum()
    }

    /// `Ok(())` when `free` bytes are enough, else the player-facing reason.
    pub fn space_check(&self, free: Option<u64>) -> Result<(), String> {
        match free {
            Some(f) if f < self.needed_bytes() => {
                Err(format!("Not enough free space on that drive: {} free, about {} needed.", crate::library::size_text(f), crate::library::size_text(self.needed_bytes())))
            }
            _ => Ok(()),
        }
    }
}

/// All files below `dir`, relative to `dir`, with sizes. Links (symlinks, junctions) are not followed.
pub fn walk(dir: &Path) -> Vec<(PathBuf, u64)> {
    fn go(base: &Path, rel: &Path, out: &mut Vec<(PathBuf, u64)>) {
        let Ok(rd) = std::fs::read_dir(base.join(rel)) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let Ok(t) = e.file_type() else { continue };
            let r = rel.join(e.file_name());
            if t.is_symlink() {
                continue;
            }
            if t.is_dir() {
                go(base, &r, out);
            } else if t.is_file() {
                out.push((r, e.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    let mut out = Vec::new();
    go(dir, Path::new(""), &mut out);
    out
}

fn size_of(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| m.len())
}

fn add_dir(ops: &mut Vec<Op>, root: &Path, rel: &Path, skip: &dyn Fn(&Path) -> bool) {
    for (r, size) in walk(&root.join(rel)) {
        if skip(&r) {
            continue;
        }
        ops.push(Op { rel: rel.join(r), size, mode: Mode::Copy, pack: None });
    }
}

/// A file beside the game exe that the game needs to run. Windows: every `.exe` and `.dll` (any case). Linux: the game binary itself
/// (the name of the exe) and shared libraries (`*.so`, `*.so.1.2`); cargo's `.d` files and the like stay out.
pub fn is_runtime_file(windows: bool, exe_name: &str, file: &str) -> bool {
    if windows {
        let ext = Path::new(file).extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        ext == "exe" || ext == "dll"
    } else {
        file == exe_name || file.ends_with(".so") || file.contains(".so.")
    }
}

/// The files the backup copies (no packs): see the module text. Pure reading, nothing is written.
pub fn run_set(root: &Path, game: &GameDef) -> Vec<Op> {
    let mut ops = Vec::new();
    let mut one = |rel: &str| {
        if let Some(size) = size_of(&root.join(rel)) {
            ops.push(Op { rel: PathBuf::from(rel), size, mode: Mode::Copy, pack: None });
        }
    };
    for f in top_files(cfg!(windows)).iter().chain(FIXED_FILES.iter()) {
        one(f);
    }
    // the exe and its DLLs (not the build caches in the folders below it)
    let exe = game.exe_path(root);
    if let Some(dir) = exe.parent() {
        let rel_dir = dir.strip_prefix(root).unwrap_or(Path::new("")).to_path_buf();
        let mut names: Vec<_> = std::fs::read_dir(dir).map(|rd| rd.flatten().filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).collect()).unwrap_or_default();
        names.sort_by_key(|e| e.file_name());
        let exe_name = exe.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        for e in names {
            let p = PathBuf::from(e.file_name());
            if is_runtime_file(cfg!(windows), &exe_name, &p.to_string_lossy()) {
                ops.push(Op { rel: rel_dir.join(&p), size: e.metadata().map(|m| m.len()).unwrap_or(0), mode: Mode::Copy, pack: None });
            }
        }
        if rel_dir != Path::new("") {
            add_dir(&mut ops, root, &rel_dir.join("saves"), &|_| false);
        }
    }
    // the player's own data: a copy, never a link
    let player = crate::play::saves_rel_of(root);
    add_dir(&mut ops, root, Path::new(&player), &|_| false);
    // a crash marker of the profiles belongs to the main install's run
    add_dir(&mut ops, root, Path::new(crate::profiles::DIR), &|r| r.file_name().map(|n| n == "running.txt").unwrap_or(false) && r.components().count() == 1);
    ops
}

/// The mods: small ones are copied, big ones are returned as packs (name, files with sizes).
fn mods_split(root: &Path, game: &GameDef, ops: &mut Vec<Op>) -> Vec<(Pack, Vec<(PathBuf, u64)>)> {
    let rel_buf = game.mods_rel();
    let rel = rel_buf.as_path();
    let dir = root.join(rel);
    let mut packs = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return packs };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() || name.ends_with(".tmp") {
            continue;
        }
        if t.is_file() {
            ops.push(Op { rel: rel.join(&name), size: e.metadata().map(|m| m.len()).unwrap_or(0), mode: Mode::Copy, pack: None });
        } else if t.is_dir() {
            let files = walk(&e.path());
            let bytes: u64 = files.iter().map(|f| f.1).sum();
            if bytes >= PACK_MIN_BYTES {
                packs.push((Pack { name, bytes, files: files.len() }, files));
            } else {
                for (r, size) in files {
                    ops.push(Op { rel: rel.join(&name).join(r), size, mode: Mode::Copy, pack: None });
                }
            }
        }
    }
    packs
}

/// Can `from` (a file in the install) be hard-linked into the folder `into_dir`? A probe link is made there and removed at once.
pub fn can_link(from: &Path, into_dir: &Path) -> bool {
    // one name per call: parallel plans (the tests, or two windows' threads) must not remove each other's probe
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let probe = into_dir.join(format!(".racl_link_probe_{}_{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let _ = std::fs::remove_file(&probe);
    let ok = std::fs::hard_link(from, &probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// The backup folder next to the install and the library name: `<install> (backup v0.3.1)` and `<install name> v0.3.1 (backup)`;
/// a second backup of the same version gets ` 2`, ` 3`, ...
pub fn dest_and_name(root: &Path, version: &str) -> Result<(PathBuf, String), String> {
    let parent = root.parent().filter(|p| !p.as_os_str().is_empty()).ok_or("The install folder has no parent folder to put the backup next to.")?;
    let tail = root.file_name().map(|n| n.to_string_lossy().to_string()).ok_or("The install folder has no name.")?;
    let mut n = 1;
    loop {
        let sfx = if n == 1 { String::new() } else { format!(" {n}") };
        let dest = parent.join(format!("{tail} (backup v{version}){sfx}"));
        if !dest.exists() {
            return Ok((dest, format!("{tail} v{version} (backup){sfx}")));
        }
        n += 1;
    }
}

/// Reads the install and makes the plan. Nothing is written except a probe link next to the install (removed at once).
pub fn plan(root: &Path, game: &GameDef, version: &str) -> Result<Plan, String> {
    if !game.exe_path(root).is_file() {
        return Err("The game is not built here, so there is nothing to back up.".to_string());
    }
    let (dest, name) = dest_and_name(root, version)?;
    let mut ops = run_set(root, game);
    let packs = mods_split(root, game, &mut ops);
    let parent = dest.parent().map(Path::to_path_buf).unwrap_or_default();
    let link_ok = match packs.first().and_then(|(p, f)| f.first().map(|(r, _)| root.join(game.mods_rel()).join(&p.name).join(r))) {
        Some(first) => can_link(&first, &parent),
        None => true,
    };
    let mut list = Vec::new();
    for (p, files) in packs {
        if link_ok {
            for (r, size) in files {
                ops.push(Op { rel: game.mods_rel().join(&p.name).join(r), size, mode: Mode::Link, pack: Some(game.mods_rel().join(&p.name)) });
            }
        }
        list.push(p);
    }
    Ok(Plan { root: root.to_path_buf(), version: version.to_string(), dest, name, ops, packs: list, link_ok })
}

/// What a running backup shows and what the worker reports.
#[derive(Default)]
pub struct State {
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub total_files: AtomicU64,
    pub done_files: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
    /// Packs the drive refused to link (left out).
    pub left_out: Mutex<Vec<String>>,
    pub result: Mutex<Option<Result<(), String>>>,
    /// The plan the worker runs (set at once by `start`, after reading the install by `start_fresh`).
    pub plan: Mutex<Option<Arc<Plan>>>,
}

impl State {
    pub fn plan(&self) -> Option<Arc<Plan>> {
        self.plan.lock().ok().and_then(|p| p.clone())
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// 0..=1 of the bytes.
    pub fn fraction(&self) -> f32 {
        let t = self.total_bytes.load(Ordering::Relaxed).max(1);
        (self.done_bytes.load(Ordering::Relaxed) as f32 / t as f32).clamp(0.0, 1.0)
    }

    /// None while it runs.
    pub fn take_result(&self) -> Option<Result<(), String>> {
        self.result.lock().ok().and_then(|mut r| r.take())
    }

    pub fn finished(&self) -> bool {
        self.result.lock().map(|r| r.is_some()).unwrap_or(true)
    }

    pub fn left_out_names(&self) -> Vec<String> {
        self.left_out.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

fn copy_file(src: &Path, dst: &Path, st: &State) -> Result<(), String> {
    use std::io::{Read, Write};
    let mut r = std::fs::File::open(src).map_err(|e| format!("Could not read {}: {e}", src.display()))?;
    let mut w = std::fs::File::create(dst).map_err(|e| format!("Could not write {}: {e}", dst.display()))?;
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if st.is_cancelled() {
            return Err(CANCELLED.to_string());
        }
        let n = r.read(&mut buf).map_err(|e| format!("Could not read {}: {e}", src.display()))?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(|e| format!("Could not write {}: {e}", dst.display()))?;
        st.done_bytes.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(())
}

/// The file whose `iso=` line names the disc (relative to the install).
const PLAY_CFG: &str = "generated/play.cfg";

/// `generated/play.cfg` as it must be written into a backup. setup.sh writes `iso=rom/x.iso` (relative to the install) for a disc inside
/// the game folder; in a backup that path would point into the backup folder, where `rom/` is not copied (4+ GB), so "Play old version"
/// could not find the disc. A relative `iso=` becomes the absolute path below the SOURCE install `root`; every other byte stays as it was
/// (other lines, line endings, an absolute or empty `iso=`, a file without an `iso=` line). Windows (`windows` = true) is returned
/// untouched: setup.ps1 writes absolute paths there, so the backup stays byte for byte the same as before.
pub fn play_cfg_for_backup(text: &str, root: &Path, windows: bool) -> String {
    if windows {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.split_inclusive('\n') {
        match line.strip_prefix("iso=") {
            Some(rest) => {
                let value = rest.trim_end_matches(['\r', '\n']);
                let tail = &rest[value.len()..];
                let path = value.trim();
                if !path.is_empty() && Path::new(path).is_relative() {
                    out.push_str("iso=");
                    out.push_str(&root.join(path).to_string_lossy());
                    out.push_str(tail);
                } else {
                    out.push_str(line);
                }
            }
            None => out.push_str(line),
        }
    }
    out
}

/// Copies `generated/play.cfg` into the backup with `play_cfg_for_backup`. Not UTF-8 or unreadable: the plain byte copy.
fn copy_play_cfg(src: &Path, dst: &Path, root: &Path, size: u64, st: &State) -> Result<(), String> {
    let new = std::fs::read(src).ok().and_then(|b| String::from_utf8(b).ok()).map(|t| play_cfg_for_backup(&t, root, cfg!(windows)));
    match new {
        Some(t) => {
            std::fs::write(dst, t).map_err(|e| format!("Could not write {}: {e}", dst.display()))?;
            st.done_bytes.fetch_add(size, Ordering::Relaxed);
            Ok(())
        }
        None => copy_file(src, dst, st),
    }
}

fn marker_text(plan: &Plan) -> String {
    format!(
        "This folder is a backup of RAC-RS {} made by the RAC-RS Launcher before an update ({}).\nIt was made from {}.\nThe launcher never updates it. To go back, open it in the Library and press Play.\nTo remove it, use Delete backup in the launcher's Library.\n",
        plan.version,
        crate::library::today(),
        plan.root.display()
    )
}

/// The worker body: makes the folder and the files. On any error the caller removes the folder.
pub fn execute(plan: &Plan, st: &State) -> Result<(), String> {
    st.total_bytes.store(plan.total_bytes(), Ordering::Relaxed);
    st.total_files.store(plan.ops.len() as u64, Ordering::Relaxed);
    std::fs::create_dir(&plan.dest).map_err(|e| format!("The backup folder could not be made ({}): {e}", plan.dest.display()))?;
    let mut dropped: Vec<PathBuf> = Vec::new();
    for op in &plan.ops {
        while TEST_HOLD.load(Ordering::SeqCst) && !st.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if st.is_cancelled() {
            return Err(CANCELLED.to_string());
        }
        if let Some(p) = &op.pack {
            if dropped.contains(p) {
                st.done_bytes.fetch_add(op.size, Ordering::Relaxed);
                st.done_files.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        }
        let (src, dst) = (plan.root.join(&op.rel), plan.dest.join(&op.rel));
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("Could not make {}: {e}", d.display()))?;
        }
        if let Ok(mut c) = st.current.lock() {
            *c = op.rel.to_string_lossy().to_string();
        }
        match op.mode {
            Mode::Copy if !cfg!(windows) && op.rel == Path::new(PLAY_CFG) => copy_play_cfg(&src, &dst, &plan.root, op.size, st)?,
            Mode::Copy => copy_file(&src, &dst, st)?,
            Mode::Link => {
                if std::fs::hard_link(&src, &dst).is_err() {
                    // the drive refused: this pack is left out (what was linked of it is removed again)
                    let dir = op.pack.clone().unwrap_or_default();
                    let _ = std::fs::remove_dir_all(plan.dest.join(&dir));
                    if let Ok(mut l) = st.left_out.lock() {
                        l.push(dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
                    }
                    dropped.push(dir);
                }
                st.done_bytes.fetch_add(op.size, Ordering::Relaxed);
            }
        }
        st.done_files.fetch_add(1, Ordering::Relaxed);
    }
    std::fs::write(plan.dest.join(MARKER), marker_text(plan)).map_err(|e| format!("Could not finish the backup: {e}"))?;
    Ok(())
}

/// Removes a backup folder that was cut off. Only call it for the folder `execute` made.
pub fn cleanup(dest: &Path) {
    let _ = std::fs::remove_dir_all(dest);
}

fn finish_run(st: &State, plan: &Plan) {
    let existed = plan.dest.exists();
    let r = if existed { Err(format!("The folder {} exists already.", plan.dest.display())) } else { execute(plan, st) };
    if r.is_err() && !existed {
        cleanup(&plan.dest);
    }
    if let Ok(mut slot) = st.result.lock() {
        *slot = Some(r);
    }
}

/// Runs `execute` on a thread; a failure or a cancel removes the folder before the result is published.
pub fn start(plan: Arc<Plan>) -> Arc<State> {
    let st = Arc::new(State::default());
    if let Ok(mut p) = st.plan.lock() {
        *p = Some(plan.clone());
    }
    let st2 = st.clone();
    std::thread::spawn(move || finish_run(&st2, &plan));
    st
}

/// Like `start`, but the worker reads the install itself first (the files may have changed since the dialog was drawn), checks the free
/// space again and then copies. Nothing is written when the check fails.
pub fn start_fresh(root: PathBuf, game: &'static GameDef, version: String) -> Arc<State> {
    let st = Arc::new(State::default());
    let st2 = st.clone();
    std::thread::spawn(move || {
        let fail = |m: String| {
            if let Ok(mut slot) = st2.result.lock() {
                *slot = Some(Err(m));
            }
        };
        let plan = match plan(&root, game, &version) {
            Ok(p) => Arc::new(p),
            Err(m) => return fail(m),
        };
        if let Err(m) = plan.space_check(plan.dest.parent().and_then(crate::library::free_space)) {
            return fail(m);
        }
        if let Ok(mut p) = st2.plan.lock() {
            *p = Some(plan.clone());
        }
        finish_run(&st2, &plan);
    });
    st
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::games::game_by_id;

    pub fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_bak_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub fn put(p: &Path, bytes: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![7u8; bytes]).unwrap();
    }

    /// A fake built install with everything a real one has, including build caches that must stay out.
    pub fn fake_install(root: &Path, version: &str) {
        put(&root.join("VERSION"), 6);
        std::fs::write(root.join("VERSION"), format!("{version}\n")).unwrap();
        put(&root.join(crate::library::SETUP_SCRIPT), 10);
        put(&root.join("Setup.cmd"), 10);
        std::fs::write(root.join(crate::games::PLAY_SCRIPT), "set \"RAC_PLAYER_DIR=%~dp0player\"\n").unwrap();
        put(&root.join("CHANGELOG.md"), 10);
        put(&root.join("Cargo.toml"), 10);
        put(&root.join("RAC-RS Launcher.exe"), 10);
        put(&root.join("library.json"), 10);
        put(&root.join("launcher.cfg"), 10);
        put(&root.join(exe_rel()), 3000);
        put(&root.join("build/release/dxcompiler.dll"), 2000);
        put(&root.join("build/release/libdxcompiler.so"), 2000);
        put(&root.join("build/release/libvk.so.1.3.2"), 2000);
        put(&root.join("build/release/rac-rs.d"), 500);
        put(&root.join("build/release/deps/big.rlib"), 5000);
        put(&root.join("build/release/incremental/x/y.bin"), 500);
        put(&root.join("generated/play.cfg"), 20);
        put(&root.join("generated/built_version.txt"), 6);
        put(&root.join("generated/pgo/prof.profdata"), 4000);
        put(&root.join("generated/logs/x.log"), 40);
        put(&root.join("oracle/stubs.txt"), 40);
        put(&root.join("oracle/hand/hand_skip.txt"), 40);
        put(&root.join("oracle/rec/l0/lib.rs"), 9000);
        put(&root.join("toolchain/cargo.exe"), 9000);
        put(&root.join("player/card.bin"), 800);
        put(&root.join("player/pc_options.txt"), 30);
        put(&root.join("player/history/a.bin"), 30);
        put(&root.join("profiles/current.txt"), 8);
        put(&root.join("profiles/default/profile.json"), 50);
        put(&root.join("profiles/default/pc_options.txt"), 30);
        put(&root.join("profiles/running.txt"), 8);
        put(&root.join("mods/active.txt"), 40);
        put(&root.join("mods/small/mod.json"), 80);
        put(&root.join("mods/small/a.png"), 1000);
        put(&root.join("mods/scratch.tmp/x.png"), 100);
    }

    /// The game exe of the fake install, relative to the root.
    pub fn exe_rel() -> String {
        format!("build/release/{}", crate::games::exe_name_for(cfg!(windows), "rac-rs.exe"))
    }

    fn rels(ops: &[Op]) -> Vec<String> {
        let mut v: Vec<String> = ops.iter().map(|o| o.rel.to_string_lossy().replace('\\', "/")).collect();
        v.sort();
        v
    }

    #[test]
    fn run_set_takes_what_the_game_reads_and_leaves_build_caches_out() {
        let d = scratch("runset");
        fake_install(&d, "0.3.1");
        let g = game_by_id("rac1").unwrap();
        let mut ops = run_set(&d, g);
        let mut packs = Vec::new();
        packs.extend(mods_split(&d, g, &mut ops));
        let r = rels(&ops);
        let (exe, runtime_lib, other_os_lib) =
            if cfg!(windows) { (exe_rel(), "build/release/dxcompiler.dll", "build/release/libvk.so.1.3.2") } else { (exe_rel(), "build/release/libvk.so.1.3.2", "build/release/dxcompiler.dll") };
        for need in [
            "VERSION",
            crate::games::PLAY_SCRIPT,
            "CHANGELOG.md",
            exe.as_str(),
            runtime_lib,
            "generated/play.cfg",
            "generated/built_version.txt",
            "oracle/stubs.txt",
            "oracle/hand/hand_skip.txt",
            "player/card.bin",
            "player/pc_options.txt",
            "player/history/a.bin",
            "profiles/current.txt",
            "profiles/default/profile.json",
            "mods/active.txt",
            "mods/small/mod.json",
            "mods/small/a.png",
        ] {
            assert!(r.contains(&need.to_string()), "missing {need}: {r:?}");
        }
        for no in [
            crate::library::SETUP_SCRIPT,
            other_os_lib,
            "Setup.cmd",
            "Cargo.toml",
            "RAC-RS Launcher.exe",
            "library.json",
            "launcher.cfg",
            "build/release/rac-rs.d",
            "build/release/deps/big.rlib",
            "build/release/incremental/x/y.bin",
            "generated/pgo/prof.profdata",
            "generated/logs/x.log",
            "oracle/rec/l0/lib.rs",
            "toolchain/cargo.exe",
            "profiles/running.txt",
            "mods/scratch.tmp/x.png",
        ] {
            assert!(!r.contains(&no.to_string()), "should be left out: {no}");
        }
        assert!(packs.is_empty());
        assert!(ops.iter().all(|o| o.mode == Mode::Copy));
    }

    #[test]
    fn a_run_set_without_optional_files_is_not_an_error() {
        let d = scratch("runset_min");
        put(&d.join(exe_rel()), 10);
        let g = game_by_id("rac1").unwrap();
        assert_eq!(rels(&run_set(&d, g)), vec![exe_rel()]);
    }

    #[test]
    fn runtime_files_per_system() {
        // Windows: exe and dll in any case; nothing else
        assert!(is_runtime_file(true, "rac-rs.exe", "rac-rs.exe") && is_runtime_file(true, "rac-rs.exe", "DXC.DLL") && is_runtime_file(true, "rac-rs.exe", "other.exe"));
        assert!(!is_runtime_file(true, "rac-rs.exe", "rac-rs.d") && !is_runtime_file(true, "rac-rs.exe", "libx.so"));
        // Linux: the game binary (the exe's own name) and shared libraries; cargo's side files and the pdb-like leftovers stay out
        assert!(is_runtime_file(false, "rac-rs", "rac-rs") && is_runtime_file(false, "rac-rs", "libvk.so") && is_runtime_file(false, "rac-rs", "libvk.so.1.3.2"));
        assert!(!is_runtime_file(false, "rac-rs", "rac-rs.d") && !is_runtime_file(false, "rac-rs", "rac-rs.pdb") && !is_runtime_file(false, "rac-rs", "other") && !is_runtime_file(false, "rac-rs", "dxcompiler.dll"));
    }

    #[test]
    fn a_linux_style_install_gets_play_sh_the_binary_and_the_player_folder() {
        // the start script of the backup follows the system, and only that one
        assert!(top_files(false).contains(&"play.sh") && !top_files(false).contains(&"Play.cmd"));
        assert!(top_files(true).contains(&"Play.cmd") && !top_files(true).contains(&"play.sh"));
        assert_eq!(top_files(cfg!(windows))[1], crate::games::PLAY_SCRIPT);
        // the walk over a Linux build folder keeps the binary and the libraries only
        let kept: Vec<&str> = ["rac-rs", "libx.so", "rac-rs.d", "other"].into_iter().filter(|n| is_runtime_file(false, "rac-rs", n)).collect();
        assert_eq!(kept, ["rac-rs", "libx.so"]);
        // the real walk, on a Linux host (run_set uses this system's file names)
        if cfg!(not(windows)) {
            let d = scratch("runset_linux");
            for f in ["VERSION", "play.sh", "build/release/rac-rs", "build/release/libx.so", "build/release/rac-rs.d", "setup.sh", "player/card.bin", "generated/play.cfg"] {
                put(&d.join(f), 10);
            }
            let r = rels(&run_set(&d, game_by_id("rac1").unwrap()));
            for need in ["VERSION", "play.sh", "build/release/rac-rs", "build/release/libx.so", "player/card.bin", "generated/play.cfg"] {
                assert!(r.iter().any(|x| x == need), "{need} missing from {r:?}");
            }
            assert!(!r.iter().any(|x| x == "build/release/rac-rs.d" || x == "setup.sh"), "{r:?}");
        }
    }

    #[test]
    fn packs_are_split_by_size_and_the_space_check_ignores_linked_packs() {
        let d = scratch("packs");
        fake_install(&d, "0.3.1");
        // a "big" pack: 33 MB in 3 files
        for i in 0..3 {
            put(&d.join(format!("mods/bigpack/replacements/{i}.png")), 11 * 1024 * 1024);
        }
        let g = game_by_id("rac1").unwrap();
        let p = plan(&d, g, "0.3.1").unwrap();
        assert!(p.link_ok);
        assert_eq!(p.packs.len(), 1);
        assert_eq!(p.packs[0].name, "bigpack");
        assert_eq!(p.packs[0].files, 3);
        assert_eq!(p.shared_bytes(), 33 * 1024 * 1024);
        assert_eq!(p.left_out_bytes(), 0);
        assert!(p.copy_bytes() < 1024 * 1024, "the packs are not in the bytes written: {}", p.copy_bytes());
        assert_eq!(p.ops.iter().filter(|o| o.mode == Mode::Link).count(), 3);
        // space: needs copy + 5% + 64 MB
        let need = p.needed_bytes();
        assert!(p.space_check(Some(need)).is_ok());
        let err = p.space_check(Some(need - 1)).unwrap_err();
        assert!(err.contains("Not enough free space"), "{err}");
        assert!(p.space_check(None).is_ok(), "unknown free space is not a reason to refuse");
        assert_eq!(p.dest, d.parent().unwrap().join(format!("{} (backup v0.3.1)", d.file_name().unwrap().to_string_lossy())));
        assert!(p.name.ends_with(" v0.3.1 (backup)"), "{}", p.name);
    }

    #[test]
    fn dest_names_do_not_collide() {
        let d = scratch("names");
        let (a, na) = dest_and_name(&d, "0.3.1").unwrap();
        std::fs::create_dir_all(&a).unwrap();
        let (b, nb) = dest_and_name(&d, "0.3.1").unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("(backup v0.3.1) 2") && nb.ends_with("(backup) 2"), "{b:?} {nb}");
        assert!(!na.ends_with('2'));
        let _ = std::fs::remove_dir_all(&a);
    }

    fn wait(st: &State) -> Result<(), String> {
        for _ in 0..2000 {
            if let Some(r) = st.take_result() {
                return r;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("the backup did not finish");
    }

    #[test]
    fn a_backup_copies_the_run_set_links_the_packs_and_changes_nothing_in_the_install() {
        let d = scratch("full");
        fake_install(&d, "0.3.1");
        for i in 0..2 {
            put(&d.join(format!("mods/bigpack/replacements/{i}.png")), 17 * 1024 * 1024);
        }
        let g = game_by_id("rac1").unwrap();
        let before = walk(&d);
        let p = Arc::new(plan(&d, g, "0.3.1").unwrap());
        let st = start(p.clone());
        wait(&st).unwrap();
        // the main install is untouched
        assert_eq!(walk(&d), before);
        // the backup holds the files, the marker, no setup.ps1
        let b = &p.dest;
        assert!(b.join(MARKER).is_file());
        assert!(b.join(exe_rel()).is_file() && b.join("player/card.bin").is_file() && b.join("generated/play.cfg").is_file());
        assert!(!b.join(crate::library::SETUP_SCRIPT).exists());
        assert_eq!(std::fs::read_to_string(b.join("VERSION")).unwrap().trim(), "0.3.1");
        assert!(crate::games::is_installed(b, g), "the backup counts as an installed game");
        // a pack file is the same file on the disk (writing through one name shows in the other); a player file is a separate copy
        let (src, dst) = (d.join("mods/bigpack/replacements/0.png"), b.join("mods/bigpack/replacements/0.png"));
        assert!(dst.is_file());
        std::fs::OpenOptions::new().write(true).open(&src).unwrap().set_len(17 * 1024 * 1024 + 5).unwrap();
        assert_eq!(std::fs::metadata(&dst).unwrap().len(), 17 * 1024 * 1024 + 5, "hard link: one file, two names");
        std::fs::write(d.join("player/card.bin"), b"changed later").unwrap();
        assert_eq!(std::fs::read(b.join("player/card.bin")).unwrap().len(), 800, "the saves are a real copy");
        assert!(st.left_out_names().is_empty());
        cleanup(b);
        assert!(src.is_file(), "removing the backup leaves the install's pack");
    }

    #[test]
    fn without_hard_links_the_packs_are_left_out_and_said_so() {
        let d = scratch("nolink");
        fake_install(&d, "0.3.1");
        put(&d.join("mods/bigpack/replacements/0.png"), 33 * 1024 * 1024);
        let g = game_by_id("rac1").unwrap();
        let mut p = plan(&d, g, "0.3.1").unwrap();
        // pretend the drive cannot link: the plan as `plan` makes it for such a drive
        p.link_ok = false;
        p.ops.retain(|o| o.mode == Mode::Copy);
        assert_eq!(p.shared_bytes(), 0);
        assert_eq!(p.left_out_bytes(), 33 * 1024 * 1024);
        let p = Arc::new(p);
        wait(&start(p.clone())).unwrap();
        assert!(!p.dest.join("mods/bigpack").exists());
        assert!(p.dest.join("mods/small/a.png").is_file());
        cleanup(&p.dest);
    }

    #[test]
    fn a_pack_the_drive_refuses_to_link_is_dropped_but_the_backup_goes_on() {
        let d = scratch("linkfail");
        fake_install(&d, "0.3.1");
        put(&d.join("mods/bigpack/replacements/0.png"), 33 * 1024 * 1024);
        let g = game_by_id("rac1").unwrap();
        let mut p = plan(&d, g, "0.3.1").unwrap();
        // a link op whose source vanished stands for a link the drive refused
        for o in p.ops.iter_mut().filter(|o| o.mode == Mode::Link) {
            o.rel = PathBuf::from("mods/bigpack/replacements/missing.png");
        }
        let p = Arc::new(p);
        let st = start(p.clone());
        wait(&st).unwrap();
        assert_eq!(st.left_out_names(), vec!["bigpack".to_string()]);
        assert!(!p.dest.join("mods/bigpack").exists());
        assert!(p.dest.join(MARKER).is_file());
        cleanup(&p.dest);
    }

    #[test]
    fn a_failed_copy_removes_its_folder_and_changes_nothing() {
        let d = scratch("fail");
        fake_install(&d, "0.3.1");
        let g = game_by_id("rac1").unwrap();
        let mut p = plan(&d, g, "0.3.1").unwrap();
        // a file that disappeared after the plan was made
        p.ops.push(Op { rel: PathBuf::from("player/vanished.bin"), size: 5, mode: Mode::Copy, pack: None });
        let before = walk(&d);
        let p = Arc::new(p);
        let err = wait(&start(p.clone())).unwrap_err();
        assert!(err.contains("vanished.bin"), "{err}");
        assert!(!p.dest.exists(), "the partial folder is removed");
        assert_eq!(walk(&d), before);
    }

    #[test]
    fn cancel_removes_the_partial_folder() {
        let d = scratch("cancel");
        fake_install(&d, "0.3.1");
        put(&d.join("player/big.bin"), 8 * 1024 * 1024);
        let g = game_by_id("rac1").unwrap();
        let p = Arc::new(plan(&d, g, "0.3.1").unwrap());
        let st = Arc::new(State::default());
        st.cancel();
        let r = execute(&p, &st);
        assert_eq!(r.unwrap_err(), CANCELLED);
        cleanup(&p.dest);
        assert!(!p.dest.exists());
        // and through the thread: a cancel set before the worker looks ends the same way
        let p2 = Arc::new(plan(&d, g, "0.3.1").unwrap());
        let st2 = start(p2.clone());
        st2.cancel();
        let r2 = wait(&st2);
        if let Err(m) = r2 {
            assert_eq!(m, CANCELLED);
            assert!(!p2.dest.exists());
        } else {
            cleanup(&p2.dest);
        }
    }

    #[test]
    fn an_existing_folder_is_never_overwritten() {
        let d = scratch("exists");
        fake_install(&d, "0.3.1");
        let g = game_by_id("rac1").unwrap();
        let p = Arc::new(plan(&d, g, "0.3.1").unwrap());
        std::fs::create_dir_all(&p.dest).unwrap();
        std::fs::write(p.dest.join("mine.txt"), "x").unwrap();
        let err = wait(&start(p.clone())).unwrap_err();
        assert!(err.contains("exists already"));
        assert!(p.dest.join("mine.txt").is_file(), "somebody else's folder stays");
        cleanup(&p.dest);
    }

    #[test]
    fn can_link_works_on_one_drive_and_leaves_no_probe() {
        let d = scratch("probe");
        put(&d.join("a/f.bin"), 4);
        assert!(can_link(&d.join("a/f.bin"), &d));
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".racl_link_probe")));
        assert!(!can_link(&d.join("a/missing.bin"), &d));
    }

    /// The backup's play.cfg (review SF-B2): a relative `iso=rom/x.iso` of a Linux install must name the disc of the SOURCE install, because
    /// `rom/` is not copied and the backup folder has no disc.
    #[test]
    fn a_relative_iso_in_the_backups_play_cfg_becomes_the_source_installs_absolute_path() {
        let root = std::env::temp_dir().join("racl_bak_src_install");
        let cfg = "iso=rom/x.iso\nskip_hash=1\n";
        let out = play_cfg_for_backup(cfg, &root, false);
        assert_eq!(out, format!("iso={}\nskip_hash=1\n", root.join("rom/x.iso").display()));
        assert!(Path::new(out.lines().next().unwrap().strip_prefix("iso=").unwrap()).is_absolute());
        // line endings and the other lines are kept exactly (CRLF, no final newline, a space before the path)
        let out = play_cfg_for_backup("a=1\r\niso=rom/y.iso\r\nb=2", &root, false);
        assert_eq!(out, format!("a=1\r\niso={}\r\nb=2", root.join("rom/y.iso").display()));
        let out = play_cfg_for_backup("iso=rom/z.iso", &root, false);
        assert_eq!(out, format!("iso={}", root.join("rom/z.iso").display()));
        let out = play_cfg_for_backup("iso= rom/w.iso \n", &root, false);
        assert_eq!(out, format!("iso={}\n", root.join("rom/w.iso").display()));
        // the first line of a file with other keys first, and a key that only ends in `iso=` is not the disc
        let out = play_cfg_for_backup("my_iso=rom/q.iso\n", &root, false);
        assert_eq!(out, "my_iso=rom/q.iso\n");
    }

    #[test]
    fn an_absolute_empty_or_missing_iso_line_stays_byte_for_byte() {
        let root = std::env::temp_dir().join("racl_bak_src_install");
        let abs = std::env::temp_dir().join("elsewhere").join("rc.iso");
        for cfg in [format!("iso={}\n", abs.display()), format!("x=1\r\niso={}\r\ny=2\r\n", abs.display()), "iso=\n".to_string(), "iso=".to_string(), "skip_hash=1\nlang=en\n".to_string(), String::new(), "\u{7}\u{7}\u{7}".to_string()] {
            assert_eq!(play_cfg_for_backup(&cfg, &root, false), cfg, "{cfg:?}");
        }
    }

    #[test]
    fn windows_play_cfg_is_never_changed() {
        let root = std::env::temp_dir().join("racl_bak_src_install");
        // setup.ps1 writes absolute paths; even a hand-edited relative one is copied as it is
        for cfg in ["iso=D:\\Games\\rc.iso\r\n", "iso=rom\\x.iso\r\n", "iso=rom/x.iso\n", "a=1\r\n"] {
            assert_eq!(play_cfg_for_backup(cfg, &root, true), cfg);
        }
    }

    #[test]
    fn a_backup_run_writes_the_absolute_disc_path_on_linux_and_the_same_bytes_on_windows() {
        let d = scratch("playcfg");
        fake_install(&d, "0.4.1");
        let g = game_by_id("rac1").unwrap();
        let src_cfg: &[u8] = if cfg!(windows) { b"iso=D:\\Games\\rc.iso\r\n" } else { b"iso=rom/x.iso\n" };
        std::fs::write(d.join("generated/play.cfg"), src_cfg).unwrap();
        let p = Arc::new(plan(&d, g, "0.4.1").unwrap());
        wait(&start(p.clone())).unwrap();
        let got = std::fs::read(p.dest.join("generated/play.cfg")).unwrap();
        if cfg!(windows) {
            assert_eq!(got, src_cfg);
        } else {
            assert_eq!(String::from_utf8(got).unwrap(), format!("iso={}\n", d.join("rom/x.iso").display()));
            // the backup finds the disc of the install it was made from, not a missing rom/ below itself
            assert_eq!(crate::games::installed_iso(&p.dest), Some(d.join("rom/x.iso")));
        }
        // the install's own play.cfg is untouched
        assert_eq!(std::fs::read(d.join("generated/play.cfg")).unwrap(), src_cfg);
        cleanup(&p.dest);
    }
}
